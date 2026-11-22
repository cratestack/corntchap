//! /api/v1/events/stream: replay from a cursor, then live, in one connection; visibility per caller;
//! resuming with Last-Event-ID.
mod common;
use corntchap::admin;
use futures::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;

/// Reads SSE frames off a response until `n` events of interest arrived (or a timeout).
struct Sse {
    body: futures::stream::BoxStream<'static, reqwest::Result<axum::body::Bytes>>,
    buf: String,
}

impl Sse {
    async fn open(app: &common::TestApp, tok: &str, query: &str, last_id: Option<i64>) -> Self {
        let mut r = app
            .http
            .get(format!("{}/api/v1/events/stream{query}", app.base))
            .bearer_auth(tok);
        if let Some(id) = last_id {
            r = r.header("last-event-id", id.to_string());
        }
        let resp = r.send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        Sse {
            body: resp.bytes_stream().boxed(),
            buf: String::new(),
        }
    }

    /// (id, event, data) of the next frame that has an id (skips ready/keep-alives).
    async fn next(&mut self) -> Option<(i64, String, Value)> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            while let Some(i) = self.buf.find("\n\n") {
                let frame: String = self.buf.drain(..i + 2).collect();
                let (mut id, mut ev, mut data) = (None, String::new(), String::new());
                for line in frame.lines() {
                    if let Some(v) = line.strip_prefix("id:") {
                        id = v.trim().parse().ok();
                    } else if let Some(v) = line.strip_prefix("event:") {
                        ev = v.trim().to_string();
                    } else if let Some(v) = line.strip_prefix("data:") {
                        data.push_str(v.trim());
                    }
                }
                if let Some(id) = id {
                    return Some((id, ev, serde_json::from_str(&data).unwrap_or(Value::Null)));
                }
            }
            let chunk = tokio::time::timeout_at(deadline, self.body.next())
                .await
                .ok()??
                .ok()?;
            self.buf
                .push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
        }
    }
}

#[tokio::test]
async fn replay_then_live_with_visibility() {
    let app = common::start(true).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let admin = tok.unwrap();
    for (team, key) in [("acme", "PAY"), ("other", "OPS")] {
        app.post(&admin, "/teams", json!({"name": team})).await;
        app.post(
            &admin,
            "/projects",
            json!({"team": team, "key": key, "name": key}),
        )
        .await;
    }
    let mut toks = vec![];
    for (name, team) in [("a1", "acme"), ("b1", "other")] {
        let (_, ag) = app
            .post(
                &admin,
                "/agents",
                json!({"name": name, "kind": "other", "runtime": "local"}),
            )
            .await;
        toks.push(ag["token"].as_str().unwrap().to_string());
        app.post(
            &admin,
            &format!("/teams/{team}/members"),
            json!({"principal": name}),
        )
        .await;
    }
    let (a1, b1) = (toks[0].clone(), toks[1].clone());

    // history before anyone listens
    app.post(&admin, "/projects/PAY/tasks", json!({"title": "old one"}))
        .await;
    app.post(&admin, "/projects/OPS/tasks", json!({"title": "not yours"}))
        .await;

    // a1 replays from 0: only PAY events
    let mut s = Sse::open(&app, &a1, "?cursor=0", None).await;
    let (id1, ev, data) = s.next().await.expect("replayed event");
    assert_eq!(
        (ev.as_str(), &data["task"]),
        ("task.created", &json!("PAY-1")),
        "{data}"
    );

    // live: a write after connecting arrives on the same connection
    let (_, b1_view) = app.get(&b1, "/projects/PAY/tasks").await; // b1 cannot see PAY at all
    assert!(b1_view["error"].as_str().is_some(), "{b1_view}");
    app.post(
        &admin,
        "/projects/OPS/tasks",
        json!({"title": "still not yours"}),
    )
    .await;
    app.post(&admin, "/projects/PAY/tasks", json!({"title": "new one"}))
        .await;
    let (id2, ev, data) = s.next().await.expect("live event");
    assert_eq!(
        (ev.as_str(), &data["task"]),
        ("task.created", &json!("PAY-2")),
        "{data}"
    );
    assert!(id2 > id1);

    // resume with Last-Event-ID: nothing before it is replayed
    app.mcp(&a1, "claim_task", json!({"id": "PAY-1"})).await;
    let mut s2 = Sse::open(&app, &a1, "", Some(id2)).await;
    let (_, ev, data) = s2.next().await.expect("event after the resume point");
    assert_eq!(
        (ev.as_str(), &data["task"]),
        ("task.claimed", &json!("PAY-1")),
        "{data}"
    );

    // filtered to a project; a project you cannot see is a 404, relays are refused
    let r = app
        .http
        .get(format!("{}/api/v1/events/stream?project=PAY", app.base))
        .bearer_auth(&b1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);
    let (_, rl) = app.post(&admin, "/relays", json!({"name": "r1"})).await;
    let r = app
        .http
        .get(format!("{}/api/v1/events/stream", app.base))
        .bearer_auth(rl["token"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 403);

    // b1's own stream only ever shows OPS
    let mut sb = Sse::open(&app, &b1, "?cursor=0", None).await;
    for _ in 0..2 {
        let (_, _, data) = sb.next().await.expect("OPS event");
        assert_eq!(data["project"], "OPS", "{data}");
    }
}
