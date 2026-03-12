use super::*;

#[derive(Default)]
pub struct GeminiProvider {
    started: bool,
    content_block_index: usize,
    text_block_open: bool,
    has_tool_use: bool,
}

impl GeminiProvider {
    pub fn new() -> Self {
        Self {
            started: false,
            content_block_index: 0,
            text_block_open: false,
            has_tool_use: false,
        }
    }

    /// Extract system prompt text from the Anthropic system field.
    fn extract_system_text(system: &serde_json::Value) -> String {
        match system {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        }
    }

    /// Look up a tool_use name by its id, scanning assistant messages in the conversation.
    fn find_tool_name_by_id(messages: &serde_json::Value, tool_use_id: &str) -> Option<String> {
        let msgs = messages.as_array()?;
        for msg in msgs {
            if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
                continue;
            }
            if let Some(blocks) = msg.get("content").and_then(|c| c.as_array()) {
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                        && block.get("id").and_then(|v| v.as_str()) == Some(tool_use_id)
                    {
                        return block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                }
            }
        }
        None
    }

    /// Translate Anthropic-format messages to Gemini-format contents.
    fn translate_messages(messages: &serde_json::Value) -> Vec<serde_json::Value> {
        let Some(msgs) = messages.as_array() else {
            return vec![];
        };

        let mut result = Vec::new();

        for msg in msgs {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let gemini_role = match role {
                "assistant" => "model",
                _ => "user",
            };
            let content = &msg["content"];

            match content {
                serde_json::Value::String(s) => {
                    result.push(serde_json::json!({
                        "role": gemini_role,
                        "parts": [{"text": s}]
                    }));
                }
                serde_json::Value::Array(blocks) => {
                    let mut parts = Vec::new();

                    for block in blocks {
                        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match block_type {
                            "text" => {
                                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                    parts.push(serde_json::json!({"text": text}));
                                }
                            }
                            "tool_use" => {
                                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                let args =
                                    block.get("input").cloned().unwrap_or(serde_json::json!({}));
                                parts.push(serde_json::json!({
                                    "functionCall": {
                                        "name": name,
                                        "args": args
                                    }
                                }));
                            }
                            "tool_result" => {
                                let tool_use_id = block
                                    .get("tool_use_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let name = Self::find_tool_name_by_id(messages, tool_use_id)
                                    .unwrap_or_else(|| tool_use_id.to_string());
                                let content_val = block.get("content");
                                let result_text = match content_val {
                                    Some(serde_json::Value::String(s)) => s.clone(),
                                    Some(serde_json::Value::Array(arr)) => arr
                                        .iter()
                                        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                        .collect::<Vec<_>>()
                                        .join(""),
                                    _ => String::new(),
                                };
                                parts.push(serde_json::json!({
                                    "functionResponse": {
                                        "name": name,
                                        "response": {"result": result_text}
                                    }
                                }));
                            }
                            _ => {}
                        }
                    }

                    if !parts.is_empty() {
                        result.push(serde_json::json!({
                            "role": gemini_role,
                            "parts": parts
                        }));
                    }
                }
                _ => {
                    result.push(serde_json::json!({
                        "role": gemini_role,
                        "parts": [{"text": ""}]
                    }));
                }
            }
        }

        result
    }

    /// Translate Anthropic-format tools to Gemini functionDeclarations.
    fn translate_tools(tools: &serde_json::Value) -> Vec<serde_json::Value> {
        let Some(tools_arr) = tools.as_array() else {
            return vec![];
        };

        tools_arr
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    "description": tool.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                    "parameters": tool.get("input_schema").cloned().unwrap_or(serde_json::json!({}))
                })
            })
            .collect()
    }

    fn emit_event(event_type: &str, data: serde_json::Value) -> AnthropicSseEvent {
        AnthropicSseEvent {
            event_type: event_type.to_string(),
            data: serde_json::to_string(&data).unwrap_or_default(),
        }
    }

    fn close_text_block(&mut self, events: &mut Vec<AnthropicSseEvent>) {
        if self.text_block_open {
            events.push(Self::emit_event(
                "content_block_stop",
                serde_json::json!({"type": "content_block_stop", "index": self.content_block_index}),
            ));
            self.text_block_open = false;
            self.content_block_index += 1;
        }
    }
}

impl AiProvider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "gemini-2.5-flash".into(),
                name: "Gemini 2.5 Flash".into(),
            },
            ModelInfo {
                id: "gemini-2.5-pro".into(),
                name: "Gemini 2.5 Pro".into(),
            },
            ModelInfo {
                id: "gemini-2.5-flash-lite".into(),
                name: "Gemini 2.5 Flash Lite".into(),
            },
        ]
    }

    fn default_model(&self) -> &str {
        "gemini-2.5-flash"
    }

    fn build_request(&self, api_key: &str, req: &StreamRequest) -> ProviderRequest {
        let model = req.model.as_deref().unwrap_or(self.default_model());
        let max_tokens = req.max_tokens.unwrap_or(8192);

        let contents = Self::translate_messages(&req.messages);

        let mut body = serde_json::json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": max_tokens
            }
        });

        if let Some(system) = &req.system {
            let text = Self::extract_system_text(system);
            if !text.is_empty() {
                body["systemInstruction"] = serde_json::json!({
                    "parts": [{"text": text}]
                });
            }
        }

        if let Some(tools) = &req.tools {
            let declarations = Self::translate_tools(tools);
            if !declarations.is_empty() {
                body["tools"] = serde_json::json!([{
                    "functionDeclarations": declarations
                }]);
            }
        }

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
            model
        );

        ProviderRequest {
            url,
            headers: vec![
                ("x-goog-api-key".into(), api_key.to_string()),
                ("Content-Type".into(), "application/json".into()),
            ],
            body,
        }
    }

    fn translate_event(&mut self, _event_type: &str, data: &str) -> Vec<AnthropicSseEvent> {
        let mut events = Vec::new();

        let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) else {
            return events;
        };

        // Emit message_start on first chunk
        if !self.started {
            self.started = true;
            events.push(Self::emit_event(
                "message_start",
                serde_json::json!({
                    "type": "message_start",
                    "message": {
                        "id": "msg_gemini",
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "model": "gemini",
                        "stop_reason": null
                    }
                }),
            ));
        }

        // Extract candidates[0]
        let Some(candidate) = chunk
            .get("candidates")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
        else {
            return events;
        };

        let finish_reason = candidate.get("finishReason").and_then(|f| f.as_str());

        // Process parts
        if let Some(parts) = candidate
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
        {
            for part in parts {
                // Text part
                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    if !self.text_block_open {
                        self.text_block_open = true;
                        events.push(Self::emit_event(
                            "content_block_start",
                            serde_json::json!({
                                "type": "content_block_start",
                                "index": self.content_block_index,
                                "content_block": {"type": "text", "text": ""}
                            }),
                        ));
                    }
                    events.push(Self::emit_event(
                        "content_block_delta",
                        serde_json::json!({
                            "type": "content_block_delta",
                            "index": self.content_block_index,
                            "delta": {"type": "text_delta", "text": text}
                        }),
                    ));
                }

                // Function call part
                if let Some(fc) = part.get("functionCall") {
                    // Close any open text block first
                    self.close_text_block(&mut events);

                    let name = fc.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let args = fc.get("args").cloned().unwrap_or(serde_json::json!({}));
                    let tool_id = format!("toolu_gemini_{}", self.content_block_index);

                    events.push(Self::emit_event(
                        "content_block_start",
                        serde_json::json!({
                            "type": "content_block_start",
                            "index": self.content_block_index,
                            "content_block": {"type": "tool_use", "id": tool_id, "name": name}
                        }),
                    ));

                    let args_json = serde_json::to_string(&args).unwrap_or_default();
                    events.push(Self::emit_event(
                        "content_block_delta",
                        serde_json::json!({
                            "type": "content_block_delta",
                            "index": self.content_block_index,
                            "delta": {"type": "input_json_delta", "partial_json": args_json}
                        }),
                    ));

                    events.push(Self::emit_event(
                        "content_block_stop",
                        serde_json::json!({"type": "content_block_stop", "index": self.content_block_index}),
                    ));

                    self.content_block_index += 1;
                    self.has_tool_use = true;
                }
            }
        }

        // Handle finish
        if let Some(reason) = finish_reason {
            self.close_text_block(&mut events);

            let stop_reason = if self.has_tool_use {
                "tool_use"
            } else {
                match reason {
                    "MAX_TOKENS" => "max_tokens",
                    _ => "end_turn",
                }
            };

            events.push(Self::emit_event(
                "message_delta",
                serde_json::json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": stop_reason}
                }),
            ));

            events.push(Self::emit_event(
                "message_stop",
                serde_json::json!({"type": "message_stop"}),
            ));
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_models() {
        let provider = GeminiProvider::new();
        let models = provider.models();
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].id, "gemini-2.5-flash");
        assert_eq!(models[1].id, "gemini-2.5-pro");
        assert_eq!(models[2].id, "gemini-2.5-flash-lite");
    }

    #[test]
    fn test_default_model() {
        let provider = GeminiProvider::new();
        assert_eq!(provider.default_model(), "gemini-2.5-flash");
    }

    #[test]
    fn test_build_request_basic() {
        let provider = GeminiProvider::new();
        let req = StreamRequest {
            messages: serde_json::json!([
                {"role": "user", "content": "Hello"}
            ]),
            tools: None,
            system: Some(serde_json::json!("You are helpful")),
            model: Some("gemini-2.5-flash".into()),
            max_tokens: Some(1024),
            provider: None,
        };

        let result = provider.build_request("test-api-key", &req);

        assert_eq!(
            result.url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );
        assert!(result
            .headers
            .iter()
            .any(|(k, v)| k == "x-goog-api-key" && v == "test-api-key"));
        assert!(result
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/json"));

        // Check system instruction
        assert_eq!(
            result.body["systemInstruction"]["parts"][0]["text"],
            "You are helpful"
        );

        // Check contents
        let contents = result.body["contents"].as_array().unwrap();
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "Hello");

        // Check generation config
        assert_eq!(result.body["generationConfig"]["maxOutputTokens"], 1024);
    }

    #[test]
    fn test_build_request_system_array() {
        let provider = GeminiProvider::new();
        let req = StreamRequest {
            messages: serde_json::json!([]),
            tools: None,
            system: Some(serde_json::json!([
                {"type": "text", "text": "Part one. "},
                {"type": "text", "text": "Part two."}
            ])),
            model: None,
            max_tokens: None,
            provider: None,
        };

        let result = provider.build_request("key", &req);
        assert_eq!(
            result.body["systemInstruction"]["parts"][0]["text"],
            "Part one. Part two."
        );
    }

    #[test]
    fn test_build_request_tools() {
        let provider = GeminiProvider::new();
        let req = StreamRequest {
            messages: serde_json::json!([{"role": "user", "content": "hi"}]),
            tools: Some(serde_json::json!([
                {
                    "name": "get_weather",
                    "description": "Get weather info",
                    "input_schema": {
                        "type": "object",
                        "properties": {"location": {"type": "string"}}
                    }
                }
            ])),
            system: None,
            model: None,
            max_tokens: None,
            provider: None,
        };

        let result = provider.build_request("key", &req);
        let tools = result.body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        let decls = tools[0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0]["name"], "get_weather");
        assert_eq!(decls[0]["description"], "Get weather info");
        assert!(decls[0]["parameters"]["properties"]["location"].is_object());
    }

    #[test]
    fn test_build_request_messages_with_tool_use_and_result() {
        let provider = GeminiProvider::new();
        let req = StreamRequest {
            messages: serde_json::json!([
                {"role": "user", "content": "What is the weather?"},
                {"role": "assistant", "content": [
                    {"type": "text", "text": "Let me check."},
                    {"type": "tool_use", "id": "toolu_gemini_0", "name": "get_weather", "input": {"location": "Oslo"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_gemini_0", "content": "Sunny, 20C"}
                ]}
            ]),
            tools: None,
            system: None,
            model: None,
            max_tokens: None,
            provider: None,
        };

        let result = provider.build_request("key", &req);
        let contents = result.body["contents"].as_array().unwrap();

        // First: user text
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "What is the weather?");

        // Second: model with text + functionCall
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "Let me check.");
        assert_eq!(
            contents[1]["parts"][1]["functionCall"]["name"],
            "get_weather"
        );
        assert_eq!(
            contents[1]["parts"][1]["functionCall"]["args"]["location"],
            "Oslo"
        );

        // Third: user with functionResponse (name looked up from assistant message)
        assert_eq!(contents[2]["role"], "user");
        assert_eq!(
            contents[2]["parts"][0]["functionResponse"]["name"],
            "get_weather"
        );
        assert_eq!(
            contents[2]["parts"][0]["functionResponse"]["response"]["result"],
            "Sunny, 20C"
        );
    }

    #[test]
    fn test_build_request_role_mapping() {
        let provider = GeminiProvider::new();
        let req = StreamRequest {
            messages: serde_json::json!([
                {"role": "user", "content": "Hi"},
                {"role": "assistant", "content": "Hello!"},
                {"role": "user", "content": "Bye"}
            ]),
            tools: None,
            system: None,
            model: None,
            max_tokens: None,
            provider: None,
        };

        let result = provider.build_request("key", &req);
        let contents = result.body["contents"].as_array().unwrap();
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[2]["role"], "user");
    }

    #[test]
    fn test_translate_event_text_streaming() {
        let mut provider = GeminiProvider::new();

        // First chunk with text
        let chunk1 = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello"}],
                    "role": "model"
                }
            }]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk1).unwrap());

        // Should get: message_start, content_block_start, content_block_delta
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, "message_start");
        assert_eq!(events[1].event_type, "content_block_start");
        assert_eq!(events[2].event_type, "content_block_delta");

        let delta_data: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
        assert_eq!(delta_data["delta"]["text"], "Hello");
        assert_eq!(delta_data["delta"]["type"], "text_delta");

        // Second chunk with more text
        let chunk2 = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": " world"}],
                    "role": "model"
                }
            }]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk2).unwrap());

        // Should get just content_block_delta (no new start)
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "content_block_delta");
    }

    #[test]
    fn test_translate_event_function_call() {
        let mut provider = GeminiProvider::new();

        let chunk = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "get_weather",
                            "args": {"location": "Oslo"}
                        }
                    }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk).unwrap());

        // message_start, content_block_start, content_block_delta, content_block_stop,
        // message_delta, message_stop
        assert_eq!(events.len(), 6);
        assert_eq!(events[0].event_type, "message_start");
        assert_eq!(events[1].event_type, "content_block_start");

        let start_data: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
        assert_eq!(start_data["content_block"]["type"], "tool_use");
        assert_eq!(start_data["content_block"]["id"], "toolu_gemini_0");
        assert_eq!(start_data["content_block"]["name"], "get_weather");

        assert_eq!(events[2].event_type, "content_block_delta");
        let delta: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
        assert_eq!(delta["delta"]["type"], "input_json_delta");
        let args: serde_json::Value =
            serde_json::from_str(delta["delta"]["partial_json"].as_str().unwrap()).unwrap();
        assert_eq!(args["location"], "Oslo");

        assert_eq!(events[3].event_type, "content_block_stop");

        // stop_reason should be "tool_use" since we had a function call
        let msg_delta: serde_json::Value = serde_json::from_str(&events[4].data).unwrap();
        assert_eq!(msg_delta["delta"]["stop_reason"], "tool_use");

        assert_eq!(events[5].event_type, "message_stop");
    }

    #[test]
    fn test_translate_event_text_then_finish() {
        let mut provider = GeminiProvider::new();

        // First chunk: text
        let chunk1 = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hi there"}],
                    "role": "model"
                }
            }]
        });
        provider.translate_event("message", &serde_json::to_string(&chunk1).unwrap());

        // Second chunk: finish
        let chunk2 = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "!"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk2).unwrap());

        // content_block_delta, content_block_stop (close text), message_delta, message_stop
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].event_type, "content_block_delta");
        assert_eq!(events[1].event_type, "content_block_stop");
        assert_eq!(events[2].event_type, "message_delta");

        let msg_delta: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
        assert_eq!(msg_delta["delta"]["stop_reason"], "end_turn");

        assert_eq!(events[3].event_type, "message_stop");
    }

    #[test]
    fn test_translate_event_max_tokens() {
        let mut provider = GeminiProvider::new();

        let chunk = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "truncated"}],
                    "role": "model"
                },
                "finishReason": "MAX_TOKENS"
            }]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk).unwrap());

        let msg_delta = events
            .iter()
            .find(|e| e.event_type == "message_delta")
            .unwrap();
        let data: serde_json::Value = serde_json::from_str(&msg_delta.data).unwrap();
        assert_eq!(data["delta"]["stop_reason"], "max_tokens");
    }

    #[test]
    fn test_translate_event_text_then_function_call() {
        let mut provider = GeminiProvider::new();

        // Chunk with text first
        let chunk1 = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Let me check."}],
                    "role": "model"
                }
            }]
        });
        provider.translate_event("message", &serde_json::to_string(&chunk1).unwrap());

        // Then function call
        let chunk2 = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "search",
                            "args": {"query": "test"}
                        }
                    }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk2).unwrap());

        // Close text block (content_block_stop), then tool start/delta/stop, then message_delta, message_stop
        let event_types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(
            event_types,
            vec![
                "content_block_stop",  // close text block
                "content_block_start", // tool_use start
                "content_block_delta", // tool args
                "content_block_stop",  // tool_use stop
                "message_delta",       // stop_reason
                "message_stop",        // done
            ]
        );

        // The tool_use block should have index 1 (text was 0)
        let tool_start: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
        assert_eq!(tool_start["index"], 1);
        assert_eq!(tool_start["content_block"]["name"], "search");

        // Stop reason should be tool_use
        let msg_delta: serde_json::Value = serde_json::from_str(&events[4].data).unwrap();
        assert_eq!(msg_delta["delta"]["stop_reason"], "tool_use");
    }

    #[test]
    fn test_translate_event_invalid_json() {
        let mut provider = GeminiProvider::new();
        let events = provider.translate_event("message", "not valid json");
        assert!(events.is_empty());
    }

    #[test]
    fn test_translate_event_no_candidates() {
        let mut provider = GeminiProvider::new();
        let events = provider.translate_event("message", "{}");
        // Should emit message_start but nothing else
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "message_start");
    }

    #[test]
    fn test_tool_result_name_lookup() {
        // Test that tool_result blocks correctly look up the function name
        let messages = serde_json::json!([
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_gemini_0", "name": "calculate", "input": {"expr": "2+2"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_gemini_0", "content": "4"}
            ]}
        ]);

        let contents = GeminiProvider::translate_messages(&messages);
        assert_eq!(contents.len(), 2);

        // The tool result should use the function name "calculate", not the id
        let func_response = &contents[1]["parts"][0]["functionResponse"];
        assert_eq!(func_response["name"], "calculate");
        assert_eq!(func_response["response"]["result"], "4");
    }

    #[test]
    fn test_build_request_default_model() {
        let provider = GeminiProvider::new();
        let req = StreamRequest {
            messages: serde_json::json!([{"role": "user", "content": "hi"}]),
            tools: None,
            system: None,
            model: None,
            max_tokens: None,
            provider: None,
        };

        let result = provider.build_request("key", &req);
        assert!(result.url.contains("gemini-2.5-flash"));
    }

    #[test]
    fn test_build_request_no_system_when_empty() {
        let provider = GeminiProvider::new();
        let req = StreamRequest {
            messages: serde_json::json!([{"role": "user", "content": "hi"}]),
            tools: None,
            system: None,
            model: None,
            max_tokens: None,
            provider: None,
        };

        let result = provider.build_request("key", &req);
        assert!(result.body.get("systemInstruction").is_none());
    }
}
