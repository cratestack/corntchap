//! Forge events: a git host tells corntchap about pull requests, checks and reviews.
//!
//! What it is and is not:
//! * Events only attach to work that exists. Nothing is imported: no issue becomes a task and no
//!   task is created from a forge.
//! * A merge or a close never changes a task's status. It is shown and the holder is told.
//! * Only repositories that belong to a pot (`project_repos`, or a PR a person linked by hand) are
//!   followed; events about any other repository are acknowledged and dropped.
//!
//! The pieces:
//! * [`Forge`] (this file): one implementation per provider turns a signed webhook delivery into
//!   normalized [`ForgeEvent`]s. [`github`] is the first.
//! * `apply` (events.rs): the provider-neutral part: tracks `forge_prs`, links a PR to a task,
//!   emits task events and wakes the holder.
//! * `store` (store.rs): connections, installations and deliveries.
//! * `web` (web.rs): `POST /webhooks/{provider}`, the GitHub App manifest flow and the admin API.
//!
//! # Adding GitLab (not built yet; a new file `gitlab.rs`, nothing else changes)
//!
//! * Verify: GitLab sends the secret itself in `X-Gitlab-Token`; compare it in constant time with the
//!   connection's secret (there is no HMAC). The delivery id is `X-Gitlab-Event-UUID` (or the
//!   `Idempotency-Key` header); the event name is `X-Gitlab-Event` (`Merge Request Hook`,
//!   `Pipeline Hook`, `Note Hook`, `Push Hook`).
//! * `Merge Request Hook` (`object_attributes.action`): `open` is [`ForgeKind::PrOpened`], `reopen`
//!   [`ForgeKind::PrReopened`], `update` with `oldrev` set [`ForgeKind::PrUpdated`], `close`
//!   [`ForgeKind::PrClosed`], `merge` [`ForgeKind::PrMerged`]. `object_attributes.iid` is the number,
//!   `source_branch` the head branch, `last_commit.id` the head sha, `work_in_progress` or `draft`
//!   the draft flag, and `merge_status` of `cannot_be_merged` (or `detailed_merge_status:
//!   conflict`) becomes [`ForgeKind::Conflict`]. The repository is `project.path_with_namespace`,
//!   `project.web_url` and `project.git_http_url`.
//! * `Pipeline Hook`: `object_attributes.status` of `success`, `failed` or `canceled` finishes the
//!   checks ([`ForgeKind::ChecksCompleted`], source Suite); failing job names are the `builds` whose
//!   `status` is `failed`; `merge_request.iid` ties it to the PR; the details url is
//!   `project.web_url/-/pipelines/{object_attributes.id}`.
//! * `Note Hook` with `object_attributes.noteable_type` `MergeRequest`: [`ForgeKind::Comment`]
//!   (`position.new_path` and `position.new_line` make it an inline review comment). An approval is
//!   a `Merge Request Hook` with `action: approved` (a [`ForgeKind::Review`] approved).
//! * `Push Hook`: [`ForgeKind::Push`] from `ref`, `after`.
//!
//! # Adding Gitea (a new file `gitea.rs`)
//!
//! * Verify: `X-Gitea-Signature` is the hex HMAC-SHA256 of the raw body (no `sha256=` prefix;
//!   `X-Hub-Signature-256` is also sent, and `X-Gogs-Signature` on old versions). The delivery id is
//!   `X-Gitea-Delivery`, the event name `X-Gitea-Event` (or `X-Gitea-Event-Type`).
//! * `pull_request` (`action`): `opened`, `reopened`, `synchronized` (new head), `closed` with
//!   `pull_request.merged` for merged. Same field names as GitHub (`number`, `head.ref`, `head.sha`,
//!   `draft`), `mergeable: false` is a conflict.
//! * `status` (a commit status, `state` of `success`, `failure` or `error`, `sha`, `context`,
//!   `target_url`): a [`ForgeKind::ChecksCompleted`] with source Suite; the PRs are found by
//!   head sha, which corntchap already tracks.
//! * `pull_request_review_approved`, `pull_request_review_rejected` and
//!   `pull_request_review_comment` (Gitea 1.20 and later also send `pull_request_review` with
//!   `review.type`): [`ForgeKind::Review`] approved, changes requested, commented, and
//!   [`ForgeKind::Comment`] (`issue_comment` with `is_pull: true` for conversation comments).
//! * `push`: as GitHub.
//!
//! Both need a branch for the provider in the `forge_connections.provider` check (the migration already
//! allows `gitlab` and `gitea`), a connection form on the admin page and one line in [`for_provider`].

pub mod events;
pub mod github;
pub mod link;
pub mod store;
pub mod web;

use http::HeaderMap;

/// A provider name as stored (`github`, later `gitlab` and `gitea`).
pub type Provider = &'static str;

/// One implementation per git host.
pub trait Forge: Send + Sync {
    fn provider(&self) -> Provider;
    /// Does the delivery carry a valid signature (or token) for this secret? Takes the raw body.
    fn verify(&self, secret: &str, headers: &HeaderMap, body: &[u8]) -> bool;
    /// The unique id of the delivery, for deduplication.
    fn delivery_id(&self, headers: &HeaderMap) -> Option<String>;
    /// The provider's name for the event (`pull_request`, `check_suite`, ...).
    fn event_name(&self, headers: &HeaderMap) -> Option<String>;
    /// Turn a delivery into normalized events (none for one corntchap does not use).
    fn parse(&self, event: &str, body: &[u8]) -> Result<Vec<ForgeEvent>, String>;
}

/// The implementation for a provider name in the URL (`/webhooks/{provider}`).
pub fn for_provider(name: &str) -> Option<&'static dyn Forge> {
    match name {
        "github" => Some(&github::GitHub),
        _ => None,
    }
}

/// A repository as the forge names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRef {
    /// `owner/name`.
    pub full_name: String,
    pub html_url: String,
    pub clone_url: String,
    /// `host/path` in lower case, as `project_repos.norm_url`.
    pub norm_url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl PrState {
    pub fn as_str(self) -> &'static str {
        match self {
            PrState::Open => "open",
            PrState::Closed => "closed",
            PrState::Merged => "merged",
        }
    }
}

/// What an event says about a pull request. Events that are not about the PR itself (a check, a
/// comment) know little of it: every field but the number is then optional.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrRef {
    pub number: i64,
    pub url: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub head_branch: Option<String>,
    pub head_sha: Option<String>,
    pub base_branch: Option<String>,
    pub author: Option<String>,
    pub draft: Option<bool>,
    pub state: Option<PrState>,
    /// `Some(false)`: it has conflicts. GitHub often leaves it out (it computes it later).
    pub mergeable: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conclusion {
    Success,
    Failure,
    Cancelled,
    TimedOut,
}

impl Conclusion {
    pub fn as_str(self) -> &'static str {
        match self {
            Conclusion::Success => "success",
            Conclusion::Failure => "failure",
            Conclusion::Cancelled => "cancelled",
            Conclusion::TimedOut => "timed_out",
        }
    }
    pub fn failed(self) -> bool {
        matches!(self, Conclusion::Failure | Conclusion::TimedOut)
    }
}

/// Which granularity a checks event has. Only a suite (or pipeline) and a workflow run finish "the
/// checks"; a single run only keeps the list of failing names up to date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckSource {
    Run,
    Suite,
    Workflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
}

impl ReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewState::Approved => "approved",
            ReviewState::ChangesRequested => "changes_requested",
            ReviewState::Commented => "commented",
            ReviewState::Dismissed => "dismissed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForgeKind {
    PrOpened,
    /// Metadata changed; `head_changed`: new commits were pushed.
    PrUpdated {
        head_changed: bool,
    },
    PrClosed,
    PrMerged,
    PrReopened,
    ChecksCompleted {
        source: CheckSource,
        /// The check, workflow or app name.
        name: String,
        conclusion: Conclusion,
        summary: String,
        details_url: Option<String>,
    },
    Review {
        state: ReviewState,
        author: String,
        excerpt: String,
        url: String,
    },
    /// A review comment (inline, with `path` and `line`) or a conversation comment on the PR.
    Comment {
        inline: bool,
        author: String,
        excerpt: String,
        path: Option<String>,
        line: Option<i64>,
        url: String,
    },
    /// The PR no longer merges cleanly.
    Conflict,
    Push {
        branch: String,
        sha: String,
        deleted: bool,
    },
    InstallationAdded {
        account: Option<String>,
        account_type: Option<String>,
        repo_selection: Option<String>,
    },
    InstallationRemoved,
    InstallationSuspended(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeEvent {
    pub provider: Provider,
    pub repo: Option<RepoRef>,
    pub pr: Option<PrRef>,
    pub kind: ForgeKind,
    pub installation_id: Option<i64>,
    /// Who did it, when the payload says.
    pub sender: Option<String>,
}

/// `text` trimmed and cut to `max` characters (on a character boundary).
pub fn excerpt(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut s: String = t.chars().take(max).collect();
    s.push('…');
    s
}
