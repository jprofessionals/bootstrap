use std::time::Duration;

use mud_e2e::harness::TestServer;
use serde_json::json;

#[tokio::test]
async fn webapp_database() {
    let server = TestServer::start().await;
    server
        .register_user(&server.client, "bob", "secret123", "Mage")
        .await;

    // Pull workspace first
    server
        .client
        .post(server.url("/editor/api/pull"))
        .json(&json!({"repo": "bob/bob"}))
        .send()
        .await
        .unwrap();

    // 1. Create a Rack app with Sequel database access
    let mud_web = r#"class MudWeb < MUD::Stdlib::Web::RackApp
  route do |r|
    r.on "api" do
      r.on "notes" do
        r.get do
          notes = area_db[:notes].all
          response['Content-Type'] = 'application/json'
          notes.to_json
        end

        r.post do
          body = JSON.parse(r.body.read)
          id = area_db[:notes].insert(title: body['title'], content: body['content'])
          response['Content-Type'] = 'application/json'
          response.status = 201
          { id: id, title: body['title'], content: body['content'] }.to_json
        end
      end
    end
  end
end
"#;

    let migration = r#"Sequel.migration do
  change do
    create_table(:notes) do
      primary_key :id
      String :title, null: false
      String :content
      DateTime :created_at, default: Sequel::CURRENT_TIMESTAMP
    end
  end
end
"#;

    // 2. Create the files
    server
        .client
        .put(server.url("/api/editor/files/mud_web.rb?repo=bob/bob"))
        .json(&json!({"content": mud_web}))
        .send()
        .await
        .unwrap();

    server
        .client
        .post(server.url("/api/editor/files/db/migrations/001_create_notes.rb?repo=bob/bob"))
        .json(&json!({"content": migration}))
        .send()
        .await
        .unwrap();

    server
        .client
        .post(server.url("/git/api/repos/bob/bob/commit"))
        .json(&json!({"message": "Add notes API with database"}))
        .send()
        .await
        .unwrap();

    // 3. Poll until the API is ready and create a note
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut created = false;
    while tokio::time::Instant::now() < deadline {
        let resp = server
            .client
            .post(server.url("/project/bob/bob@dev/api/notes"))
            .json(&json!({"title": "First note", "content": "Hello from the test!"}))
            .send()
            .await;
        if let Ok(r) = resp {
            if r.status() == 201 {
                let body: serde_json::Value = r.json().await.unwrap();
                assert_eq!(body["title"], "First note");
                assert!(body["id"].as_i64().unwrap() > 0);
                created = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(created, "should have created a note within timeout");

    // 4. Create a second note
    let resp = server
        .client
        .post(server.url("/project/bob/bob@dev/api/notes"))
        .json(&json!({"title": "Second note", "content": "More content"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // 5. List all notes
    let resp = server
        .client
        .get(server.url("/project/bob/bob@dev/api/notes"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let notes: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(notes.len(), 2, "should have 2 notes");
}
