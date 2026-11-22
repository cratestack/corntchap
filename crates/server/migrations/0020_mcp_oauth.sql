-- OAuth 2.1 for MCP clients (OpenCode, Claude Code, Codex, claude.ai connectors): dynamic client
-- registration, authorization code + PKCE, refresh tokens. A token a client gets this way belongs
-- to a cook; when the client asked for a pot's MCP URL (/mcp/<KEY>) it only works there.
alter table tokens add column pot_id uuid references projects(id) on delete cascade;
alter table tokens add column client_id text;

create table oauth_clients (
  id text primary key,
  name text not null,
  redirect_uris text[] not null,
  created_at timestamptz not null default now(),
  last_used_at timestamptz
);

-- A consent screen shown to a signed-in person, waiting for their decision.
create table oauth_requests (
  id text primary key,
  user_id uuid not null references principals(id) on delete cascade,
  params jsonb not null,
  expires_at timestamptz not null
);

create table oauth_codes (
  code_hash bytea primary key,
  client_id text not null references oauth_clients(id) on delete cascade,
  redirect_uri text not null,
  challenge text not null,
  principal_id uuid not null references principals(id) on delete cascade,
  pot_id uuid references projects(id) on delete cascade,
  approved_by uuid not null references principals(id) on delete cascade,
  expires_at timestamptz not null,
  used_at timestamptz
);

create table oauth_refresh (
  token_hash bytea primary key,
  client_id text not null references oauth_clients(id) on delete cascade,
  principal_id uuid not null references principals(id) on delete cascade,
  pot_id uuid references projects(id) on delete cascade,
  access_token_id uuid references tokens(id) on delete set null,
  created_at timestamptz not null default now(),
  expires_at timestamptz not null,
  used_at timestamptz
);
create index oauth_refresh_principal on oauth_refresh(principal_id);
