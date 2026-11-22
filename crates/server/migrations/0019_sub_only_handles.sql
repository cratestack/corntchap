-- Identity is the OIDC subject and nothing else. A person's `name` is only a handle (the @mention
-- and the thing admins type); it is no longer a sign-in link. `oidc_login` (preferred_username) is
-- gone, which also drops its unique constraint. `display_name` is how people are shown to people,
-- refreshed from the ID token at every sign-in; it is never used for lookup.
alter table principals drop column oidc_login;
alter table principals add column display_name text
  check (display_name is null or char_length(display_name) <= 120);
