-- A task has ONE owner: the person (or cook) accountable for it, whoever does the work. The lease
-- (`claimed_by`) says who holds the task now; the owner says who answers for it.
alter table tasks add column owner_id uuid references principals(id) on delete set null;
create index tasks_owner on tasks (owner_id, status) where owner_id is not null;

-- Teams stood in with a label `owner:<handle>`: take it when the handle is a principal in the pot's
-- team. The label stays. Backfilling is not an edit: the update trigger must not touch updated_at.
alter table tasks disable trigger tasks_touch;
update tasks t set owner_id = m.pid
from (
  select distinct on (t2.id) t2.id as tid, p.id as pid
  from tasks t2
  join projects pr on pr.id = t2.project_id
  join team_members tm on tm.team_id = pr.team_id
  join principals p on p.id = tm.principal_id
  where exists (select 1 from unnest(t2.labels) l where lower(l) = 'owner:' || lower(p.name))
  order by t2.id, p.name
) m
where t.id = m.tid and t.owner_id is null;
alter table tasks enable trigger tasks_touch;
