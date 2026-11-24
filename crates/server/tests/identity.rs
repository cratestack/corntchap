//! Handles and identity: random kitchen handles, race-safe just-in-time creation, renaming, admins.
mod common;
use corntchap::admin::{self, OidcIdentity, RESERVED_HANDLES};
use serde_json::{Value, json};

fn id_of(sub: &str) -> OidcIdentity<'_> {
    OidcIdentity {
        issuer: "https://idp.test",
        sub,
    }
}

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
    let id = admin::provision_user(db, &id_of("sub-a"), None, &mut seeded(11))
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
    let id = admin::provision_user(db, &id_of("sub-b"), None, &mut |_| 0)
        .await
        .unwrap();
    let got: String = sqlx::query_scalar("select name from principals where id = $1")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(got, stuck_long, "longer suffix after the short attempts");

    // and it gives up (does not loop) when even those are taken (sub-b holds the long one now)
    let err = admin::provision_user(db, &id_of("sub-c"), None, &mut |_| 0)
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
            admin::provision_user(&db, &id_of("sub-same"), Some("Same"), &mut seeded(100 + i))
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
            admin::provision_user(&db, &id_of(&format!("sub-{i}")), None, &mut seeded(5))
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
        v["message"].as_str().unwrap().contains("not available"),
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
    let (st, v) = rename(&app, &ta, json!("c1")).await;
    assert_eq!(st, 409, "{v}");
    assert!(
        v["message"].as_str().unwrap().contains("already taken"),
        "a cook's name: {v}"
    );

    // refused: reserved, mention syntax, too long, empty, odd characters, missing
    for bad in [
        "all",
        "ALL",
        "@x",
        &"a".repeat(65),
        "",
        "  ",
        "a b",
        "é",
        "all.",
        "--help",
        "-h",
        ".",
        "..",
        "a.",
        "_a",
        "st_x",
    ] {
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
    let (sub, role, src): (Option<String>, String, String) =
        sqlx::query_as("select oidc_sub, role, role_source from principals where name = $1")
            .bind(&name)
            .fetch_one(db)
            .await
            .unwrap();
    assert_eq!(
        (sub, role.as_str(), src.as_str()),
        (None, "super_admin", "init")
    );
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
async fn super_admins_are_made_and_unmade_but_never_the_last_one() {
    let app = common::start(false).await;
    let (root_name, root) = admin::init_admin(&app.state.db, None)
        .await
        .map(|(n, t)| (n, t.unwrap()))
        .unwrap();
    let (h, t) = person(&app, "sub-a").await;
    let set = |tok: String, who: String, on: bool| {
        let role = if on { "super_admin" } else { "member" };
        let app = &app;
        async move {
            app.rest(
                &tok,
                reqwest::Method::PATCH,
                &format!("/users/{who}"),
                Some(json!({"role": role})),
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
    assert!(
        v["message"].as_str().unwrap().contains("last super-admin"),
        "{v}"
    );
    assert_eq!(app.get(&t, "/me").await.1["role"], "super_admin");
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

#[test]
fn handle_syntax() {
    for bad in [
        ".",
        "..",
        "-h",
        "--help",
        "all.",
        "a.",
        ".a",
        "_a",
        "a_",
        "-a",
        "a-",
        "st_x",
        "ST_Y",
        "a b",
        "a/b",
        "a@b",
        "",
        " ",
        "é",
        &"a".repeat(65),
    ] {
        assert!(admin::norm_handle(bad).is_err(), "{bad:?}");
        assert!(admin::norm_handle_syntax(bad).is_err(), "{bad:?}");
    }
    for good in [
        "a",
        "a-b",
        "a.b_c",
        "x1",
        "0",
        &"a".repeat(64),
        "stx",
        "s_t",
        "Bob ",
    ] {
        assert!(admin::norm_handle(good).is_ok(), "{good:?}");
    }
    // cooks and fires follow the same syntax, but not the reserved list
    assert!(admin::norm_handle_syntax("all").is_ok());
    assert!(admin::norm_handle("all").is_err());
}

#[tokio::test]
async fn a_cook_cannot_be_named_like_a_path_segment() {
    let app = common::start(false).await;
    let (_, root) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .map(|(n, t)| (n, t.unwrap()))
        .unwrap();
    for bad in ["..", ".", "-h", "st_x", "all."] {
        let (st, v) = app
            .post(
                &root,
                "/agents",
                json!({"name": bad, "kind": "opencode", "runtime": "local"}),
            )
            .await;
        assert_eq!(st, 400, "{bad}: {v}");
    }
    let (st, _) = app
        .post(
            &root,
            "/agents",
            json!({"name": "ok.cook", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    assert_eq!(st, 200);
}

#[test]
fn word_lists_are_disjoint_and_without_duplicates() {
    let mut seen = std::collections::BTreeSet::new();
    for w in admin::ADJECTIVES.iter().chain(admin::DISHES) {
        assert!(seen.insert(*w), "{w} appears twice (in a list or in both)");
        assert!(w.chars().all(|c| c.is_ascii_lowercase()), "{w}");
    }
}

#[tokio::test]
async fn a_handle_is_never_given_to_a_different_account() {
    let app = common::start(false).await;
    let (bob, tb) = person(&app, "sub-bob").await;
    let (_mallory, tm) = person(&app, "sub-mallory").await;
    assert_eq!(rename(&app, &tb, json!("bob-2")).await.0, 200);
    // the vacated handle is refused to another person ...
    let (st, v) = rename(&app, &tm, json!(bob)).await;
    assert_eq!(st, 409, "{v}");
    assert!(
        v["message"].as_str().unwrap().contains("not available"),
        "{v}"
    );
    // ... to a cook or fire ...
    let (st, v) = app
        .post(
            &tm,
            "/agents",
            json!({"name": bob, "kind": "opencode", "runtime": "local"}),
        )
        .await;
    assert_eq!(st, 409, "{v}");
    // ... to a person named by init ...
    assert!(admin::init_admin(&app.state.db, Some(&bob)).await.is_err());
    // but the same account may take its own old handle back
    let (st, v) = rename(&app, &tb, json!(bob)).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(rename(&app, &tb, json!("bob-3")).await.0, 200);
    assert_eq!(rename(&app, &tb, json!(bob)).await.0, 200);
}

#[tokio::test]
async fn random_handles_skip_every_handle_ever_claimed() {
    let app = common::start(false).await;
    let db = &app.state.db;
    // a person who once had the handle the seed draws first, and renamed away
    let first = admin::generate_handle(&mut seeded(21), false);
    let ghost = uuid::Uuid::new_v4();
    sqlx::query("insert into handle_claims (name, principal_id) values ($1, $2)")
        .bind(&first)
        .bind(ghost)
        .execute(db)
        .await
        .unwrap();
    let id = admin::provision_user(db, &id_of("sub-gen"), None, &mut seeded(21))
        .await
        .unwrap();
    let got: String = sqlx::query_scalar("select name from principals where id = $1")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_ne!(got, first, "a claimed handle is not handed out again");
    let claim: uuid::Uuid =
        sqlx::query_scalar("select principal_id from handle_claims where name = $1")
            .bind(&got)
            .fetch_one(db)
            .await
            .unwrap();
    assert_eq!(claim, id, "the new handle is claimed by its holder");
}

/// Admin rows nobody can sign in as do not count as admins who could undo a demotion.
#[tokio::test]
async fn the_last_admin_guard_counts_only_admins_who_can_sign_in() {
    let app = common::start(false).await;
    let db = &app.state.db;
    app.sign_in_as("sub-a", "A", true);
    app.browser_login("/").await;
    let a = app.handle_of("sub-a").await;
    let ta = admin::operator_token(db, &a, None).await.unwrap();
    // a legacy admin that is not linked to any sign-in, and a linked admin that is disabled
    sqlx::query("insert into principals (kind, name, role, role_source) values ('user', 'legacy', 'super_admin', 'manual')")
        .execute(db)
        .await
        .unwrap();
    app.sign_in_as("sub-d", "D", true);
    app.browser_login("/").await;
    let d = app.handle_of("sub-d").await;
    sqlx::query("update principals set disabled_at = now() where name = $1")
        .bind(&d)
        .execute(db)
        .await
        .unwrap();
    let patch = |who: &str, body: Value| {
        let (app, ta, who) = (&app, ta.clone(), who.to_string());
        async move {
            app.rest(
                &ta,
                reqwest::Method::PATCH,
                &format!("/users/{who}"),
                Some(body),
            )
            .await
        }
    };
    let (st, v) = patch(&a, json!({"role": "member"})).await;
    assert_eq!(st, 409, "legacy and disabled admins do not count: {v}");
    let (st, v) = patch(&a, json!({"disabled": true})).await;
    assert_eq!(st, 409, "{v}");
    // an `init` admin (no sign-in by design) does count
    admin::init_admin(db, Some("boot")).await.unwrap();
    assert_eq!(patch(&a, json!({"role": "member"})).await.0, 200);
}

/// Two demotions in flight cannot both pass: the admin rows are locked. (A transaction stands in for the first one.)
#[tokio::test]
async fn demotions_are_serialised() {
    let app = common::start(false).await;
    let db = app.state.db.clone();
    for (sub, who) in [("sub-a", "A"), ("sub-b", "B")] {
        app.sign_in_as(sub, who, true);
        app.browser_login("/").await;
    }
    let (a, b) = (app.handle_of("sub-a").await, app.handle_of("sub-b").await);
    let pa = app.principal(&a).await;
    let mut hold = db.begin().await.unwrap();
    sqlx::query("select id from principals where role = 'super_admin' for update")
        .fetch_all(&mut *hold)
        .await
        .unwrap();
    let (db2, pa2, b2) = (db.clone(), pa.clone(), b.clone());
    let mut demote = tokio::spawn(async move {
        corntchap::roles::set_role(&db2, &pa2, &b2, corntchap::state::Role::Member).await
    });
    let early = tokio::time::timeout(std::time::Duration::from_millis(500), &mut demote).await;
    assert!(
        early.is_err(),
        "the demotion must wait for the lock on the admin rows"
    );
    // the other demotion lands first, then the waiting one sees one admin left and is refused
    sqlx::query("update principals set role = 'member', role_source = 'manual' where name = $1")
        .bind(&a)
        .execute(&mut *hold)
        .await
        .unwrap();
    hold.commit().await.unwrap();
    let r = demote.await.unwrap();
    assert!(r.is_err(), "{r:?}");
    let left: i64 = sqlx::query_scalar(
        "select count(*) from principals where role = 'super_admin' and disabled_at is null",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(left, 1);
}

#[tokio::test]
async fn disabling_a_person_ends_their_access() {
    let app = common::start(false).await;
    let db = &app.state.db;
    let (_, root) = admin::init_admin(db, Some("octo"))
        .await
        .map(|(n, t)| (n, t.unwrap()))
        .unwrap();
    app.sign_in_as("sub-p", "P", false);
    let cookie = app.browser_login("/").await;
    let p = app.handle_of("sub-p").await;
    let tp = admin::operator_token(db, &p, None).await.unwrap();
    let api = |c: &str, path: &'static str| {
        let r = app
            .http
            .get(format!("{}{path}", app.base))
            .header("cookie", c)
            .header("x-corntchap-console", "1");
        async move { r.send().await.unwrap().status().as_u16() }
    };
    assert_eq!(app.get(&tp, "/me").await.0, 200);
    assert_eq!(api(&cookie, "/api/v1/me").await, 200);
    // a non-admin cannot; an admin can
    assert_eq!(
        app.rest(
            &tp,
            reqwest::Method::PATCH,
            &format!("/users/{p}"),
            Some(json!({"disabled": true}))
        )
        .await
        .0,
        403
    );
    assert_eq!(
        app.rest(
            &root,
            reqwest::Method::PATCH,
            &format!("/users/{p}"),
            Some(json!({"disabled": true}))
        )
        .await
        .0,
        200
    );
    assert_eq!(app.get(&tp, "/me").await.0, 401);
    assert_eq!(api(&cookie, "/api/v1/me").await, 401);
    assert_eq!(api(&cookie, "/auth/session").await, 401);
    // disabling revoked them for good: enabling does not revive the old token or session
    let revoked: i64 = sqlx::query_scalar("select count(*) from tokens t join principals p on p.id = t.principal_id where p.name = $1 and t.revoked_at is null").bind(&p).fetch_one(db).await.unwrap();
    let sessions: i64 = sqlx::query_scalar("select count(*) from auth_sessions s join principals p on p.id = s.user_id where p.name = $1").bind(&p).fetch_one(db).await.unwrap();
    assert_eq!((revoked, sessions), (0, 0));
    assert_eq!(
        app.rest(
            &root,
            reqwest::Method::PATCH,
            &format!("/users/{p}"),
            Some(json!({"disabled": false}))
        )
        .await
        .0,
        200
    );
    assert_eq!(app.get(&tp, "/me").await.0, 401);
    assert_eq!(api(&cookie, "/api/v1/me").await, 401);
    // the last admin who can sign in cannot be disabled, not even by themselves
    let (st, v) = app
        .rest(
            &root,
            reqwest::Method::PATCH,
            "/users/octo",
            Some(json!({"disabled": true})),
        )
        .await;
    assert_eq!(st, 409, "{v}");
}

/// Access guards that do not depend on the revocation: a row disabled behind the server's back is still locked out.
#[tokio::test]
async fn a_disabled_row_is_refused_by_every_door() {
    let app = common::start(false).await;
    let db = &app.state.db;
    app.sign_in_as("sub-p", "P", false);
    let cookie = app.browser_login("/").await;
    let p = app.handle_of("sub-p").await;
    let tp = admin::operator_token(db, &p, None).await.unwrap();
    sqlx::query("update principals set disabled_at = now() where name = $1")
        .bind(&p)
        .execute(db)
        .await
        .unwrap();
    assert_eq!(app.get(&tp, "/me").await.0, 401, "bearer token");
    for path in ["/api/v1/me", "/auth/session"] {
        let r = app
            .http
            .get(format!("{}{path}", app.base))
            .header("cookie", &cookie)
            .header("x-corntchap-console", "1")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 401, "{path}");
    }
}

/// A browser session is only ever for a person: a session row pointing at a cook authenticates nothing.
#[tokio::test]
async fn a_session_for_a_cook_authenticates_nothing() {
    use authkestra::core::auth::SessionStore;
    let app = common::start(false).await;
    let db = &app.state.db;
    let cook: uuid::Uuid = sqlx::query_scalar(
        "insert into principals (kind, name) values ('agent', 'cook-q') returning id",
    )
    .fetch_one(db)
    .await
    .unwrap();
    let store = corntchap::sessions::PgSessionStore { db: db.clone() };
    store
        .save_session(&corntchap::sessions::new_session(
            "tok-cook".into(),
            cook,
            "cook-q",
            "csrf".into(),
            None,
            600,
        ))
        .await
        .unwrap();
    for path in ["/api/v1/me", "/auth/session"] {
        let r = app
            .http
            .get(format!("{}{path}", app.base))
            .header("cookie", "ct_session=tok-cook")
            .header("x-corntchap-console", "1")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 401, "{path}");
    }
}

#[test]
fn display_names_drop_invisible_and_lookalike_characters() {
    // every one of these is removed by category (Cc Cf Co Cs Zl Zp) or by the fillers / separator lists
    let removed: &[(&str, char)] = &[
        ("Cc NUL", '\u{0}'),
        ("Cc BEL", '\u{7}'),
        ("Cc DEL", '\u{7F}'),
        ("Cc NEL", '\u{85}'),
        ("Cf ALM", '\u{61C}'),
        ("Cf SHY", '\u{AD}'),
        ("Cf MVS", '\u{180E}'),
        ("Cf ZWSP", '\u{200B}'),
        ("Cf LRM", '\u{200E}'),
        ("Cf RLM", '\u{200F}'),
        ("Cf LRE", '\u{202A}'),
        ("Cf RLO", '\u{202E}'),
        ("Cf WJ", '\u{2060}'),
        ("Cf 2064", '\u{2064}'),
        ("Cf LRI", '\u{2066}'),
        ("Cf RLI", '\u{2067}'),
        ("Cf FSI", '\u{2068}'),
        ("Cf PDI", '\u{2069}'),
        ("Cf 206A", '\u{206A}'),
        ("Cf 206F", '\u{206F}'),
        ("Cf BOM", '\u{FEFF}'),
        ("Cf FFF9", '\u{FFF9}'),
        ("Cf FFFB", '\u{FFFB}'),
        ("Cf TAG", '\u{E0001}'),
        ("Cf TAG A", '\u{E0041}'),
        ("Co", '\u{E000}'),
        ("Co plane 15", '\u{F0000}'),
        ("Zl", '\u{2028}'),
        ("Zp", '\u{2029}'),
        ("CGJ", '\u{34F}'),
        ("Hangul choseong filler", '\u{115F}'),
        ("Hangul jungseong filler", '\u{1160}'),
        ("Hangul filler", '\u{3164}'),
        ("halfwidth Hangul filler", '\u{FFA0}'),
        ("middle dot", '\u{B7}'),
        ("bullet", '\u{2022}'),
        ("hyphenation point", '\u{2027}'),
        ("katakana middle dot", '\u{30FB}'),
        ("halfwidth katakana middle dot", '\u{FF65}'),
    ];
    for (what, c) in removed {
        let got = admin::clean_display_name(&format!("A{c}B")).unwrap();
        assert_eq!(got, "AB", "{what} U+{:04X} survived", *c as u32);
    }
    // ordinary text survives: marks, emoji, CJK, ordinary spaces (a zero-width joiner is format, so ZWJ emoji
    // sequences fall apart into their parts)
    assert_eq!(
        admin::clean_display_name("Ada  Ünïcode 山田 太郎 👩‍💻").as_deref(),
        Some("Ada  Ünïcode 山田 太郎 👩💻")
    );
    // a fake "name · @handle" is flattened
    assert_eq!(
        admin::clean_display_name("Alice Admin · @alice").as_deref(),
        Some("Alice Admin  @alice")
    );
    // nothing left (or only space) is no name
    for empty in ["", "   ", "\u{200B}\u{3164}", "\n\t", " \u{B7} "] {
        assert_eq!(admin::clean_display_name(empty), None, "{empty:?}");
    }
}

#[test]
fn display_names_are_cut_on_a_grapheme_boundary() {
    // a combining accent at the boundary: the whole cluster goes, never a bare letter
    let s = format!("{}e\u{301}", "a".repeat(119));
    let got = admin::clean_display_name(&s).unwrap();
    assert_eq!(
        got,
        "a".repeat(119),
        "e + combining acute is one cluster of two chars"
    );
    // a flag (two regional indicators) at the boundary: not cut in half
    let s = format!("{}🇫🇷", "a".repeat(119));
    assert_eq!(admin::clean_display_name(&s).unwrap(), "a".repeat(119));
    // fits exactly
    let s = format!("{}🇫🇷", "a".repeat(118));
    let got = admin::clean_display_name(&s).unwrap();
    assert_eq!(got.chars().count(), 120);
    assert!(got.ends_with("🇫🇷"));
    // many accents on one letter beyond the limit: dropped as a cluster
    let s = format!("{}a{}", "b".repeat(100), "\u{301}".repeat(50));
    assert_eq!(admin::clean_display_name(&s).unwrap(), "b".repeat(100));
}

#[tokio::test]
async fn init_refuses_a_person_who_signs_in_through_the_idp() {
    let app = common::start(false).await;
    let (h, _) = person(&app, "sub-a").await;
    let err = admin::init_admin(&app.state.db, Some(&h))
        .await
        .unwrap_err();
    assert!(matches!(err, corntchap::error::Error::Conflict(_)), "{err}");
    let flag: bool =
        sqlx::query_scalar("select role = 'super_admin' from principals where name = $1")
            .bind(&h)
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert!(!flag, "nothing changed");
}
