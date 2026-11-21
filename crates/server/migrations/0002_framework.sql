-- Support for the cratestack-generated REST surface (schema/corntchap.cstack).
--
-- 1. Surrogate ids: cratestack models need a single scalar @id (composite @@id is not
--    generated yet, cratestack issue #136). The composite primary keys stay as they are; the
--    new unique `id` is only what the generated routes address rows by.
alter table team_members add column id uuid not null default gen_random_uuid();
create unique index team_members_id on team_members(id);
alter table task_deps add column id uuid not null default gen_random_uuid();
create unique index task_deps_id on task_deps(id);
alter table items add column id uuid not null default gen_random_uuid();
create unique index items_id on items(id);

-- 2. Text mirrors of uuid columns. cratestack compares an `auth().id` claim (a string) as
--    `column = $1::text`, which Postgres rejects against a uuid column, so every policy that
--    compares a uuid column with the caller goes through a generated text column instead.
alter table principals add column id_ref text generated always as (id::text) stored;
alter table principals add column owner_ref text generated always as (owner_id::text) stored;
alter table team_members add column principal_ref text generated always as (principal_id::text) stored;
alter table tasks add column claimed_ref text generated always as (claimed_by::text) stored;

-- 3. updated_at follows every edit, including those made through generated routes.
create function touch_updated_at() returns trigger language plpgsql as $$
begin
  new.updated_at = now();
  return new;
end $$;
create trigger tasks_touch before update on tasks
  for each row execute function touch_updated_at();

-- 4. cratestack's `Int` is a 64-bit integer: widen the int4 columns the models expose.
alter table principals alter column max_leases type bigint;
alter table projects alter column next_number type bigint;
alter table tasks alter column number type bigint;
alter table tasks alter column priority type bigint;
alter table items alter column position type bigint;
