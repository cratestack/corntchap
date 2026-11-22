//! Handles and identity: random kitchen handles, race-safe just-in-time creation, renaming, admins.
mod common;
use corntchap::admin::{self, RESERVED_HANDLES};
use serde_json::{Value, json};

/// A seeded source of randomness (xorshift), so a test can force the same draws twice.
fn seeded(seed: u64) -> impl FnMut(usize) -> usize + Send {
    let mut x = seed.max(1);
    move |n| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % n as u64) as usize
    }
}

#[test]
fn generated_handles_are_valid_and_never_reserved() {
    let mut pick = seeded(7);
    for i in 0..20_000 {
        let h = admin::generate_handle(&mut pick, i % 3 == 0);
        let n = admin::norm_handle(&h).unwrap_or_else(|e| panic!("{h}: {e}"));
        assert_eq!(n, h, "already normal");
        assert!(!RESERVED_HANDLES.contains(&h.as_str()));
        assert!(h.len() <= 64);
        let digits = h.rsplit('-').next().unwrap();
        assert!(digits.chars().all(|c| c.is_ascii_digit()));
        assert!(
            if i % 3 == 0 {
                digits.len() == 6
            } else {
                (2..=3).contains(&digits.len())
            },
            "{h}"
        );
    }
    // reserved words are refused as handles, in any case
    for r in RESERVED_HANDLES.iter().chain(&["all", "everyone"]) {
        assert!(admin::norm_handle(r).is_err(), "{r}");
        assert!(admin::norm_handle(&r.to_uppercase()).is_err(), "{r}");
    }
    assert_eq!(
        admin::norm_handle("  Saffron-Okra-42 ").unwrap(),
        "saffron-okra-42"
    );
    for bad in ["", "   ", "a@b", "@x", "has space", &"a".repeat(65)] {
        assert!(admin::norm_handle(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn display_names_are_cleaned() {
    assert_eq!(
        admin::clean_display_name("  Ada \u{7} Lovelace\u{200B}\n").as_deref(),
        Some("Ada  Lovelace")
    );
    assert_eq!(admin::clean_display_name("\u{7}\u{202E} \n"), None);
    assert_eq!(
        admin::clean_display_name("é<b>&\"").as_deref(),
        Some("é<b>&\"")
    );
    let long = admin::clean_display_name(&"é".repeat(500)).unwrap();
    assert_eq!(long.chars().count(), admin::MAX_DISPLAY_NAME);
}

#[tokio::test]
async fn handle_collisions_retry_with_bounded_attempts() {
    let app = common::start(false).await;
    let db = &app.state.db;
    // the first draw of this seed is taken: the second draw is used
    let first = admin::generate_handle(&mut seeded(11), false);
    app.add_user(&first).await;
    let id = admin::provision_user(db, "sub-a", None, &mut seeded(11))
        .await
        .unwrap();
    let got: String = sqlx::query_scalar("select name from principals where id = $1")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_ne!(got, first);
    assert!(admin::norm_handle(&got).is_ok());

    // a stuck source of randomness: after the short attempts the suffix grows to six digits
    let stuck_short = admin::generate_handle(&mut |_| 0, false);
    let stuck_long = admin::generate_handle(&mut |_| 0, true);
    assert_ne!(stuck_short, stuck_long);
    app.add_user(&stuck_short).await;
    let id = admin::provision_user(db, "sub-b", None, &mut |_| 0)
        .await
        .unwrap();
    let got: String = sqlx::query_scalar("select name from principals where id = $1")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(got, stuck_long, "longer suffix after the short attempts");

    // and it gives up (does not loop) when even those are taken (sub-b holds the long one now)
    let err = admin::provision_user(db, "sub-c", None, &mut |_| 0)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("internal") || err.to_string().contains("collisions"),
        "{err}"
    );
    let n: i64 = sqlx::query_scalar("select count(*) from principals where oidc_sub = 'sub-c'")
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn concurrent_first_sign_ins_end_in_one_principal_per_sub() {
    let app = common::start(false).await;
    let db = app.state.db.clone();
    // the same sub, many at once, each drawing a different handle: exactly one principal
    let mut tasks = vec![];
    for i in 0..16u64 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            admin::provision_user(&db, "sub-same", Some("Same"), &mut seeded(100 + i))
                .await
                .unwrap()
        }));
    }
    let mut ids = std::collections::BTreeSet::new();
    for t in tasks {
        ids.insert(t.await.unwrap());
    }
    assert_eq!(ids.len(), 1, "one id for every racer");
    let n: i64 = sqlx::query_scalar("select count(*) from principals where oidc_sub = 'sub-same'")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(n, 1);

    // two different subs drawing the very same handle: two principals, two handles
    let mut tasks = vec![];
    for i in 0..8 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            admin::provision_user(&db, &format!("sub-{i}"), None, &mut seeded(5))
                .await
                .unwrap()
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let (subs, handles): (i64, i64) = sqlx::query_as(
        "select count(distinct oidc_sub), count(distinct name) from principals where oidc_sub like 'sub-%' and oidc_sub <> 'sub-same'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!((subs, handles), (8, 8));
}

/// Sign `sub` in and give back (handle, bearer token).
async fn person(app: &common::TestApp, sub: &str) -> (String, String) {
    app.sign_in_as(sub, sub, false);
    app.browser_login("/").await;
    let h = app.handle_of(sub).await;
    let t = admin::operator_token(&app.state.db, &h, None)
        .await
        .unwrap();
    (h, t)
}

async fn rename(app: &common::TestApp, token: &str, name: Value) -> (u16, Value) {
    app.rest(
        token,
        reqwest::Method::PATCH,
        "/me",
        Some(json!({ "name": name })),
    )
    .await
}

#[tokio::test]
async fn rename_rules() {
    let app = common::start(false).await;
    let (a, ta) = person(&app, "sub-a").await;
    let (b, _tb) = person(&app, "sub-b").await;

    // a new handle works, is normalised, is recorded as an event, and /me shows it
    let (st, v) = rename(&app, &ta, json!("Saffron-Okra-1")).await;
    assert_eq!(
        (st, v["name"].as_str(), v["previous"].as_str()),
        (200, Some("saffron-okra-1"), Some(a.as_str())),
        "{v}"
    );
    let (_, me) = app.get(&ta, "/me").await;
    assert_eq!(me["name"], "saffron-okra-1");
    let ev: Value =
        sqlx::query_scalar("select payload from events where kind = 'principal.renamed'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(ev, json!({"from": a, "to": "saffron-okra-1"}));
    // renaming to your own handle is a no-op, not an event or an error
    assert_eq!(rename(&app, &ta, json!("saffron-okra-1")).await.0, 200);
    let n: i64 = sqlx::query_scalar("select count(*) from events where kind = 'principal.renamed'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(n, 1);

    // taken: 409 with a clear message (another person's handle, or a cook's name)
    let (st, v) = rename(&app, &ta, json!(b)).await;
    assert_eq!(st, 409, "{v}");
    assert!(
        v["message"].as_str().unwrap().contains("already taken"),
        "{v}"
    );
    let (st, _) = app
        .post(
            &ta,
            "/agents",
            json!({"name": "c1", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(rename(&app, &ta, json!("c1")).await.0, 409);

    // refused: reserved, mention syntax, too long, empty, odd characters, missing
    for bad in ["all", "ALL", "@x", &"a".repeat(65), "", "  ", "a b", "é"] {
        let (st, v) = rename(&app, &ta, json!(bad)).await;
        assert_eq!(st, 400, "{bad:?}: {v}");
    }
    assert_eq!(
        app.rest(&ta, reqwest::Method::PATCH, "/me", Some(json!({})))
            .await
            .0,
        400
    );
    assert_eq!(
        app.get(&ta, "/me").await.1["name"],
        "saffron-okra-1",
        "refusals changed nothing"
    );
}

#[tokio::test]
async fn only_an_enabled_person_can_rename() {
    let app = common::start(false).await;
    let (h, t) = person(&app, "sub-a").await;
    // a cook (even one owned by this person) cannot use the self-rename endpoint
    let (_, ag) = app
        .post(
            &t,
            "/agents",
            json!({"name": "c1", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    let cook = ag["token"].as_str().unwrap().to_string();
    let (st, v) = rename(&app, &cook, json!("cooked")).await;
    assert_eq!(st, 403, "{v}");
    assert!(
        v["message"].as_str().unwrap().contains("only a person"),
        "{v}"
    );
    let (_, fire) = app.post(&t, "/relays", json!({"name": "f1"})).await;
    if let Some(ft) = fire["token"].as_str() {
        assert_eq!(rename(&app, ft, json!("fired")).await.0, 403);
    }
    let name: String = sqlx::query_scalar("select name from principals where name = 'c1'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(name, "c1");
    // a disabled person: the token stops working, and the function itself refuses too
    let p = app.principal(&h).await;
    sqlx::query("update principals set disabled_at = now() where name = $1")
        .bind(&h)
        .execute(&app.state.db)
        .await
        .unwrap();
    assert_eq!(rename(&app, &t, json!("zombie")).await.0, 401);
    let err = admin::rename_self(&app.state.db, &p, "zombie")
        .await
        .unwrap_err();
    assert!(
        matches!(err, corntchap::error::Error::Forbidden(_)),
        "{err}"
    );
    let still: i64 = sqlx::query_scalar("select count(*) from principals where name = $1")
        .bind(&h)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(still, 1);
}

#[tokio::test]
async fn after_a_rename_the_new_handle_resolves_and_the_old_does_not() {
    let app = common::start(false).await;
    let (_, root) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .map(|(n, t)| (n, t.unwrap()))
        .unwrap();
    let (h, t) = person(&app, "sub-a").await;
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    app.post(&root, "/teams/acme/members", json!({"principal": h}))
        .await;
    let (_, ag) = app
        .post(
            &root,
            "/agents",
            json!({"name": "c1", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    app.post(&root, "/teams/acme/members", json!({"principal": "c1"}))
        .await;
    let cook = ag["token"].as_str().unwrap().to_string();
    let to = |who: &str| json!({"project": "PAY", "message": format!("ping @{who}"), "to": who});
    let r = app.mcp(&cook, "pass", to(&h)).await;
    assert_ne!(r["isError"], true, "{r}");

    assert_eq!(rename(&app, &t, json!("merry-ladle-9")).await.0, 200);
    let r = app.mcp(&cook, "pass", to("merry-ladle-9")).await;
    assert_ne!(r["isError"], true, "new handle resolves: {r}");
    let r = app.mcp(&cook, "pass", to(&h)).await;
    assert_eq!(r["isError"], true, "the old handle no longer resolves: {r}");
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no chef named"),
        "{r}"
    );
    // team membership followed the person, not the name
    let (_, members) = app.get(&root, "/teams/acme/members").await;
    assert!(
        members
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["name"] == "merry-ladle-9"),
        "{members}"
    );
}

#[tokio::test]
async fn init_makes_a_local_admin_without_an_oidc_link() {
    let app = common::start(false).await;
    let db = &app.state.db;
    let (name, tok) = admin::init_admin(db, None).await.unwrap();
    assert!(admin::norm_handle(&name).is_ok());
    let (sub, admin_flag, src): (Option<String>, bool, String) =
        sqlx::query_as("select oidc_sub, is_admin, admin_source from principals where name = $1")
            .bind(&name)
            .fetch_one(db)
            .await
            .unwrap();
    assert_eq!((sub, admin_flag, src.as_str()), (None, true, "init"));
    assert_eq!(app.get(&tok.unwrap(), "/me").await.1["name"], json!(name));
    // re-running without a name reuses the same admin (a fresh token), a named one creates another
    let (again, t2) = admin::init_admin(db, None).await.unwrap();
    assert_eq!(again, name);
    assert!(t2.is_some());
    let (named, _) = admin::init_admin(db, Some("Chef-One")).await.unwrap();
    assert_eq!(named, "chef-one");
    assert!(
        admin::init_admin(db, Some("all")).await.is_err(),
        "reserved"
    );
}

#[tokio::test]
async fn admins_are_made_and_unmade_but_never_the_last_one() {
    let app = common::start(false).await;
    let (root_name, root) = admin::init_admin(&app.state.db, None)
        .await
        .map(|(n, t)| (n, t.unwrap()))
        .unwrap();
    let (h, t) = person(&app, "sub-a").await;
    let set = |tok: String, who: String, on: bool| {
        let app = &app;
        async move {
            app.rest(
                &tok,
                reqwest::Method::PATCH,
                &format!("/users/{who}"),
                Some(json!({"is_admin": on})),
            )
            .await
        }
    };
    // a non-admin cannot
    assert_eq!(set(t.clone(), h.clone(), true).await.0, 403);
    // an admin can promote, and the promoted one may then demote the first
    assert_eq!(set(root.clone(), h.clone(), true).await.0, 200);
    let (st, v) = set(t.clone(), root_name.clone(), false).await;
    assert_eq!(st, 200, "{v}");
    // now `h` is the last admin: demoting yourself, or the last admin by anyone, is refused
    let (st, v) = set(t.clone(), h.clone(), false).await;
    assert_eq!(st, 409, "{v}");
    assert!(v["message"].as_str().unwrap().contains("last admin"), "{v}");
    assert_eq!(app.get(&t, "/me").await.1["is_admin"], true);
    assert_eq!(set(t.clone(), "nobody-here".into(), true).await.0, 404);
    assert_eq!(
        app.rest(
            &t,
            reqwest::Method::PATCH,
            &format!("/users/{h}"),
            Some(json!({}))
        )
        .await
        .0,
        400
    );
}

#[tokio::test]
async fn pre_provisioning_is_gone() {
    let app = common::start(false).await;
    let (_, root) = admin::init_admin(&app.state.db, None)
        .await
        .map(|(n, t)| (n, t.unwrap()))
        .unwrap();
    let (st, _) = app.post(&root, "/users", json!({"login": "alice"})).await;
    assert_eq!(st, 405, "POST /users no longer exists");
    let (st, users) = app.get(&root, "/users").await;
    assert_eq!(st, 200);
    assert!(
        users
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u.get("login").is_none()),
        "{users}"
    );
    // the MCP tool is gone too
    let r = app
        .mcp(&root, "create_user", json!({"login": "alice"}))
        .await;
    assert!(r.is_null() || r["isError"] == true, "{r}");
}
