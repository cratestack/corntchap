-- When each chef last read a pot's pantry (read_pantry, recall, get_memory): what was saved or
-- changed since, by someone else, is that chef's pantry news (shown when it claims work).
create table pantry_reads (
  principal_id uuid not null references principals(id) on delete cascade,
  project_id uuid not null references projects(id) on delete cascade,
  last_read_at timestamptz not null default now(),
  primary key (principal_id, project_id)
);
