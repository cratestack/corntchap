//! Migration 0040: the restaurant becomes a principal and takes over what the server did on its own.
//! Run on data shaped like the old schema's.
mod common;

const RESTAURANT: &str = "00000000-0000-4000-8000-524553545552";

async fn old_database() -> sqlx::PgPool {
    let admin = sqlx::PgPool::connect(&common::admin_url()).await.unwrap();
    let name = format!("ct_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("create database {name}")))
        .execute(&admin)
        .await
        .unwrap();
    let url = common::admin_url().rsplit_once('/').unwrap().0.to_string() + "/" + &name;
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let mut m = sqlx::migrate!("./migrations");
    m.migrations = std::borrow::Cow::Owned(
        m.migrations
            .iter()
            .filter(|x| x.version < 40)
            .cloned()
            .collect(),
    );
    m.run(&pool).await.unwrap();
    pool
}

async fn exec(pool: &sqlx::PgPool, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_server_becomes_an_actor_and_its_history_follows() {
    let pool = old_database().await;
    // a person already named restaurant, with a handle claim
    exec(
        &pool,
        "insert into principals (kind, name) values ('user', 'restaurant'), ('user', 'octo')",
    )
    .await;
    exec(
        &pool,
        "insert into handle_claims (name, principal_id) select name, id from principals where name = 'restaurant' on conflict do nothing",
    )
    .await;
    exec(&pool, "insert into teams (name) values ('acme')").await;
    exec(
        &pool,
        "insert into projects (team_id, key, name) select id, 'PAY', 'Pay' from teams",
    )
    .await;
    exec(
        &pool,
        "insert into tasks (project_id, number, title, status) select id, 1, 'one', 'open' from projects",
    )
    .await;
    // what the server did on its own: no actor
    for kind in [
        "task.lease_expired",
        "task.blocked",
        "task.completed",
        "wake",
        "wake.failed",
        "forge.linked",
        "forge.backfill",
        "forge.pr",
        "forge.checks",
    ] {
        exec(
            &pool,
            &format!("insert into events (project_id, kind, payload) select id, '{kind}', '{{}}' from projects"),
        )
        .await;
    }
    // and a person's event, which keeps its actor
    exec(
        &pool,
        "insert into events (kind, actor_id, payload) select 'task.updated', id, '{}' from principals where name = 'octo'",
    )
    .await;
    for (body, author) in [
        ("[released by the server] no progress for 2h", false),
        ("[blocked by the server] bounced 5x", false),
        ("Upstream change passed on (shop: told c1).", false),
        ("a note from a deleted cook", false),
        ("a persons note", true),
    ] {
        exec(
            &pool,
            &format!(
                "insert into comments (task_id, author_id, body) select t.id, {}, '{body}' from tasks t",
                if author {
                    "(select id from principals where name = 'octo')"
                } else {
                    "null"
                }
            ),
        )
        .await;
    }

    sqlx::raw_sql(include_str!("../migrations/0040_restaurant.sql"))
        .execute(&pool)
        .await
        .unwrap();

    // the restaurant: one row, disabled, a member, with the fixed id
    let (id, kind, display, role, disabled): (String, String, String, String, bool) = sqlx::query_as(
        "select id::text, kind, display_name, role, disabled_at is not null from principals where name = 'restaurant'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (
            id.as_str(),
            kind.as_str(),
            display.as_str(),
            role.as_str(),
            disabled
        ),
        (RESTAURANT, "restaurant", "restaurant", "member", true)
    );
    assert_eq!(id, corntchap::state::RESTAURANT.to_string());

    // the person called restaurant was renamed first, and said so
    let renamed: String = sqlx::query_scalar(
        "select name from principals where kind = 'user' and name like 'restaurant-%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        renamed.len() == "restaurant-".len() + 4
            && renamed["restaurant-".len()..]
                .chars()
                .all(|c| c.is_ascii_hexdigit()),
        "{renamed}"
    );
    let said: i64 = sqlx::query_scalar(
        "select count(*) from events where kind = 'principal.renamed' and payload->>'from' = 'restaurant' and payload->>'to' = $1",
    )
    .bind(&renamed)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(said, 1);
    // the old handle is still claimed by the person it belonged to
    let claimed: i64 = sqlx::query_scalar(
        "select count(*) from handle_claims h join principals p on p.id = h.principal_id where h.name = 'restaurant' and p.kind = 'user'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(claimed, 1);

    // backfill: the server's events name the restaurant, forge events stay GitHub's
    let rows: Vec<(String, bool)> = sqlx::query_as(
        "select kind, coalesce(actor_id = $1::uuid, false) from events where actor_id is not null or kind like 'forge.%' order by kind, seq",
    )
    .bind(RESTAURANT)
    .fetch_all(&pool)
    .await
    .unwrap();
    let by: std::collections::BTreeMap<_, _> = rows.into_iter().collect();
    for kind in [
        "task.lease_expired",
        "task.blocked",
        "task.completed",
        "wake",
        "wake.failed",
        "forge.linked",
        "forge.backfill",
    ] {
        assert!(by[kind], "{kind} is the restaurant's");
    }
    let null_forge: i64 = sqlx::query_scalar(
        "select count(*) from events where kind in ('forge.pr', 'forge.checks') and actor_id is null",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(null_forge, 2, "forge.pr and forge.checks keep no actor");
    let person: bool =
        sqlx::query_scalar("select actor_id <> $1::uuid from events where kind = 'task.updated'")
            .bind(RESTAURANT)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(person);

    // the notes the server wrote are the restaurant's (text kept); other orphans stay orphans
    let notes: Vec<(String, Option<String>)> =
        sqlx::query_as("select body, author_id::text from comments order by body")
            .fetch_all(&pool)
            .await
            .unwrap();
    for (body, author) in &notes {
        match body.as_str() {
            b if b.starts_with("[released by the server]")
                || b.starts_with("[blocked by the server]")
                || b.starts_with("Upstream change passed on (") =>
            {
                assert_eq!(author.as_deref(), Some(RESTAURANT), "{body}")
            }
            "a note from a deleted cook" => assert_eq!(*author, None),
            _ => assert!(author.is_some() && author.as_deref() != Some(RESTAURANT)),
        }
    }

    // never a second one, never a person in disguise, never a credential
    for sql in [
        "insert into principals (kind, name) values ('restaurant', 'another')",
        "insert into tokens (principal_id, token_hash) select id, '\\x01' from principals where kind = 'restaurant'",
        "insert into team_members (team_id, principal_id, role) select t.id, p.id, 'member' from teams t, principals p where p.kind = 'restaurant'",
    ] {
        assert!(
            sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
                .execute(&pool)
                .await
                .is_err(),
            "{sql}"
        );
    }
    // the partial index for a cook's log is there
    let idx: i64 = sqlx::query_scalar(
        "select count(*) from pg_indexes where indexname = 'events_restaurant_agent'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(idx, 1);
}

#[tokio::test]
async fn a_database_without_a_restaurant_name_clash_just_gains_the_row() {
    let pool = old_database().await;
    exec(
        &pool,
        "insert into principals (kind, name) values ('user', 'octo')",
    )
    .await;
    sqlx::raw_sql(include_str!("../migrations/0040_restaurant.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let names: Vec<String> = sqlx::query_scalar("select name from principals order by name")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(names, ["octo", "restaurant"]);
}
