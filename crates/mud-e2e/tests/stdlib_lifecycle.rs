use mud_e2e::harness::TestServer;

#[tokio::test]
async fn stdlib_lifecycle() {
    let server = TestServer::start().await;

    // 1. Login page (ERB template) renders correctly
    let resp = server
        .client
        .get(server.url("/account/login"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("username"), "login form should have username field");
    assert!(body.contains("password"), "login form should have password field");
    assert!(body.contains("<form"), "login page should have a form");

    // 2. Register page (ERB template) renders correctly
    let resp = server
        .client
        .get(server.url("/account/register"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("username"),
        "register form should have username field"
    );
    assert!(
        body.contains("character"),
        "register form should have character field"
    );

    // 3. Register and verify the characters page renders
    server
        .register_user(&server.client, "alice", "secret123", "Warrior")
        .await;
    let resp = server
        .client
        .get(server.url("/account/characters"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("Warrior"), "characters page should show character");
    assert!(body.contains("<"), "should be rendered HTML, not raw ERB");
    assert!(
        !body.contains("<%"),
        "should not contain unrendered ERB tags"
    );

    // 4. Root page renders
    let resp = server.client.get(server.url("/")).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    // 5. Git dashboard renders (ERB)
    let resp = server
        .client
        .get(server.url("/git/"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<"), "git dashboard should be rendered HTML");

    // 6. Editor dashboard renders (ERB)
    let resp = server
        .client
        .get(server.url("/editor/"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}
