mod common;
use corntchap::{admin, service, state::Principal};
use std::collections::HashMap;

/// Build 10 agents in one project with 50 tasks; each agent loops claim → complete.
/// Every task must end up done exactly once.
#[tokio::test]
async fn ten_agents_race_over_fifty_tasks() {
    let (pool, _) = common::fresh_db().await;
    let (_, _) = admin::init_admin(&pool, "octo").await.unwrap();
    let admin_p: Principal =
        sqlx::query_as::<_, (uuid::Uuid,)>("select id from principals where name='octo'")
            .fetch_one(&pool)
            .await
            .map(|(id,)| Principal {
                id,
                kind: "user".into(),
                name: "octo".into(),
                owner_id: None,
                is_admin: true,
                max_leases: 1,
                runtime: None,
                agent_kind: None,
            })
            .unwrap();
    admin::create_team(&pool, &admin_p, "acme").await.unwrap();
    admin::create_project(&pool, &admin_p, "acme", "PAY", "Payments", None, None)
        .await
        .unwrap();

    let mut agents = vec![];
    for i in 0..10 {
        let name = format!("agent-{i}");
        admin::create_principal(
            &pool,
            &admin_p,
            "agent",
            &name,
            Some("other"),
            Some("local"),
            None,
            None,
        )
        .await
        .unwrap();
        admin::add_member(&pool, &admin_p, "acme", &name, "member")
            .await
            .unwrap();
        let id: uuid::Uuid = sqlx::query_scalar("select id from principals where name=$1")
            .bind(&name)
            .fetch_one(&pool)
            .await
            .unwrap();
        agents.push(Principal {
            id,
            kind: "agent".into(),
            name,
            owner_id: Some(admin_p.id),
            is_admin: false,
            max_leases: 1,
            runtime: Some("local".into()),
            agent_kind: Some("other".into()),
        });
    }
    for n in 0..50 {
        service::create_task(
            &pool,
            &admin_p,
            service::NewTask {
                project: "PAY".into(),
                title: format!("task {n}"),
                body: String::new(),
                items: vec!["a".into()],
                parent: None,
                depends_on: vec![],
                labels: vec![],
                priority: n % 3,
            },
        )
        .await
        .unwrap();
    }

    let mut handles = vec![];
    for a in agents {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            let mut done = vec![];
            loop {
                let r = service::claim_next(&pool, &a, "PAY", &[]).await.unwrap();
                if r["none"] == true {
                    break;
                }
                let id = r["task"]["id"].as_str().unwrap().to_string();
                let epoch = r["lease"]["epoch"].as_i64().unwrap();
                service::check_item(&pool, &a, &id, Some(epoch), 1, true)
                    .await
                    .unwrap();
                service::complete(&pool, &a, &id, epoch, "ok")
                    .await
                    .unwrap();
                done.push(id);
            }
            done
        }));
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for h in handles {
        for id in h.await.unwrap() {
            *seen.entry(id).or_default() += 1;
        }
    }
    assert_eq!(seen.len(), 50, "every task was claimed");
    assert!(seen.values().all(|n| *n == 1), "no task completed twice");
    let done: i64 = sqlx::query_scalar("select count(*) from tasks where status = 'done'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(done, 50);
}

/// A stale epoch must not overwrite the task's new holder (fencing).
#[tokio::test]
async fn expired_lease_is_fenced() {
    let (pool, _) = common::fresh_db().await;
    admin::init_admin(&pool, "octo").await.unwrap();
    let id: uuid::Uuid = sqlx::query_scalar("select id from principals where name='octo'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let admin_p = Principal {
        id,
        kind: "user".into(),
        name: "octo".into(),
        owner_id: None,
        is_admin: true,
        max_leases: 1,
        runtime: None,
        agent_kind: None,
    };
    admin::create_team(&pool, &admin_p, "acme").await.unwrap();
    admin::create_project(&pool, &admin_p, "acme", "PAY", "Payments", None, None)
        .await
        .unwrap();
    let mut ag = vec![];
    for n in ["a1", "a2"] {
        admin::create_principal(&pool, &admin_p, "agent", n, None, None, None, None)
            .await
            .unwrap();
        admin::add_member(&pool, &admin_p, "acme", n, "member")
            .await
            .unwrap();
        let id: uuid::Uuid = sqlx::query_scalar("select id from principals where name=$1")
            .bind(n)
            .fetch_one(&pool)
            .await
            .unwrap();
        ag.push(Principal {
            id,
            kind: "agent".into(),
            name: n.into(),
            owner_id: Some(admin_p.id),
            is_admin: false,
            max_leases: 1,
            runtime: None,
            agent_kind: None,
        });
    }
    service::create_task(
        &pool,
        &admin_p,
        service::NewTask {
            project: "PAY".into(),
            title: "t".into(),
            body: String::new(),
            items: vec![],
            parent: None,
            depends_on: vec![],
            labels: vec![],
            priority: 0,
        },
    )
    .await
    .unwrap();
    let first = service::claim_next(&pool, &ag[0], "PAY", &[])
        .await
        .unwrap();
    let e1 = first["lease"]["epoch"].as_i64().unwrap();
    // lease runs out; the second agent takes over
    sqlx::query("update tasks set lease_expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    let second = service::claim_next(&pool, &ag[1], "PAY", &[])
        .await
        .unwrap();
    let e2 = second["lease"]["epoch"].as_i64().unwrap();
    assert!(e2 > e1);
    let err = service::complete(&pool, &ag[0], "PAY-1", e1, "late")
        .await
        .unwrap_err();
    assert_eq!(err.code(), "lease_lost");
    let err = service::check_item(&pool, &ag[0], "PAY-1", Some(e1), 1, true)
        .await
        .unwrap_err();
    assert!(matches!(err.code(), "lease_lost" | "not_found"));
    service::complete(&pool, &ag[1], "PAY-1", e2, "ok")
        .await
        .unwrap();
}
