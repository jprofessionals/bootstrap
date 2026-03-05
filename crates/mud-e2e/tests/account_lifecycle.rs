use mud_e2e::harness::TestServer;

#[tokio::test]
async fn account_lifecycle() {
    let server = TestServer::start().await;

    // 1. Register page loads
    let resp = server.client.get(server.url("/account/register")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.to_lowercase().contains("register"), "register page should mention 'register'");

    // 2. Register a new account
    server.register_user(&server.client, "alice", "secret123", "Warrior").await;

    // 3. Authenticated access works (session cookie set by registration)
    let resp = server.client.get(server.url("/account/characters")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("Warrior"), "should see character name");

    // 4. Logout
    let resp = server.client.post(server.url("/account/logout")).send().await.unwrap();
    assert_eq!(resp.status(), 302);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert!(location.ends_with("/account/login"), "logout should redirect to login");

    // 5. After logout, authenticated pages redirect to login
    let resp = server.client.get(server.url("/account/characters")).send().await.unwrap();
    assert_eq!(resp.status(), 302);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert!(location.ends_with("/account/login"));

    // 6. Login with correct credentials
    let status = server.login_user(&server.client, "alice", "secret123").await;
    assert_eq!(status, 302, "successful login should redirect");

    // 7. Session restored after login
    let resp = server.client.get(server.url("/account/characters")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("Warrior"));

    // 8. Wrong password rejected
    let bad_client = server.new_client();
    let resp = bad_client
        .post(server.url("/account/login"))
        .form(&[("username", "alice"), ("password", "wrongpassword")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "failed login returns 200 with error message");
    let body = resp.text().await.unwrap();
    assert!(body.contains("Invalid credentials"), "should show error for wrong password");

    // 9. Nonexistent user rejected
    let resp = bad_client
        .post(server.url("/account/login"))
        .form(&[("username", "nonexistent"), ("password", "whatever")])
        .send()
        .await
        .unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains("Invalid credentials"), "should show error for unknown user");

    // 10. Duplicate registration rejected
    let dup_client = server.new_client();
    let resp = dup_client
        .post(server.url("/account/register"))
        .form(&[
            ("username", "alice"),
            ("password", "other123"),
            ("character", "Mage"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "duplicate registration returns 200 with error");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("already taken") || body.contains("Username already taken"),
        "should show duplicate error"
    );
}
