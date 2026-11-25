-- The console pages done and cancelled tasks newest change first (taskPage): keyset on (updated_at, number).
create index tasks_closed_page on tasks (project_id, status, updated_at desc, number desc)
  where status in ('done','cancelled');
