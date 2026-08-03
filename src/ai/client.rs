use anyhow::{Result, anyhow};

/// OpenAI 兼容的 Chat Completions 客户端。
/// `api_base` 形如 `https://api.openai.com/v1` 或 `https://api.deepseek.com/v1`。
#[derive(Clone)]
pub struct ChatClient {
    http: reqwest::Client,
    api_base: String,
    api_key: String,
    model: String,
}

impl ChatClient {
    pub fn new(api_base: String, api_key: String, model: String) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("构建 HTTP client 失败");
        Self {
            http,
            api_base,
            api_key,
            model,
        }
    }

    /// 发送一次对话，要求模型返回 JSON 对象（`response_format: json_object`）。
    /// 若服务端不支持该参数，会在错误信息里提示。
    pub async fn chat_json(&self, system: &str, user: &str) -> Result<serde_json::Value> {
        let url = format!("{}/chat/completions", self.api_base.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "temperature": 0.3,
            "response_format": { "type": "json_object" },
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("AI API 请求失败（{status}）: {text}"));
        }

        let payload: serde_json::Value = resp.json().await?;
        let content = payload["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow!("AI 响应中缺少 choices[0].message.content"))?;
        serde_json::from_str(content).map_err(|e| anyhow!("AI 返回的不是合法 JSON: {e}"))
    }
}
