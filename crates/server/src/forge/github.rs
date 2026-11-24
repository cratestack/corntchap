//! GitHub: webhook signatures and payloads of a GitHub App.
//!
//! Headers (docs.github.com, "Webhook events and payloads" and "Validating webhook deliveries"):
//! `X-GitHub-Event` (the event name), `X-GitHub-Delivery` (a GUID, the same on a redelivery) and
//! `X-Hub-Signature-256` (`sha256=` and the hex HMAC-SHA256 of the raw body, keyed by the app's
//! webhook secret).

use http::HeaderMap;
use serde_json::Value;
use subtle::ConstantTimeEq;

use super::{
    CheckSource, Conclusion, Forge, ForgeEvent, ForgeKind, PrRef, PrState, RepoRef, ReviewState,
    excerpt,
};
use crate::repos::normalize_git_url;

pub struct GitHub;

fn header(h: &HeaderMap, name: &str) -> Option<String> {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `sha256=<hex>` of the body under `secret`.
pub fn signature(secret: &str, body: &[u8]) -> String {
    format!(
        "sha256={}",
        hex::encode(crate::cloud::hmac_sha256(secret.as_bytes(), body))
    )
}

impl Forge for GitHub {
    fn provider(&self) -> super::Provider {
        "github"
    }

    fn verify(&self, secret: &str, headers: &HeaderMap, body: &[u8]) -> bool {
        let Some(given) = header(headers, "x-hub-signature-256") else {
            return false;
        };
        let want = signature(secret, body);
        given.as_bytes().ct_eq(want.as_bytes()).into()
    }

    fn delivery_id(&self, headers: &HeaderMap) -> Option<String> {
        header(headers, "x-github-delivery")
    }

    fn event_name(&self, headers: &HeaderMap) -> Option<String> {
        header(headers, "x-github-event")
    }

    fn parse(&self, event: &str, body: &[u8]) -> Result<Vec<ForgeEvent>, String> {
        let v: Value = serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
        Ok(parse_event(event, &v))
    }
}

fn s(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn repo_of(v: &Value) -> Option<RepoRef> {
    let r = &v["repository"];
    let full_name = s(&r["full_name"])?;
    let html_url = s(&r["html_url"]).unwrap_or_default();
    let clone_url = s(&r["clone_url"]).unwrap_or_else(|| html_url.clone());
    let norm_url = normalize_git_url(if html_url.is_empty() {
        &clone_url
    } else {
        &html_url
    });
    Some(RepoRef {
        full_name,
        html_url,
        clone_url,
        norm_url,
    })
}

fn pr_state(p: &Value) -> Option<PrState> {
    if p["merged"].as_bool() == Some(true) || !p["merged_at"].is_null() {
        return Some(PrState::Merged);
    }
    match p["state"].as_str() {
        Some("open") => Some(PrState::Open),
        Some("closed") => Some(PrState::Closed),
        _ => None,
    }
}

fn pr_ref(p: &Value) -> PrRef {
    PrRef {
        number: p["number"].as_i64().unwrap_or(0),
        url: s(&p["html_url"]).unwrap_or_default(),
        title: s(&p["title"]),
        body: s(&p["body"]),
        head_branch: s(&p["head"]["ref"]),
        head_sha: s(&p["head"]["sha"]),
        base_branch: s(&p["base"]["ref"]),
        author: s(&p["user"]["login"]),
        draft: p["draft"].as_bool(),
        state: pr_state(p),
        mergeable: match (p["mergeable"].as_bool(), p["mergeable_state"].as_str()) {
            (Some(false), _) | (_, Some("dirty")) => Some(false),
            (Some(true), _) => Some(true),
            _ => None,
        },
    }
}

fn conclusion(c: &Value) -> Option<Conclusion> {
    Some(match c.as_str()? {
        "success" | "neutral" | "skipped" => Conclusion::Success,
        "failure" | "startup_failure" | "action_required" => Conclusion::Failure,
        "timed_out" => Conclusion::TimedOut,
        "cancelled" | "stale" => Conclusion::Cancelled,
        _ => return None,
    })
}

pub fn parse_event(event: &str, v: &Value) -> Vec<ForgeEvent> {
    let action = v["action"].as_str().unwrap_or("");
    let repo = repo_of(v);
    let installation_id = v["installation"]["id"].as_i64();
    let sender = s(&v["sender"]["login"]);
    let mk = |pr: Option<PrRef>, kind: ForgeKind| ForgeEvent {
        provider: "github",
        repo: repo.clone(),
        pr,
        kind,
        installation_id,
        sender: sender.clone(),
    };
    let mut out = vec![];
    match event {
        "pull_request" => {
            let p = &v["pull_request"];
            let mut pr = pr_ref(p);
            if pr.number == 0 {
                pr.number = v["number"].as_i64().unwrap_or(0);
            }
            let kind = match action {
                "opened" => ForgeKind::PrOpened,
                "reopened" => ForgeKind::PrReopened,
                "closed" if pr.state == Some(PrState::Merged) => ForgeKind::PrMerged,
                "closed" => ForgeKind::PrClosed,
                "synchronize" => ForgeKind::PrUpdated { head_changed: true },
                "edited" | "ready_for_review" | "converted_to_draft" => ForgeKind::PrUpdated {
                    head_changed: false,
                },
                _ => return out,
            };
            let conflict = pr.state == Some(PrState::Open)
                && (p["mergeable"].as_bool() == Some(false)
                    || p["mergeable_state"].as_str() == Some("dirty"));
            out.push(mk(Some(pr.clone()), kind));
            if conflict {
                out.push(mk(Some(pr), ForgeKind::Conflict));
            }
        }
        "pull_request_review" => {
            let r = &v["review"];
            let state = match (
                action,
                r["state"].as_str().map(str::to_lowercase).as_deref(),
            ) {
                ("dismissed", _) => ReviewState::Dismissed,
                ("submitted", Some("approved")) => ReviewState::Approved,
                ("submitted", Some("changes_requested")) => ReviewState::ChangesRequested,
                ("submitted", Some("commented")) => ReviewState::Commented,
                _ => return out,
            };
            out.push(mk(
                Some(pr_ref(&v["pull_request"])),
                ForgeKind::Review {
                    state,
                    author: s(&r["user"]["login"]).unwrap_or_default(),
                    excerpt: excerpt(r["body"].as_str().unwrap_or(""), 300),
                    url: s(&r["html_url"]).unwrap_or_default(),
                },
            ));
        }
        "pull_request_review_comment" if action == "created" => {
            let c = &v["comment"];
            out.push(mk(
                Some(pr_ref(&v["pull_request"])),
                ForgeKind::Comment {
                    inline: true,
                    author: s(&c["user"]["login"]).unwrap_or_default(),
                    excerpt: excerpt(c["body"].as_str().unwrap_or(""), 300),
                    path: s(&c["path"]),
                    line: c["line"].as_i64().or(c["original_line"].as_i64()),
                    url: s(&c["html_url"]).unwrap_or_default(),
                },
            ));
        }
        "issue_comment" if action == "created" && !v["issue"]["pull_request"].is_null() => {
            let i = &v["issue"];
            let c = &v["comment"];
            let pr = PrRef {
                number: i["number"].as_i64().unwrap_or(0),
                url: s(&i["pull_request"]["html_url"])
                    .or_else(|| s(&i["html_url"]))
                    .unwrap_or_default(),
                title: s(&i["title"]),
                body: s(&i["body"]),
                author: s(&i["user"]["login"]),
                ..PrRef::default()
            };
            out.push(mk(
                Some(pr),
                ForgeKind::Comment {
                    inline: false,
                    author: s(&c["user"]["login"]).unwrap_or_default(),
                    excerpt: excerpt(c["body"].as_str().unwrap_or(""), 300),
                    path: None,
                    line: None,
                    url: s(&c["html_url"]).unwrap_or_default(),
                },
            ));
        }
        "check_suite" | "check_run" | "workflow_run" if action == "completed" => {
            let (o, source) = match event {
                "check_suite" => (&v["check_suite"], CheckSource::Suite),
                "check_run" => (&v["check_run"], CheckSource::Run),
                _ => (&v["workflow_run"], CheckSource::Workflow),
            };
            let Some(conclusion) = conclusion(&o["conclusion"]) else {
                return out;
            };
            let name = match source {
                CheckSource::Suite => s(&o["app"]["name"]).unwrap_or_else(|| "checks".into()),
                _ => s(&o["name"]).unwrap_or_else(|| "checks".into()),
            };
            let sha = s(&o["head_sha"]);
            let branch = s(&o["head_branch"]).or_else(|| s(&o["check_suite"]["head_branch"]));
            let summary = match source {
                CheckSource::Run => {
                    let t = o["output"]["title"].as_str().unwrap_or("");
                    let b = o["output"]["summary"].as_str().unwrap_or("");
                    excerpt(if t.is_empty() { b } else { t }, 300)
                }
                _ => String::new(),
            };
            let prs: Vec<i64> = o["pull_requests"]
                .as_array()
                .map(|a| a.iter().filter_map(|p| p["number"].as_i64()).collect())
                .unwrap_or_default();
            // a fork's PR is not listed: number 0 means "the PR with this head sha"
            let prs = if prs.is_empty() { vec![0] } else { prs };
            let html = repo
                .as_ref()
                .map(|r| r.html_url.clone())
                .unwrap_or_default();
            for n in prs {
                let url = if n > 0 {
                    format!("{html}/pull/{n}")
                } else {
                    String::new()
                };
                let details = s(&o["html_url"])
                    .filter(|_| source != CheckSource::Suite)
                    .or_else(|| (n > 0).then(|| format!("{url}/checks")));
                out.push(mk(
                    Some(PrRef {
                        number: n,
                        url,
                        head_branch: branch.clone(),
                        head_sha: sha.clone(),
                        ..PrRef::default()
                    }),
                    ForgeKind::ChecksCompleted {
                        source,
                        name: name.clone(),
                        conclusion,
                        summary: summary.clone(),
                        details_url: details,
                    },
                ));
            }
        }
        "push" => {
            let Some(branch) = v["ref"]
                .as_str()
                .and_then(|r| r.strip_prefix("refs/heads/"))
            else {
                return out;
            };
            out.push(mk(
                None,
                ForgeKind::Push {
                    branch: branch.to_string(),
                    sha: s(&v["after"]).unwrap_or_default(),
                    deleted: v["deleted"].as_bool() == Some(true),
                },
            ));
        }
        "installation" => {
            let i = &v["installation"];
            let kind = match action {
                "created" => ForgeKind::InstallationAdded {
                    account: s(&i["account"]["login"]),
                    account_type: s(&i["account"]["type"]),
                    repo_selection: s(&i["repository_selection"]),
                },
                "deleted" => ForgeKind::InstallationRemoved,
                "suspend" => ForgeKind::InstallationSuspended(true),
                "unsuspend" => ForgeKind::InstallationSuspended(false),
                _ => return out,
            };
            out.push(mk(None, kind));
        }
        _ => {}
    }
    out
}

/// The app manifest the connect flow sends to GitHub: read-only permissions and the events the
/// forge module understands. `hook_url` is `PUBLIC_URL/webhooks/github`.
pub fn manifest(name: &str, public_url: &str) -> Value {
    serde_json::json!({
        "name": name,
        "url": public_url,
        "hook_attributes": {"url": format!("{public_url}/webhooks/github"), "active": true},
        "redirect_url": format!("{public_url}/forges/github/callback"),
        "setup_url": format!("{public_url}/admin/forges"),
        "description": "Tells corntchap about pull requests, checks and reviews of the repositories of your pots. Read only.",
        "public": false,
        "default_permissions": {
            "pull_requests": "read", "checks": "read", "statuses": "read", "contents": "read",
            "metadata": "read", "issues": "read",
            // workflow_run events need it
            "actions": "read"
        },
        "default_events": ["pull_request", "pull_request_review", "pull_request_review_comment",
            "issue_comment", "check_suite", "check_run", "workflow_run", "push"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn repo() -> Value {
        json!({"full_name": "Acme/Shop", "html_url": "https://github.com/Acme/Shop",
               "clone_url": "https://github.com/Acme/Shop.git"})
    }

    fn pr(extra: Value) -> Value {
        let mut p = json!({"number": 7, "html_url": "https://github.com/Acme/Shop/pull/7",
            "title": "SHOP-12 fix", "body": "b", "state": "open", "draft": false,
            "head": {"ref": "shop-12-fix", "sha": "abc"}, "base": {"ref": "main"},
            "user": {"login": "ann"}});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    }

    #[test]
    fn signature_is_checked_in_constant_time_form() {
        let g = GitHub;
        let body = br#"{"zen":"x"}"#;
        let mut h = HeaderMap::new();
        h.insert(
            "x-hub-signature-256",
            signature("s3cret", body).parse().unwrap(),
        );
        assert!(g.verify("s3cret", &h, body));
        assert!(!g.verify("other", &h, body));
        assert!(!g.verify("s3cret", &h, b"changed"));
        assert!(!g.verify("s3cret", &HeaderMap::new(), body));
        // GitHub's documented example
        assert_eq!(
            signature("It's a Secret to Everybody", b"Hello, World!"),
            "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17"
        );
    }

    #[test]
    fn pull_request_actions() {
        let body = |a: &str, p: Value| {
            json!({"action": a, "number": 7, "pull_request": p, "repository": repo(),
            "installation": {"id": 99}, "sender": {"login": "ann"}})
        };
        let ev = parse_event("pull_request", &body("opened", pr(json!({}))));
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, ForgeKind::PrOpened);
        let r = ev[0].repo.as_ref().unwrap();
        assert_eq!(r.full_name, "Acme/Shop");
        assert_eq!(r.norm_url, "github.com/acme/shop");
        assert_eq!(ev[0].installation_id, Some(99));
        let p = ev[0].pr.as_ref().unwrap();
        assert_eq!(
            (p.number, p.head_branch.as_deref(), p.head_sha.as_deref()),
            (7, Some("shop-12-fix"), Some("abc"))
        );

        let ev = parse_event("pull_request", &body("synchronize", pr(json!({}))));
        assert_eq!(ev[0].kind, ForgeKind::PrUpdated { head_changed: true });
        let ev = parse_event(
            "pull_request",
            &body("closed", pr(json!({"state": "closed", "merged": true}))),
        );
        assert_eq!(ev[0].kind, ForgeKind::PrMerged);
        assert_eq!(ev[0].pr.as_ref().unwrap().state, Some(PrState::Merged));
        let ev = parse_event(
            "pull_request",
            &body("closed", pr(json!({"state": "closed", "merged": false}))),
        );
        assert_eq!(ev[0].kind, ForgeKind::PrClosed);
        let ev = parse_event("pull_request", &body("reopened", pr(json!({}))));
        assert_eq!(ev[0].kind, ForgeKind::PrReopened);
        assert!(parse_event("pull_request", &body("labeled", pr(json!({})))).is_empty());
    }

    #[test]
    fn conflict_is_its_own_event() {
        let v = json!({"action": "synchronize", "pull_request": pr(json!({"mergeable": false, "mergeable_state": "dirty"})), "repository": repo()});
        let ev = parse_event("pull_request", &v);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[1].kind, ForgeKind::Conflict);
        let v = json!({"action": "synchronize", "pull_request": pr(json!({"mergeable": null})), "repository": repo()});
        assert_eq!(parse_event("pull_request", &v).len(), 1);
    }

    #[test]
    fn checks_one_event_per_pr() {
        let v = json!({"action": "completed", "repository": repo(),
            "check_suite": {"conclusion": "failure", "head_sha": "abc", "head_branch": "shop-12-fix",
                            "app": {"name": "GitHub Actions"}, "pull_requests": [{"number": 7}, {"number": 8}]}});
        let ev = parse_event("check_suite", &v);
        assert_eq!(ev.len(), 2);
        match &ev[0].kind {
            ForgeKind::ChecksCompleted {
                source,
                name,
                conclusion,
                details_url,
                ..
            } => {
                assert_eq!(*source, CheckSource::Suite);
                assert_eq!(name, "GitHub Actions");
                assert_eq!(*conclusion, Conclusion::Failure);
                assert_eq!(
                    details_url.as_deref(),
                    Some("https://github.com/Acme/Shop/pull/7/checks")
                );
            }
            k => panic!("{k:?}"),
        }
        // a fork's PR is not listed: number 0, found by sha
        let v = json!({"action": "completed", "repository": repo(),
            "workflow_run": {"name": "ci", "conclusion": "timed_out", "head_sha": "abc", "html_url": "https://github.com/Acme/Shop/actions/runs/1", "pull_requests": []}});
        let ev = parse_event("workflow_run", &v);
        assert_eq!(ev[0].pr.as_ref().unwrap().number, 0);
        assert!(
            matches!(&ev[0].kind, ForgeKind::ChecksCompleted { conclusion: Conclusion::TimedOut, details_url: Some(u), .. } if u.ends_with("/runs/1"))
        );
        // not completed, or no conclusion yet
        let v = json!({"action": "requested", "repository": repo(), "check_suite": {"conclusion": null}});
        assert!(parse_event("check_suite", &v).is_empty());
    }

    #[test]
    fn reviews_and_comments() {
        let v = json!({"action": "submitted", "repository": repo(), "pull_request": pr(json!({})),
            "review": {"state": "CHANGES_REQUESTED", "body": "please rename", "user": {"login": "bob"}, "html_url": "https://github.com/x#r1"}});
        let ev = parse_event("pull_request_review", &v);
        assert!(
            matches!(&ev[0].kind, ForgeKind::Review { state: ReviewState::ChangesRequested, author, excerpt, .. } if author == "bob" && excerpt == "please rename")
        );

        let v = json!({"action": "created", "repository": repo(), "pull_request": pr(json!({})),
            "comment": {"body": "nit", "path": "src/a.rs", "line": 12, "user": {"login": "bob"}, "html_url": "https://github.com/x#c1"}});
        let ev = parse_event("pull_request_review_comment", &v);
        assert!(
            matches!(&ev[0].kind, ForgeKind::Comment { inline: true, path: Some(p), line: Some(12), .. } if p == "src/a.rs")
        );

        let v = json!({"action": "created", "repository": repo(),
            "issue": {"number": 7, "title": "SHOP-12 fix", "pull_request": {"html_url": "https://github.com/Acme/Shop/pull/7"}, "user": {"login": "ann"}},
            "comment": {"body": "ping", "user": {"login": "bob"}, "html_url": "https://github.com/x#c2"}});
        let ev = parse_event("issue_comment", &v);
        assert_eq!(ev[0].pr.as_ref().unwrap().number, 7);
        // a comment on a plain issue is not ours
        let v = json!({"action": "created", "repository": repo(), "issue": {"number": 3}, "comment": {"body": "x"}});
        assert!(parse_event("issue_comment", &v).is_empty());
    }

    #[test]
    fn push_installation_and_ping() {
        let v = json!({"ref": "refs/heads/shop-12-fix", "after": "def", "repository": repo()});
        assert!(
            matches!(&parse_event("push", &v)[0].kind, ForgeKind::Push { branch, sha, .. } if branch == "shop-12-fix" && sha == "def")
        );
        assert!(
            parse_event(
                "push",
                &json!({"ref": "refs/tags/v1", "repository": repo()})
            )
            .is_empty()
        );
        let v = json!({"action": "created", "installation": {"id": 5, "account": {"login": "acme", "type": "Organization"}, "repository_selection": "all"}});
        let ev = parse_event("installation", &v);
        assert_eq!(ev[0].installation_id, Some(5));
        assert!(
            matches!(&ev[0].kind, ForgeKind::InstallationAdded { account: Some(a), .. } if a == "acme")
        );
        assert_eq!(
            parse_event(
                "installation",
                &json!({"action": "deleted", "installation": {"id": 5}})
            )[0]
            .kind,
            ForgeKind::InstallationRemoved
        );
        assert!(parse_event("ping", &json!({"zen": "x"})).is_empty());
    }

    #[test]
    fn manifest_is_read_only() {
        let m = manifest("corntchap", "https://c.example");
        assert_eq!(
            m["hook_attributes"]["url"],
            "https://c.example/webhooks/github"
        );
        assert_eq!(
            m["redirect_url"],
            "https://c.example/forges/github/callback"
        );
        assert!(
            m["default_permissions"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v == "read")
        );
    }
}
