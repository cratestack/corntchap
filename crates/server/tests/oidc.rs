mod common;
use corntchap::admin;
use serde_json::{Value, json};

async fn row(app: &common::TestApp, sub: &str) -> (bool, String) {
    sqlx::query_as("select is_admin, admin_source from principals where oidc_sub = $1")
        .bind(sub)
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

/// Run the login dance without asserting success; returns the callback response.
async fn login_raw(app: &common::TestApp) -> reqwest::Response {
    let r = app
        .http
        .get(format!("{}/auth/oidc/login", app.base))
        .send()
        .await
        .unwrap();
    let nonce = r.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let r = app
        .http
        .get(r.headers()["location"].to_str().unwrap())
        .send()
        .await
        .unwrap();
    app.http
        .get(r.headers()["location"].to_str().unwrap())
        .header("cookie", nonce)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn jit_creates_user_and_maps_admin_role() {
    let app = common::start(false).await;
    app.sign_in_as("sub-dev", "Dev", false);
    app.browser_login("/").await;
    assert_eq!(row(&app, "sub-dev").await, (false, "oidc".to_string()));

    // the admin role is synced on every sign-in, both ways
    app.sign_in_as("sub-dev", "Dev", true);
    app.browser_login("/").await;
    assert_eq!(row(&app, "sub-dev").await, (true, "oidc".to_string()));
    app.sign_in_as("sub-dev", "Dev", false);
    app.browser_login("/").await;
    assert_eq!(row(&app, "sub-dev").await, (false, "oidc".to_string()));

    // an admin made by `init` is never demoted by the IdP (here: a signed-in person the operator promoted)
    app.sign_in_as("sub-boss", "Boss", false);
    app.browser_login("/").await;
    let boss = app.handle_of("sub-boss").await;
    admin::init_admin(&app.state.db, Some(&boss)).await.unwrap();
    app.sign_in_as("sub-boss", "Boss", false);
    app.browser_login("/").await;
    assert_eq!(row(&app, "sub-boss").await, (true, "init".to_string()));
    // ... and an admin promoted by hand is not demoted either
    app.sign_in_as("sub-dev", "Dev", true);
    app.browser_login("/").await;
    let dev = app.handle_of("sub-dev").await;
    let boss_p = app.principal(&boss).await;
    admin::set_admin(&app.state.db, &boss_p, &dev, true)
        .await
        .unwrap();
    app.sign_in_as("sub-dev", "Dev", false);
    app.browser_login("/").await;
    assert_eq!(row(&app, "sub-dev").await, (true, "manual".to_string()));
}

#[tokio::test]
async fn logout_revokes_token_and_session() {
    let app = common::start(false).await;
    admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let t1 = admin::operator_token(&app.state.db, "octo", None)
        .await
        .unwrap();
    let t2 = admin::operator_token(&app.state.db, "octo", None)
        .await
        .unwrap();
    assert_eq!(app.get(&t1, "/me").await.0, 200);
    assert_eq!(app.post(&t1, "/logout", json!({})).await.0, 200);
    assert_eq!(app.get(&t1, "/me").await.0, 401, "token is revoked");
    assert_eq!(app.get(&t2, "/me").await.0, 200, "other tokens survive");

    // browser: GET shows a button, POST needs the CSRF token, clears the session and goes to the IdP
    let cookie = app.browser_login("/").await;
    let page = app
        .http
        .get(format!("{}/logout", app.base))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let csrf = page
        .split("name=csrf value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    let bad = app
        .http
        .post(format!("{}/logout", app.base))
        .header("cookie", &cookie)
        .form(&[("csrf", "nope")])
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status().as_u16(), 403);
    let ok = app
        .http
        .post(format!("{}/logout", app.base))
        .header("cookie", &cookie)
        .form(&[("csrf", csrf.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 303);
    let loc = ok.headers()["location"].to_str().unwrap().to_string();
    assert!(
        loc.starts_with(&format!("{}/end-session?", app.gh)) && loc.contains("id_token_hint="),
        "{loc}"
    );
    assert!(
        ok.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let after = app
        .http
        .get(format!("{}/auth/session", app.base))
        .header("cookie", &cookie)
        .header("x-corntchap-console", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(after.status().as_u16(), 401, "session is gone");
}

#[tokio::test]
async fn serves_authorization_server_metadata() {
    let app = common::start(false).await;
    let v: Value = app
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
    assert_eq!(v["issuer"], app.base);
    assert_eq!(
        v["device_authorization_endpoint"],
        format!("{}/oauth/device/code", app.base)
    );
    assert_eq!(v["token_endpoint"], format!("{}/oauth/token", app.base));
    let r = app.get("ct_garbage", "/me").await;
    assert_eq!(r.0, 401);
}

#[tokio::test]
async fn console_cookie_auth_needs_header_and_origin() {
    let app = common::start(false).await;
    admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    app.sign_in_as("sub-octo", "Octo", false);
    let cookie = app.browser_login("/").await;
    let get = |path: &str, header: bool| {
        let mut r = app
            .http
            .get(format!("{}{path}", app.base))
            .header("cookie", &cookie);
        if header {
            r = r.header("x-corntchap-console", "1");
        }
        r.send()
    };
    // cookie + console header: accepted on both /api/v1 and /api/v2
    assert_eq!(
        get("/api/v1/me", true).await.unwrap().status().as_u16(),
        200
    );
    assert_eq!(
        get("/api/v2/teams", true).await.unwrap().status().as_u16(),
        200
    );
    // cookie alone: rejected
    assert_eq!(
        get("/api/v1/me", false).await.unwrap().status().as_u16(),
        401
    );
    assert_eq!(
        get("/api/v2/teams", false).await.unwrap().status().as_u16(),
        401
    );
    // unsafe method: the Origin must be PUBLIC_URL's
    let post = |origin: Option<&str>| {
        let mut r = app
            .http
            .post(format!("{}/api/v2/$procs/whoami", app.base))
            .header("cookie", &cookie)
            .header("x-corntchap-console", "1")
            .json(&json!({}));
        if let Some(o) = origin {
            r = r.header("origin", o);
        }
        r.send()
    };
    let own = app.state.config.public_url.clone();
    assert_eq!(post(Some(&own)).await.unwrap().status().as_u16(), 200);
    assert_eq!(
        post(Some("https://evil.example"))
            .await
            .unwrap()
            .status()
            .as_u16(),
        401
    );
    assert_eq!(post(None).await.unwrap().status().as_u16(), 401);
    // MCP never takes cookies
    let r = app
        .http
        .post(format!("{}/mcp", app.base))
        .header("cookie", &cookie)
        .header("x-corntchap-console", "1")
        .header("origin", &own)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_ne!(r.status().as_u16(), 200);
}

/// An ID token without a usable subject never signs anyone in or creates anyone.
#[tokio::test]
async fn missing_sub_is_refused() {
    let app = common::start(false).await;
    app.sign_in_with(common::Ident {
        sub: String::new(),
        ..Default::default()
    });
    let r = login_raw(&app).await;
    assert!(
        r.status().is_client_error() || r.status().is_server_error(),
        "{}",
        r.status()
    );
    assert!(
        r.headers()
            .get("set-cookie")
            .is_none_or(|c| !c.to_str().unwrap().contains("ct_session="))
    );
    let n: i64 = sqlx::query_scalar("select count(*) from principals where kind = 'user'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(n, 0, "no principal without a subject");
}

/// Identity is the subject alone: the same username or e-mail (verified or not) under two subjects is two people,
/// and nobody can sign in as a pre-existing handle by claiming its name.
#[tokio::test]
async fn only_the_subject_links_an_account() {
    let app = common::start(false).await;
    // a person already exists under the handle `alice`, never signed in (no subject)
    app.add_user("alice").await;
    let ident = |sub: &str, verified: bool| common::Ident {
        sub: sub.into(),
        username: "alice".into(),
        email: "alice@example.test".into(),
        email_verified: verified,
        ..Default::default()
    };
    app.sign_in_with(ident("sub-1", true));
    app.browser_login("/").await;
    app.sign_in_with(ident("sub-2", false));
    app.browser_login("/").await;
    app.sign_in_with(ident("sub-3", true));
    app.browser_login("/").await;
    let (h1, h2, h3) = (
        app.handle_of("sub-1").await,
        app.handle_of("sub-2").await,
        app.handle_of("sub-3").await,
    );
    assert!(h1 != h2 && h2 != h3 && h1 != h3, "{h1} {h2} {h3}");
    for h in [&h1, &h2, &h3] {
        assert_ne!(h, "alice", "the IdP username never becomes the handle");
    }
    let sub_of_alice: Option<String> =
        sqlx::query_scalar("select oidc_sub from principals where name = 'alice'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(sub_of_alice, None, "the pre-existing alice was not claimed");
    // signing in again finds the same person by subject
    app.sign_in_with(ident("sub-2", true));
    app.browser_login("/").await;
    assert_eq!(app.handle_of("sub-2").await, h2);
    let n: i64 = sqlx::query_scalar("select count(*) from principals where kind = 'user'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(n, 4);
}

/// The display name comes from the ID token (`name`, else `email`, else none), is cleaned, and is shown escaped.
#[tokio::test]
async fn display_name_comes_from_the_token_and_is_escaped() {
    let app = common::start(false).await;
    let display = |app: &common::TestApp, sub: &str| {
        let (db, sub) = (app.state.db.clone(), sub.to_string());
        async move {
            sqlx::query_scalar::<_, Option<String>>(
                "select display_name from principals where oidc_sub = $1",
            )
            .bind(sub)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    // name wins; refreshed at every sign-in
    app.sign_in_with(common::Ident {
        sub: "s1".into(),
        name: Some("Selast Lambou".into()),
        ..Default::default()
    });
    app.browser_login("/").await;
    assert_eq!(display(&app, "s1").await.as_deref(), Some("Selast Lambou"));
    app.sign_in_with(common::Ident {
        sub: "s1".into(),
        name: Some("S. Lambou".into()),
        ..Default::default()
    });
    app.browser_login("/").await;
    assert_eq!(display(&app, "s1").await.as_deref(), Some("S. Lambou"));
    // no name: the e-mail
    app.sign_in_with(common::Ident {
        sub: "s2".into(),
        name: None,
        email: "x@y.test".into(),
        ..Default::default()
    });
    app.browser_login("/").await;
    assert_eq!(display(&app, "s2").await.as_deref(), Some("x@y.test"));
    // neither (empty strings): none, and still signs in
    app.sign_in_with(common::Ident {
        sub: "s3".into(),
        name: Some(String::new()),
        email: String::new(),
        ..Default::default()
    });
    app.browser_login("/").await;
    assert_eq!(display(&app, "s3").await, None);

    // hostile: markup, control characters and 2 KB
    let hostile = format!(
        "<script>alert(1)</script>\u{7}\u{202E}x{}",
        "y".repeat(2000)
    );
    app.sign_in_with(common::Ident {
        sub: "s4".into(),
        name: Some(hostile),
        ..Default::default()
    });
    let cookie = app.browser_login("/").await;
    let stored = display(&app, "s4").await.unwrap();
    assert_eq!(stored.chars().count(), 120, "truncated");
    assert!(
        !stored.chars().any(|c| c.is_control() || c == '\u{202E}'),
        "stripped"
    );
    assert!(stored.starts_with("<script>alert(1)</script>x"), "{stored}");
    // the server-rendered logout page shows it escaped
    let page = app
        .http
        .get(format!("{}/logout", app.base))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        page.contains("&lt;script&gt;alert(1)&lt;/script&gt;x"),
        "{page}"
    );
    assert!(!page.contains("<script>alert(1)"), "unescaped markup");
    assert!(page.contains(&format!("&middot; @{}", app.handle_of("s4").await)));
    // /auth/session carries it for the console (which escapes by rendering as text)
    let who: Value = app
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
    assert_eq!(who["display_name"], json!(stored));
}
