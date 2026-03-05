use std::time::Duration;

use mud_e2e::harness::{poll_until_contains, TestServer};
use serde_json::json;

/// Test SPA build and static asset serving via /project/ paths.
///
/// Creates a minimal Vite SPA, commits (triggering a build), and verifies
/// that the built HTML and JS assets are served with the correct base URL.
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

    // Create SPA source files
    let package_json = r#"{
  "name": "test-spa",
  "private": true,
  "version": "0.0.0",
  "type": "module",
  "scripts": {
    "build": "vite build"
  },
  "devDependencies": {
    "vite": "^6.0.0"
  }
}"#;

    let vite_config = r#"import { defineConfig } from 'vite'
export default defineConfig({
  base: process.env.MUD_BASE_URL || './',
  build: {
    outDir: 'dist',
    emptyOutDir: true
  }
})"#;

    let index_html = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>Test SPA</title>
</head>
<body>
  <div id="app">SPA Loading...</div>
  <script type="module" src="./main.js"></script>
</body>
</html>"#;

    let main_js = r#"document.getElementById('app').innerHTML = '<h1>SPA Works!</h1>';"#;

    // Create the files via editor API
    for (path, content) in [
        ("web/src/package.json", package_json),
        ("web/src/vite.config.js", vite_config),
        ("web/src/index.html", index_html),
        ("web/src/main.js", main_js),
    ] {
        let resp = server
            .client
            .post(server.url(&format!(
                "/api/editor/files/{path}?repo=alice/alice"
            )))
            .json(&json!({"content": content}))
            .send()
            .await
            .unwrap();
        assert!(
            resp.status().is_success(),
            "creating {path} failed: {}",
            resp.status()
        );
    }

    // Commit triggers SPA build for @dev branch
    let resp = server
        .client
        .post(server.url("/git/api/repos/alice/alice/commit"))
        .json(&json!({"message": "Add SPA source files"}))
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
    let end = rest.find('"').or_else(|| rest.find('\'')).unwrap_or(rest.len());
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
