-- One station per cloud run (a Claude routine run, a Devin session, a Coder run, a webhook or
-- GitHub-comment run): everything the run does (claims, comments, passes) is attributed to it, and
-- the station links to the run's session page. `kind = 'session'` is the registered sessions of
-- before (opencode, claude-code, ...); `cloud-run` stations are created by the dispatcher.
alter table stations add column kind text not null default 'session' check (kind in ('session', 'cloud-run'));
alter table stations add column session_url text;
-- a cloud run is over when its cook holds nothing for it; a session is never "ended"
alter table stations add column ended_at timestamptz;
alter table stations add column target_id uuid references wake_targets(id) on delete set null;
-- the delivery event (events.seq) that started or woke the run
alter table stations add column delivery_seq bigint;

alter table stations drop constraint stations_runtime_check;
alter table stations add constraint stations_runtime_check
  check (runtime in ('opencode', 'claude-code', 'codex', 'copilot', 'antigravity', 'other',
                     'webhook', 'github-comment', 'claude-routine', 'coder', 'devin'));

create index stations_cloud_runs on stations (agent_id, project_id, created_at desc) where kind = 'cloud-run';

-- a Coder or Devin run keeps its station: later wakes are follow-ups in the same run
alter table coder_runs add column station_id uuid references stations(id) on delete set null;
-- the run that opened a pull request (link_pr)
alter table forge_prs add column station_id uuid references stations(id) on delete set null;

-- everything that points at a station: a delete (a failed run's station, remove_station) sets these
-- null, and "what did this run do" / "is anything still on it" look them up by station
create index tasks_station on tasks (station_id) where station_id is not null;
create index comments_station on comments (station_id) where station_id is not null;
create index pass_messages_station on pass_messages (station_id) where station_id is not null;
create index pass_messages_to_station on pass_messages (to_station_id) where to_station_id is not null;
create index questions_station on questions (station_id) where station_id is not null;
create index forge_prs_station on forge_prs (station_id) where station_id is not null;
create index coder_runs_station on coder_runs (station_id) where station_id is not null;
