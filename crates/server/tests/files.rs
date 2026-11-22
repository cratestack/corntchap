//! Files against real S3-compatible storage (RustFS: `just infra`, a service in CI). An agent gets
//! an upload slot, runs the very curl command it was given, the watcher sees the upload land, and
//! the permanent link serves it behind the login. Skipped when no S3 endpoint is configured.
mod common;
use base64::Engine;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}
fn refused(r: &Value, what: &str) -> String {
    assert_eq!(r["isError"], true, "{what} should fail: {r}");
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}

#[tokio::test]
async fn upload_watch_link_download() {
    if std::env::var("S3_ENDPOINT").is_err() {
        eprintln!("skipped: no S3_ENDPOINT (run `just infra`, then load .env.dev)");
        return;
    }
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for (team, key) in [("acme", "PAY"), ("other", "OPS")] {
        app.post(&root, "/teams", json!({"name": team})).await;
        app.post(
            &root,
            "/projects",
            json!({"team": team, "key": key, "name": key}),
        )
        .await;
    }
    let mut toks = vec![];
    for (name, team) in [("a1", "acme"), ("b1", "other")] {
        let (_, ag) = app
            .post(
                &root,
                "/agents",
                json!({"name": name, "kind": "other", "runtime": "local"}),
            )
            .await;
        toks.push(ag["token"].as_str().unwrap().to_string());
        app.post(
            &root,
            &format!("/teams/{team}/members"),
            json!({"principal": name}),
        )
        .await;
    }
    let (a1, b1) = (toks[0].clone(), toks[1].clone());
    app.post(
        &root,
        "/projects/PAY/tasks",
        json!({"title": "Screenshot the bug"}),
    )
    .await;

    // --- an upload slot for a task, then the agent runs the exact command it got ---
    let dir = tempfile_dir();
    let path = dir.join("bug shot.png");
    let bytes: Vec<u8> = (0..20_000u32).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    let slot = ok(
        &app.mcp(
            &a1,
            "create_upload",
            json!({"task": "PAY-1", "name": "bug shot.png", "size": bytes.len()}),
        )
        .await,
        "create_upload",
    );
    let file = &slot["file"];
    assert_eq!(
        (&file["id"], &file["status"], &file["content_type"]),
        (&json!("PAY-F1"), &json!("uploading"), &json!("image/png"))
    );
    let link = file["url"].as_str().unwrap().to_string();
    assert!(link.ends_with("/files/PAY-F1/bug%20shot.png"), "{link}");
    // the link exists before the bytes: "still uploading"
    let r = app.http.get(&link).bearer_auth(&a1).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 409);
    let cmd = slot["command"]
        .as_str()
        .unwrap()
        .replace("'./bug shot.png'", &format!("'{}'", path.display()));
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "curl failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        slot["background_command"]
            .as_str()
            .unwrap()
            .starts_with("nohup curl")
    );

    // nobody confirms: the watcher sees it land
    corntchap::files::watch(&app.state).await.unwrap();
    let got = ok(
        &app.mcp(&a1, "get_file", json!({"id": "PAY-F1"})).await,
        "get_file",
    );
    assert_eq!(
        (&got["file"]["status"], &got["file"]["size"]),
        (&json!("ready"), &json!(bytes.len()))
    );
    let dl = got["download_url"].as_str().unwrap();
    assert_eq!(
        reqwest::get(dl)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap()
            .as_ref(),
        bytes.as_slice()
    );
    let task = ok(
        &app.mcp(&a1, "get_task", json!({"id": "PAY-1"})).await,
        "task files",
    );
    assert_eq!(task["files"][0]["id"], "PAY-F1");

    // --- the permanent link: behind the login, then a redirect to storage ---
    let r = app.http.get(&link).bearer_auth(&a1).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 302);
    let to = r.headers()["location"].to_str().unwrap().to_string();
    let body = reqwest::get(&to).await.unwrap().bytes().await.unwrap();
    assert_eq!(body.as_ref(), bytes.as_slice());
    let r = app.http.get(&link).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 401, "no credentials");
    let r = app
        .http
        .get(&link)
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 303, "a browser is sent to sign in");
    assert!(
        r.headers()["location"]
            .to_str()
            .unwrap()
            .contains("/auth/oidc/login?next=%2Ffiles%2FPAY-F1")
    );
    let r = app.http.get(&link).bearer_auth(&b1).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 404, "another team cannot see the pot");

    // --- inline upload (no shell), attached to a question ---
    ok(
        &app.mcp(
            &a1,
            "ask",
            json!({"project": "PAY", "question": "Is this the right colour?"}),
        )
        .await,
        "ask",
    );
    let inline = ok(
        &app.mcp(
            &a1,
            "upload_file",
            json!({"question": "PAY-Q1", "name": "notes.txt",
            "contentBase64": base64::engine::general_purpose::STANDARD.encode("hello chef")}),
        )
        .await,
        "upload_file",
    );
    assert_eq!(
        (&inline["status"], &inline["size"], &inline["question"]),
        (&json!("ready"), &json!(10), &json!("PAY-Q1"))
    );
    let q = ok(
        &app.mcp(&a1, "get_question", json!({"id": "PAY-Q1"})).await,
        "question files",
    );
    assert_eq!(q["files"][0]["name"], "notes.txt");
    let e = refused(
        &app.mcp(&a1, "upload_file", json!({"project": "PAY", "name": "big.bin",
            "contentBase64": base64::engine::general_purpose::STANDARD.encode(vec![0u8; 70 * 1024])})).await,
        "inline too big",
    );
    assert!(e.contains("inline limit"), "{e}");

    // --- the console's streaming upload ---
    let r = app
        .http
        .post(format!(
            "{}/api/v1/projects/PAY/files?name=report.csv&task=PAY-1",
            app.base
        ))
        .bearer_auth(&root)
        .header("content-type", "text/csv")
        .body("a,b\n1,2\n")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let f: Value = r.json().await.unwrap();
    assert_eq!(
        (&f["id"], &f["status"], &f["size"]),
        (&json!("PAY-F3"), &json!("ready"), &json!(8))
    );

    // --- a wrong-size upload fails; over the limit is refused up front ---
    let slot = ok(
        &app.mcp(
            &a1,
            "create_upload",
            json!({"project": "PAY", "name": "x.bin", "size": 100}),
        )
        .await,
        "slot 2",
    );
    let small = dir.join("x.bin");
    std::fs::write(&small, b"short").unwrap();
    let cmd = slot["command"]
        .as_str()
        .unwrap()
        .replace("'./x.bin'", &format!("'{}'", small.display()));
    assert!(
        std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .status()
            .unwrap()
            .success()
    );
    corntchap::files::watch(&app.state).await.unwrap();
    let got = ok(
        &app.mcp(&a1, "get_file", json!({"id": "PAY-F4"})).await,
        "failed file",
    );
    assert_eq!(got["file"]["status"], "failed");
    assert!(
        got["file"]["error"]
            .as_str()
            .unwrap()
            .contains("expected 100")
    );
    assert!(got["download_url"].is_null());
    let e = refused(
        &app.mcp(
            &a1,
            "create_upload",
            json!({"project": "PAY", "name": "huge.iso", "size": 5_000_000}),
        )
        .await,
        "over the limit",
    );
    assert!(e.contains("limit"), "{e}");

    // --- list, delete, the feed ---
    let all = ok(
        &app.mcp(&a1, "list_files", json!({"project": "PAY"})).await,
        "list",
    );
    assert_eq!(all.as_array().unwrap().len(), 4);
    let on_task = ok(
        &app.mcp(
            &a1,
            "list_files",
            json!({"project": "PAY", "task": "PAY-1"}),
        )
        .await,
        "task list",
    );
    assert_eq!(on_task.as_array().unwrap().len(), 2);
    let e = refused(
        &app.mcp(&a1, "delete_file", json!({"id": "PAY-F3"})).await,
        "not mine",
    );
    assert!(e.contains("someone else"), "{e}");
    ok(
        &app.mcp(&a1, "delete_file", json!({"id": "PAY-F1"})).await,
        "delete mine",
    );
    let r = app.http.get(&link).bearer_auth(&a1).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 404);
    let feed = ok(
        &app.mcp(
            &a1,
            "changes_since",
            json!({"cursor": 0, "project": "PAY", "limit": 500}),
        )
        .await,
        "feed",
    );
    let kinds: Vec<&str> = feed["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .filter(|k| k.starts_with("file."))
        .collect();
    for k in ["file.created", "file.ready", "file.failed", "file.deleted"] {
        assert!(kinds.contains(&k), "{k} in {kinds:?}");
    }
}

fn tempfile_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("corntchap-files-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&d).unwrap();
    d
}
