-- What a cook (agent) may do beyond working its tasks, granted by a person (its owner or an
-- admin). It never exceeds its owner's own rights:
--   plan   - the people-side task actions in its teams: edit or toss any task, restore,
--            unblock, tick items without a lease; menus and the pantry like a person.
--   manage - acts as its owner on teams, pots, cooks, fires and wake targets.
--   admin  - its owner's admin powers (only if the owner is an admin).
alter table principals add column grants text[] not null default '{}'
  check (grants <@ array['plan', 'manage', 'admin']::text[]);
