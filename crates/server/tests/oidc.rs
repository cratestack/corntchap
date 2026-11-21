mod common;
use corntchap::admin;
use serde_json::{Value, json};

async fn row(app: &common::TestApp, name: &str) -> (bool, String) {
    sqlx::query_as("select is_admin, admin_source from principals where name = $1")
        .bind(name)
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
    app.browser_login("/board").await;
    assert_eq!(row(&app, "dev").await, (false, "oidc".to_string()));

    // the admin role is synced on every sign-in, both ways
    app.sign_in_as("sub-dev", "Dev", true);
    app.browser_login("/board").await;
    assert_eq!(row(&app, "dev").await, (true, "oidc".to_string()));
    app.sign_in_as("sub-dev", "Dev", false);
    app.browser_login("/board").await;
    assert_eq!(row(&app, "dev").await, (false, "oidc".to_string()));

    // admins made by `init` are never demoted by the IdP
    admin::init_admin(&app.state.db, "boss").await.unwrap();
    app.sign_in_as("sub-boss", "Boss", false);
    app.browser_login("/board").await;
    assert_eq!(row(&app, "boss").await, (true, "init".to_string()));

    // a name held by an agent is refused with a clear page
    sqlx::query("insert into principals (kind, name) values ('agent', 'clash')")
        .execute(&app.state.db)
        .await
        .unwrap();
    app.sign_in_as("sub-clash", "Clash", false);
    let r = login_raw(&app).await;
    assert_eq!(r.status().as_u16(), 409);
    assert!(r.text().await.unwrap().contains("already belongs"));
}

#[tokio::test]
async fn logout_revokes_token_and_session() {
    let app = common::start(false).await;
    admin::init_admin(&app.state.db, "octo").await.unwrap();
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
    let cookie = app.browser_login("/board").await;
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
        .get(format!("{}/board", app.base))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(
        after.status().as_u16(),
        303,
        "session is gone: redirected to login"
    );
    assert!(
        after.headers()["location"]
            .to_str()
            .unwrap()
            .starts_with("/auth/oidc/login")
    );
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
