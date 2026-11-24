-- Leases end when the work stops: a holder that makes no progress (a comment, a ticked item, an
-- update, a question, a review request, a saved memory, a forge event) for STALL_MINUTES, or holds
-- the task for MAX_HOLD_HOURS, loses it. `held_since` is when the current holder claimed it;
-- `progress_at` the last progress. Bare heartbeats and activity renewals are not progress.
alter table tasks add column held_since timestamptz;
alter table tasks add column progress_at timestamptz;
-- Backfilling is not an edit: the update trigger must not touch updated_at.
alter table tasks disable trigger tasks_touch;
update tasks set held_since = updated_at, progress_at = updated_at where status = 'claimed';
alter table tasks enable trigger tasks_touch;

-- An agent back after an absence checks the board first: the first call that resolves it after 30
-- minutes of silence sets `back_at` (and `away_since`, when it was last seen); its next claim in a pot
-- returns a catch-up instead of a lease until it has been briefed there (what_now and the other reads
-- brief it).
alter table principals add column away_since timestamptz;
alter table principals add column back_at timestamptz;
create table pot_briefings (
  principal_id uuid not null references principals(id) on delete cascade,
  project_id uuid not null references projects(id) on delete cascade,
  briefed_at timestamptz not null default now(),
  primary key (principal_id, project_id)
);

-- Memories that get read: how often, and when last. Reading must not look like an edit (it is not
-- pantry news), so the touch trigger fires only when the content columns change.
alter table memories add column uses int not null default 0;
alter table memories add column last_used_at timestamptz;
drop trigger memories_touch on memories;
create trigger memories_touch before update of key, title, body, kind, tags, task_id, updated_by, revision on memories
  for each row execute function touch_updated_at();
