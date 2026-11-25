-- The restaurant: the server itself as a visible actor. When the backend takes a decision (a lease that
-- lapsed, a task blocked after five give-backs, a cook that is not woken any more), the event or the note
-- names an actor like any other, with a reason. It is a principal row so the feed, the task sheet and the
-- MCP tools show it the way they show a person or a cook, and so nothing can sign in as it: the row is
-- disabled, and a trigger refuses it every credential, session, membership, wake target and station.

-- A person or cook already named "restaurant" is renamed first (its history follows the id).
do $$
declare
  old record;
  fresh text;
begin
  for old in select id, name, kind from principals where name = 'restaurant' loop
    fresh := 'restaurant-' || substr(md5(old.id::text), 1, 4);
    update principals set name = fresh where id = old.id;
    if old.kind = 'user' then
      insert into handle_claims (name, principal_id) values (fresh, old.id) on conflict (name) do nothing;
    end if;
    insert into events (kind, actor_id, payload)
      values ('principal.renamed', old.id, jsonb_build_object('from', 'restaurant', 'to', fresh, 'kind', old.kind,
                                                              'reason', 'the name restaurant is the server''s'));
  end loop;
end $$;

alter table principals drop constraint principals_kind_check;
alter table principals add constraint principals_kind_check check (kind in ('user','agent','relay','restaurant'));

insert into principals (id, kind, name, display_name, role, disabled_at)
values ('00000000-0000-4000-8000-524553545552', 'restaurant', 'restaurant', 'restaurant', 'member', now());

-- At most one, and never a person or a cook in disguise.
create unique index principals_one_restaurant on principals ((true)) where kind = 'restaurant';
alter table principals add constraint principals_restaurant_shape
  check (kind <> 'restaurant' or (oidc_sub is null and owner_id is null and role = 'member'));

-- Refuse the restaurant every way in (the column that names the principal is the trigger's argument).
create function forbid_restaurant() returns trigger language plpgsql as $$
begin
  if to_jsonb(new) ->> tg_argv[0] = '00000000-0000-4000-8000-524553545552' then
    raise exception 'the restaurant is the server itself: it cannot have rows in %', tg_table_name
      using errcode = 'check_violation';
  end if;
  return new;
end $$;

create trigger tokens_no_restaurant before insert or update on tokens
  for each row execute function forbid_restaurant('principal_id');
create trigger auth_sessions_no_restaurant before insert or update on auth_sessions
  for each row execute function forbid_restaurant('user_id');
create trigger oauth_codes_no_restaurant before insert or update on oauth_codes
  for each row execute function forbid_restaurant('principal_id');
create trigger oauth_refresh_no_restaurant before insert or update on oauth_refresh
  for each row execute function forbid_restaurant('principal_id');
create trigger team_members_no_restaurant before insert or update on team_members
  for each row execute function forbid_restaurant('principal_id');
create trigger wake_targets_no_restaurant before insert or update on wake_targets
  for each row execute function forbid_restaurant('agent_id');
create trigger stations_no_restaurant before insert or update on stations
  for each row execute function forbid_restaurant('agent_id');
create trigger handle_claims_no_restaurant before insert or update on handle_claims
  for each row execute function forbid_restaurant('principal_id');

-- The row itself: who it is never changes and it is never deleted (bookkeeping columns may move).
create function keep_restaurant() returns trigger language plpgsql as $$
begin
  if old.id = '00000000-0000-4000-8000-524553545552' and (tg_op = 'DELETE'
     or new.id <> old.id or new.kind <> old.kind or new.name <> old.name or new.role <> old.role
     or new.disabled_at is distinct from old.disabled_at or new.oidc_sub is distinct from old.oidc_sub
     or new.owner_id is distinct from old.owner_id) then
    raise exception 'the restaurant is the server itself: its row cannot change'
      using errcode = 'check_violation';
  end if;
  return coalesce(new, old);
end $$;
create trigger principals_keep_restaurant before update or delete on principals
  for each row execute function keep_restaurant();

-- What a cook claimed since a decision (restaurant.rs `holding_line`).
create index events_claimed_by_actor on events (actor_id, seq) where kind = 'task.claimed';

-- History: what the server decided on its own was recorded without an actor (forge events stay that way:
-- GitHub is their actor, except the two the server itself makes).
update events set actor_id = '00000000-0000-4000-8000-524553545552'
 where actor_id is null and (kind not like 'forge.%' or kind in ('forge.linked', 'forge.backfill'));
update comments set author_id = '00000000-0000-4000-8000-524553545552'
 where author_id is null
   and (starts_with(body, '[released by the server]') or starts_with(body, '[blocked by the server]')
        or starts_with(body, 'Upstream change passed on ('));

-- The restaurant's decisions about one cook (the cook page, what_now).
create index events_restaurant_agent on events ((payload ->> 'agent_id'), seq)
  where actor_id = '00000000-0000-4000-8000-524553545552';

-- Decisions the restaurant already told about: one event per key per window, the rest only counted
-- (restaurant.rs `decide`).
create table restaurant_quiet (
  key text primary key,
  last_seq bigint,
  last_at timestamptz not null default now(),
  suppressed int not null default 0
);
