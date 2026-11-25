-- The console's board pages each column (taskPage): ready, on the fire and waiting walk the tasks still in
-- play by (priority desc, number) under a pot.
create index tasks_active_page on tasks (project_id, priority desc, number)
  where status in ('open','claimed','blocked');
