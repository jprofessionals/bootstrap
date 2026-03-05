use super::*;

#[derive(Default)]
pub struct AnthropicProvider;

impl AnthropicProvider {
    pub fn new() -> Self {
        Self
    }
}

impl AiProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "claude-sonnet-4-5-20250929".into(),
                name: "Claude Sonnet 4.5".into(),
            },
            ModelInfo {
                id: "claude-haiku-4-5-20251001".into(),
                name: "Claude Haiku 4.5".into(),
            },
        ]
    }

    fn default_model(&self) -> &str {
        "claude-sonnet-4-5-20250929"
    }

    fn build_request(&self, api_key: &str, req: &StreamRequest) -> ProviderRequest {
        let model = req.model.as_deref().unwrap_or(self.default_model());
        let max_tokens = req.max_tokens.unwrap_or(8192);

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": req.messages,
            "stream": true,
        });
        if let Some(tools) = &req.tools {
            body["tools"] = tools.clone();
        }
        if let Some(system) = &req.system {
            body["system"] = system.clone();
        }

        ProviderRequest {
            url: "https://api.anthropic.com/v1/messages".into(),
            headers: vec![
                ("x-api-key".into(), api_key.into()),
                ("anthropic-version".into(), "2023-06-01".into()),
                ("content-type".into(), "application/json".into()),
            ],
            body,
        }
    }

    fn translate_event(&mut self, event_type: &str, data: &str) -> Vec<AnthropicSseEvent> {
        vec![AnthropicSseEvent {
            event_type: event_type.to_string(),
            data: data.to_string(),
        }]
    }
}
