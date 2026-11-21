-- Browser sessions move to authkestra's SessionStore (src/sessions.rs). The sign-in attempt
-- state now lives in an encrypted cookie (authkestra OAuth2State), so oauth_states goes away.
-- Existing browser sessions are dropped: users sign in again.
drop table oauth_states;
drop table web_sessions;

create table auth_sessions (
  id_hash bytea primary key,
  user_id uuid not null references principals(id) on delete cascade,
  data jsonb not null,
  expires_at timestamptz not null
);
create index auth_sessions_expires on auth_sessions(expires_at);
create index auth_sessions_user on auth_sessions(user_id);
