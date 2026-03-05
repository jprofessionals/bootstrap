use std::time::Duration;

use mud_e2e::harness::TestServer;
use serde_json::json;
use testcontainers::core::{IntoContainerPort, Mount};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

/// Test AI streaming through a custom provider backed by a wiremock container.
///
/// Flow:
/// 1. Start wiremock container with a canned OpenAI SSE response
/// 2. Register user, store an API key for the built-in "anthropic" provider
/// 3. Create a custom provider (api_mode: "openai") pointing at wiremock
/// 4. Stream a request via the custom provider
/// 5. Verify SSE events arrive (translated to Anthropic format)
#[tokio::test]
async fn ai_custom_provider_streaming() {
    let server = TestServer::start().await;

    // Write wiremock mappings to a temp dir
    let mappings_dir = tempfile::tempdir().expect("create mappings dir");
    let mappings_path = mappings_dir.path().join("mappings");
    std::fs::create_dir_all(&mappings_path).unwrap();

    // Wiremock stub: respond to POST /v1/chat/completions with SSE chunks
    let sse_body = [
        "data: {\"id\":\"chatcmpl-mock\",\"model\":\"mock-model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-mock\",\"model\":\"mock-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-mock\",\"model\":\"mock-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    ]
    .join("");

    let mapping = json!({
        "request": {
            "method": "POST",
            "urlPathPattern": "/v1/chat/completions"
        },
        "response": {
            "status": 200,
            "headers": {
                "Content-Type": "text/event-stream",
                "Cache-Control": "no-cache"
            },
            "body": sse_body
        }
    });

    std::fs::write(
        mappings_path.join("openai_chat.json"),
        serde_json::to_string_pretty(&mapping).unwrap(),
    )
    .unwrap();

    // Start wiremock container on the same Docker network
    let wiremock_name = format!("wiremock-{}", std::process::id());
    let wiremock = GenericImage::new("wiremock/wiremock", "latest")
        .with_exposed_port(8080_u16.tcp())
        .with_network(&server.network_name)
        .with_container_name(&wiremock_name)
        .with_mount(Mount::bind_mount(
            mappings_dir.path().to_str().unwrap(),
            "/home/wiremock",
        ))
        .start()
        .await
        .expect("start wiremock container");

    // Wait for wiremock to be ready
    let wiremock_port: u16 = wiremock
        .get_host_port_ipv4(8080_u16.tcp())
        .await
        .expect("get wiremock port");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::time::Instant::now() > deadline {
            panic!("Wiremock did not become ready");
        }
        if let Ok(resp) = reqwest::get(format!(
            "http://127.0.0.1:{wiremock_port}/__admin/mappings"
        ))
        .await
        {
            if resp.status().is_success() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // Register a user
    server
        .register_user(&server.client, "alice", "secret123", "Mage")
        .await;

    // Create a custom provider pointing at wiremock
    let resp = server
        .client
        .post(server.url("/api/ai/custom-provider"))
        .json(&json!({
            "name": "Mock LLM",
            "base_url": format!("http://{wiremock_name}:8080"),
            "api_mode": "openai",
            "api_key": "mock-key-123"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "should create custom provider, got: {}",
        resp.text().await.unwrap_or_default()
    );

    // Check the provider status endpoint includes our custom provider
    let resp = server
        .client
        .get(server.url("/api/ai/apikey"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let status: serde_json::Value = resp.json().await.unwrap();
    assert!(
        !status["custom"].as_array().unwrap().is_empty(),
        "should have at least one custom provider"
    );
    let custom = &status["custom"][0];
    assert_eq!(custom["name"], "Mock LLM");
    assert_eq!(custom["api_mode"], "openai");
    assert_eq!(custom["enabled"], true);
    let custom_id = custom["id"].as_i64().unwrap();

    // Stream a request through the custom provider
    let resp = server
        .client
        .post(server.url("/api/ai/stream"))
        .json(&json!({
            "messages": [{"role": "user", "content": "Say hello"}],
            "provider": format!("custom:{custom_id}"),
            "max_tokens": 100
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "stream should succeed, got: {}",
        resp.status()
    );

    // Read the SSE response body
    let body = resp.text().await.unwrap();

    // The response should contain Anthropic-format events (translated from OpenAI)
    assert!(
        body.contains("message_start"),
        "should contain message_start event, got: {body}"
    );
    assert!(
        body.contains("content_block_delta"),
        "should contain content_block_delta events"
    );
    assert!(
        body.contains("Hello"),
        "should contain 'Hello' text delta"
    );
    assert!(
        body.contains("world"),
        "should contain 'world' text delta"
    );
    assert!(
        body.contains("message_stop"),
        "should contain message_stop event"
    );

    // Clean up wiremock container
    drop(wiremock);
}

/// Test that the built-in provider toggle works (enable/disable).
#[tokio::test]
async fn ai_provider_toggle() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "bob", "secret123", "Warrior")
        .await;

    // Initially no providers configured
    let resp = server
        .client
        .get(server.url("/api/ai/apikey"))
        .send()
        .await
        .unwrap();
    let status: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status["providers"]["anthropic"]["configured"], false);
    assert_eq!(status["providers"]["anthropic"]["enabled"], false);

    // Store an API key (auto-enables)
    let resp = server
        .client
        .post(server.url("/api/ai/apikey"))
        .json(&json!({"api_key": "sk-ant-test-key", "provider": "anthropic"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Check it's now configured and enabled
    let resp = server
        .client
        .get(server.url("/api/ai/apikey"))
        .send()
        .await
        .unwrap();
    let status: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status["providers"]["anthropic"]["configured"], true);
    assert_eq!(status["providers"]["anthropic"]["enabled"], true);

    // Toggle it off
    let resp = server
        .client
        .post(server.url("/api/ai/provider/toggle"))
        .json(&json!({"provider": "anthropic", "enabled": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify it's disabled
    let resp = server
        .client
        .get(server.url("/api/ai/apikey"))
        .send()
        .await
        .unwrap();
    let status: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status["providers"]["anthropic"]["configured"], true);
    assert_eq!(status["providers"]["anthropic"]["enabled"], false);

    // Streaming should fail when provider is disabled
    let resp = server
        .client
        .post(server.url("/api/ai/stream"))
        .json(&json!({
            "messages": [{"role": "user", "content": "test"}],
            "provider": "anthropic"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        400,
        "should reject streaming to disabled provider"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("not enabled"));

    // Toggle back on
    let resp = server
        .client
        .post(server.url("/api/ai/provider/toggle"))
        .json(&json!({"provider": "anthropic", "enabled": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Delete the key
    let resp = server
        .client
        .delete(server.url("/api/ai/apikey"))
        .json(&json!({"provider": "anthropic"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify it's gone
    let resp = server
        .client
        .get(server.url("/api/ai/apikey"))
        .send()
        .await
        .unwrap();
    let status: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status["providers"]["anthropic"]["configured"], false);
}

/// Test custom provider CRUD operations.
#[tokio::test]
async fn ai_custom_provider_crud() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "carol", "secret123", "Thief")
        .await;

    // Create a custom provider
    let resp = server
        .client
        .post(server.url("/api/ai/custom-provider"))
        .json(&json!({
            "name": "My Ollama",
            "base_url": "http://localhost:11434",
            "api_mode": "ollama",
            "api_key": "optional-key"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let created: serde_json::Value = resp.json().await.unwrap();
    let provider_id = created["id"].as_i64().unwrap();
    assert_eq!(created["name"], "My Ollama");
    assert_eq!(created["api_mode"], "ollama");
    assert_eq!(created["enabled"], true);

    // List should include it
    let resp = server
        .client
        .get(server.url("/api/ai/apikey"))
        .send()
        .await
        .unwrap();
    let status: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status["custom"].as_array().unwrap().len(), 1);

    // Update the provider (rename, toggle off)
    let resp = server
        .client
        .post(server.url(&format!("/api/ai/custom-provider/{provider_id}")))
        .json(&json!({"name": "Renamed Ollama", "enabled": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify the update
    let resp = server
        .client
        .get(server.url("/api/ai/apikey"))
        .send()
        .await
        .unwrap();
    let status: serde_json::Value = resp.json().await.unwrap();
    let custom = &status["custom"][0];
    assert_eq!(custom["name"], "Renamed Ollama");
    assert_eq!(custom["enabled"], false);

    // Delete the provider
    let resp = server
        .client
        .delete(server.url(&format!(
            "/api/ai/custom-provider/{provider_id}"
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify it's gone
    let resp = server
        .client
        .get(server.url("/api/ai/apikey"))
        .send()
        .await
        .unwrap();
    let status: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status["custom"].as_array().unwrap().len(), 0);
}
