use mud_e2e::harness::TestServer;
use serde_json::json;

#[tokio::test]
async fn git_workflow() {
    let server = TestServer::start().await;

    // 1. Register + login
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    // 2. Registration auto-creates area repo — verify via git API
    let resp = server
        .client
        .get(server.url("/git/api/repos"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("alice/alice"),
        "should have auto-created repo"
    );

    // 3. List branches
    let resp = server
        .client
        .get(server.url("/git/api/repos/alice/alice/branches"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("main"), "should have main branch");
    assert!(body.contains("develop"), "should have develop branch");

    // 4. View commit log
    let resp = server
        .client
        .get(server.url("/git/api/repos/alice/alice/log?branch=develop"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Initial area template"),
        "should have initial commit"
    );

    // 5. Check diff (should be empty initially)
    let resp = server
        .client
        .get(server.url("/git/api/repos/alice/alice/diff?branch=develop"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 6. Pull workspace
    let resp = server
        .client
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "alice/alice"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 7. Modify a file via editor API
    let resp = server
        .client
        .put(server.url("/api/editor/files/rooms/entrance.rb?repo=alice/alice"))
        .json(
            &json!({"content": "class Entrance < Room\n  title \"The Modified Entrance\"\nend\n"}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 8. Diff should now show changes
    let resp = server
        .client
        .get(server.url("/git/api/repos/alice/alice/diff?branch=develop"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("entrance.rb"),
        "diff should show modified file"
    );

    // 9. Commit the change
    let resp = server
        .client
        .post(server.url("/git/api/repos/alice/alice/commit"))
        .json(&json!({"message": "Update entrance room"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 10. Verify commit in log
    let resp = server
        .client
        .get(server.url("/git/api/repos/alice/alice/log?branch=develop"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Update entrance room"),
        "should see new commit"
    );

    // 11. Create a new branch
    let resp = server
        .client
        .post(server.url("/git/api/repos/alice/alice/branches"))
        .json(&json!({"name": "feature_quest"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // 12. Verify branch exists
    let resp = server
        .client
        .get(server.url("/git/api/repos/alice/alice/branches"))
        .send()
        .await
        .unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains("feature_quest"));

    // 13. Start play session and verify the modified entrance is live
    let resp = server
        .client
        .post(server.url("/play/start"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Modified Entrance"),
        "game should reflect code changes"
    );

    // 14. Multi-user access control: bob can't see alice's git data
    let bob = server.new_client();
    server
        .register_user(&bob, "bob", "bobpass123", "Wizard")
        .await;

    // Note: /api/editor/files does not enforce per-user ACL yet (TODO).
    // Test git endpoint which does check ownership via Ruby portal.
    let resp = bob
        .get(server.url("/git/api/repos/alice/alice/branches"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "bob should not access alice's branches");
}
