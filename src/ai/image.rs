use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use super::{
    progress::{ImportCancellation, ImportEvent, ImportEventReporter, ImportProgress, ImportStage},
    source::ImageSource,
};
use crate::diagnostics;

const MAX_IMAGES_PER_IMPORT: usize = 120;
const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_REQUEST_ATTEMPTS: usize = 3;
static NEXT_VISION_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct VisionClient {
    http: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
}

impl VisionClient {
    pub fn new(api_base: String, api_key: String, model: String) -> Result<Self> {
        let api_base = api_base.trim().trim_end_matches('/').to_string();
        let api_key = api_key.trim().to_string();
        let model = model.trim().to_string();
        if !(api_base.starts_with("https://") || api_base.starts_with("http://")) {
            return Err(anyhow!("图像接口地址必须以 http:// 或 https:// 开头"));
        }
        if api_key.is_empty() || model.is_empty() {
            return Err(anyhow!("图像接口密钥和模型不能为空"));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            http,
            api_base,
            api_key,
            model: model.chars().take(120).collect(),
        })
    }

    async fn describe_image_json_for(
        &self,
        filename: &str,
        mime_type: &str,
        bytes: &[u8],
        cancellation: &ImportCancellation,
    ) -> Result<serde_json::Value> {
        if bytes.is_empty() {
            return Err(anyhow!("图片文件为空: {filename}"));
        }
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(anyhow!(
                "图片 {} 超过 {} MB 限制",
                filename,
                MAX_IMAGE_BYTES / (1024 * 1024)
            ));
        }
        let request_id = NEXT_VISION_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        let url = format!("{}/chat/completions", self.api_base);
        let data_url = format!("data:{mime_type};base64,{}", BASE64_STANDARD.encode(bytes));
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": "你是学习资料图像描述器。只根据图片可见内容输出 JSON，不要猜测看不清的文字。字段必须是 description、visible_text、semantic_role、confidence。description 用简洁中文描述图片结构和含义，visible_text 是图片中确实可读的文字数组，semantic_role 只能是 diagram、chart、table、photo、screenshot、formula、illustration、unknown 之一，confidence 是 0 到 1 的数字。"
                },
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": format!("请描述学习材料中的图片：{filename}") },
                        { "type": "image_url", "image_url": { "url": data_url } }
                    ]
                }
            ],
            "max_tokens": 4_096,
            "response_format": { "type": "json_object" }
        });
        diagnostics::info(
            "ai.image.request.started",
            "Vision image description started",
            serde_json::json!({
                "request_id": request_id,
                "model": self.model,
                "filename": filename,
                "mime_type": mime_type,
                "bytes": bytes.len(),
            }),
        );

        let mut response = None;
        for attempt in 1..=MAX_REQUEST_ATTEMPTS {
            cancellation.ensure_active()?;
            let request = self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .timeout(Duration::from_secs(120))
                .json(&body)
                .send();
            let result = tokio::select! {
                response = request => response,
                _ = cancellation.cancelled() => return Err(anyhow!("导入任务已取消")),
            };
            match result {
                Ok(candidate)
                    if retryable_status(candidate.status()) && attempt < MAX_REQUEST_ATTEMPTS =>
                {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(250 * attempt as u64)) => {},
                        _ = cancellation.cancelled() => return Err(anyhow!("导入任务已取消")),
                    }
                }
                Ok(candidate) => {
                    response = Some(candidate);
                    break;
                }
                Err(error) if attempt < MAX_REQUEST_ATTEMPTS => {
                    diagnostics::warn(
                        "ai.image.request.retrying",
                        "Vision request failed; retrying",
                        serde_json::json!({
                            "request_id": request_id,
                            "attempt": attempt,
                            "error": format!("{error:#}"),
                        }),
                    );
                }
                Err(error) => return Err(anyhow!("图像识别请求发送失败: {error}")),
            }
        }
        let response = response.ok_or_else(|| anyhow!("图像识别请求没有返回结果"))?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(anyhow!("图像识别响应超过 2 MB 限制"));
        }
        if !status.is_success() {
            return Err(anyhow!("图像识别接口返回状态 {status}"));
        }
        let payload: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| anyhow!("图像识别接口响应不是合法 JSON: {error}"))?;
        let content = message_content(&payload)
            .ok_or_else(|| anyhow!("图像识别响应缺少 choices[0].message.content"))?;
        let value = parse_json_object(&content)?;
        diagnostics::info(
            "ai.image.request.completed",
            "Vision image description completed",
            serde_json::json!({
                "request_id": request_id,
                "elapsed_ms": started.elapsed().as_millis(),
                "content_chars": content.chars().count(),
            }),
        );
        Ok(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageDescription {
    pub description: String,
    #[serde(default)]
    pub visible_text: Vec<String>,
    pub semantic_role: String,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct DescribedImage {
    pub source: ImageSource,
    pub description: ImageDescription,
}

pub async fn describe_images(
    client: &VisionClient,
    images: &[ImageSource],
    progress: &ImportEventReporter,
    cancellation: &ImportCancellation,
) -> Result<Vec<DescribedImage>> {
    if images.len() > MAX_IMAGES_PER_IMPORT {
        return Err(anyhow!(
            "本次扫描发现 {} 张图片，超过单次上限 {}，请缩小目录范围",
            images.len(),
            MAX_IMAGES_PER_IMPORT
        ));
    }
    let mut described = Vec::with_capacity(images.len());
    for (index, image) in images.iter().enumerate() {
        cancellation.ensure_active()?;
        progress(ImportEvent::Stage(ImportProgress::stage(
            ImportStage::DescribingImages,
            format!(
                "正在识别图片 {}/{}：{}",
                index + 1,
                images.len(),
                image.relative_path.display()
            ),
        )));
        let bytes = fs::read(&image.path)
            .map_err(|error| anyhow!("读取图片 {} 失败: {error}", image.path.display()))?;
        let value = client
            .describe_image_json_for(
                &image.relative_path.to_string_lossy(),
                mime_type_for(&image.path),
                &bytes,
                cancellation,
            )
            .await?;
        let mut description: ImageDescription = serde_json::from_value(value)
            .map_err(|error| anyhow!("图片描述响应格式不对: {error}"))?;
        validate_description(&mut description)?;
        described.push(DescribedImage {
            source: image.clone(),
            description,
        });
    }
    Ok(described)
}

pub fn append_image_context(text: &mut String, images: &[DescribedImage]) {
    if images.is_empty() {
        return;
    }
    text.push_str("\n\n# 本地图片与视觉描述\n");
    for image in images {
        let name = image.source.relative_path.to_string_lossy();
        // The UI consumes this private marker as a local image block. Keeping
        // the source path here avoids routing file:// through an HTTP loader.
        let path = image.source.path.to_string_lossy();
        text.push_str(&format!(
            "\n\n<!-- ruiz-image: {path} -->\n\n> 图片来源：{name}\n> 图像类型：{}\n> 图像描述：{}\n",
            image.description.semantic_role,
            image.description.description.trim(),
        ));
        if !image.description.visible_text.is_empty() {
            text.push_str("> 可见文字：");
            text.push_str(&image.description.visible_text.join("；"));
            text.push('\n');
        }
    }
}

fn retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_EARLY
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn message_content(payload: &serde_json::Value) -> Option<String> {
    let content = &payload["choices"][0]["message"]["content"];
    if let Some(content) = content.as_str() {
        return Some(content.to_string());
    }
    content.as_array().map(|parts| {
        parts
            .iter()
            .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("")
    })
}

fn parse_json_object(content: &str) -> Result<serde_json::Value> {
    if let Ok(value) = serde_json::from_str(content) {
        return Ok(value);
    }
    let trimmed = content
        .trim()
        .strip_prefix("```json")
        .or_else(|| content.trim().strip_prefix("```"))
        .unwrap_or(content)
        .trim()
        .strip_suffix("```")
        .unwrap_or(content.trim())
        .trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    let start = content.find('{');
    let end = content.rfind('}');
    match (start, end) {
        (Some(start), Some(end)) if start < end => serde_json::from_str(&content[start..=end])
            .map_err(|error| anyhow!("图像模型返回的不是合法 JSON: {error}")),
        _ => Err(anyhow!("图像模型没有返回 JSON 对象")),
    }
}

fn validate_description(description: &mut ImageDescription) -> Result<()> {
    description.description = description.description.trim().chars().take(2_000).collect();
    description.visible_text = description
        .visible_text
        .iter()
        .filter_map(|text| {
            let text = text.trim();
            (!text.is_empty()).then(|| text.chars().take(500).collect())
        })
        .take(100)
        .collect();
    if description.description.is_empty() {
        return Err(anyhow!("图像模型返回了空描述"));
    }
    if !matches!(
        description.semantic_role.as_str(),
        "diagram"
            | "chart"
            | "table"
            | "photo"
            | "screenshot"
            | "formula"
            | "illustration"
            | "unknown"
    ) {
        description.semantic_role = "unknown".into();
    }
    if !description.confidence.is_finite() {
        description.confidence = 0.0;
    }
    description.confidence = description.confidence.clamp(0.0, 1.0);
    Ok(())
}

fn mime_type_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => "image/png",
    }
}

#[cfg(test)]
mod tests {
    use super::{ImageDescription, validate_description};

    #[test]
    fn validates_and_normalizes_image_description() {
        let mut description = ImageDescription {
            description: "  一张流程图  ".into(),
            visible_text: vec![" A ".into(), "".into()],
            semantic_role: "unexpected".into(),
            confidence: 2.0,
        };
        validate_description(&mut description).unwrap();
        assert_eq!(description.description, "一张流程图");
        assert_eq!(description.visible_text, vec!["A"]);
        assert_eq!(description.semantic_role, "unknown");
        assert_eq!(description.confidence, 1.0);
    }
}
