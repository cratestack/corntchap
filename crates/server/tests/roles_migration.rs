//! Migration 0037: the admin flag becomes a role. Run on data shaped like the old schema's.
mod common;

async fn old_database() -> sqlx::PgPool {
    let admin = sqlx::PgPool::connect(&common::admin_url()).await.unwrap();
    let name = format!("ct_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("create database {name}")))
        .execute(&admin)
        .await
        .unwrap();
    let url = common::admin_url().rsplit_once('/').unwrap().0.to_string() + "/" + &name;
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    // everything before this migration
    let mut m = sqlx::migrate!("./migrations");
    m.migrations = std::borrow::Cow::Owned(
        m.migrations
            .iter()
            .filter(|x| x.version < 37)
            .cloned()
            .collect(),
    );
    m.run(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn admins_become_super_admins_and_team_owners_orchestrators() {
    let pool = old_database().await;
    for (name, admin, source) in [
        ("boss", true, "oidc"),
        ("booted", true, "init"),
        ("owner1", false, "oidc"),
        ("both", true, "manual"),
        ("chef", false, "oidc"),
        ("loner", false, "oidc"),
    ] {
        sqlx::query("insert into principals (kind, name, is_admin, admin_source) values ('user', $1, $2, $3)")
            .bind(name)
            .bind(admin)
            .bind(source)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("insert into principals (kind, name, owner_id) select 'agent', 'cook1', id from principals where name = 'owner1'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("insert into teams (name) values ('acme')")
        .execute(&pool)
        .await
        .unwrap();
    for (who, role) in [("owner1", "owner"), ("both", "owner"), ("chef", "member")] {
        sqlx::query("insert into team_members (team_id, principal_id, role) select t.id, p.id, $2 from teams t, principals p where t.name = 'acme' and p.name = $1")
            .bind(who)
            .bind(role)
            .execute(&pool)
            .await
            .unwrap();
    }

    sqlx::raw_sql(include_str!("../migrations/0037_roles.sql"))
        .execute(&pool)
        .await
        .unwrap();

    let rows: Vec<(String, String, String)> =
        sqlx::query_as("select name, role, role_source from principals order by name")
            .fetch_all(&pool)
            .await
            .unwrap();
    let got: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("booted", "super_admin", "init"),
            ("boss", "super_admin", "oidc"),
            ("both", "super_admin", "manual"),
            ("chef", "member", "oidc"),
            ("cook1", "member", "manual"),
            ("loner", "member", "oidc"),
            // a non-admin team owner becomes an orchestrator the IdP sync leaves alone
            ("owner1", "orchestrator", "manual"),
        ]
    );
    // the owner rows are kept as they were
    let owners: i64 = sqlx::query_scalar("select count(*) from team_members where role = 'owner'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owners, 2);
    // the old columns are gone; only people have a role
    for col in ["is_admin", "admin_source"] {
        let n: i64 = sqlx::query_scalar(
            "select count(*) from information_schema.columns where table_name = 'principals' and column_name = $1",
        )
        .bind(col)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 0, "{col} is dropped");
    }
    assert!(
        sqlx::query("update principals set role = 'orchestrator' where name = 'cook1'")
            .execute(&pool)
            .await
            .is_err(),
        "a cook is always a member"
    );
    assert!(
        sqlx::query("update principals set role = 'root' where name = 'loner'")
            .execute(&pool)
            .await
            .is_err()
    );
}
