//! OAuth for MCP clients: a client registers itself, the person signs in and picks a cook, the
//! client gets a token by authorization code + PKCE and refreshes it. A token asked for a pot's MCP
//! URL (`/mcp/<KEY>`) works there only, defaults tools to that pot and refuses others.
mod common;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use corntchap::admin;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const REDIRECT: &str = "http://127.0.0.1:5555/cb";
const VERIFIER: &str = "a-verifier-that-is-long-enough-for-pkce-0123456789";

fn challenge(v: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes()))
}

fn hidden(html: &str, name: &str) -> String {
    let needle = format!("name={name} value=\"");
    let i = html
        .find(&needle)
        .unwrap_or_else(|| panic!("no {name} in {html}"))
        + needle.len();
    html[i..].split('"').next().unwrap().to_string()
}

fn query_param(url: &str, k: &str) -> Option<String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(a, _)| a == k)
        .map(|(_, v)| v.to_string())
}

async fn mcp(
    app: &common::TestApp,
    path: &str,
    token: &str,
    method: &str,
    params: Value,
) -> (u16, Value, String) {
    let r = app
        .http
        .post(format!("{}{path}", app.base))
        .bearer_auth(token)
        .header("accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-11-25")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    let www = r
        .headers()
        .get("www-authenticate")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    (status, r.json().await.unwrap_or(Value::Null), www)
}

async fn form(
    app: &common::TestApp,
    path: &str,
    body: &[(&str, &str)],
    cookie: Option<&str>,
) -> reqwest::Response {
    let mut r = app
        .http
        .post(format!("{}{path}", app.base))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(serde_urlencoded::to_string(body).unwrap());
    if let Some(c) = cookie {
        r = r.header("cookie", c);
    }
    r.send().await.unwrap()
}

#[tokio::test]
async fn an_mcp_client_signs_in_with_oauth_and_is_bound_to_its_pot() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(&root, "/teams", json!({"name": "other"})).await;
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    app.post(
        &root,
        "/projects",
        json!({"team": "other", "key": "OPS", "name": "Ops"}),
    )
    .await;

    // the person signs in once in the browser, and owns team acme
    app.sign_in_as("sub-alice", "alice", false);
    let cookie = app.browser_login("/").await;
    let me: Value = app
        .http
        .get(format!("{}/auth/session", app.base))
        .header("cookie", &cookie)
        .header("x-corntchap-console", "1")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let handle = me["name"]
        .as_str()
        .or(me["user"]["name"].as_str())
        .unwrap_or_else(|| panic!("{me}"))
        .to_string();
    let (code, _) = app
        .post(
            &root,
            "/teams/acme/members",
            json!({"principal": handle, "role": "owner"}),
        )
        .await;
    assert_eq!(code, 200);
    app.post(
        &root,
        "/projects/PAY/tasks",
        json!({"title": "Refund rounding"}),
    )
    .await;

    // discovery: the pot's URL asks for a token and says where to get one
    let (status, _, www) = mcp(&app, "/mcp/PAY", "", "tools/list", json!({})).await;
    assert_eq!(status, 401);
    assert!(
        www.contains("/.well-known/oauth-protected-resource/mcp/PAY"),
        "{www}"
    );
    let prm: Value = app
        .http
        .get(format!(
            "{}/.well-known/oauth-protected-resource/mcp/PAY",
            app.base
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(prm["resource"], format!("{}/mcp/PAY", app.base));
    let asm: Value = app
        .http
        .get(format!(
            "{}/.well-known/oauth-authorization-server",
            app.base
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));
    assert!(
        asm["registration_endpoint"]
            .as_str()
            .unwrap()
            .ends_with("/oauth/register")
    );

    // dynamic client registration
    let bad: Value = app
        .http
        .post(format!("{}/oauth/register", app.base))
        .json(&json!({"redirect_uris": ["http://evil.example/cb"], "client_name": "x"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(bad["error"], "invalid_redirect_uri");
    let reg: Value = app
        .http
        .post(format!("{}/oauth/register", app.base))
        .json(&json!({"redirect_uris": [REDIRECT], "client_name": "OpenCode", "token_endpoint_auth_method": "none"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let client_id = reg["client_id"].as_str().unwrap().to_string();

    // authorize: signed out goes to sign-in, signed in shows the consent screen
    let auth_url = format!(
        "/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state=st8&resource={}",
        urlencoding(REDIRECT),
        challenge(VERIFIER),
        urlencoding(&format!("{}/mcp/PAY", app.base)),
    );
    let r = app
        .http
        .get(format!("{}{auth_url}", app.base))
        .send()
        .await
        .unwrap();
    assert!(
        r.headers()["location"]
            .to_str()
            .unwrap()
            .contains("/auth/oidc/login")
    );
    let page = app
        .http
        .get(format!("{}{auth_url}", app.base))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        page.contains("Connect OpenCode")
            && page.contains("<code>PAY</code>")
            && page.contains("127.0.0.1:5555"),
        "{page}"
    );
    let (req_id, csrf) = (hidden(&page, "request"), hidden(&page, "csrf"));

    // a forged decision (wrong CSRF) is refused
    let r = form(
        &app,
        "/oauth/authorize",
        &[
            ("request", &req_id),
            ("csrf", "nope"),
            ("action", "approve"),
        ],
        Some(&cookie),
    )
    .await;
    assert_eq!(r.status().as_u16(), 403);
    // allow: a new cook, added to the pot's team
    let r = form(
        &app,
        "/oauth/authorize",
        &[
            ("request", &req_id),
            ("csrf", &csrf),
            ("action", "approve"),
            ("cook", "__new"),
            ("new_name", "alice-opencode"),
            ("new_kind", "opencode"),
            ("add_team", "1"),
        ],
        Some(&cookie),
    )
    .await;
    let back = r.headers()["location"].to_str().unwrap().to_string();
    assert!(back.starts_with(REDIRECT), "{back}");
    assert_eq!(query_param(&back, "state").as_deref(), Some("st8"));
    let code = query_param(&back, "code").unwrap();
    // the request is answered once
    let r = form(
        &app,
        "/oauth/authorize",
        &[
            ("request", &req_id),
            ("csrf", &csrf),
            ("action", "approve"),
            ("cook", "__new"),
            ("new_name", "x2"),
        ],
        Some(&cookie),
    )
    .await;
    assert!(r.text().await.unwrap().contains("Expired"));

    // token: the PKCE verifier must match, and a code works once
    let tok = |body: Vec<(&'static str, String)>| {
        let app = &app;
        async move {
            let r = app
                .http
                .post(format!("{}/oauth/token", app.base))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(serde_urlencoded::to_string(&body).unwrap())
                .send()
                .await
                .unwrap();
            r.json::<Value>().await.unwrap()
        }
    };
    let wrong = tok(vec![
        ("grant_type", "authorization_code".into()),
        ("code", code.clone()),
        ("client_id", client_id.clone()),
        ("redirect_uri", REDIRECT.into()),
        ("code_verifier", "wrong-verifier".into()),
    ])
    .await;
    assert_eq!(wrong["error"], "invalid_grant", "{wrong}");
    let t = tok(vec![
        ("grant_type", "authorization_code".into()),
        ("code", code.clone()),
        ("client_id", client_id.clone()),
        ("redirect_uri", REDIRECT.into()),
        ("code_verifier", VERIFIER.into()),
    ])
    .await;
    let access = t["access_token"]
        .as_str()
        .unwrap_or_else(|| panic!("{t}"))
        .to_string();
    let refresh = t["refresh_token"].as_str().unwrap().to_string();
    assert_eq!(t["pot"], "PAY");
    let again = tok(vec![
        ("grant_type", "authorization_code".into()),
        ("code", code.clone()),
        ("client_id", client_id.clone()),
        ("redirect_uri", REDIRECT.into()),
        ("code_verifier", VERIFIER.into()),
    ])
    .await;
    assert_eq!(again["error"], "invalid_grant");

    // the cook exists, is the person's, in team acme, with a relay wake target
    let (owner, in_team, relay): (String, bool, bool) = sqlx::query_as(
        "select (select name from principals where id = a.owner_id),
                exists (select 1 from team_members m join teams t on t.id = m.team_id where m.principal_id = a.id and t.name = 'acme'),
                exists (select 1 from wake_targets w where w.agent_id = a.id and w.type = 'relay')
         from principals a where a.name = 'alice-opencode'",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert_eq!(
        (owner.as_str(), in_team, relay),
        (handle.as_str(), true, true)
    );

    // on /mcp/PAY the token works, tools default to PAY and refuse other pots
    let (s, w, _) = mcp(
        &app,
        "/mcp/PAY",
        &access,
        "tools/call",
        json!({"name": "whoami", "arguments": {}}),
    )
    .await;
    assert_eq!(s, 200);
    assert!(
        w["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("alice-opencode"),
        "{w}"
    );
    let (_, l, _) = mcp(
        &app,
        "/mcp/PAY",
        &access,
        "tools/call",
        json!({"name": "list_tasks", "arguments": {}}),
    )
    .await;
    assert_ne!(l["result"]["isError"], true, "{l}");
    assert!(
        l["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Refund rounding"),
        "{l}"
    );
    let (_, x, _) = mcp(
        &app,
        "/mcp/PAY",
        &access,
        "tools/call",
        json!({"name": "list_tasks", "arguments": {"project": "OPS"}}),
    )
    .await;
    assert_eq!(x["result"]["isError"], true);
    assert!(x.to_string().contains("this MCP URL is for pot PAY"), "{x}");
    let (_, x, _) = mcp(
        &app,
        "/mcp/PAY",
        &access,
        "tools/call",
        json!({"name": "get_task", "arguments": {"id": "OPS-1"}}),
    )
    .await;
    assert!(x.to_string().contains("OPS-1 is in pot OPS"), "{x}");
    let (_, tl, _) = mcp(&app, "/mcp/PAY", &access, "tools/list", json!({})).await;
    let lt = tl["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "list_tasks")
        .unwrap()
        .clone();
    assert!(
        !lt["inputSchema"]["required"]
            .as_array()
            .map(|r| r.contains(&json!("project")))
            .unwrap_or(false),
        "{lt}"
    );
    let (_, rl, _) = mcp(&app, "/mcp/PAY", &access, "resources/list", json!({})).await;
    assert_eq!(
        rl["result"]["resources"].as_array().unwrap().len(),
        1,
        "only the connect form: {rl}"
    );
    let (_, init, _) = mcp(
        &app,
        "/mcp/PAY",
        &access,
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {}}),
    )
    .await;
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("for pot PAY")
    );
    assert!(
        init["result"]["serverInfo"]["icons"][0]["src"]
            .as_str()
            .unwrap()
            .ends_with("/favicon.svg")
    );

    // the newest MCP revision (cratestack's path) gets the same defaults and refusals
    let modern = |args: Value| {
        let app = &app;
        let access = access.clone();
        async move {
            let meta = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}});
            let r = app
                .http
                .post(format!("{}/mcp/PAY", app.base))
                .bearer_auth(&access)
                .header("accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", "2026-07-28")
                .header("Mcp-Method", "tools/call")
                .header("Mcp-Name", "list_tasks")
                .json(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                              "params": {"name": "list_tasks", "arguments": args, "_meta": meta}}))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status().as_u16(), 200);
            r.json::<Value>().await.unwrap()
        }
    };
    let m = modern(json!({})).await;
    assert!(
        m["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("Refund rounding"),
        "{m}"
    );
    let m = modern(json!({"project": "OPS"})).await;
    assert_eq!(m["result"]["isError"], true, "{m}");

    // ...and nowhere else: not on /mcp, not on another pot, not on REST
    let (s, _, _) = mcp(&app, "/mcp", &access, "tools/list", json!({})).await;
    assert_eq!(s, 401);
    let (s, _, www) = mcp(&app, "/mcp/OPS", &access, "tools/list", json!({})).await;
    assert_eq!(s, 401);
    assert!(www.contains("/mcp/OPS"));
    let (s, _) = app.get(&access, "/projects").await;
    assert_eq!(s, 401);

    // refresh: a new pair, the old access token stops, a refresh token works once
    let t2 = tok(vec![
        ("grant_type", "refresh_token".into()),
        ("refresh_token", refresh.clone()),
        ("client_id", client_id.clone()),
    ])
    .await;
    let access2 = t2["access_token"]
        .as_str()
        .unwrap_or_else(|| panic!("{t2}"))
        .to_string();
    let refresh2 = t2["refresh_token"].as_str().unwrap().to_string();
    let (s, _, _) = mcp(&app, "/mcp/PAY", &access, "tools/list", json!({})).await;
    assert_eq!(s, 401, "the refreshed-away token is revoked");
    let (s, _, _) = mcp(&app, "/mcp/PAY", &access2, "tools/list", json!({})).await;
    assert_eq!(s, 200);
    let reuse = tok(vec![
        ("grant_type", "refresh_token".into()),
        ("refresh_token", refresh.clone()),
    ])
    .await;
    assert_eq!(reuse["error"], "invalid_grant");

    // rotating the cook (cook card) cuts the client off for good
    let (s, _) = app
        .post(&root, "/agents/alice-opencode/rotate", json!({}))
        .await;
    assert_eq!(s, 200);
    let cut = tok(vec![
        ("grant_type", "refresh_token".into()),
        ("refresh_token", refresh2.clone()),
    ])
    .await;
    assert_eq!(cut["error"], "invalid_grant", "{cut}");

    // a general client (resource /mcp) picks an existing cook; saying no sends the client an error
    let auth_url = format!(
        "/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state=s2",
        urlencoding("http://127.0.0.1:6000/cb"),
        challenge(VERIFIER),
    );
    let page = app
        .http
        .get(format!("{}{auth_url}", app.base))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("every pot of the cook"), "{page}");
    let r = form(
        &app,
        "/oauth/authorize",
        &[
            ("request", &hidden(&page, "request")),
            ("csrf", &hidden(&page, "csrf")),
            ("action", "deny"),
        ],
        Some(&cookie),
    )
    .await;
    let back = r.headers()["location"].to_str().unwrap().to_string();
    assert!(
        back.starts_with("http://127.0.0.1:6000/cb")
            && back.contains("error=access_denied")
            && back.contains("state=s2"),
        "{back}"
    );
    // an unregistered redirect is never followed
    let r = app
        .http
        .get(format!(
            "{}/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}&code_challenge=x&code_challenge_method=S256",
            app.base,
            urlencoding("https://evil.example/cb")
        ))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    // PKCE is mandatory
    let r = app
        .http
        .get(format!(
            "{}/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}",
            app.base,
            urlencoding(REDIRECT)
        ))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert!(
        r.headers()["location"]
            .to_str()
            .unwrap()
            .contains("error=invalid_request")
    );
}

fn urlencoding(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
