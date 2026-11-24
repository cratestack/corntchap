-- Repositories that send forge events although no pot follows them (the delivery is ignored, "in no
-- pot"): remembered so an admin can see them and add them to a pot. A row goes away when the
-- repository joins a pot (repos.rs) or when an admin dismisses it.
create table forge_unfollowed (
  provider text not null,
  norm_url text not null,
  repo text not null,
  html_url text not null default '',
  connection_id uuid references forge_connections(id) on delete cascade,
  installation_id bigint,
  first_seen timestamptz not null default now(),
  last_seen timestamptz not null default now(),
  events integer not null default 1,
  primary key (provider, norm_url)
);
