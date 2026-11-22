//! On shutdown the endless streams end (with a `shutdown` event) and long-polls return, so the
//! graceful drain is not held open by connected relays, consoles or agents.
mod common;
use std::time::Duration;

use corntchap::admin;
use futures::StreamExt;
use serde_json::json;

#[tokio::test]
async fn streams_end_when_the_server_stops() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    let (_, relay) = app.post(&root, "/relays", json!({"name": "r1"})).await;
    let relay = relay["token"].as_str().unwrap().to_string();

    let open = |path: &'static str, token: String| {
        let (http, base) = (app.http.clone(), app.base.clone());
        async move {
            let r = http
                .get(format!("{base}/api/v1{path}"))
                .bearer_auth(token)
                .header("accept", "text/event-stream")
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200, "{path}");
            r.bytes_stream()
        }
    };
    let mut wakes = open("/relay/stream", relay).await;
    let mut feed = open("/events/stream", root.clone()).await;
    // the first frames (ping / ready) arrive while the server runs
    for s in [&mut wakes, &mut feed] {
        tokio::time::timeout(Duration::from_secs(5), s.next())
            .await
            .expect("a first frame");
    }
    let poll = {
        let (http, base, t) = (app.http.clone(), app.base.clone(), root.clone());
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            let r = http
                .get(format!("{base}/api/v1/wait?timeout_s=50"))
                .bearer_auth(t)
                .send()
                .await
                .unwrap();
            (r.status().as_u16(), started.elapsed())
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;

    app.state.shut_down();

    for (name, s) in [("relay stream", &mut wakes), ("event stream", &mut feed)] {
        let mut text = String::new();
        let ended = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(chunk) = s.next().await {
                text.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
            }
        })
        .await;
        assert!(ended.is_ok(), "{name} still open after shutdown: {text}");
        assert!(text.contains("event: shutdown"), "{name}: {text}");
    }
    let (status, took) = poll.await.unwrap();
    assert!(
        took < Duration::from_secs(10),
        "the long-poll returned at once on shutdown (HTTP {status}, {took:?})"
    );
}
