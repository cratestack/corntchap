-- Identity is (OIDC issuer, subject) and nothing else. A person's `name` is only a handle (the @mention
-- and the thing admins type); it is no longer a sign-in link. `oidc_login` (preferred_username) is
-- gone, which also drops its unique constraint. `display_name` is how people are shown to people,
-- refreshed from the ID token at every sign-in; it is never used for lookup.
alter table principals drop column oidc_login;
alter table principals add column display_name text
  check (display_name is null or char_length(display_name) <= 120);

-- The subject alone is only unique within its issuer. Rows that signed in before issuers were recorded
-- keep a NULL issuer until the server adopts them at startup (corntchap serve), so new rows must carry one.
alter table principals add column oidc_iss text;
alter table principals drop constraint principals_oidc_sub_key;
alter table principals add constraint principals_oidc_identity_key unique (oidc_iss, oidc_sub);
alter table principals add constraint principals_sub_has_issuer
  check (oidc_sub is null or oidc_iss is not null) not valid;

-- Every handle a person ever had, with whom it belongs to: a handle is never assigned to a different
-- account, so a name said yesterday (an @mention, an admin's command) cannot reach a new holder tomorrow.
-- The primary key makes the claim atomic. No foreign key: a claim outlives anything.
create table handle_claims (
  name text primary key,
  principal_id uuid not null,
  claimed_at timestamptz not null default now()
);
insert into handle_claims (name, principal_id) select name, id from principals where kind = 'user';
