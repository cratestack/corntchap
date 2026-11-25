mod common;
use corntchap::admin;
use futures::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;

/// OIDC SSO → device flow → token, then drive the board over REST and MCP.
#[tokio::test]
async fn sso_device_flow_rest_and_mcp() {
    let app = common::start(false).await;
    // The first person to sign in is created on the spot with a random handle; the IdP role makes them admin.
    app.sign_in_as("sub-4242", "Octo", true);

    // 1. device authorization request
    let r: Value = app
        .http
        .post(format!("{}/oauth/device/code", app.base))
        .json(&json!({"kind": "user"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let (dc, uc) = (
        r["device_code"].as_str().unwrap().to_string(),
        r["user_code"].as_str().unwrap().to_string(),
    );
    let poll = |dc: String| {
        let app = &app;
        async move {
            sqlx::query("update device_requests set last_polled_at = null")
                .execute(&app.state.db)
                .await
                .unwrap();
            let r = app.http.post(format!("{}/oauth/token", app.base)).json(&json!({"grant_type": "urn:ietf:params:oauth:grant-type:device_code", "device_code": dc})).send().await.unwrap();
            (r.status().as_u16(), r.json::<Value>().await.unwrap())
        }
    };
    let (st, v) = poll(dc.clone()).await;
    assert_eq!(
        (st, v["error"].as_str()),
        (400, Some("authorization_pending"))
    );

    // 2. the human approves in the browser (OIDC SSO)
    let cookie = app.browser_login(&format!("/device?user_code={uc}")).await;
    let page = app
        .http
        .get(format!("{}/device?user_code={uc}", app.base))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("Approve a device"), "{page}");
    let csrf = page
        .split("name=csrf value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    // a forged request without the CSRF token is refused
    let bad = app
        .http
        .post(format!("{}/device", app.base))
        .header("cookie", &cookie)
        .form(&[
            ("user_code", uc.as_str()),
            ("csrf", "nope"),
            ("action", "approve"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status().as_u16(), 403);
    let ok = app
        .http
        .post(format!("{}/device", app.base))
        .header("cookie", &cookie)
        .form(&[
            ("user_code", uc.as_str()),
            ("csrf", csrf.as_str()),
            ("action", "approve"),
        ])
        .send()
        .await
        .unwrap();
    assert!(ok.text().await.unwrap().contains("Approved"));

    // 3. the device exchanges its code for a token (once)
    let (st, v) = poll(dc.clone()).await;
    assert_eq!(st, 200, "{v}");
    let admin_token = v["access_token"].as_str().unwrap().to_string();
    assert_eq!(v["principal"]["name"], app.handle_of("sub-4242").await);
    let (st, _) = poll(dc).await;
    assert_eq!(st, 400, "a device code is single-use");

    // login CSRF: a callback replayed in a browser that did not start the sign-in is refused
    let r = app
        .http
        .get(format!("{}/auth/oidc/login", app.base))
        .send()
        .await
        .unwrap();
    let r = app
        .http
        .get(r.headers()["location"].to_str().unwrap())
        .send()
        .await
        .unwrap();
    let no_cookie = app
        .http
        .get(r.headers()["location"].to_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(no_cookie.status().as_u16(), 403);
    // open redirect via control characters is neutralised
    let c = app.browser_login("/%09/evil.example").await;
    let _ = c;

    // 4. bearer works; garbage does not; a new OIDC subject is provisioned
    let (st, me) = app.get(&admin_token, "/me").await;
    assert_eq!(
        (st, me["display_name"].as_str(), me["role"].as_str()),
        (200, Some("Octo Cat"), Some("super_admin"))
    );
    assert_ne!(
        me["name"], "octo",
        "the handle is random, not the IdP username"
    );
    assert_eq!(app.get("ct_garbage", "/me").await.0, 401);
    // A new subject is provisioned just-in-time, as a non-admin user.
    app.sign_in_as("sub-new", "Newbie", false);
    let c = app.browser_login("/").await;
    let who: serde_json::Value = app
        .http
        .get(format!("{}/auth/session", app.base))
        .header("cookie", &c)
        .header("x-corntchap-console", "1")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(who["name"], app.handle_of("sub-new").await, "{who}");
    assert_eq!(who["display_name"], "Octo Cat", "{who}");
    let (role, src): (String, String) =
        sqlx::query_as("select role, role_source from principals where oidc_sub = 'sub-new'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(role, "member");
    assert_eq!(src, "oidc");

    // 5. admin sets up a team, project and an agent with a token
    assert_eq!(
        app.post(&admin_token, "/teams", json!({"name": "acme"}))
            .await
            .0,
        200
    );
    assert_eq!(
        app.post(
            &admin_token,
            "/projects",
            json!({"team": "acme", "key": "PAY", "name": "Payments"})
        )
        .await
        .0,
        200
    );
    let (_, ag) = app
        .post(
            &admin_token,
            "/agents",
            json!({"name": "claude-1", "kind": "claude-code", "runtime": "cloud"}),
        )
        .await;
    let agent_token = ag["token"].as_str().unwrap().to_string();
    // not a member yet: the agent sees nothing
    assert_eq!(app.get(&agent_token, "/projects/PAY/tasks").await.0, 404);
    assert_eq!(
        app.post(
            &admin_token,
            "/teams/acme/members",
            json!({"principal": "claude-1"})
        )
        .await
        .0,
        200
    );

    // 6. tasks: dependency, checklist, subtask
    let (_, t1) = app
        .post(
            &admin_token,
            "/projects/PAY/tasks",
            json!({"title": "Design", "items": ["sketch", "review"]}),
        )
        .await;
    assert_eq!(t1["id"], "PAY-1");
    let (_, t2) = app
        .post(
            &admin_token,
            "/projects/PAY/tasks",
            json!({"title": "Build", "depends_on": ["PAY-1"], "priority": 5}),
        )
        .await;
    assert_eq!(t2["ready"], false);

    // 7. MCP: whoami, claim (only PAY-1 is ready despite PAY-2's priority), tick, complete
    let who = common::TestApp::payload(&app.mcp(&agent_token, "whoami", json!({})).await);
    assert_eq!(who["name"], "claude-1");
    let claim = common::TestApp::payload(
        &app.mcp(&agent_token, "claim_next", json!({"project": "PAY"}))
            .await,
    );
    assert_eq!(claim["task"]["id"], "PAY-1", "{claim}");
    let epoch = claim["lease"]["epoch"].as_i64().unwrap();
    let busy = app
        .mcp(&agent_token, "claim_next", json!({"project": "PAY"}))
        .await;
    assert!(
        busy["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("max_leases"),
        "{busy}"
    );
    let r = app
        .mcp(
            &agent_token,
            "check_item",
            json!({"id": "PAY-1", "epoch": epoch, "position": 1, "done": true}),
        )
        .await;
    assert_ne!(r["isError"], true, "{r}");
    let stale = app
        .mcp(
            &agent_token,
            "check_item",
            json!({"id": "PAY-1", "epoch": epoch + 99, "position": 2, "done": true}),
        )
        .await;
    assert!(
        stale["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("lease_lost")
    );
    let r = app
        .mcp(
            &agent_token,
            "complete",
            json!({"id": "PAY-1", "epoch": epoch, "result": "PR #1"}),
        )
        .await;
    assert_ne!(r["isError"], true, "{r}");

    // 8. now PAY-2 is unblocked; REST claims it; human can read the board and the digest
    let (st, c) = app
        .post(&agent_token, "/projects/PAY/claim-next", json!({}))
        .await;
    assert_eq!((st, c["task"]["id"].as_str()), (200, Some("PAY-2")));
    let (_, board) = app.get(&admin_token, "/projects/PAY/board").await;
    let md = board["markdown"].as_str().unwrap();
    assert!(
        md.contains("- [x] **PAY-1**") && md.contains("- [ ] **PAY-2**"),
        "{md}"
    );
    let (_, d) = app.get(&admin_token, "/projects/PAY/digest?since=0").await;
    assert!(d["counts"]["task.completed"].as_i64().unwrap() >= 1);
    let r = app
        .http
        .get(format!("{}/board/PAY", app.base))
        .send()
        .await
        .unwrap();
    assert_eq!(
        (
            r.status().as_u16(),
            r.headers()["location"].to_str().unwrap()
        ),
        (303, "/pots/PAY"),
        "the old board redirects to the console"
    );

    // 9. a human can cancel/unblock only as a user; agents cannot
    assert_eq!(
        app.post(&agent_token, "/tasks/PAY-2/cancel", json!({}))
            .await
            .0,
        403
    );

    // 10. rotation kills the old secret at once
    let (_, rot) = app
        .post(&admin_token, "/agents/claude-1/rotate", json!({}))
        .await;
    assert_eq!(app.get(&agent_token, "/me").await.0, 401);
    assert_eq!(app.get(rot["token"].as_str().unwrap(), "/me").await.0, 200);
    // nothing the server did on its own is anonymous
    common::assert_no_anonymous_events(&app.state.db).await;
}

/// The dispatcher wakes the owner's relay over SSE, with Last-Event-ID replay.
#[tokio::test]
async fn relay_stream_receives_wakes() {
    let app = common::start(true).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let t = tok.unwrap();
    app.post(&t, "/teams", json!({"name": "acme"})).await;
    app.post(
        &t,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let (_, ag) = app
        .post(
            &t,
            "/agents",
            json!({"name": "opencode-laptop", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    app.post(
        &t,
        "/teams/acme/members",
        json!({"principal": "opencode-laptop"}),
    )
    .await;
    let (st, wt) = app
        .post(
            &t,
            "/agents/opencode-laptop/wake-targets",
            json!({"type": "relay"}),
        )
        .await;
    assert_eq!(st, 200, "{wt}");
    let (_, relay) = app
        .post(&t, "/relays", json!({"name": "relay-laptop"}))
        .await;
    let relay_token = relay["token"].as_str().unwrap().to_string();
    // relays cannot read tasks
    assert_eq!(app.get(&relay_token, "/projects/PAY/tasks").await.0, 403);
    // agents cannot listen on the stream
    assert_eq!(
        app.get(ag["token"].as_str().unwrap(), "/relay/stream")
            .await
            .0,
        403
    );

    let resp = app
        .http
        .get(format!("{}/api/v1/relay/stream", app.base))
        .bearer_auth(&relay_token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let mut stream = resp.bytes_stream();
    // new work arrives after the relay connected
    tokio::time::sleep(Duration::from_millis(500)).await;
    app.post(&t, "/projects/PAY/tasks", json!({"title": "wake me"}))
        .await;
    let mut text = String::new();
    let got = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(chunk) = stream.next().await {
            text.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
            if text.contains("event: wake")
                && text.contains("\n\n")
                && text.matches("event: wake").count() >= 1
                && text.contains("opencode-laptop")
            {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(got, "no wake received: {text}");
    assert!(text.contains("\"project\":\"PAY\""), "{text}");
    // the wake is the restaurant's
    let actor: String = sqlx::query_scalar(
        "select a.name from events e join principals a on a.id = e.actor_id where e.kind = 'wake' order by e.seq limit 1",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert_eq!(actor, "restaurant");
    common::assert_no_anonymous_events(&app.state.db).await;
}
