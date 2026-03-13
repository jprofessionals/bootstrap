use std::time::Duration;

use mud_e2e::harness::TestServer;
use serde_json::json;

/// Test that the JVM adapter connects successfully alongside the Ruby adapter.
///
/// Verifies:
/// 1. Both adapters complete MOP handshake (server won't start otherwise)
/// 2. Ruby portal still works with JVM adapter running
/// 3. Default area uses the Ruby template (not a JVM template)
#[tokio::test]
async fn jvm_adapter_connects_with_ruby() {
    let server = TestServer::start().await;

    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    let status = server
        .login_user(&server.client, "alice", "secret123")
        .await;
    assert_eq!(status, 302, "login should redirect (302)");

    // Default area should use Ruby template, not JVM
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
        !files.contains(&"build.gradle.kts"),
        "default area should NOT use JVM template, got: {files:?}"
    );
}

/// Test that JVM area templates are registered and available.
///
/// The JVM launcher sends kotlin:ktor, kotlin:quarkus, and kotlin:spring-boot
/// templates during handshake. Verify they appear in the templates API.
#[tokio::test]
async fn jvm_templates_registered() {
    let server = TestServer::start().await;

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
        names.contains(&"kotlin:ktor"),
        "JVM 'kotlin:ktor' template should be registered, got: {names:?}"
    );
    assert!(
        names.contains(&"kotlin:quarkus"),
        "JVM 'kotlin:quarkus' template should be registered, got: {names:?}"
    );
    assert!(
        names.contains(&"kotlin:spring-boot"),
        "JVM 'kotlin:spring-boot' template should be registered, got: {names:?}"
    );
}

/// Test creating a repository from a JVM template and verifying its contents.
///
/// Creates a repo using the kotlin:ktor template, pulls the workspace,
/// then checks that the files match the expected Kotlin/Gradle structure
/// with correct placeholder substitution.
#[tokio::test]
async fn jvm_template_area_creation() {
    let server = TestServer::start().await;

    // Register a user so we have auth context
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    // Create a repo from the kotlin:ktor template
    let resp = server
        .client
        .post(server.url("/api/repos/create"))
        .json(&json!({
            "namespace": "alice",
            "name": "myktor",
            "template": "kotlin:ktor"
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
        .json(&json!({"repo": "alice/myktor"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "workspace pull should succeed");

    // List files — should be Kotlin/Gradle files, not Ruby
    let resp = server
        .client
        .get(server.url("/api/editor/files?repo=alice/myktor"))
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

    // Verify JVM template structure
    assert!(
        files.contains(&"build.gradle.kts"),
        "JVM area should have build.gradle.kts, got: {files:?}"
    );
    assert!(
        files.contains(&"settings.gradle.kts"),
        "JVM area should have settings.gradle.kts, got: {files:?}"
    );
    assert!(
        files.contains(&"src/main/kotlin/MudArea.kt"),
        "JVM area should have MudArea.kt, got: {files:?}"
    );
    assert!(
        files.contains(&"mud.yaml"),
        "JVM area should have mud.yaml, got: {files:?}"
    );

    // Verify it does NOT contain Ruby template files
    assert!(
        !files.contains(&"mud_web.rb"),
        "JVM area should not have Ruby files, got: {files:?}"
    );
    assert!(
        !files.contains(&"mud_loader.rb"),
        "JVM area should not have Ruby files, got: {files:?}"
    );

    // Verify template placeholder substitution in a Kotlin source file
    let resp = server
        .client
        .get(server.url("/api/editor/files/src/main/kotlin/MudArea.kt?repo=alice/myktor"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let file_body: serde_json::Value = resp.json().await.unwrap();
    let content = file_body["content"].as_str().unwrap();
    assert!(
        !content.contains("{{namespace}}") && !content.contains("{{area_name}}"),
        "template placeholders should be substituted in MudArea.kt"
    );

    // Verify mud.yaml references ktor framework
    let resp = server
        .client
        .get(server.url("/api/editor/files/mud.yaml?repo=alice/myktor"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let file_body: serde_json::Value = resp.json().await.unwrap();
    let content = file_body["content"].as_str().unwrap();
    assert!(
        content.contains("ktor"),
        "mud.yaml should reference ktor framework, got: {content}"
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
                 Check driver/JVM logs above for build or migration errors.",
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

/// Create a kotlin:ktor area repo, commit to trigger reload, and return the
/// TestServer. Caller must have already registered a user named "alice".
async fn create_and_load_ktor_area(server: &TestServer, area_name: &str) {
    // Create repo from kotlin:ktor template
    let resp = server
        .client
        .post(server.url("/api/repos/create"))
        .json(&json!({
            "namespace": "alice",
            "name": area_name,
            "template": "kotlin:ktor"
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
        .json(&json!({"repo": format!("alice/{area_name}")}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "workspace pull should succeed");

    // Commit triggers area_reload via Ruby portal → routed to JVM adapter
    let resp = server
        .client
        .post(server.url(&format!("/git/api/repos/alice/{area_name}/commit")))
        .json(&json!({"message": "Initial ktor area"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "commit should succeed");
}

/// Test that a Ktor area loads successfully after commit.
///
/// Exercises the full flow:
/// 1. Create repo from kotlin:ktor template
/// 2. Commit (triggers area_reload)
/// 3. Language-aware routing sends ReloadArea to JVM adapter
/// 4. JVM adapter: Gradle build → spawn child → FlywayRunner → AreaProcess
/// 5. Area reports loaded via area_loaded message
/// 6. Template web page is served from the area's web/templates/
#[tokio::test]
async fn jvm_area_loads_and_serves_template() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    create_and_load_ktor_area(&server, "myktor").await;

    // Wait for the JVM area to finish building and loading (Gradle + child process)
    wait_for_area_loaded(&server, "alice/myktor", Duration::from_secs(180)).await;

    // The area's web/templates/index.html should be served via Tera rendering
    let resp = server
        .client
        .get(server.url("/project/alice/myktor/"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "template web page should be served for loaded area"
    );
    let body = resp.text().await.unwrap();
    // The Tera template renders {{area_name}} and {{namespace}}
    assert!(
        body.contains("myktor") || body.contains("alice"),
        "template should contain area name or namespace, got: {}",
        &body[..body.len().min(500)]
    );
}

/// Test that JVM database migrations run successfully when an area loads.
///
/// Creates a Ktor area, adds a migration SQL file, commits, and verifies
/// the area loads without errors (FlywayRunner ran the migration).
#[tokio::test]
async fn jvm_migrations_run_on_load() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    // Create ktor area
    let resp = server
        .client
        .post(server.url("/api/repos/create"))
        .json(&json!({
            "namespace": "alice",
            "name": "migtest",
            "template": "kotlin:ktor"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Pull workspace
    let resp = server
        .client
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "alice/migtest"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Add a migration SQL file
    let migration_sql = "CREATE TABLE IF NOT EXISTS test_items (\n\
        id SERIAL PRIMARY KEY,\n\
        name VARCHAR(255) NOT NULL,\n\
        created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP\n\
    );\n";
    let resp =
        server
            .client
            .post(server.url(
                "/api/editor/files/db/migrations/V1__create_test_items.sql?repo=alice/migtest",
            ))
            .json(&json!({"content": migration_sql}))
            .send()
            .await
            .unwrap();
    assert!(
        resp.status().is_success(),
        "adding migration file should succeed: {}",
        resp.status()
    );

    // Commit triggers area_reload → JVM adapter → Gradle build → FlywayRunner
    let resp = server
        .client
        .post(server.url("/git/api/repos/alice/migtest/commit"))
        .json(&json!({"message": "Add migration"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "commit should succeed");

    // Wait for area to load — if migration fails, area_error is sent instead
    // of area_loaded, so this would time out.
    wait_for_area_loaded(&server, "alice/migtest", Duration::from_secs(180)).await;

    // Area loaded successfully — FlywayRunner ran V1__create_test_items.sql
    // without errors. The template page should also be served.
    let resp = server
        .client
        .get(server.url("/project/alice/migtest/"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "template page should be served after successful migration"
    );
}

/// Test that a Ktor area starts a web server and the driver proxies API requests.
///
/// This verifies the full JVM SPA backend flow:
/// 1. Area loads with framework: ktor
/// 2. AreaProcess starts embedded Ktor HTTP server
/// 3. AreaProcess sends register_area_web to driver
/// 4. Driver proxies /project/ns/name/api/* to the Ktor server
#[tokio::test]
async fn jvm_ktor_api_backend() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    create_and_load_ktor_area(&server, "apitest").await;

    // Wait for area to load
    wait_for_area_loaded(&server, "alice/apitest", Duration::from_secs(180)).await;

    // Wait for the web socket to be registered (might be slightly after area_loaded)
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::time::Instant::now() > deadline {
            panic!("Timed out waiting for area web socket registration");
        }
        if let Ok(resp) = server
            .client
            .get(server.url("/api/areas/status"))
            .send()
            .await
        {
            if resp.status() == 200 {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(sockets) = body["web_sockets"].as_array() {
                        if sockets.iter().any(|s| s.as_str() == Some("alice/apitest")) {
                            break;
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    // Now proxy an API request through the driver to the Ktor server
    let resp = server
        .client
        .get(server.url("/project/alice/apitest/api/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "API status endpoint should respond via proxy"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["framework"], "ktor");
    assert_eq!(body["area"], "alice/apitest");

    // Also check web-data endpoint
    let resp = server
        .client
        .get(server.url("/project/alice/apitest/api/web-data"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "web-data endpoint should respond");
}

/// Test that JVM templates are available via disk scanning even when the
/// JVM adapter process is not running.
///
/// The driver scans `bootstrap/jvm/templates/area/` at boot time and
/// registers kotlin:ktor, kotlin:quarkus, kotlin:spring-boot templates from
/// the base + overlay directories on disk.  This allows builders to create
/// JVM repos from the portal even if the JVM adapter is disabled.
#[tokio::test]
async fn jvm_templates_available_without_adapter() {
    // Start server with only Ruby adapter — no JVM adapter process
    let server = TestServer::start_ruby_only().await;

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

    // Ruby "default" template should still be registered
    assert!(
        names.contains(&"default"),
        "Ruby 'default' template should be registered, got: {names:?}"
    );

    // JVM templates should be registered via disk scanning
    assert!(
        names.contains(&"kotlin:ktor"),
        "Disk-scanned 'kotlin:ktor' template should be available, got: {names:?}"
    );
    assert!(
        names.contains(&"kotlin:quarkus"),
        "Disk-scanned 'kotlin:quarkus' template should be available, got: {names:?}"
    );
    assert!(
        names.contains(&"kotlin:spring-boot"),
        "Disk-scanned 'kotlin:spring-boot' template should be available, got: {names:?}"
    );

    // Verify the templates have actual file content (not empty)
    for template in body["templates"].as_array().unwrap() {
        let name = template["name"].as_str().unwrap();
        let count = template["file_count"].as_u64().unwrap();
        assert!(
            count > 0,
            "Template '{name}' should have files, got file_count={count}"
        );
    }
}
