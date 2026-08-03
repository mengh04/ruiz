use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use anyhow::{Result, anyhow};
use reqwest::{Response, StatusCode};

use crate::diagnostics;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
pub const DEEPSEEK_API_BASE: &str = "https://api.deepseek.com";
pub const DEEPSEEK_FLASH_MODEL: &str = "deepseek-v4-flash";
pub const DEEPSEEK_PRO_MODEL: &str = "deepseek-v4-pro";
const DEEPSEEK_MAX_OUTPUT_TOKENS: u32 = 384_000;

/// DeepSeek Chat Completions 客户端。
#[derive(Clone)]
pub struct ChatClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
}

struct ResponseText {
    status: StatusCode,
    content_type: String,
    body: String,
}

impl ChatClient {
    pub fn new(api_key: String, model: String) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("构建 HTTP client 失败");
        let model = match model.as_str() {
            DEEPSEEK_PRO_MODEL => DEEPSEEK_PRO_MODEL,
            _ => DEEPSEEK_FLASH_MODEL,
        }
        .to_string();
        Self {
            http,
            api_key,
            model,
        }
    }

    /// 发送一次具名 AI 请求并返回模型生成的 JSON 对象。
    /// 每个阶段和请求编号都会写入诊断日志。
    pub async fn chat_json_for(
        &self,
        operation: &str,
        system: &str,
        user: &str,
    ) -> Result<serde_json::Value> {
        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        let url = format!("{DEEPSEEK_API_BASE}/chat/completions");
        let max_tokens = max_output_tokens_for(operation);
        let mut body = json_request_body(&self.model, system, user, max_tokens);

        diagnostics::info(
            "ai.request.started",
            "AI request started",
            serde_json::json!({
                "request_id": request_id,
                "operation": operation,
                "model": self.model,
                "endpoint": url,
                "system_chars": system.chars().count(),
                "user_chars": user.chars().count(),
                "max_tokens": max_tokens,
                "thinking": "enabled",
                "reasoning_effort": "high",
                "response_format": true,
            }),
        );

        let response = self
            .send_request(request_id, operation, &url, &body)
            .await?;
        let mut response = read_response(request_id, operation, response).await?;

        let mut content = response_content(request_id, operation, &response)?;
        if content.trim().is_empty() {
            diagnostics::warn(
                "ai.model.empty_content_retry",
                "AI returned empty JSON-mode content; retrying once",
                serde_json::json!({
                    "request_id": request_id,
                    "operation": operation,
                }),
            );
            body["messages"][0]["content"] = serde_json::json!(format!(
                "{system}\n\n# JSON 重试约束\n上一次调用返回了空 content。请不要输出空白、思考过程或解释，立即按照上方 JSON 样例输出一个完整合法的 JSON 对象。"
            ));
            let retry = self
                .send_request(request_id, operation, &url, &body)
                .await?;
            response = read_response(request_id, operation, retry).await?;
            content = response_content(request_id, operation, &response)?;
        }
        if content.trim().is_empty() {
            diagnostics::error(
                "ai.model.output_empty",
                "AI returned empty content twice",
                serde_json::json!({
                    "request_id": request_id,
                    "operation": operation,
                }),
            );
            return Err(anyhow!("AI 连续两次返回空内容（请求 #{request_id}）"));
        }

        diagnostics::info(
            "ai.model.output",
            "AI model output received",
            serde_json::json!({
                "request_id": request_id,
                "operation": operation,
                "content_chars": content.chars().count(),
                "content": diagnostics::truncate(&content),
            }),
        );
        let parsed = parse_json_content(&content).map_err(|error| {
            diagnostics::error(
                "ai.model.output_invalid",
                "AI model output is not valid JSON",
                serde_json::json!({
                    "request_id": request_id,
                    "operation": operation,
                    "error": format!("{error:#}"),
                    "content": diagnostics::truncate(&content),
                }),
            );
            anyhow!("模型输出不是合法 JSON（请求 #{request_id}）：{error}")
        })?;
        diagnostics::info(
            "ai.request.completed",
            "AI request completed",
            serde_json::json!({
                "request_id": request_id,
                "operation": operation,
                "elapsed_ms": started.elapsed().as_millis(),
            }),
        );
        Ok(parsed)
    }

    async fn send_request(
        &self,
        request_id: u64,
        operation: &str,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<Response> {
        self.http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(body)
            .send()
            .await
            .map_err(|error| {
                diagnostics::error(
                    "ai.request.send_failed",
                    "Failed to send AI request",
                    serde_json::json!({
                        "request_id": request_id,
                        "operation": operation,
                        "error": format!("{error:#}"),
                    }),
                );
                anyhow!("AI 请求发送失败（请求 #{request_id}）：{error}")
            })
    }
}

fn json_request_body(model: &str, system: &str, user: &str, max_tokens: u32) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "max_tokens": max_tokens,
        "thinking": { "type": "enabled" },
        "reasoning_effort": "high",
        "response_format": { "type": "json_object" },
    })
}

fn max_output_tokens_for(operation: &str) -> u32 {
    if operation.starts_with("import.organize") || operation.starts_with("answer.judge") {
        32_768
    } else {
        DEEPSEEK_MAX_OUTPUT_TOKENS
    }
}

fn response_content(request_id: u64, operation: &str, response: &ResponseText) -> Result<String> {
    if !response.status.is_success() {
        diagnostics::error(
            "ai.request.http_failed",
            "AI provider returned a non-success status",
            serde_json::json!({
                "request_id": request_id,
                "operation": operation,
                "status": response.status.as_u16(),
                "content_type": response.content_type,
                "response_body": diagnostics::truncate(&response.body),
            }),
        );
        return Err(anyhow!(
            "AI API 请求失败（请求 #{request_id}，状态 {}）：{}",
            response.status,
            diagnostics::truncate(&response.body)
        ));
    }

    let payload: serde_json::Value = serde_json::from_str(&response.body).map_err(|error| {
        diagnostics::error(
            "ai.response.envelope_invalid",
            "AI response envelope is not valid JSON",
            serde_json::json!({
                "request_id": request_id,
                "operation": operation,
                "status": response.status.as_u16(),
                "content_type": response.content_type,
                "error": error.to_string(),
                "response_body": diagnostics::truncate(&response.body),
            }),
        );
        anyhow!("AI API 响应不是合法 JSON（请求 #{request_id}）：{error}")
    })?;
    let choice = &payload["choices"][0];
    let reasoning_chars = text_content(&choice["message"]["reasoning_content"])
        .map(|content| content.chars().count())
        .unwrap_or_default();
    diagnostics::info(
        "ai.response.metadata",
        "DeepSeek response metadata received",
        serde_json::json!({
            "request_id": request_id,
            "operation": operation,
            "finish_reason": choice["finish_reason"].as_str().unwrap_or("unknown"),
            "reasoning_chars": reasoning_chars,
            "prompt_tokens": payload["usage"]["prompt_tokens"].as_u64(),
            "completion_tokens": payload["usage"]["completion_tokens"].as_u64(),
            "total_tokens": payload["usage"]["total_tokens"].as_u64(),
        }),
    );
    extract_message_content(&payload).ok_or_else(|| {
        diagnostics::error(
            "ai.response.content_missing",
            "AI response is missing choices[0].message.content",
            serde_json::json!({
                "request_id": request_id,
                "operation": operation,
                "response_body": diagnostics::truncate(&response.body),
            }),
        );
        anyhow!("AI 响应缺少消息正文（请求 #{request_id}）")
    })
}

async fn read_response(
    request_id: u64,
    operation: &str,
    response: Response,
) -> Result<ResponseText> {
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_string();
    let content_length = response.content_length();
    let content_encoding = response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("none")
        .to_string();
    let provider_request_id = ["x-request-id", "request-id", "cf-ray"]
        .iter()
        .find_map(|name| {
            response
                .headers()
                .get(*name)
                .and_then(|value| value.to_str().ok())
        })
        .unwrap_or("unknown")
        .to_string();
    let body = response.text().await.map_err(|error| {
        diagnostics::error(
            "ai.response.body_read_failed",
            "Failed to read AI response body",
            serde_json::json!({
                "request_id": request_id,
                "operation": operation,
                "status": status.as_u16(),
                "content_type": content_type,
                "content_length": content_length,
                "content_encoding": content_encoding,
                "provider_request_id": provider_request_id,
                "error": format!("{error:#}"),
            }),
        );
        anyhow!("读取 AI 响应正文失败（请求 #{request_id}，状态 {status}）：{error}")
    })?;
    diagnostics::info(
        "ai.response.received",
        "AI HTTP response received",
        serde_json::json!({
            "request_id": request_id,
            "operation": operation,
            "status": status.as_u16(),
            "content_type": content_type,
            "content_length": content_length,
            "content_encoding": content_encoding,
            "provider_request_id": provider_request_id,
            "body_chars": body.chars().count(),
        }),
    );
    Ok(ResponseText {
        status,
        content_type,
        body,
    })
}

fn extract_message_content(payload: &serde_json::Value) -> Option<String> {
    text_content(&payload["choices"][0]["message"]["content"])
}

fn text_content(content: &serde_json::Value) -> Option<String> {
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

fn parse_json_content(content: &str) -> Result<serde_json::Value> {
    if let Ok(value) = serde_json::from_str(content) {
        return Ok(value);
    }
    let without_prefix = content
        .trim()
        .strip_prefix("```json")
        .or_else(|| content.trim().strip_prefix("```JSON"))
        .or_else(|| content.trim().strip_prefix("```"))
        .unwrap_or(content)
        .trim();
    let trimmed = without_prefix
        .strip_suffix("```")
        .unwrap_or(without_prefix)
        .trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    let start = content.find('{');
    let end = content.rfind('}');
    match (start, end) {
        (Some(start), Some(end)) if start < end => serde_json::from_str(&content[start..=end])
            .map_err(|error| anyhow!("AI 返回的不是合法 JSON: {error}")),
        _ => Err(anyhow!("AI 返回的不是合法 JSON 对象")),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEEPSEEK_FLASH_MODEL, extract_message_content, json_request_body, max_output_tokens_for,
        parse_json_content,
    };

    #[test]
    fn parses_plain_and_fenced_json() {
        assert_eq!(parse_json_content(r#"{"ok":true}"#).unwrap()["ok"], true);
        assert_eq!(
            parse_json_content("```json\n{\"ok\": true}\n```").unwrap()["ok"],
            true
        );
        assert_eq!(
            parse_json_content("结果如下：\n{\"ok\": true}\n完成").unwrap()["ok"],
            true
        );
    }

    #[test]
    fn accepts_text_part_message_content() {
        let payload = serde_json::json!({
            "choices": [{ "message": { "content": [{ "type": "text", "text": "{}" }] } }]
        });
        assert_eq!(extract_message_content(&payload).as_deref(), Some("{}"));
    }

    #[test]
    fn deepseek_output_budgets_match_workflow_size() {
        assert_eq!(max_output_tokens_for("answer.judge"), 32_768);
        assert_eq!(max_output_tokens_for("plan.extract"), 384_000);
        assert_eq!(max_output_tokens_for("plan.reconcile"), 384_000);
        assert_eq!(max_output_tokens_for("import.clean"), 384_000);
        assert_eq!(max_output_tokens_for("questions.generate"), 384_000);
    }

    #[test]
    fn all_deepseek_json_requests_use_high_effort_thinking() {
        let body = json_request_body(DEEPSEEK_FLASH_MODEL, "输出 JSON", "{}", 65_536);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["max_tokens"], 65_536);
        assert!(body["temperature"].is_null());
    }
}
