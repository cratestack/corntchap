-- A cook's main station in a pot: the session that answers messages and mentions (and the first
-- one offered new work while idle). At most one per cook and pot; deleting the station drops it.
alter table stations add column main boolean not null default false;
create unique index stations_main on stations (agent_id, project_id) where main;
