-- Pots with several repositories (src/repos.rs). All of it is optional: a pot with no repositories,
-- a task with no `repos` and a cook with no scope behave as before.

-- A pot's repositories. `url` is as given; `norm_url` is `host/path` in lower case, whatever the
-- spelling (the CLI normalizes the same way), so a checkout's `origin` finds its repository.
-- `name` is the short alias tasks and cooks use (`backend` for github.com/acme/backend.git).
create table project_repos (
  id uuid primary key default gen_random_uuid(),
  project_id uuid not null references projects(id) on delete cascade,
  seq bigint generated always as identity,
  url text not null,
  norm_url text not null,
  name text not null,
  default_branch text,
  created_at timestamptz not null default now(),
  unique (project_id, name),
  unique (project_id, norm_url)
);
create index project_repos_norm on project_repos (norm_url);

-- Every existing `repo_url` becomes the pot's first repository. `projects.repo_url` stays, kept equal
-- to the first repository's url, for old clients.
with n as (
  select id, repo_url,
    regexp_replace(regexp_replace(regexp_replace(regexp_replace(
      case
        when btrim(repo_url) like '%://%'
          then regexp_replace(btrim(repo_url), '^[A-Za-z][A-Za-z0-9+.-]*://([^/]*@)?([^/:]+)(:[0-9]*)?/?', '\2/')
        else regexp_replace(btrim(repo_url), '^([^/@:]*@)?([^/:]+):(.*)$', '\2/\3')
      end, '/+$', ''), '\.git$', ''), '/+$', ''), '^/+', '') as norm
  from projects
  where repo_url is not null and btrim(repo_url) <> ''
), named as (
  select id, repo_url, lower(norm) as norm,
    coalesce(nullif(regexp_replace(regexp_replace(lower(regexp_replace(norm, '^.*/', '')), '[^a-z0-9._-]+', '-', 'g'), '^[^a-z0-9]+', ''), ''), 'repo') as alias
  from n
)
insert into project_repos (project_id, url, norm_url, name)
select id, btrim(repo_url), norm, left(alias, 40) from named;

-- What a task is about: aliases of its pot's repositories. Empty: any repository.
alter table tasks add column repos text[] not null default '{}';

-- The repositories a cook handles in a pot (aliases). No row: all of them. Only non-empty scopes are stored.
create table cook_repos (
  agent_id uuid not null references principals(id) on delete cascade,
  project_id uuid not null references projects(id) on delete cascade,
  repos text[] not null check (cardinality(repos) > 0),
  updated_at timestamptz not null default now(),
  primary key (agent_id, project_id)
);

-- The repository a station's folder matches (an alias), when `station scan`/`start` found one.
alter table stations add column repo text;

-- Links between a pot's repositories: `repo` depends on `depends_on` (app on backend). When a task
-- naming the upstream repository is served, the dependent's cooks hear of it (mode notify) or get a
-- follow-up task (mode task). One hop only; cycles are allowed, never followed.
create table repo_links (
  repo_id uuid not null references project_repos(id) on delete cascade,
  depends_on_id uuid not null references project_repos(id) on delete cascade,
  mode text not null default 'notify' check (mode in ('notify','task')),
  created_at timestamptz not null default now(),
  primary key (repo_id, depends_on_id),
  check (repo_id <> depends_on_id)
);

-- A follow-up task made by propagation: serving it does not propagate again, so a cycle cannot ping-pong.
alter table tasks add column from_upstream boolean not null default false;
