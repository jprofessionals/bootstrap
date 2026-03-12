use mud_e2e::harness::TestServer;
use serde_json::json;

#[tokio::test]
async fn access_control() {
    let server = TestServer::start().await;
    let anon = server.new_client();

    // --- Part 1: Unauthenticated access redirects to login ---

    let protected_paths = ["/git/", "/editor/", "/play/"];
    for path in &protected_paths {
        let resp = anon.get(server.url(path)).send().await.unwrap();
        assert_eq!(
            resp.status(),
            302,
            "GET {path} should redirect unauthenticated users"
        );
        let location = resp
            .headers()
            .get("location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            location.ends_with("/account/login"),
            "GET {path} should redirect to /account/login, got {location}"
        );
    }

    // --- Part 2: Cross-user isolation ---

    // Create alice and bob
    let alice = server.new_client();
    server
        .register_user(&alice, "alice", "secret123", "Warrior")
        .await;

    let bob = server.new_client();
    server
        .register_user(&bob, "bob", "bobpass123", "Wizard")
        .await;

    // Alice pulls her workspace (via Ruby portal endpoint)
    alice
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "alice/alice"}))
        .send()
        .await
        .unwrap();

    // Bob cannot access alice's git branches (Ruby portal checks access)
    let resp = bob
        .get(server.url("/git/api/repos/alice/alice/branches"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "bob should not access alice's branches");

    // Bob cannot access alice's git log
    let resp = bob
        .get(server.url("/git/api/repos/alice/alice/log"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "bob should not access alice's log");

    // Note: /api/editor/files does not enforce per-user access control yet
    // (TODO in editor_files.rs). Cross-user file isolation is tested through
    // the git endpoints above which do check ownership.

    // Bob CAN access his own repo via the portal pull + git
    bob.post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "bob/bob"}))
        .send()
        .await
        .unwrap();

    let resp = bob
        .get(server.url("/git/api/repos/bob/bob/branches"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "bob should access his own branches");

    // --- Part 3: @dev branch access control ---

    // Alice creates content for builder
    alice
        .put(server.url("/api/editor/files/rooms/entrance.rb?repo=alice/alice"))
        .json(&json!({"content": "class Entrance < Room\n  title \"Hall\"\nend\n"}))
        .send()
        .await
        .unwrap();
    alice
        .post(server.url("/git/api/repos/alice/alice/commit"))
        .json(&json!({"message": "Setup for access test"}))
        .send()
        .await
        .unwrap();

    // Bob cannot access alice's @dev builder (proxied to Ruby portal)
    let resp = bob
        .get(server.url("/project/alice/alice@dev/"))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == 403 || resp.status() == 302,
        "bob should not access alice's @dev builder, got {}",
        resp.status()
    );

    // Anonymous cannot access @dev builder (should redirect to login)
    let resp = anon
        .get(server.url("/project/alice/alice@dev/"))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == 302 || resp.status() == 403,
        "anonymous should be blocked from @dev, got {}",
        resp.status()
    );
}
