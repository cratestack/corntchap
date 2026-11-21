-- Menus: milestones of a project ("MVP", "v1.2"). A task belongs to at most one menu; a menu is
-- `open` while it is being cooked and `served` once it shipped.
create table menus (
  id uuid primary key default gen_random_uuid(),
  project_id uuid not null references projects(id) on delete cascade,
  key text not null check (key ~ '^[a-z0-9][a-z0-9._-]{0,39}$'),
  name text not null check (length(name) between 1 and 80),
  description text not null default '',
  due_on date,
  status text not null default 'open' check (status in ('open', 'served')),
  position bigint not null default 0,
  created_by uuid references principals(id) on delete set null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  served_at timestamptz,
  unique (project_id, key)
);
create trigger menus_touch before update on menus
  for each row execute function touch_updated_at();

alter table tasks add column menu_id uuid references menus(id) on delete set null;
create index tasks_menu on tasks (menu_id) where menu_id is not null;
