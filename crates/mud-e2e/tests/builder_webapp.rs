use std::time::Duration;

use mud_e2e::harness::{poll_until_contains, TestServer};
use serde_json::json;

/// Test that area template rendering works via the Rust project routes.
#[tokio::test]
async fn builder_template_rendering() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    // Pull workspace (creates the @dev checkout and populates world dir)
    server
        .client
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "alice/alice"}))
        .send()
        .await
        .unwrap();

    // Create area files with room code
    let resp = server
        .client
        .put(server.url(
            "/api/editor/files/rooms/entrance.rb?repo=alice/alice",
        ))
        .json(&json!({"content": "class Entrance < Room\n  title \"The Entrance\"\n  description \"A grand entrance hall.\"\n  exit :north, to: \"garden\"\nend\n"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Commit the changes (triggers area reload which publishes to main)
    let resp = server
        .client
        .post(server.url("/git/api/repos/alice/alice/commit"))
        .json(&json!({"message": "Initial area setup"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Wait for Tera template rendering at /project/alice/alice/
    let body = poll_until_contains(
        &server.client,
        &server.url("/project/alice/alice/"),
        "alice",
        Duration::from_secs(15),
    )
    .await;
    assert!(body.contains("Rooms:"), "builder page should list rooms section");
}

/// Test @dev branch access control (proxied to Ruby portal's BuilderApp).
#[tokio::test]
async fn builder_dev_access_control() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    server
        .client
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "alice/alice"}))
        .send()
        .await
        .unwrap();

    // Commit so the area is loaded
    server
        .client
        .post(server.url("/git/api/repos/alice/alice/commit"))
        .json(&json!({"message": "Initial commit for access test"}))
        .send()
        .await
        .unwrap();

    // Anonymous user cannot access @dev branch
    let anon = server.new_client();
    let resp = anon
        .get(server.url("/project/alice/alice@dev/"))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == 302 || resp.status() == 403,
        "anonymous should be redirected or denied, got {}",
        resp.status()
    );

    // Authenticated owner can access @dev
    let resp = server
        .client
        .get(server.url("/project/alice/alice@dev/"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "owner should access @dev");

    // Another user cannot access alice's @dev branch
    let bob = server.new_client();
    server
        .register_user(&bob, "bob", "bobpass123", "Wizard")
        .await;
    let resp = bob
        .get(server.url("/project/alice/alice@dev/"))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == 403 || resp.status() == 302,
        "bob should not access alice's @dev branch, got {}",
        resp.status()
    );
}

/// Test Rack web app API routing through /project/ paths.
///
/// Requires: MUD::Stdlib::Web::RackApp base class.
#[tokio::test]
async fn builder_rack_webapp() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    server
        .client
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "alice/alice"}))
        .send()
        .await
        .unwrap();

    let rack_app = r#"class MudWeb < MUD::Stdlib::Web::RackApp
  route do |r|
    r.on "api" do
      r.get "hello" do
        response['Content-Type'] = 'application/json'
        '{"message":"Hello from Rack!"}'
      end

      r.post "echo" do
        body = JSON.parse(r.body.read)
        response['Content-Type'] = 'application/json'
        body.to_json
      end
    end
  end
end
"#;
    let resp = server
        .client
        .put(server.url("/api/editor/files/mud_web.rb?repo=alice/alice"))
        .json(&json!({"content": rack_app}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let resp = server
        .client
        .post(server.url("/git/api/repos/alice/alice/commit"))
        .json(&json!({"message": "Add Rack web app"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Test /api/hello JSON endpoint
    let body = poll_until_contains(
        &server.client,
        &server.url("/project/alice/alice@dev/api/hello"),
        "Hello from Rack",
        Duration::from_secs(15),
    )
    .await;
    let json_resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json_resp["message"], "Hello from Rack!");

    // Test /api/echo POST endpoint
    let resp = server
        .client
        .post(server.url("/project/alice/alice@dev/api/echo"))
        .json(&json!({"foo": "bar", "num": 42}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json_resp: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json_resp["foo"], "bar");
    assert_eq!(json_resp["num"], 42);
}
