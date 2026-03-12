use serde::{Deserialize, Serialize};

/// Information about a model offered by a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

/// A fully-built HTTP request ready to send to a provider.
pub struct ProviderRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: serde_json::Value,
}

/// An SSE event in Anthropic format, ready to forward to the browser.
pub struct AnthropicSseEvent {
    pub event_type: String,
    pub data: String,
}

/// The common request format (Anthropic-style, from the browser).
#[derive(Debug, Deserialize)]
pub struct StreamRequest {
    pub messages: serde_json::Value,
    #[serde(default)]
    pub tools: Option<serde_json::Value>,
    #[serde(default)]
    pub system: Option<serde_json::Value>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub provider: Option<String>,
}

/// Trait for AI provider adapters.
pub trait AiProvider: Send + Sync {
    fn name(&self) -> &str;
    fn models(&self) -> Vec<ModelInfo>;
    fn default_model(&self) -> &str;
    fn build_request(&self, api_key: &str, req: &StreamRequest) -> ProviderRequest;
    fn translate_event(&mut self, event_type: &str, data: &str) -> Vec<AnthropicSseEvent>;
}

/// Get the appropriate provider adapter for the given name.
pub fn get_provider(name: &str) -> Option<Box<dyn AiProvider>> {
    match name {
        "anthropic" => Some(Box::new(anthropic::AnthropicProvider::new())),
        "openai" => Some(Box::new(openai::OpenAiProvider::new())),
        "gemini" => Some(Box::new(gemini::GeminiProvider::new())),
        _ => None,
    }
}

pub mod anthropic;
pub mod gemini;
pub mod openai;

#[cfg(test)]
mod integration_tests {
    use super::*;

    /// Helper: collect event types from a sequence of AnthropicSseEvents.
    fn event_types(events: &[AnthropicSseEvent]) -> Vec<&str> {
        events.iter().map(|e| e.event_type.as_str()).collect()
    }

    /// Helper: parse the JSON data of an event.
    fn parse_data(event: &AnthropicSseEvent) -> serde_json::Value {
        serde_json::from_str(&event.data).unwrap()
    }

    /// Standard request with system prompt, messages, and tools (Anthropic format).
    fn sample_request() -> StreamRequest {
        StreamRequest {
            messages: serde_json::json!([
                {"role": "user", "content": "What is the weather in Oslo?"}
            ]),
            tools: Some(serde_json::json!([
                {
                    "name": "get_weather",
                    "description": "Get current weather for a location",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "location": {"type": "string", "description": "City name"}
                        },
                        "required": ["location"]
                    }
                }
            ])),
            system: Some(serde_json::json!("You are a helpful weather assistant.")),
            model: None,
            max_tokens: Some(4096),
            provider: None,
        }
    }

    // =========================================================================
    // 1. Anthropic passthrough
    // =========================================================================

    #[test]
    fn anthropic_passthrough_request_body_preserved() {
        let provider = anthropic::AnthropicProvider::new();
        let req = sample_request();
        let result = provider.build_request("sk-ant-key", &req);

        // The body should contain the original messages, tools, and system verbatim
        assert_eq!(result.body["messages"], req.messages);
        assert_eq!(result.body["tools"], req.tools.unwrap());
        assert_eq!(
            result.body["system"],
            serde_json::json!("You are a helpful weather assistant.")
        );
        assert_eq!(result.body["stream"], true);
        assert_eq!(result.body["max_tokens"], 4096);
    }

    #[test]
    fn anthropic_passthrough_events_identical() {
        let mut provider = anthropic::AnthropicProvider::new();

        let test_events = vec![
            (
                "message_start",
                r#"{"type":"message_start","message":{"id":"msg_1","role":"assistant","content":[],"model":"claude-sonnet-4-5-20250929"}}"#,
            ),
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"The weather"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" in Oslo is sunny."}}"#,
            ),
            (
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
            (
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            ),
            ("message_stop", r#"{"type":"message_stop"}"#),
        ];

        for (event_type, data) in &test_events {
            let result = provider.translate_event(event_type, data);
            assert_eq!(
                result.len(),
                1,
                "Anthropic passthrough should emit exactly 1 event per input"
            );
            assert_eq!(result[0].event_type, *event_type);
            assert_eq!(result[0].data, *data);
        }
    }

    // =========================================================================
    // 2. OpenAI full cycle — text + tool call
    // =========================================================================

    #[test]
    fn openai_full_cycle_request_build() {
        let provider = openai::OpenAiProvider::new();
        let req = sample_request();
        let result = provider.build_request("sk-openai-key", &req);

        assert_eq!(result.url, "https://api.openai.com/v1/chat/completions");

        // System prompt should be first message
        let messages = result.body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(
            messages[0]["content"],
            "You are a helpful weather assistant."
        );
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "What is the weather in Oslo?");

        // Tools should be translated to OpenAI function format
        let tools = result.body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "get_weather");
        assert!(tools[0]["function"]["parameters"]["properties"]["location"].is_object());
    }

    #[test]
    fn openai_full_cycle_text_then_tool_call() {
        let mut provider = openai::OpenAiProvider::new();
        let mut all_events: Vec<AnthropicSseEvent> = Vec::new();

        // Chunk 1: role chunk with first text (OpenAI sends role on first chunk)
        let chunk1 = serde_json::json!({
            "id": "chatcmpl-abc123",
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Let me "}, "finish_reason": null}]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk1).unwrap()));

        // Chunk 2: more text
        let chunk2 = serde_json::json!({
            "id": "chatcmpl-abc123",
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"content": "check the weather."}, "finish_reason": null}]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk2).unwrap()));

        // Chunk 3: tool call start (id + name + empty args)
        let chunk3 = serde_json::json!({
            "id": "chatcmpl-abc123",
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"tool_calls": [{
                "index": 0,
                "id": "call_weather_1",
                "type": "function",
                "function": {"name": "get_weather", "arguments": ""}
            }]}, "finish_reason": null}]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk3).unwrap()));

        // Chunk 4: tool call args fragment 1
        let chunk4 = serde_json::json!({
            "id": "chatcmpl-abc123",
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"tool_calls": [{
                "index": 0,
                "function": {"arguments": "{\"loca"}
            }]}, "finish_reason": null}]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk4).unwrap()));

        // Chunk 5: tool call args fragment 2
        let chunk5 = serde_json::json!({
            "id": "chatcmpl-abc123",
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"tool_calls": [{
                "index": 0,
                "function": {"arguments": "tion\":\"Oslo\"}"}
            }]}, "finish_reason": null}]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk5).unwrap()));

        // Chunk 6: finish with tool_calls reason
        let chunk6 = serde_json::json!({
            "id": "chatcmpl-abc123",
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk6).unwrap()));

        // Chunk 7: [DONE]
        all_events.extend(provider.translate_event("message", "[DONE]"));

        // Verify the full event sequence
        let types = event_types(&all_events);
        assert_eq!(
            types,
            vec![
                // Chunk 1: first chunk triggers message_start, then text block opens
                "message_start",
                "content_block_start", // text block
                "content_block_delta", // "Let me "
                // Chunk 2: continuing text
                "content_block_delta", // "check the weather."
                // Chunk 3: tool call starts — close text block first
                "content_block_stop",  // close text block
                "content_block_start", // tool_use block
                // Chunk 4: args fragment
                "content_block_delta", // input_json_delta
                // Chunk 5: args fragment
                "content_block_delta", // input_json_delta
                // Chunk 6: finish
                "content_block_stop", // close tool block
                "message_delta",      // stop_reason: tool_use
                // Chunk 7: [DONE]
                "message_stop",
            ]
        );

        // Verify specific event contents
        // message_start has model
        let msg_start = parse_data(&all_events[0]);
        assert_eq!(msg_start["message"]["model"], "gpt-4o");
        assert_eq!(msg_start["message"]["role"], "assistant");

        // Text block start
        let text_start = parse_data(&all_events[1]);
        assert_eq!(text_start["content_block"]["type"], "text");
        assert_eq!(text_start["index"], 0);

        // Text deltas
        let delta1 = parse_data(&all_events[2]);
        assert_eq!(delta1["delta"]["text"], "Let me ");
        let delta2 = parse_data(&all_events[3]);
        assert_eq!(delta2["delta"]["text"], "check the weather.");

        // Tool block start
        let tool_start = parse_data(&all_events[5]);
        assert_eq!(tool_start["content_block"]["type"], "tool_use");
        assert_eq!(tool_start["content_block"]["id"], "call_weather_1");
        assert_eq!(tool_start["content_block"]["name"], "get_weather");
        assert_eq!(tool_start["index"], 1);

        // Tool args deltas
        let args1 = parse_data(&all_events[6]);
        assert_eq!(args1["delta"]["type"], "input_json_delta");
        assert_eq!(args1["delta"]["partial_json"], "{\"loca");
        let args2 = parse_data(&all_events[7]);
        assert_eq!(args2["delta"]["partial_json"], "tion\":\"Oslo\"}");

        // Stop reason
        let msg_delta = parse_data(&all_events[9]);
        assert_eq!(msg_delta["delta"]["stop_reason"], "tool_use");
    }

    #[test]
    fn openai_full_cycle_text_only() {
        let mut provider = openai::OpenAiProvider::new();
        let mut all_events: Vec<AnthropicSseEvent> = Vec::new();

        // Chunk 1: first text
        let chunk1 = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"delta": {"role": "assistant", "content": "Hello, "}, "finish_reason": null}]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk1).unwrap()));

        // Chunk 2: more text
        let chunk2 = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"delta": {"content": "how can I help?"}, "finish_reason": null}]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk2).unwrap()));

        // Chunk 3: finish
        let chunk3 = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"delta": {}, "finish_reason": "stop"}]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk3).unwrap()));

        // Chunk 4: [DONE]
        all_events.extend(provider.translate_event("message", "[DONE]"));

        let types = event_types(&all_events);
        assert_eq!(
            types,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );

        let msg_delta = parse_data(&all_events[5]);
        assert_eq!(msg_delta["delta"]["stop_reason"], "end_turn");
    }

    // =========================================================================
    // 3. Gemini full cycle — text + function call
    // =========================================================================

    #[test]
    fn gemini_full_cycle_request_build() {
        let provider = gemini::GeminiProvider::new();
        let req = sample_request();
        let result = provider.build_request("gemini-api-key", &req);

        assert!(result.url.contains("streamGenerateContent"));
        assert!(result.url.contains("gemini-2.5-flash"));

        // System instruction
        assert_eq!(
            result.body["systemInstruction"]["parts"][0]["text"],
            "You are a helpful weather assistant."
        );

        // Contents (messages)
        let contents = result.body["contents"].as_array().unwrap();
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(
            contents[0]["parts"][0]["text"],
            "What is the weather in Oslo?"
        );

        // Tools as functionDeclarations
        let tools = result.body["tools"].as_array().unwrap();
        let decls = tools[0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(decls[0]["name"], "get_weather");
        assert!(decls[0]["parameters"]["properties"]["location"].is_object());
    }

    #[test]
    fn gemini_full_cycle_text_then_function_call() {
        let mut provider = gemini::GeminiProvider::new();
        let mut all_events: Vec<AnthropicSseEvent> = Vec::new();

        // Chunk 1: text response
        let chunk1 = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Let me look up the weather for you."}],
                    "role": "model"
                }
            }]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk1).unwrap()));

        // Chunk 2: more text
        let chunk2 = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": " Checking now..."}],
                    "role": "model"
                }
            }]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk2).unwrap()));

        // Chunk 3: function call with finish
        let chunk3 = serde_json::json!({
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
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk3).unwrap()));

        // Verify the full event sequence
        let types = event_types(&all_events);
        assert_eq!(
            types,
            vec![
                // Chunk 1: first chunk
                "message_start",
                "content_block_start", // text block (index 0)
                "content_block_delta", // text delta
                // Chunk 2: more text
                "content_block_delta", // text delta
                // Chunk 3: function call — close text, emit tool, finish
                "content_block_stop",  // close text block
                "content_block_start", // tool_use block (index 1)
                "content_block_delta", // input_json_delta
                "content_block_stop",  // close tool block
                "message_delta",       // stop_reason: tool_use
                "message_stop",
            ]
        );

        // Verify content details
        let msg_start = parse_data(&all_events[0]);
        assert_eq!(msg_start["message"]["role"], "assistant");

        let text_start = parse_data(&all_events[1]);
        assert_eq!(text_start["content_block"]["type"], "text");
        assert_eq!(text_start["index"], 0);

        let text_delta1 = parse_data(&all_events[2]);
        assert_eq!(
            text_delta1["delta"]["text"],
            "Let me look up the weather for you."
        );

        let text_delta2 = parse_data(&all_events[3]);
        assert_eq!(text_delta2["delta"]["text"], " Checking now...");

        // Tool block
        let tool_start = parse_data(&all_events[5]);
        assert_eq!(tool_start["content_block"]["type"], "tool_use");
        assert_eq!(tool_start["content_block"]["name"], "get_weather");
        assert_eq!(tool_start["index"], 1);

        let tool_delta = parse_data(&all_events[6]);
        assert_eq!(tool_delta["delta"]["type"], "input_json_delta");
        let args: serde_json::Value =
            serde_json::from_str(tool_delta["delta"]["partial_json"].as_str().unwrap()).unwrap();
        assert_eq!(args["location"], "Oslo");

        // Stop reason
        let msg_delta = parse_data(&all_events[8]);
        assert_eq!(msg_delta["delta"]["stop_reason"], "tool_use");
    }

    #[test]
    fn gemini_full_cycle_text_only() {
        let mut provider = gemini::GeminiProvider::new();
        let mut all_events: Vec<AnthropicSseEvent> = Vec::new();

        // Chunk 1: text
        let chunk1 = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "The weather in Oslo is sunny, 20C."}],
                    "role": "model"
                }
            }]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk1).unwrap()));

        // Chunk 2: finish with text
        let chunk2 = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": " Have a great day!"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        });
        all_events
            .extend(provider.translate_event("message", &serde_json::to_string(&chunk2).unwrap()));

        let types = event_types(&all_events);
        assert_eq!(
            types,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );

        let msg_delta = parse_data(&all_events[5]);
        assert_eq!(msg_delta["delta"]["stop_reason"], "end_turn");
    }

    // =========================================================================
    // 4. Cross-provider consistency
    // =========================================================================

    #[test]
    fn cross_provider_text_only_produces_same_event_type_sequence() {
        // All providers should produce the same Anthropic event type sequence
        // for a simple text-only response.

        let expected_types = vec![
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop",
        ];

        // --- Anthropic (passthrough) ---
        let mut anthropic = anthropic::AnthropicProvider::new();
        let anthropic_events_data = vec![
            (
                "message_start",
                r#"{"type":"message_start","message":{"id":"msg_1","role":"assistant","content":[],"model":"claude-sonnet-4-5-20250929"}}"#,
            ),
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello "}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world"}}"#,
            ),
            (
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
            (
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            ),
            ("message_stop", r#"{"type":"message_stop"}"#),
        ];
        let mut anthro_all = Vec::new();
        for (et, data) in anthropic_events_data {
            anthro_all.extend(anthropic.translate_event(et, data));
        }
        assert_eq!(
            event_types(&anthro_all),
            expected_types,
            "Anthropic event types mismatch"
        );

        // --- OpenAI ---
        let mut openai = openai::OpenAiProvider::new();
        let mut openai_all = Vec::new();
        let oai_chunks = vec![
            serde_json::json!({"model":"gpt-4o","choices":[{"delta":{"role":"assistant","content":"Hello "},"finish_reason":null}]}),
            serde_json::json!({"model":"gpt-4o","choices":[{"delta":{"content":"world"},"finish_reason":null}]}),
            serde_json::json!({"model":"gpt-4o","choices":[{"delta":{},"finish_reason":"stop"}]}),
        ];
        for chunk in &oai_chunks {
            openai_all
                .extend(openai.translate_event("message", &serde_json::to_string(chunk).unwrap()));
        }
        openai_all.extend(openai.translate_event("message", "[DONE]"));
        assert_eq!(
            event_types(&openai_all),
            expected_types,
            "OpenAI event types mismatch"
        );

        // --- Gemini ---
        let mut gemini = gemini::GeminiProvider::new();
        let mut gemini_all = Vec::new();
        let gem_chunks = vec![
            serde_json::json!({"candidates":[{"content":{"parts":[{"text":"Hello "}],"role":"model"}}]}),
            serde_json::json!({"candidates":[{"content":{"parts":[{"text":"world"}],"role":"model"},"finishReason":"STOP"}]}),
        ];
        for chunk in &gem_chunks {
            gemini_all
                .extend(gemini.translate_event("message", &serde_json::to_string(chunk).unwrap()));
        }
        assert_eq!(
            event_types(&gemini_all),
            expected_types,
            "Gemini event types mismatch"
        );
    }

    #[test]
    fn cross_provider_tool_use_produces_consistent_event_pattern() {
        // For a tool-use response, all providers should produce events following
        // the pattern: message_start → content blocks → message_delta(tool_use) → message_stop
        //
        // The exact sequence may differ (OpenAI has [DONE] separately, Gemini bundles
        // finish into the last chunk), but the core pattern should match.

        // --- OpenAI tool-only response ---
        let mut openai = openai::OpenAiProvider::new();
        let mut openai_all = Vec::new();
        let oai_chunks = vec![
            serde_json::json!({"model":"gpt-4o","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}),
            serde_json::json!({"model":"gpt-4o","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"location\":\"Oslo\"}"}}]},"finish_reason":null}]}),
            serde_json::json!({"model":"gpt-4o","choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
        ];
        for chunk in &oai_chunks {
            openai_all
                .extend(openai.translate_event("message", &serde_json::to_string(chunk).unwrap()));
        }
        openai_all.extend(openai.translate_event("message", "[DONE]"));

        // --- Gemini tool-only response ---
        let mut gemini = gemini::GeminiProvider::new();
        let mut gemini_all = Vec::new();
        let gem_chunk = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"functionCall": {"name": "get_weather", "args": {"location": "Oslo"}}}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        });
        gemini_all
            .extend(gemini.translate_event("message", &serde_json::to_string(&gem_chunk).unwrap()));

        // Both should start with message_start and end with message_delta + message_stop
        let oai_types = event_types(&openai_all);
        let gem_types = event_types(&gemini_all);

        assert_eq!(
            oai_types.first(),
            Some(&"message_start"),
            "OpenAI should start with message_start"
        );
        assert_eq!(
            gem_types.first(),
            Some(&"message_start"),
            "Gemini should start with message_start"
        );

        assert_eq!(
            oai_types.last(),
            Some(&"message_stop"),
            "OpenAI should end with message_stop"
        );
        assert_eq!(
            gem_types.last(),
            Some(&"message_stop"),
            "Gemini should end with message_stop"
        );

        // Both should have a message_delta with stop_reason: tool_use
        let oai_delta = openai_all
            .iter()
            .find(|e| e.event_type == "message_delta")
            .unwrap();
        let gem_delta = gemini_all
            .iter()
            .find(|e| e.event_type == "message_delta")
            .unwrap();
        let oai_data = parse_data(oai_delta);
        let gem_data = parse_data(gem_delta);
        assert_eq!(oai_data["delta"]["stop_reason"], "tool_use");
        assert_eq!(gem_data["delta"]["stop_reason"], "tool_use");

        // Both should have content_block_start with type: tool_use
        let oai_tool_start = openai_all
            .iter()
            .find(|e| e.event_type == "content_block_start" && e.data.contains("tool_use"))
            .unwrap();
        let gem_tool_start = gemini_all
            .iter()
            .find(|e| e.event_type == "content_block_start" && e.data.contains("tool_use"))
            .unwrap();
        let oai_ts = parse_data(oai_tool_start);
        let gem_ts = parse_data(gem_tool_start);
        assert_eq!(oai_ts["content_block"]["type"], "tool_use");
        assert_eq!(gem_ts["content_block"]["type"], "tool_use");
        assert_eq!(oai_ts["content_block"]["name"], "get_weather");
        assert_eq!(gem_ts["content_block"]["name"], "get_weather");

        // Both should have input_json_delta with the args
        let oai_args_event = openai_all
            .iter()
            .find(|e| e.data.contains("input_json_delta"))
            .unwrap();
        let gem_args_event = gemini_all
            .iter()
            .find(|e| e.data.contains("input_json_delta"))
            .unwrap();
        let oai_args_data = parse_data(oai_args_event);
        let gem_args_data = parse_data(gem_args_event);
        assert_eq!(oai_args_data["delta"]["type"], "input_json_delta");
        assert_eq!(gem_args_data["delta"]["type"], "input_json_delta");
    }

    #[test]
    fn cross_provider_all_build_valid_requests() {
        // All three providers should produce valid ProviderRequests from the same input
        let req = sample_request();

        let anthropic = anthropic::AnthropicProvider::new();
        let openai = openai::OpenAiProvider::new();
        let gemini = gemini::GeminiProvider::new();

        let a_req = anthropic.build_request("key-a", &req);
        let o_req = openai.build_request("key-o", &req);
        let g_req = gemini.build_request("key-g", &req);

        // All should have non-empty URLs
        assert!(!a_req.url.is_empty());
        assert!(!o_req.url.is_empty());
        assert!(!g_req.url.is_empty());

        // All should have at least an auth header and content-type
        assert!(a_req.headers.len() >= 2);
        assert!(o_req.headers.len() >= 2);
        assert!(g_req.headers.len() >= 2);

        // All should have stream=true (Anthropic and OpenAI in body, Gemini in URL)
        assert_eq!(a_req.body["stream"], true);
        assert_eq!(o_req.body["stream"], true);
        assert!(
            g_req.url.contains("stream") || g_req.body.get("stream").is_none(),
            "Gemini uses SSE via URL param, not body"
        );

        // All should include the user message content somewhere
        let a_str = serde_json::to_string(&a_req.body).unwrap();
        let o_str = serde_json::to_string(&o_req.body).unwrap();
        let g_str = serde_json::to_string(&g_req.body).unwrap();
        assert!(a_str.contains("What is the weather in Oslo?"));
        assert!(o_str.contains("What is the weather in Oslo?"));
        assert!(g_str.contains("What is the weather in Oslo?"));

        // All should include the tool name
        assert!(a_str.contains("get_weather"));
        assert!(o_str.contains("get_weather"));
        assert!(g_str.contains("get_weather"));

        // All should include the system prompt content
        assert!(a_str.contains("helpful weather assistant"));
        assert!(o_str.contains("helpful weather assistant"));
        assert!(g_str.contains("helpful weather assistant"));
    }
}
