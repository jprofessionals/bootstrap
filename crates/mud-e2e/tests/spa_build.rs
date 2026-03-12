use std::time::Duration;

use mud_e2e::harness::{poll_until_contains, TestServer};
use serde_json::json;

/// Test SPA build and static asset serving via /project/ paths.
///
/// The area template already includes SPA source files (web/src/). This test
/// enables SPA mode by updating mud_web.rb, commits (triggering a build), and
/// verifies that the built HTML and JS assets are served with the correct base URL.
#[tokio::test]
async fn spa_build_and_serve() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    // Pull workspace
    server
        .client
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "alice/alice"}))
        .send()
        .await
        .unwrap();

    // Enable SPA mode by updating mud_web.rb
    let mud_web = "web_mode :spa\n";
    let resp = server
        .client
        .put(server.url("/api/editor/files/mud_web.rb?repo=alice/alice"))
        .json(&json!({"content": mud_web}))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "updating mud_web.rb failed: {}",
        resp.status()
    );

    // Commit triggers SPA build for @dev branch
    let resp = server
        .client
        .post(server.url("/git/api/repos/alice/alice/commit"))
        .json(&json!({"message": "Enable SPA mode"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "commit should succeed");

    // Poll until the SPA is built and served at @dev path.
    // The built index.html should contain the __MUD__ global and our app content.
    let body = poll_until_contains(
        &server.client,
        &server.url("/project/alice/alice@dev/"),
        "__MUD__",
        Duration::from_secs(60),
    )
    .await;

    // Verify the base URL is correctly set to /project/ (not /builder/)
    assert!(
        body.contains("/project/alice/alice@dev/"),
        "index.html should have /project/ base URL, got: {}",
        &body[..body.len().min(500)]
    );
    assert!(
        !body.contains("/builder/"),
        "index.html should NOT reference /builder/"
    );

    // Extract a JS asset path from the HTML and verify it loads
    // Vite generates paths like: /project/alice/alice@dev/assets/main-HASH.js
    let asset_prefix = "/project/alice/alice@dev/assets/";
    let asset_start = body.find(asset_prefix);
    assert!(
        asset_start.is_some(),
        "HTML should reference assets at {asset_prefix}"
    );

    // Extract the full asset path (up to the closing quote)
    let rest = &body[asset_start.unwrap()..];
    let end = rest
        .find('"')
        .or_else(|| rest.find('\''))
        .unwrap_or(rest.len());
    let asset_path = &rest[..end];

    // Fetch the JS asset and verify it returns 200
    let resp = server
        .client
        .get(server.url(asset_path))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "asset at {asset_path} should return 200, got {}",
        resp.status()
    );
}
