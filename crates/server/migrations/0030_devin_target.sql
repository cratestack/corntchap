-- Devin (devin.ai) as a cloud wake target: a wake starts a Devin session, a later wake for the same
-- cook and pot is a follow-up message to it. The sessions are kept in `coder_runs` (api = 'devin',
-- remote_id = the Devin session id); `session_url` is the session's web page, shown on the cook page.
alter table wake_targets drop constraint wake_targets_type_check;
alter table wake_targets add constraint wake_targets_type_check
  check (type in ('relay', 'webhook', 'github-comment', 'claude-routine', 'coder', 'devin'));
alter table cloud_setups drop constraint cloud_setups_type_check;
alter table cloud_setups add constraint cloud_setups_type_check
  check (type in ('claude-routine', 'webhook', 'github-comment', 'coder', 'devin'));

alter table coder_runs add column session_url text;
