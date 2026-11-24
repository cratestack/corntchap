-- Roles replace the admin flag: super_admin (what an admin was) > orchestrator (creates and manages their own
-- teams, pots, cooks and fires) > member. Only people have a role; cooks and fires are members and act through
-- their owner.
alter table principals
  add column role text not null default 'member' check (role in ('super_admin','orchestrator','member'));

update principals set role = 'super_admin' where kind = 'user' and is_admin;

-- Owning a team now needs the orchestrator role: people who own one today become orchestrators. The setting is
-- manual, so the IdP role sync never takes it back.
update principals p set role = 'orchestrator', admin_source = 'manual'
 where p.kind = 'user' and p.role = 'member'
   and exists (select 1 from team_members m where m.principal_id = p.id and m.role = 'owner');

alter table principals add constraint principals_role_people check (role = 'member' or kind = 'user');

-- Who owns the role: 'init' (corntchap init; never changed by the IdP), 'manual' (set by a super-admin or
-- migrated; never changed by the IdP), 'oidc' (follows the IdP role claims at each sign-in).
alter table principals rename column admin_source to role_source;
alter table principals rename constraint principals_admin_source_check to principals_role_source_check;
alter table principals drop column is_admin;
