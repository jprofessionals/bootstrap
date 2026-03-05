use std::collections::HashMap;

use super::*;

/// Tracks the state of a single OpenAI tool call being streamed.
#[derive(Debug, Default)]
struct ToolCallState {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
pub struct OpenAiProvider {
    content_block_index: usize,
    started: bool,
    text_block_open: bool,
    tool_calls: HashMap<usize, ToolCallState>,
}

impl OpenAiProvider {
    pub fn new() -> Self {
        Self {
            content_block_index: 0,
            started: false,
            text_block_open: false,
            tool_calls: HashMap::new(),
        }
    }

    /// Translate Anthropic system field to an OpenAI system message.
    fn system_to_message(system: &serde_json::Value) -> serde_json::Value {
        let text = match system {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(arr) => {
                arr.iter()
                    .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("")
            }
            _ => String::new(),
        };
        serde_json::json!({"role": "system", "content": text})
    }

    /// Translate Anthropic-format messages to OpenAI-format messages.
    fn translate_messages(messages: &serde_json::Value) -> Vec<serde_json::Value> {
        let Some(msgs) = messages.as_array() else {
            return vec![];
        };

        let mut result = Vec::new();

        for msg in msgs {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let content = &msg["content"];

            match content {
                // Simple string content
                serde_json::Value::String(s) => {
                    result.push(serde_json::json!({"role": role, "content": s}));
                }
                // Array of content blocks
                serde_json::Value::Array(blocks) => {
                    let mut text_parts = Vec::new();
                    let mut tool_calls_list = Vec::new();
                    let mut tool_results = Vec::new();

                    for block in blocks {
                        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match block_type {
                            "text" => {
                                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                    text_parts.push(text.to_string());
                                }
                            }
                            "tool_use" => {
                                let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                let name =
                                    block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                let input = &block["input"];
                                tool_calls_list.push(serde_json::json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": serde_json::to_string(input).unwrap_or_default()
                                    }
                                }));
                            }
                            "tool_result" => {
                                let tool_call_id = block
                                    .get("tool_use_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let content_val = block.get("content");
                                let content_str = match content_val {
                                    Some(serde_json::Value::String(s)) => s.clone(),
                                    Some(serde_json::Value::Array(arr)) => arr
                                        .iter()
                                        .filter_map(
                                            |b| b.get("text").and_then(|t| t.as_str()),
                                        )
                                        .collect::<Vec<_>>()
                                        .join(""),
                                    _ => String::new(),
                                };
                                tool_results.push(serde_json::json!({
                                    "role": "tool",
                                    "tool_call_id": tool_call_id,
                                    "content": content_str
                                }));
                            }
                            _ => {}
                        }
                    }

                    // Emit assistant message with text + tool_calls if applicable
                    if role == "assistant" {
                        let mut assistant_msg = serde_json::json!({"role": "assistant"});
                        if !text_parts.is_empty() {
                            assistant_msg["content"] =
                                serde_json::Value::String(text_parts.join(""));
                        }
                        if !tool_calls_list.is_empty() {
                            assistant_msg["tool_calls"] =
                                serde_json::Value::Array(tool_calls_list);
                        }
                        result.push(assistant_msg);
                    } else if !tool_results.is_empty() {
                        // User message containing tool results: emit each as separate role:tool message
                        // But first emit any text as a user message
                        if !text_parts.is_empty() {
                            result.push(
                                serde_json::json!({"role": "user", "content": text_parts.join("")}),
                            );
                        }
                        result.extend(tool_results);
                    } else {
                        // Regular user/other message with text blocks
                        result.push(
                            serde_json::json!({"role": role, "content": text_parts.join("")}),
                        );
                    }
                }
                _ => {
                    result.push(serde_json::json!({"role": role, "content": ""}));
                }
            }
        }

        result
    }

    /// Translate Anthropic-format tools to OpenAI-format tools.
    fn translate_tools(tools: &serde_json::Value) -> Vec<serde_json::Value> {
        let Some(tools_arr) = tools.as_array() else {
            return vec![];
        };

        tools_arr
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "description": tool.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                        "parameters": tool.get("input_schema").cloned().unwrap_or(serde_json::json!({}))
                    }
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

impl AiProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "gpt-4o".into(),
                name: "GPT-4o".into(),
            },
            ModelInfo {
                id: "gpt-4o-mini".into(),
                name: "GPT-4o Mini".into(),
            },
            ModelInfo {
                id: "o3-mini".into(),
                name: "o3-mini".into(),
            },
        ]
    }

    fn default_model(&self) -> &str {
        "gpt-4o"
    }

    fn build_request(&self, api_key: &str, req: &StreamRequest) -> ProviderRequest {
        let model = req.model.as_deref().unwrap_or(self.default_model());
        let max_tokens = req.max_tokens.unwrap_or(8192);

        let mut messages = Vec::new();

        // System prompt becomes the first message
        if let Some(system) = &req.system {
            messages.push(Self::system_to_message(system));
        }

        // Translate user/assistant messages
        messages.extend(Self::translate_messages(&req.messages));

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": messages,
            "stream": true,
        });

        if let Some(tools) = &req.tools {
            let translated = Self::translate_tools(tools);
            if !translated.is_empty() {
                body["tools"] = serde_json::Value::Array(translated);
            }
        }

        ProviderRequest {
            url: "https://api.openai.com/v1/chat/completions".into(),
            headers: vec![
                ("Authorization".into(), format!("Bearer {}", api_key)),
                ("Content-Type".into(), "application/json".into()),
            ],
            body,
        }
    }

    fn translate_event(&mut self, _event_type: &str, data: &str) -> Vec<AnthropicSseEvent> {
        let mut events = Vec::new();

        if data.trim() == "[DONE]" {
            events.push(Self::emit_event(
                "message_stop",
                serde_json::json!({"type": "message_stop"}),
            ));
            return events;
        }

        let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) else {
            return events;
        };

        let model = chunk
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown");

        // Emit message_start on first chunk
        if !self.started {
            self.started = true;
            events.push(Self::emit_event(
                "message_start",
                serde_json::json!({
                    "type": "message_start",
                    "message": {
                        "id": "msg_openai",
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "model": model,
                        "stop_reason": null
                    }
                }),
            ));
        }

        let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) else {
            return events;
        };

        for choice in choices {
            let delta = &choice["delta"];
            let finish_reason = choice
                .get("finish_reason")
                .and_then(|f| f.as_str());

            // Handle text content
            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                if !content.is_empty() {
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
                            "delta": {"type": "text_delta", "text": content}
                        }),
                    ));
                }
            }

            // Handle tool calls
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    let id = tc.get("id").and_then(|v| v.as_str());
                    let func = &tc["function"];
                    let name = func.get("name").and_then(|n| n.as_str());
                    let arguments = func.get("arguments").and_then(|a| a.as_str());

                    if let (Some(id), Some(name)) = (id, name) {
                        // First chunk for this tool call — close text block if open
                        self.close_text_block(&mut events);

                        let state = self.tool_calls.entry(idx).or_default();
                        state.id = id.to_string();
                        state.name = name.to_string();

                        events.push(Self::emit_event(
                            "content_block_start",
                            serde_json::json!({
                                "type": "content_block_start",
                                "index": self.content_block_index,
                                "content_block": {"type": "tool_use", "id": id, "name": name}
                            }),
                        ));
                    }

                    if let Some(args) = arguments {
                        if !args.is_empty() {
                            let state = self.tool_calls.entry(idx).or_default();
                            state.arguments.push_str(args);

                            events.push(Self::emit_event(
                                "content_block_delta",
                                serde_json::json!({
                                    "type": "content_block_delta",
                                    "index": self.content_block_index,
                                    "delta": {"type": "input_json_delta", "partial_json": args}
                                }),
                            ));
                        }
                    }
                }
            }

            // Handle finish
            if let Some(reason) = finish_reason {
                self.close_text_block(&mut events);

                // Close any open tool call blocks
                for _idx in self.tool_calls.drain() {
                    events.push(Self::emit_event(
                        "content_block_stop",
                        serde_json::json!({"type": "content_block_stop", "index": self.content_block_index}),
                    ));
                    self.content_block_index += 1;
                }

                let stop_reason = match reason {
                    "tool_calls" => "tool_use",
                    _ => "end_turn",
                };

                events.push(Self::emit_event(
                    "message_delta",
                    serde_json::json!({
                        "type": "message_delta",
                        "delta": {"stop_reason": stop_reason}
                    }),
                ));
            }
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_request_basic() {
        let provider = OpenAiProvider::new();
        let req = StreamRequest {
            messages: serde_json::json!([
                {"role": "user", "content": "Hello"}
            ]),
            tools: None,
            system: Some(serde_json::json!("You are helpful")),
            model: Some("gpt-4o".into()),
            max_tokens: Some(1024),
            provider: None,
        };

        let result = provider.build_request("sk-test-key", &req);

        assert_eq!(result.url, "https://api.openai.com/v1/chat/completions");
        assert!(result
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer sk-test-key"));

        let messages = result.body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "You are helpful");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "Hello");
        assert_eq!(result.body["model"], "gpt-4o");
        assert_eq!(result.body["stream"], true);
    }

    #[test]
    fn test_build_request_system_array() {
        let provider = OpenAiProvider::new();
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
        let messages = result.body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["content"], "Part one. Part two.");
    }

    #[test]
    fn test_build_request_tools() {
        let provider = OpenAiProvider::new();
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
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "get_weather");
        assert_eq!(tools[0]["function"]["description"], "Get weather info");
        assert!(tools[0]["function"]["parameters"]["properties"]["location"].is_object());
    }

    #[test]
    fn test_build_request_messages_with_tool_use() {
        let provider = OpenAiProvider::new();
        let req = StreamRequest {
            messages: serde_json::json!([
                {"role": "user", "content": "What is the weather?"},
                {"role": "assistant", "content": [
                    {"type": "text", "text": "Let me check."},
                    {"type": "tool_use", "id": "call_1", "name": "get_weather", "input": {"location": "Oslo"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "call_1", "content": "Sunny, 20C"}
                ]}
            ]),
            tools: None,
            system: None,
            model: None,
            max_tokens: None,
            provider: None,
        };

        let result = provider.build_request("key", &req);
        let messages = result.body["messages"].as_array().unwrap();

        // First message: user
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "What is the weather?");

        // Second: assistant with text + tool_calls
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "Let me check.");
        let tc = &messages[1]["tool_calls"].as_array().unwrap()[0];
        assert_eq!(tc["id"], "call_1");
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "get_weather");
        let args: serde_json::Value =
            serde_json::from_str(tc["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["location"], "Oslo");

        // Third: tool result
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_1");
        assert_eq!(messages[2]["content"], "Sunny, 20C");
    }

    #[test]
    fn test_translate_event_text_streaming() {
        let mut provider = OpenAiProvider::new();

        // First chunk with text
        let chunk1 = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"delta": {"content": "Hello"}, "finish_reason": null}]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk1).unwrap());

        // Should get: message_start, content_block_start, content_block_delta
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, "message_start");
        assert_eq!(events[1].event_type, "content_block_start");
        assert_eq!(events[2].event_type, "content_block_delta");

        let delta_data: serde_json::Value = serde_json::from_str(&events[2].data).unwrap();
        assert_eq!(delta_data["delta"]["text"], "Hello");

        // Second chunk with more text
        let chunk2 = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"delta": {"content": " world"}, "finish_reason": null}]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk2).unwrap());

        // Should get just content_block_delta (no new start)
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "content_block_delta");
    }

    #[test]
    fn test_translate_event_tool_call_streaming() {
        let mut provider = OpenAiProvider::new();

        // First chunk: message start + tool call start
        let chunk1 = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"delta": {"tool_calls": [
                {"index": 0, "id": "call_abc", "function": {"name": "get_weather", "arguments": ""}}
            ]}, "finish_reason": null}]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk1).unwrap());

        assert_eq!(events[0].event_type, "message_start");
        assert_eq!(events[1].event_type, "content_block_start");
        let start_data: serde_json::Value = serde_json::from_str(&events[1].data).unwrap();
        assert_eq!(start_data["content_block"]["type"], "tool_use");
        assert_eq!(start_data["content_block"]["id"], "call_abc");
        assert_eq!(start_data["content_block"]["name"], "get_weather");

        // Second chunk: arguments
        let chunk2 = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"delta": {"tool_calls": [
                {"index": 0, "function": {"arguments": "{\"loc"}}
            ]}, "finish_reason": null}]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk2).unwrap());

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "content_block_delta");
        let delta: serde_json::Value = serde_json::from_str(&events[0].data).unwrap();
        assert_eq!(delta["delta"]["type"], "input_json_delta");
        assert_eq!(delta["delta"]["partial_json"], "{\"loc");
    }

    #[test]
    fn test_translate_event_finish_and_done() {
        let mut provider = OpenAiProvider::new();

        // Start text
        let chunk1 = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"delta": {"content": "Hi"}, "finish_reason": null}]
        });
        provider.translate_event("message", &serde_json::to_string(&chunk1).unwrap());

        // Finish
        let chunk2 = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"delta": {}, "finish_reason": "stop"}]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk2).unwrap());

        // Should close text block + message_delta
        assert!(events.iter().any(|e| e.event_type == "content_block_stop"));
        assert!(events.iter().any(|e| e.event_type == "message_delta"));
        let msg_delta = events.iter().find(|e| e.event_type == "message_delta").unwrap();
        let data: serde_json::Value = serde_json::from_str(&msg_delta.data).unwrap();
        assert_eq!(data["delta"]["stop_reason"], "end_turn");

        // [DONE]
        let events = provider.translate_event("message", "[DONE]");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "message_stop");
    }

    #[test]
    fn test_translate_event_tool_calls_finish_reason() {
        let mut provider = OpenAiProvider::new();

        // Tool call start
        let chunk1 = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"delta": {"tool_calls": [
                {"index": 0, "id": "call_1", "function": {"name": "search", "arguments": "{}"}}
            ]}, "finish_reason": null}]
        });
        provider.translate_event("message", &serde_json::to_string(&chunk1).unwrap());

        // Finish with tool_calls reason
        let chunk2 = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"delta": {}, "finish_reason": "tool_calls"}]
        });
        let events = provider.translate_event("message", &serde_json::to_string(&chunk2).unwrap());

        let msg_delta = events.iter().find(|e| e.event_type == "message_delta").unwrap();
        let data: serde_json::Value = serde_json::from_str(&msg_delta.data).unwrap();
        assert_eq!(data["delta"]["stop_reason"], "tool_use");
    }

    #[test]
    fn test_models() {
        let provider = OpenAiProvider::new();
        let models = provider.models();
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].id, "gpt-4o");
        assert_eq!(models[1].id, "gpt-4o-mini");
        assert_eq!(models[2].id, "o3-mini");
    }

    #[test]
    fn test_default_model() {
        let provider = OpenAiProvider::new();
        assert_eq!(provider.default_model(), "gpt-4o");
    }
}
