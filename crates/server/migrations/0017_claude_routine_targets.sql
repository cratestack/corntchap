-- Claude Code routines (on the web) as wake targets: corntchap fires the routine's API trigger.
alter table wake_targets drop constraint wake_targets_type_check;
alter table wake_targets add constraint wake_targets_type_check
  check (type in ('relay', 'webhook', 'github-comment', 'claude-routine'));
