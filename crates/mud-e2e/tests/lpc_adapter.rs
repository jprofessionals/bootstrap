use std::time::Duration;

use mud_e2e::harness::TestServer;
use serde_json::json;

/// Test that the LPC adapter connects successfully alongside Ruby and JVM adapters.
///
/// Verifies:
/// 1. All three adapters complete MOP handshake (server won't start otherwise)
/// 2. Ruby portal still works with LPC adapter running
/// 3. Default area still uses the Ruby template
#[tokio::test]
async fn lpc_adapter_connects_with_ruby() {
    let server = TestServer::start_with_lpc().await;

    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    let status = server
        .login_user(&server.client, "alice", "secret123")
        .await;
    assert_eq!(status, 302, "login should redirect (302)");

    // Default area should use Ruby template, not LPC
    let resp = server
        .client
        .get(server.url("/api/editor/files?repo=alice/alice"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let files: Vec<&str> = body["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        files.contains(&"mud_web.rb"),
        "default area should use Ruby template (has mud_web.rb), got: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f.ends_with(".c")),
        "default area should NOT have LPC .c files, got: {files:?}"
    );
}

/// Test that LPC area templates are registered and available.
///
/// The LPC adapter sends an "lpc" template during handshake.
/// Verify it appears in the templates API.
#[tokio::test]
async fn lpc_templates_registered() {
    let server = TestServer::start_with_lpc().await;

    let resp = server
        .client
        .get(server.url("/api/repos/templates"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let names: Vec<&str> = body["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();

    assert!(
        names.contains(&"default"),
        "Ruby 'default' template should be registered, got: {names:?}"
    );
    assert!(
        names.contains(&"lpc"),
        "LPC 'lpc' template should be registered, got: {names:?}"
    );
}

/// Test creating a repository from the LPC template and verifying its contents.
///
/// Creates a repo using the lpc template, pulls the workspace,
/// then checks that the files match the expected LPC structure
/// with correct placeholder substitution.
#[tokio::test]
async fn lpc_template_area_creation() {
    let server = TestServer::start_with_lpc().await;

    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    // Create a repo from the lpc template
    let resp = server
        .client
        .post(server.url("/api/repos/create"))
        .json(&json!({
            "namespace": "alice",
            "name": "lpcworld",
            "template": "lpc"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "repo creation should succeed: {}",
        resp.text().await.unwrap_or_default()
    );

    // Pull workspace to create the @dev checkout
    let resp = server
        .client
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "alice/lpcworld"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "workspace pull should succeed");

    // List files — should be LPC .c files, not Ruby
    let resp = server
        .client
        .get(server.url("/api/editor/files?repo=alice/lpcworld"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let files: Vec<&str> = body["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();

    // Verify LPC template structure
    assert!(
        files.contains(&"mud.yaml"),
        "LPC area should have mud.yaml, got: {files:?}"
    );
    assert!(
        files.iter().any(|f| f.ends_with(".c")),
        "LPC area should have .c files, got: {files:?}"
    );
    assert!(
        files.contains(&"rooms/entrance.c"),
        "LPC area should have rooms/entrance.c, got: {files:?}"
    );
    assert!(
        files.contains(&"daemons/area_daemon.c"),
        "LPC area should have daemons/area_daemon.c, got: {files:?}"
    );

    // Verify it does NOT contain Ruby template files
    assert!(
        !files.contains(&"mud_web.rb"),
        "LPC area should not have Ruby files, got: {files:?}"
    );

    // Verify mud.yaml references lpc language
    let resp = server
        .client
        .get(server.url("/api/editor/files/mud.yaml?repo=alice/lpcworld"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let file_body: serde_json::Value = resp.json().await.unwrap();
    let content = file_body["content"].as_str().unwrap();
    assert!(
        content.contains("lpc"),
        "mud.yaml should reference lpc language, got: {content}"
    );

    // Verify template placeholder substitution in an LPC source file
    let resp = server
        .client
        .get(server.url("/api/editor/files/rooms/entrance.c?repo=alice/lpcworld"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let file_body: serde_json::Value = resp.json().await.unwrap();
    let content = file_body["content"].as_str().unwrap();
    assert!(
        !content.contains("{{area_name}}"),
        "template placeholders should be substituted in entrance.c"
    );
}

// ---------------------------------------------------------------------------
// Helper: wait for an area to appear in the loaded_areas list
// ---------------------------------------------------------------------------

async fn wait_for_area_loaded(server: &TestServer, area_key: &str, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if tokio::time::Instant::now() > deadline {
            panic!(
                "Timed out waiting for area '{}' to load. \
                 Check driver/LPC logs above for compile or load errors.",
                area_key
            );
        }
        if let Ok(resp) = server
            .client
            .get(server.url("/api/areas/status"))
            .send()
            .await
        {
            if resp.status() == 200 {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(areas) = body["loaded_areas"].as_array() {
                        if areas.iter().any(|a| a.as_str() == Some(area_key)) {
                            return;
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Test that an LPC area loads successfully after commit.
///
/// Exercises the full flow:
/// 1. Create repo from lpc template
/// 2. Commit (triggers area_reload)
/// 3. Language-aware routing sends LoadArea to LPC adapter
/// 4. LPC adapter: compiles .c files, calls create()
/// 5. Area reports loaded via area_loaded message
#[tokio::test]
async fn lpc_area_loads_after_commit() {
    let server = TestServer::start_with_lpc().await;
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    // Create repo from lpc template
    let resp = server
        .client
        .post(server.url("/api/repos/create"))
        .json(&json!({
            "namespace": "alice",
            "name": "lpctest",
            "template": "lpc"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "repo creation should succeed: {}",
        resp.text().await.unwrap_or_default()
    );

    // Pull workspace
    let resp = server
        .client
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "alice/lpctest"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "workspace pull should succeed");

    // Commit triggers area_reload → routed to LPC adapter
    let resp = server
        .client
        .post(server.url("/git/api/repos/alice/lpctest/commit"))
        .json(&json!({"message": "Initial LPC area"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "commit should succeed");

    // Wait for the LPC area to compile and load (should be fast, no build step)
    wait_for_area_loaded(&server, "alice/lpctest", Duration::from_secs(30)).await;
}
