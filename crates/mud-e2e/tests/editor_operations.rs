use mud_e2e::harness::TestServer;
use serde_json::json;

#[tokio::test]
async fn editor_operations() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    // 1. Pull workspace
    let resp = server
        .client
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "alice/alice"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 2. List files
    let resp = server
        .client
        .get(server.url("/api/editor/files?repo=alice/alice"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("entrance.rb"), "should have default entrance file");

    // 3. Read a file
    let resp = server
        .client
        .get(server.url(
            "/api/editor/files/rooms/entrance.rb?repo=alice/alice",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("Entrance"), "should contain Entrance class");

    // 4. Create a new file
    let resp = server
        .client
        .post(server.url(
            "/api/editor/files/rooms/treasure_room.rb?repo=alice/alice",
        ))
        .json(&json!({"content": "class TreasureRoom < Room\n  def setup\n    self.short = 'The Treasure Room'\n  end\nend\n"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "creating a new file should return 201");

    // 5. Read back the created file
    let resp = server
        .client
        .get(server.url(
            "/api/editor/files/rooms/treasure_room.rb?repo=alice/alice",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("TreasureRoom"));

    // 6. Update the file
    let resp = server
        .client
        .put(server.url(
            "/api/editor/files/rooms/treasure_room.rb?repo=alice/alice",
        ))
        .json(&json!({"content": "class TreasureRoom < Room\n  def setup\n    self.short = 'The Grand Treasure Room'\n  end\nend\n"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 7. Verify the update
    let resp = server
        .client
        .get(server.url(
            "/api/editor/files/rooms/treasure_room.rb?repo=alice/alice",
        ))
        .send()
        .await
        .unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains("Grand Treasure Room"));

    // 8. Delete the file
    let resp = server
        .client
        .delete(server.url(
            "/api/editor/files/rooms/treasure_room.rb?repo=alice/alice",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 9. Verify deletion
    let resp = server
        .client
        .get(server.url(
            "/api/editor/files/rooms/treasure_room.rb?repo=alice/alice",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404, "deleted file should return 404");
}
