use mud_e2e::harness::TestServer;
use serde_json::json;

#[tokio::test]
async fn game_session() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;

    // Pull workspace first
    server
        .client
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "alice/alice"}))
        .send()
        .await
        .unwrap();

    // 1. Set up area code via editor (entrance + garden rooms)
    let entrance = r#"class Entrance < Room
  title "The Entrance"
  description "A grand entrance hall. A doorway leads north."
  exit :north, to: "garden"
end
"#;
    let garden = r#"class Garden < Room
  title "The Garden"
  description "A peaceful garden with flowers."
  exit :south, to: "entrance"
end
"#;

    server
        .client
        .put(server.url(
            "/api/editor/files/rooms/entrance.rb?repo=alice/alice",
        ))
        .json(&json!({"content": entrance}))
        .send()
        .await
        .unwrap();

    server
        .client
        .post(server.url(
            "/api/editor/files/rooms/garden.rb?repo=alice/alice",
        ))
        .json(&json!({"content": garden}))
        .send()
        .await
        .unwrap();

    server
        .client
        .post(server.url("/git/api/repos/alice/alice/commit"))
        .json(&json!({"message": "Set up rooms for play testing"}))
        .send()
        .await
        .unwrap();

    // 2. Start play session
    let resp = server
        .client
        .post(server.url("/play/start"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let output = body["output"]
        .as_str()
        .expect("play response should have output");
    assert!(output.contains("Entrance"), "should start in the entrance");

    // 3. Look command
    let resp = server
        .client
        .post(server.url("/play/command"))
        .json(&json!({"input": "look"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let output = body["output"].as_str().unwrap();
    assert!(output.contains("Entrance"));

    // 4. Move north
    let resp = server
        .client
        .post(server.url("/play/command"))
        .json(&json!({"input": "north"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let output = body["output"].as_str().unwrap();
    assert!(
        output.contains("Garden"),
        "should be in the garden after moving north"
    );

    // 5. Move south (back to entrance)
    let resp = server
        .client
        .post(server.url("/play/command"))
        .json(&json!({"input": "south"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let output = body["output"].as_str().unwrap();
    assert!(output.contains("Entrance"));

    // 6. Help command
    let resp = server
        .client
        .post(server.url("/play/command"))
        .json(&json!({"input": "help"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let output = body["output"].as_str().unwrap();
    assert!(
        output.contains("Available commands") || output.contains("help"),
        "help should list available commands"
    );

    // 7. Who command
    let resp = server
        .client
        .post(server.url("/play/command"))
        .json(&json!({"input": "who"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let output = body["output"].as_str().unwrap();
    assert!(
        output.contains("Players online") || output.contains("Warrior"),
        "who should show online players"
    );

    // 8. Unauthenticated access to play rejected
    let anon = server.new_client();
    let resp = anon.get(server.url("/play/")).send().await.unwrap();
    assert_eq!(resp.status(), 302, "anonymous should be redirected");
    let location = resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.ends_with("/account/login"));
}
