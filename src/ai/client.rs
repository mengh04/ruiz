use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use anyhow::{Result, anyhow};
use reqwest::{Response, StatusCode};

use super::progress::ImportCancellation;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Thinking(String),
    Content(String),
}

struct StreamOutput {
    content: String,
    reasoning_chars: usize,
    finish_reason: String,
    usage: serde_json::Value,
}

enum SseFrame {
    Data(serde_json::Value),
    Done,
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
        let reasoning_effort = reasoning_effort_for(operation);
        let mut body = json_request_body(&self.model, system, user, max_tokens, reasoning_effort);

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
                "reasoning_effort": reasoning_effort,
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

    /// 流式发送具名 JSON 请求，实时转发模型思考和答案增量。
    /// 最终答案仍会完整校验为 JSON 后返回。
    pub async fn chat_json_stream_for(
        &self,
        operation: &str,
        system: &str,
        user: &str,
        on_event: &(dyn Fn(StreamEvent) + Send + Sync),
        cancellation: Option<&ImportCancellation>,
    ) -> Result<serde_json::Value> {
        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        let url = format!("{DEEPSEEK_API_BASE}/chat/completions");
        let max_tokens = max_output_tokens_for(operation);
        let reasoning_effort = reasoning_effort_for(operation);
        let mut body = json_request_body(&self.model, system, user, max_tokens, reasoning_effort);
        body["stream"] = serde_json::json!(true);

        diagnostics::info(
            "ai.request.started",
            "Streaming AI request started",
            serde_json::json!({
                "request_id": request_id,
                "operation": operation,
                "model": self.model,
                "endpoint": url,
                "system_chars": system.chars().count(),
                "user_chars": user.chars().count(),
                "max_tokens": max_tokens,
                "thinking": "enabled",
                "reasoning_effort": reasoning_effort,
                "response_format": true,
                "streaming": true,
            }),
        );

        let response = self
            .send_request_with_cancellation(request_id, operation, &url, &body, cancellation)
            .await?;
        let mut output =
            read_stream_response(request_id, operation, response, on_event, cancellation).await?;

        if output.content.trim().is_empty() {
            diagnostics::warn(
                "ai.model.empty_content_retry",
                "Streaming AI request returned empty content; retrying once",
                serde_json::json!({
                    "request_id": request_id,
                    "operation": operation,
                }),
            );
            body["messages"][0]["content"] = serde_json::json!(format!(
                "{system}\n\n# JSON 重试约束\n上一次调用返回了空 content。请不要输出空白、思考过程或解释，立即按照上方 JSON 样例输出一个完整合法的 JSON 对象。"
            ));
            let retry = self
                .send_request_with_cancellation(request_id, operation, &url, &body, cancellation)
                .await?;
            output =
                read_stream_response(request_id, operation, retry, on_event, cancellation).await?;
        }
        if output.content.trim().is_empty() {
            diagnostics::error(
                "ai.model.output_empty",
                "Streaming AI request returned empty content twice",
                serde_json::json!({
                    "request_id": request_id,
                    "operation": operation,
                }),
            );
            return Err(anyhow!("AI 连续两次返回空内容（请求 #{request_id}）"));
        }

        diagnostics::info(
            "ai.model.output",
            "Streaming AI model output received",
            serde_json::json!({
                "request_id": request_id,
                "operation": operation,
                "content_chars": output.content.chars().count(),
                "content": diagnostics::truncate(&output.content),
            }),
        );
        let parsed = parse_json_content(&output.content).map_err(|error| {
            diagnostics::error(
                "ai.model.output_invalid",
                "Streaming AI model output is not valid JSON",
                serde_json::json!({
                    "request_id": request_id,
                    "operation": operation,
                    "error": format!("{error:#}"),
                    "content": diagnostics::truncate(&output.content),
                }),
            );
            anyhow!("模型输出不是合法 JSON（请求 #{request_id}）：{error}")
        })?;
        diagnostics::info(
            "ai.request.completed",
            "Streaming AI request completed",
            serde_json::json!({
                "request_id": request_id,
                "operation": operation,
                "elapsed_ms": started.elapsed().as_millis(),
                "streaming": true,
                "finish_reason": output.finish_reason,
                "reasoning_chars": output.reasoning_chars,
                "prompt_tokens": output.usage["prompt_tokens"].as_u64(),
                "completion_tokens": output.usage["completion_tokens"].as_u64(),
                "total_tokens": output.usage["total_tokens"].as_u64(),
            }),
        );
        Ok(parsed)
    }

    async fn send_request_with_cancellation(
        &self,
        request_id: u64,
        operation: &str,
        url: &str,
        body: &serde_json::Value,
        cancellation: Option<&ImportCancellation>,
    ) -> Result<Response> {
        let request = self.send_request(request_id, operation, url, body);
        if let Some(cancellation) = cancellation {
            tokio::select! {
                response = request => response,
                _ = cancellation.cancelled() => Err(anyhow!("导入任务已取消")),
            }
        } else {
            request.await
        }
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

fn json_request_body(
    model: &str,
    system: &str,
    user: &str,
    max_tokens: u32,
    reasoning_effort: &str,
) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "max_tokens": max_tokens,
        "thinking": { "type": "enabled" },
        "reasoning_effort": reasoning_effort,
        "response_format": { "type": "json_object" },
    })
}

fn reasoning_effort_for(operation: &str) -> &'static str {
    if operation.starts_with("import.clean")
        || operation.starts_with("import.organize")
        || operation.starts_with("questions.generate")
    {
        "medium"
    } else {
        "high"
    }
}

fn max_output_tokens_for(operation: &str) -> u32 {
    if operation.starts_with("review.question.generate") {
        8_192
    } else if operation.starts_with("import.organize") || operation.starts_with("answer.judge") {
        32_768
    } else {
        DEEPSEEK_MAX_OUTPUT_TOKENS
    }
}

fn response_content(request_id: u64, operation: &str, response: &ResponseText) -> Result<String> {
    response_parts(request_id, operation, response).map(|(_, content)| content)
}

fn response_parts(
    request_id: u64,
    operation: &str,
    response: &ResponseText,
) -> Result<(String, String)> {
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
    let reasoning = text_content(&choice["message"]["reasoning_content"]).unwrap_or_default();
    let reasoning_chars = reasoning.chars().count();
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
    let content = extract_message_content(&payload).ok_or_else(|| {
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
    })?;
    Ok((reasoning, content))
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

async fn read_stream_response(
    request_id: u64,
    operation: &str,
    mut response: Response,
    on_event: &(dyn Fn(StreamEvent) + Send + Sync),
    cancellation: Option<&ImportCancellation>,
) -> Result<StreamOutput> {
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    if !status.is_success() || !content_type.contains("text/event-stream") {
        let response = read_response(request_id, operation, response).await?;
        let payload = serde_json::from_str::<serde_json::Value>(&response.body).ok();
        let (reasoning, content) = response_parts(request_id, operation, &response)?;
        if !reasoning.is_empty() {
            on_event(StreamEvent::Thinking(reasoning.clone()));
        }
        if !content.is_empty() {
            on_event(StreamEvent::Content(content.clone()));
        }
        let finish_reason = payload
            .as_ref()
            .and_then(|payload| payload["choices"][0]["finish_reason"].as_str())
            .unwrap_or("unknown")
            .to_string();
        let usage = payload
            .as_ref()
            .map(|payload| payload["usage"].clone())
            .unwrap_or(serde_json::Value::Null);
        return Ok(StreamOutput {
            reasoning_chars: reasoning.chars().count(),
            content,
            finish_reason,
            usage,
        });
    }

    let content_length = response.content_length();
    let mut buffer = Vec::new();
    let mut content = String::new();
    let mut reasoning_chars = 0;
    let mut finish_reason = "unknown".to_string();
    let mut usage = serde_json::Value::Null;
    let mut body_bytes = 0usize;
    let mut done = false;

    while !done {
        let chunk = if let Some(cancellation) = cancellation {
            tokio::select! {
                chunk = response.chunk() => chunk,
                _ = cancellation.cancelled() => return Err(anyhow!("导入任务已取消")),
            }
        } else {
            response.chunk().await
        }
        .map_err(|error| {
            diagnostics::error(
                "ai.response.body_read_failed",
                "Failed to read streaming AI response body",
                serde_json::json!({
                    "request_id": request_id,
                    "operation": operation,
                    "status": status.as_u16(),
                    "content_type": content_type,
                    "error": format!("{error:#}"),
                }),
            );
            anyhow!("读取 AI 流式响应失败（请求 #{request_id}）：{error}")
        })?;
        let Some(chunk) = chunk else {
            break;
        };
        body_bytes += chunk.len();
        buffer.extend_from_slice(&chunk);
        while let Some(frame) = take_sse_frame(&mut buffer) {
            done = consume_sse_frame(
                &frame,
                &mut content,
                &mut reasoning_chars,
                &mut finish_reason,
                &mut usage,
                on_event,
            )?;
            if done {
                break;
            }
        }
    }
    if !done && buffer.iter().any(|byte| !byte.is_ascii_whitespace()) {
        consume_sse_frame(
            &buffer,
            &mut content,
            &mut reasoning_chars,
            &mut finish_reason,
            &mut usage,
            on_event,
        )?;
    }

    diagnostics::info(
        "ai.response.received",
        "Streaming AI HTTP response received",
        serde_json::json!({
            "request_id": request_id,
            "operation": operation,
            "status": status.as_u16(),
            "content_type": content_type,
            "content_length": content_length,
            "body_bytes": body_bytes,
            "streaming": true,
        }),
    );
    Ok(StreamOutput {
        content,
        reasoning_chars,
        finish_reason,
        usage,
    })
}

fn take_sse_frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let (index, boundary_len) = find_sse_boundary(buffer)?;
    let remainder = buffer.split_off(index + boundary_len);
    buffer.truncate(index);
    let frame = std::mem::replace(buffer, remainder);
    Some(frame)
}

fn find_sse_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    for index in 0..buffer.len() {
        if buffer[index..].starts_with(b"\r\n\r\n") {
            return Some((index, 4));
        }
        if buffer[index..].starts_with(b"\n\n") {
            return Some((index, 2));
        }
    }
    None
}

fn parse_sse_frame(frame: &[u8]) -> Result<Option<SseFrame>> {
    let frame = std::str::from_utf8(frame)
        .map_err(|error| anyhow!("AI 流式响应包含无效 UTF-8：{error}"))?;
    let data = frame
        .lines()
        .filter_map(|line| {
            line.trim_end_matches('\r')
                .strip_prefix("data:")
                .map(|data| data.strip_prefix(' ').unwrap_or(data))
        })
        .collect::<Vec<_>>()
        .join("\n");
    if data.trim().is_empty() {
        return Ok(None);
    }
    if data.trim() == "[DONE]" {
        return Ok(Some(SseFrame::Done));
    }
    serde_json::from_str(&data)
        .map(SseFrame::Data)
        .map(Some)
        .map_err(|error| anyhow!("AI 流式事件不是合法 JSON：{error}"))
}

fn consume_sse_frame(
    frame: &[u8],
    content: &mut String,
    reasoning_chars: &mut usize,
    finish_reason: &mut String,
    usage: &mut serde_json::Value,
    on_event: &(dyn Fn(StreamEvent) + Send + Sync),
) -> Result<bool> {
    let Some(frame) = parse_sse_frame(frame)? else {
        return Ok(false);
    };
    let SseFrame::Data(payload) = frame else {
        return Ok(true);
    };
    if !payload["usage"].is_null() {
        *usage = payload["usage"].clone();
    }
    let Some(choice) = payload["choices"]
        .as_array()
        .and_then(|choices| choices.first())
    else {
        return Ok(false);
    };
    if let Some(reason) = choice["finish_reason"].as_str() {
        *finish_reason = reason.to_string();
    }
    if let Some(thinking) = text_content(&choice["delta"]["reasoning_content"])
        && !thinking.is_empty()
    {
        *reasoning_chars += thinking.chars().count();
        on_event(StreamEvent::Thinking(thinking));
    }
    if let Some(delta) = text_content(&choice["delta"]["content"])
        && !delta.is_empty()
    {
        content.push_str(&delta);
        on_event(StreamEvent::Content(delta));
    }
    Ok(false)
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
        DEEPSEEK_FLASH_MODEL, SseFrame, extract_message_content, json_request_body,
        max_output_tokens_for, parse_json_content, parse_sse_frame, reasoning_effort_for,
        take_sse_frame,
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
        assert_eq!(max_output_tokens_for("review.question.generate"), 8_192);
        assert_eq!(max_output_tokens_for("answer.judge"), 32_768);
        assert_eq!(max_output_tokens_for("plan.extract"), 384_000);
        assert_eq!(max_output_tokens_for("plan.reconcile"), 384_000);
        assert_eq!(max_output_tokens_for("import.clean"), 384_000);
        assert_eq!(max_output_tokens_for("questions.generate"), 384_000);
    }

    #[test]
    fn deepseek_json_requests_enable_thinking_with_selected_effort() {
        let body = json_request_body(
            DEEPSEEK_FLASH_MODEL,
            "输出 JSON",
            "{}",
            65_536,
            reasoning_effort_for("import.clean"),
        );
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "medium");
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["max_tokens"], 65_536);
        assert!(body["temperature"].is_null());
    }

    #[test]
    fn reasoning_effort_matches_workflow_stage() {
        assert_eq!(reasoning_effort_for("import.clean"), "medium");
        assert_eq!(reasoning_effort_for("import.organize"), "medium");
        assert_eq!(reasoning_effort_for("questions.generate"), "medium");
        assert_eq!(reasoning_effort_for("plan.extract"), "high");
        assert_eq!(reasoning_effort_for("plan.reconcile"), "high");
        assert_eq!(reasoning_effort_for("plan.repair"), "high");
        assert_eq!(reasoning_effort_for("answer.judge"), "high");
    }

    #[test]
    fn parses_fragmented_sse_frames_without_corrupting_utf8() {
        let first = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"正在整理\"}}]}\n\n";
        let second = "data: [DONE]\r\n\r\n";
        let bytes = format!("{first}{second}").into_bytes();
        let split = first.find("整理").unwrap() + 1;
        let mut buffer = bytes[..split].to_vec();
        assert!(take_sse_frame(&mut buffer).is_none());

        buffer.extend_from_slice(&bytes[split..]);
        let frame = take_sse_frame(&mut buffer).unwrap();
        assert!(matches!(
            parse_sse_frame(&frame).unwrap(),
            Some(SseFrame::Data(_))
        ));
        let done = take_sse_frame(&mut buffer).unwrap();
        assert!(matches!(
            parse_sse_frame(&done).unwrap(),
            Some(SseFrame::Done)
        ));
        assert!(buffer.is_empty());
    }
}
