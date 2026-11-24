//! Picking a pot's repositories from GitHub (src/forge/catalog.rs and pick.rs) against a fake GitHub:
//! the app JWT, the installation token and its reuse, pagination, `in_pot`, the query filter, who may
//! list and add, and the bulk add (names, default branches, skips).
mod common;
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use common::TestApp;
use corntchap::admin;
use serde_json::{Value, json};
use std::collections::HashMap;

const PEM: &str = include_str!("common/test_rsa.pem");
const N: &str = include_str!("common/test_rsa_n.txt");

#[derive(Default)]
struct Fake {
    base: String,
    /// (installation id, the JWT it was requested with)
    token_requests: Vec<(i64, String)>,
    /// (installation token, page) per listing request
    listings: Vec<(String, i64)>,
    /// the title the fake gives acme/shop#5
    title5: String,
}

type Shared = Arc<Mutex<Fake>>;

fn repo(full: &str, branch: &str, pushed: &str, private: bool, archived: bool) -> Value {
    json!({"full_name": full, "html_url": format!("https://github.com/{full}"),
           "clone_url": format!("https://github.com/{full}.git"), "default_branch": branch,
           "private": private, "archived": archived, "description": format!("the {full} repo"),
           "pushed_at": pushed})
}

fn repos_of(installation: i64) -> Vec<Value> {
    match installation {
        77 => vec![
            repo("acme/shop", "main", "2026-11-01T10:00:00Z", true, false),
            repo("acme/api", "develop", "2026-11-20T10:00:00Z", false, false),
            repo("acme/web", "main", "2026-11-10T10:00:00Z", false, false),
        ],
        _ => vec![
            repo("other/api", "trunk", "2026-11-15T10:00:00Z", false, false),
            repo("other/old", "main", "2026-12-30T10:00:00Z", false, true),
        ],
    }
}

async fn start_fake() -> Shared {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let shared: Shared = Arc::new(Mutex::new(Fake {
        base,
        title5: "Refund screen".into(),
        ..Default::default()
    }));
    async fn token(
        State(s): State<Shared>,
        Path(id): Path<i64>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let jwt = headers["authorization"]
            .to_str()
            .unwrap()
            .trim_start_matches("Bearer ")
            .to_string();
        let mut f = s.lock().unwrap();
        f.token_requests.push((id, jwt));
        let n = f.token_requests.len();
        // installation 88's tokens are nearly expired: they cannot be reused
        let life = if id == 88 { 30 } else { 3600 };
        let exp = chrono::Utc::now() + chrono::Duration::seconds(life);
        (
            StatusCode::CREATED,
            Json(json!({"token": format!("ghs_{id}_{n}"), "expires_at": exp.to_rfc3339()})),
        )
    }
    async fn list(
        State(s): State<Shared>,
        Query(q): Query<HashMap<String, String>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let tok = headers["authorization"]
            .to_str()
            .unwrap()
            .trim_start_matches("Bearer ")
            .to_string();
        let page: i64 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
        assert_eq!(q.get("per_page").map(String::as_str), Some("100"));
        let inst: i64 = tok
            .split('_')
            .nth(1)
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        let base = {
            let mut f = s.lock().unwrap();
            f.listings.push((tok.clone(), page));
            f.base.clone()
        };
        // two repositories per page, to make the client follow the Link header
        let all = repos_of(inst);
        let chunk: Vec<Value> = all
            .iter()
            .skip(((page - 1) * 2) as usize)
            .take(2)
            .cloned()
            .collect();
        let mut h = HeaderMap::new();
        if (page * 2) < all.len() as i64 {
            let next = format!(
                "<{base}/installation/repositories?per_page=100&page={}>; rel=\"next\"",
                page + 1
            );
            h.insert("link", HeaderValue::from_str(&next).unwrap());
        }
        (
            h,
            Json(
                json!({"total_count": all.len(), "repository_selection": "all", "repositories": chunk}),
            ),
        )
    }
    fn pr_json(repo: &str, n: i64, title5: &str) -> Option<Value> {
        let (title, branch, state, merged, sha) = match (repo, n) {
            ("acme/shop", 5) => (title5, "feat/refunds", "closed", true, "abc5"),
            ("acme/web", 1) => ("Web part", "shop-1-web-part", "open", false, "def1"),
            ("acme/web", 2) => ("Something else", "random", "open", false, "def2"),
            _ => return None,
        };
        Some(
            json!({"number": n, "html_url": format!("https://github.com/{repo}/pull/{n}"), "title": title,
            "body": "", "state": state, "merged": merged, "draft": false,
            "head": {"ref": branch, "sha": sha}, "base": {"ref": "main",
                "repo": {"full_name": repo, "html_url": format!("https://github.com/{repo}"), "clone_url": format!("https://github.com/{repo}.git")}},
            "user": {"login": "ann"}, "mergeable": null, "mergeable_state": "unknown"}),
        )
    }
    async fn repo_installation(
        Path((o, _r)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        assert!(
            headers["authorization"]
                .to_str()
                .unwrap()
                .starts_with("Bearer ey"),
            "the app JWT"
        );
        match o.as_str() {
            "acme" => (StatusCode::OK, Json(json!({"id": 77}))),
            "other" => (StatusCode::OK, Json(json!({"id": 88}))),
            _ => (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))),
        }
    }
    async fn pull(
        State(s): State<Shared>,
        Path((o, r, n)): Path<(String, String, i64)>,
    ) -> impl IntoResponse {
        let t = s.lock().unwrap().title5.clone();
        match pr_json(&format!("{o}/{r}"), n, &t) {
            Some(v) => (StatusCode::OK, Json(v)),
            None => (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))),
        }
    }
    async fn pulls(State(s): State<Shared>, Path((o, r)): Path<(String, String)>) -> Json<Value> {
        let t = s.lock().unwrap().title5.clone();
        let repo = format!("{o}/{r}");
        Json(Value::Array(
            (1..10)
                .filter_map(|n| pr_json(&repo, n, &t))
                .filter(|p| p["state"] == "open")
                .collect(),
        ))
    }
    async fn reviews(Path((o, r, n)): Path<(String, String, i64)>) -> Json<Value> {
        Json(if (o.as_str(), r.as_str(), n) == ("acme", "shop", 5) {
            json!([
                {"state": "CHANGES_REQUESTED", "user": {"login": "carol"}, "body": "no", "html_url": "https://x/1"},
                {"state": "APPROVED", "user": {"login": "carol"}, "body": "ok now", "html_url": "https://x/2"},
                {"state": "COMMENTED", "user": {"login": "dave"}, "body": "hm", "html_url": "https://x/3"}
            ])
        } else {
            json!([])
        })
    }
    async fn check_runs(Path((_o, _r, sha)): Path<(String, String, String)>) -> Json<Value> {
        Json(match sha.as_str() {
            "abc5" => json!({"total_count": 2, "check_runs": [
                {"name": "lint", "status": "completed", "conclusion": "success", "html_url": "https://x/lint", "output": {"title": "ok"}},
                {"name": "test", "status": "completed", "conclusion": "failure", "html_url": "https://x/test", "output": {"title": "2 failed"}}]}),
            "def1" => json!({"total_count": 1, "check_runs": [
                {"name": "build", "status": "in_progress", "conclusion": null, "html_url": "https://x/b", "output": {"title": ""}}]}),
            _ => json!({"total_count": 0, "check_runs": []}),
        })
    }
    let router = Router::new()
        .route("/repos/{o}/{r}/installation", get(repo_installation))
        .route("/repos/{o}/{r}/pulls", get(pulls))
        .route("/repos/{o}/{r}/pulls/{n}", get(pull))
        .route("/repos/{o}/{r}/pulls/{n}/reviews", get(reviews))
        .route("/repos/{o}/{r}/commits/{sha}/check-runs", get(check_runs))
        .route("/app/installations/{id}/access_tokens", post(token))
        .route("/installation/repositories", get(list))
        .with_state(shared.clone());
    tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
    shared
}

async fn tool(app: &TestApp, tok: &str, name: &str, args: Value) -> Value {
    let r = app.mcp(tok, name, args.clone()).await;
    assert_ne!(r["isError"], true, "{name}({args}): {r}");
    TestApp::payload(&r)
}

async fn refused(app: &TestApp, tok: &str, name: &str, args: Value) -> String {
    let r = app.mcp(tok, name, args.clone()).await;
    assert_eq!(r["isError"], true, "{name}({args}) should be refused: {r}");
    r.to_string()
}

struct World {
    app: TestApp,
    fake: Shared,
    root: String,
    alice: String,
    bob: String,
    carol: String,
}

/// octo (admin), alice (owner of team acme and its pot SHOP, which has acme/shop), bob (member),
/// carol (in no team), one app connection pointing at the fake, installed on acme (77) and other (88).
async fn world() -> World {
    let app = common::start(false).await;
    let fake = start_fake().await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "bob", "carol"] {
        app.add_user(u).await;
    }
    app.set_role("alice", "orchestrator").await; // owning a team needs the role
    let token = |u: &'static str| {
        let db = app.state.db.clone();
        async move { admin::operator_token(&db, u, None).await.unwrap() }
    };
    let (alice, bob, carol) = (
        token("alice").await,
        token("bob").await,
        token("carol").await,
    );
    tool(&app, &root, "create_team", json!({"name": "acme"})).await;
    for (who, role) in [("alice", "owner"), ("bob", "member")] {
        tool(
            &app,
            &root,
            "add_team_member",
            json!({"team": "acme", "principal": who, "role": role}),
        )
        .await;
    }
    tool(
        &app,
        &alice,
        "create_project",
        json!({"team": "acme", "key": "SHOP", "name": "Shop"}),
    )
    .await;
    tool(
        &app,
        &alice,
        "add_repo",
        json!({"project": "SHOP", "url": "https://github.com/acme/shop.git"}),
    )
    .await;
    let base = fake.lock().unwrap().base.clone();
    let conn: uuid::Uuid = sqlx::query_scalar(
        "insert into forge_connections (provider, name, api_url, web_url, app_id, slug, private_key, webhook_secret)
         values ('github','test',$1,'https://github.com',4242,'t',$2,'whsec') returning id",
    )
    .bind(&base)
    .bind(PEM)
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    for (id, account) in [(77_i64, "acme"), (88, "other")] {
        sqlx::query("insert into forge_installations (connection_id, installation_id, account) values ($1,$2,$3)")
            .bind(conn)
            .bind(id)
            .bind(account)
            .execute(&app.state.db)
            .await
            .unwrap();
    }
    World {
        app,
        fake,
        root,
        alice,
        bob,
        carol,
    }
}

fn names(v: &Value) -> Vec<String> {
    v["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["full_name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn lists_what_the_installations_see() {
    let w = world().await;
    let v = tool(
        &w.app,
        &w.alice,
        "list_forge_repos",
        json!({"project": "SHOP"}),
    )
    .await;
    assert_eq!(v["connected"], true);
    assert_eq!(v["project"], "SHOP");
    // newest push first, the archived one last, both installations, both pages of acme's
    assert_eq!(
        names(&v),
        [
            "acme/api",
            "other/api",
            "acme/web",
            "acme/shop",
            "other/old"
        ]
    );
    assert_eq!(v["total"], 5);
    let by = |n: &str| {
        v["repos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["full_name"] == n)
            .unwrap()
            .clone()
    };
    // the pot's own repository is marked, with its alias; the others are not
    assert_eq!(by("acme/shop")["in_pot"], true);
    assert_eq!(by("acme/shop")["pot_name"], "shop");
    assert_eq!(by("acme/api")["in_pot"], false);
    // what the picker shows
    let api = by("acme/api");
    assert_eq!(api["default_branch"], "develop");
    assert_eq!(api["private"], false);
    assert_eq!(api["account"], "acme");
    assert_eq!(api["installation"], 77);
    assert_eq!(api["description"], "the acme/api repo");
    assert_eq!(api["suggested_name"], "api");
    assert_eq!(by("acme/shop")["private"], true);
    assert_eq!(by("other/old")["archived"], true);
    // suggestions are made against the pot as it is now: no alias api yet
    assert_eq!(by("other/api")["suggested_name"], "api");

    // pagination: acme has three repositories and the fake serves two per page
    let f = w.fake.lock().unwrap();
    let pages_77: Vec<i64> = f
        .listings
        .iter()
        .filter(|(t, _)| t.starts_with("ghs_77"))
        .map(|(_, p)| *p)
        .collect();
    assert_eq!(pages_77, [1, 2]);
    assert!(f.listings.iter().all(|(t, _)| t.starts_with("ghs_")));
}

#[tokio::test]
async fn the_app_jwt_is_rs256_with_the_app_id() {
    let w = world().await;
    tool(
        &w.app,
        &w.alice,
        "list_forge_repos",
        json!({"project": "SHOP", "installation": 77}),
    )
    .await;
    let (inst, jwt) = w.fake.lock().unwrap().token_requests[0].clone();
    assert_eq!(inst, 77);
    let header = jsonwebtoken::decode_header(&jwt).unwrap();
    assert_eq!(header.alg, jsonwebtoken::Algorithm::RS256);
    let key = jsonwebtoken::DecodingKey::from_rsa_components(N.trim(), "AQAB").unwrap();
    let mut val = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    val.set_required_spec_claims(&["exp", "iss"]);
    val.set_issuer(&["4242"]);
    val.validate_aud = false;
    let claims = jsonwebtoken::decode::<Value>(&jwt, &key, &val)
        .unwrap()
        .claims;
    let (iat, exp) = (
        claims["iat"].as_i64().unwrap(),
        claims["exp"].as_i64().unwrap(),
    );
    let now = chrono::Utc::now().timestamp();
    assert!(
        (now - 60 - iat).abs() < 30,
        "iat is 60 seconds back: {iat} vs {now}"
    );
    assert_eq!(
        exp - iat,
        600,
        "valid for 9 minutes from now, 10 from iat: {exp} {iat}"
    );
    assert!(exp - now <= 600, "at most 10 minutes ahead");
}

#[tokio::test]
async fn the_installation_token_is_requested_once_and_reused_until_it_expires() {
    let w = world().await;
    for _ in 0..3 {
        tool(
            &w.app,
            &w.alice,
            "list_forge_repos",
            json!({"project": "SHOP"}),
        )
        .await;
    }
    let f = w.fake.lock().unwrap();
    let count = |id: i64| f.token_requests.iter().filter(|(i, _)| *i == id).count();
    // 77's token lives an hour: one request for three listings; 88's lives 30 seconds, under the
    // one minute margin: every listing asks again
    assert_eq!(count(77), 1);
    assert_eq!(count(88), 3);
    // the first listing of 77 used the token the fake gave it, the later ones the same token
    assert!(
        f.listings
            .iter()
            .filter(|(t, _)| t.starts_with("ghs_77"))
            .all(|(t, _)| t
                == &f
                    .listings
                    .iter()
                    .find(|(t, _)| t.starts_with("ghs_77"))
                    .unwrap()
                    .0)
    );
}

#[tokio::test]
async fn query_filters_by_full_name_and_installation() {
    let w = world().await;
    let v = tool(
        &w.app,
        &w.alice,
        "list_forge_repos",
        json!({"project": "SHOP", "query": "API"}),
    )
    .await;
    assert_eq!(names(&v), ["acme/api", "other/api"]);
    let v = tool(
        &w.app,
        &w.alice,
        "list_forge_repos",
        json!({"project": "SHOP", "query": "other/"}),
    )
    .await;
    assert_eq!(names(&v), ["other/api", "other/old"]);
    let v = tool(
        &w.app,
        &w.alice,
        "list_forge_repos",
        json!({"project": "SHOP", "installation": 88}),
    )
    .await;
    assert_eq!(names(&v), ["other/api", "other/old"]);
    // the REST form
    let (code, v) = w
        .app
        .get(&w.alice, "/forges/repos?project=SHOP&q=web")
        .await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(names(&v), ["acme/web"]);
}

#[tokio::test]
async fn only_those_who_may_add_a_repository_list() {
    let w = world().await;
    // bob is a member of the team, not an owner: add_repo refuses him, so does the list
    refused(
        &w.app,
        &w.bob,
        "list_forge_repos",
        json!({"project": "SHOP"}),
    )
    .await;
    refused(
        &w.app,
        &w.bob,
        "add_repos",
        json!({"project": "SHOP", "repos": "acme/api"}),
    )
    .await;
    // carol is in no team: she does not even see the pot
    refused(
        &w.app,
        &w.carol,
        "list_forge_repos",
        json!({"project": "SHOP"}),
    )
    .await;
    // without a pot only admins list
    refused(&w.app, &w.alice, "list_forge_repos", json!({})).await;
    let v = tool(&w.app, &w.root, "list_forge_repos", json!({})).await;
    assert_eq!(v["project"], Value::Null);
    assert_eq!(v["total"], 5);
    assert!(
        v["repos"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["in_pot"] == false)
    );
    // and an admin may name any pot
    let v = tool(
        &w.app,
        &w.root,
        "list_forge_repos",
        json!({"project": "SHOP", "query": "shop"}),
    )
    .await;
    assert_eq!(v["repos"][0]["in_pot"], true);
    // nothing was added by the refused calls
    let (_, pots) = w.app.get(&w.alice, "/projects").await;
    assert_eq!(pots[0]["repos"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn without_a_connected_app_the_list_says_so() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    tool(&app, &root, "create_team", json!({"name": "acme"})).await;
    tool(
        &app,
        &root,
        "create_project",
        json!({"team": "acme", "key": "SHOP", "name": "Shop"}),
    )
    .await;
    let v = tool(&app, &root, "list_forge_repos", json!({"project": "SHOP"})).await;
    assert_eq!(v["connected"], false);
    assert_eq!(v["total"], 0);
}

#[tokio::test]
async fn one_failing_installation_does_not_hide_the_others() {
    let w = world().await;
    // an installation the fake cannot serve a listing for: its token request works but the app key is bad
    sqlx::query("update forge_connections set private_key = 'not a key'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    let v = tool(
        &w.app,
        &w.alice,
        "list_forge_repos",
        json!({"project": "SHOP"}),
    )
    .await;
    assert_eq!(v["total"], 0);
    assert_eq!(v["errors"].as_array().unwrap().len(), 2);
    assert!(v["errors"][0].as_str().unwrap().contains("private key"));
}

#[tokio::test]
async fn bulk_add_names_dedupes_and_takes_the_default_branch() {
    let w = world().await;
    let v = tool(
        &w.app,
        &w.alice,
        "add_repos",
        json!({"project": "SHOP", "repos":
            "acme/api\nacme/web, https://github.com/other/api.git  acme/shop\nnope/missing\nhttps://git.example.com/x/lib.git"}),
    )
    .await;
    let added: Vec<(String, Option<String>)> = v["added"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap().to_string(),
                r["default_branch"].as_str().map(str::to_string),
            )
        })
        .collect();
    assert_eq!(
        added,
        [
            ("api".to_string(), Some("develop".to_string())),
            ("web".to_string(), Some("main".to_string())),
            // the name api is taken: owner-name, and its own default branch
            ("other-api".to_string(), Some("trunk".to_string())),
            // not seen by the app: added as a plain url, no branch
            ("lib".to_string(), None),
        ]
    );
    let skipped: Vec<(String, String)> = v["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["input"].as_str().unwrap().to_string(),
                r["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(skipped.len(), 2, "{skipped:?}");
    assert_eq!(
        skipped[0],
        ("acme/shop".to_string(), "already in the pot".to_string())
    );
    assert!(skipped[1].1.contains("not visible"), "{skipped:?}");
    assert_eq!(v["repos"].as_array().unwrap().len(), 5);
    // the stored url is the clone url, and the list now marks them
    let l = tool(
        &w.app,
        &w.alice,
        "list_forge_repos",
        json!({"project": "SHOP"}),
    )
    .await;
    let in_pot: Vec<&str> = l["repos"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["in_pot"] == true)
        .map(|r| r["full_name"].as_str().unwrap())
        .collect();
    assert_eq!(in_pot.len(), 4, "{in_pot:?}");
    assert!(!in_pot.contains(&"other/old"));
    let (_, pots) = w.app.get(&w.alice, "/projects").await;
    let urls: Vec<&str> = pots[0]["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["url"].as_str().unwrap())
        .collect();
    assert!(
        urls.contains(&"https://github.com/acme/api.git"),
        "{urls:?}"
    );
    // adding the same ones again adds nothing
    let again = tool(
        &w.app,
        &w.alice,
        "add_repos",
        json!({"project": "SHOP", "repos": "acme/api, acme/web"}),
    )
    .await;
    assert!(again["added"].as_array().unwrap().is_empty());
    assert_eq!(again["skipped"].as_array().unwrap().len(), 2);
    // an empty list is refused; the REST form works too
    refused(
        &w.app,
        &w.alice,
        "add_repos",
        json!({"project": "SHOP", "repos": " , "}),
    )
    .await;
    let (code, r) = w
        .app
        .post(
            &w.alice,
            "/projects/SHOP/repos/bulk",
            json!({"repos": "other/old"}),
        )
        .await;
    assert_eq!(code, 200, "{r}");
    assert_eq!(r["added"][0]["name"], "old");
}

// ---------- reading pull requests from GitHub ----------

async fn wait_for<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..100 {
        if f().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

fn pr_of(task: &Value, number: i64) -> &Value {
    task["prs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["number"] == number)
        .unwrap_or_else(|| panic!("no PR {number} in {task}"))
}

#[tokio::test]
async fn link_pr_reads_the_pull_request_from_github() {
    let w = world().await;
    tool(
        &w.app,
        &w.alice,
        "create_task",
        json!({"project": "SHOP", "title": "Refunds"}),
    )
    .await;
    let t = tool(
        &w.app,
        &w.alice,
        "link_pr",
        json!({"id": "SHOP-1", "url": "https://github.com/acme/shop/pull/5"}),
    )
    .await;
    assert_eq!(t["warning"], Value::Null);
    let pr = pr_of(&t, 5);
    // a PR that was merged long ago is shown as merged, with everything GitHub knows
    assert_eq!(pr["title"], "Refund screen");
    assert_eq!(pr["state"], "merged");
    assert_eq!(pr["head_branch"], "feat/refunds");
    assert_eq!(pr["author"], "ann");
    assert_eq!(pr["draft"], false);
    assert_eq!(pr["checks"], "failure");
    assert_eq!(pr["failing"], json!(["test"]));
    assert_eq!(
        pr["review"], "approved",
        "carol's approval took back her request, dave only commented"
    );
    assert_eq!(pr["explicit"], true);
    // the app is not installed on this repository: the row is made, empty, and the answer says why
    let t = tool(
        &w.app,
        &w.alice,
        "link_pr",
        json!({"id": "SHOP-1", "url": "https://github.com/nobody/thing/pull/1"}),
    )
    .await;
    let warning = t["warning"].as_str().unwrap();
    assert!(
        warning.contains("not installed on nobody/thing"),
        "{warning}"
    );
    let pr = pr_of(&t, 1);
    assert_eq!(pr["title"], "");
    assert_eq!(pr["state"], "open");
    // refresh_pr reads it again
    w.fake.lock().unwrap().title5 = "Refund screen v2".into();
    let t = tool(
        &w.app,
        &w.alice,
        "refresh_pr",
        json!({"id": "SHOP-1", "url": "https://github.com/acme/shop/pull/5"}),
    )
    .await;
    assert_eq!(pr_of(&t, 5)["title"], "Refund screen v2");
    // linking again does too
    w.fake.lock().unwrap().title5 = "Refund screen v3".into();
    let t = tool(
        &w.app,
        &w.alice,
        "link_pr",
        json!({"id": "SHOP-1", "url": "https://github.com/acme/shop/pull/5"}),
    )
    .await;
    assert_eq!(pr_of(&t, 5)["title"], "Refund screen v3");
    // refreshing everything of the task warns about the one it cannot read
    let t = tool(&w.app, &w.alice, "refresh_pr", json!({"id": "SHOP-1"})).await;
    assert!(t["warning"].as_str().unwrap().contains("nobody/thing"));
}

#[tokio::test]
async fn a_repository_joining_a_pot_links_its_open_prs_that_name_a_task() {
    let w = world().await;
    tool(
        &w.app,
        &w.alice,
        "create_task",
        json!({"project": "SHOP", "title": "Web part"}),
    )
    .await;
    let v = tool(
        &w.app,
        &w.alice,
        "add_repos",
        json!({"project": "SHOP", "repos": "acme/web"}),
    )
    .await;
    assert_eq!(v["added"].as_array().unwrap().len(), 1);
    // in the background: the PR whose branch names SHOP-1 is linked and filled, the other is left alone
    let db = w.app.state.db.clone();
    wait_for("the backfill event", || {
        let db = db.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "select exists (select 1 from events where kind = 'forge.backfill')",
            )
            .fetch_one(&db)
            .await
            .unwrap()
        }
    })
    .await;
    let (title, branch, checks, task): (String, String, String, Option<String>) = sqlx::query_as(
        "select p.title, p.head_branch, p.checks, (select t.number::text from tasks t where t.id = p.task_id)
           from forge_prs p where lower(p.repo) = 'acme/web' and p.number = 1",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        (title.as_str(), branch.as_str()),
        ("Web part", "shop-1-web-part")
    );
    assert_eq!(task.as_deref(), Some("1"));
    assert_eq!(checks, "pending", "its build is still running");
    let other: i64 = sqlx::query_scalar(
        "select count(*) from forge_prs where lower(repo) = 'acme/web' and number = 2",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(other, 0);
    let payload: Value =
        sqlx::query_scalar("select payload from events where kind = 'forge.backfill'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        (payload["listed"].as_i64(), payload["linked"].as_i64()),
        (Some(2), Some(1)),
        "{payload}"
    );
    // the single add_repo does it too, and a repository the app does not cover says so
    tool(
        &w.app,
        &w.alice,
        "add_repo",
        json!({"project": "SHOP", "url": "https://github.com/nobody/thing.git"}),
    )
    .await;
    wait_for("the second backfill event", || {
        let db = db.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "select count(*) from events where kind = 'forge.backfill'",
            )
            .fetch_one(&db)
            .await
            .unwrap()
                == 2
        }
    })
    .await;
    let note: String = sqlx::query_scalar("select payload->>'note' from events where kind = 'forge.backfill' and payload->>'repo' = 'nobody/thing'")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(note.contains("not installed"), "{note}");
}

#[tokio::test]
async fn an_ignored_delivery_is_remembered_until_the_repository_is_followed() {
    let w = world().await;
    let body = json!({"action": "opened", "number": 3, "installation": {"id": 77}, "sender": {"login": "ann"},
        "repository": {"full_name": "acme/api", "html_url": "https://github.com/acme/api", "clone_url": "https://github.com/acme/api.git"},
        "pull_request": {"number": 3, "html_url": "https://github.com/acme/api/pull/3", "title": "x", "state": "open",
            "head": {"ref": "x", "sha": "s"}, "base": {"ref": "main"}, "user": {"login": "ann"}}});
    let raw = serde_json::to_vec(&body).unwrap();
    for i in 0..2 {
        let r = w
            .app
            .http
            .post(format!("{}/webhooks/github", w.app.base))
            .header("x-github-event", "pull_request")
            .header("x-github-delivery", format!("d{i}"))
            .header(
                "x-hub-signature-256",
                corntchap::forge::github::signature("whsec", &raw),
            )
            .header("content-type", "application/json")
            .body(raw.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 202);
    }
    let (app, root) = (&w.app, w.root.clone());
    wait_for("the unfollowed list", || {
        let root = root.clone();
        async move {
            let (_, v) = app.get(&root, "/forges").await;
            v["unfollowed"][0]["events"] == 2
        }
    })
    .await;
    let (_, v) = w.app.get(&w.root, "/forges").await;
    assert_eq!(v["unfollowed"].as_array().unwrap().len(), 1);
    assert_eq!(v["unfollowed"][0]["repo"], "acme/api");
    assert_eq!(v["unfollowed"][0]["installation"], 77);
    // a plain member does not see the connections
    let (code, _) = w.app.get(&w.alice, "/forges").await;
    assert_eq!(code, 403);
    // the picker marks it
    let l = tool(
        &w.app,
        &w.alice,
        "list_forge_repos",
        json!({"project": "SHOP", "query": "acme/api"}),
    )
    .await;
    assert_eq!(l["repos"][0]["unfollowed_events"], 2);
    assert_eq!(
        tool(
            &w.app,
            &w.alice,
            "list_forge_repos",
            json!({"project": "SHOP", "query": "acme/web"})
        )
        .await["repos"][0]["unfollowed_events"],
        Value::Null
    );
    // only admins dismiss
    refused(
        &w.app,
        &w.alice,
        "dismiss_forge_repo",
        json!({"repo": "acme/api"}),
    )
    .await;
    // adding it to a pot takes it off the list
    tool(
        &w.app,
        &w.alice,
        "add_repos",
        json!({"project": "SHOP", "repos": "acme/api"}),
    )
    .await;
    let (_, v) = w.app.get(&w.root, "/forges").await;
    assert_eq!(v["unfollowed"], json!([]));
    // dismissing: an admin takes another one off
    let body2 = json!({"action": "opened", "number": 1, "installation": {"id": 88},
        "repository": {"full_name": "other/old", "html_url": "https://github.com/other/old", "clone_url": "https://github.com/other/old.git"},
        "pull_request": {"number": 1, "html_url": "https://github.com/other/old/pull/1", "title": "x", "state": "open",
            "head": {"ref": "x", "sha": "s"}, "base": {"ref": "main"}, "user": {"login": "ann"}}});
    let raw2 = serde_json::to_vec(&body2).unwrap();
    w.app
        .http
        .post(format!("{}/webhooks/github", w.app.base))
        .header("x-github-event", "pull_request")
        .header("x-github-delivery", "d9")
        .header(
            "x-hub-signature-256",
            corntchap::forge::github::signature("whsec", &raw2),
        )
        .body(raw2)
        .send()
        .await
        .unwrap();
    wait_for("the second unfollowed repo", || {
        let root = root.clone();
        async move {
            let (_, v) = app.get(&root, "/forges").await;
            v["unfollowed"].as_array().unwrap().len() == 1
        }
    })
    .await;
    tool(
        &w.app,
        &w.root,
        "dismiss_forge_repo",
        json!({"repo": "other/old"}),
    )
    .await;
    let (_, v) = w.app.get(&w.root, "/forges").await;
    assert_eq!(v["unfollowed"], json!([]));
}
