-- Deleting cooks, fires and pots (src/admin.rs).
--
-- Rows that merely record WHO did something must not block deleting the principal, and must not take
-- the history with them: the author column becomes null (the text of a comment, a done checklist item
-- or a task stays). Until now these five had no `on delete` action (= refuse the delete).
alter table tasks drop constraint tasks_claimed_by_fkey;
alter table tasks add constraint tasks_claimed_by_fkey foreign key (claimed_by) references principals(id) on delete set null;
alter table tasks drop constraint tasks_created_by_fkey;
alter table tasks add constraint tasks_created_by_fkey foreign key (created_by) references principals(id) on delete set null;
alter table items drop constraint items_done_by_fkey;
alter table items add constraint items_done_by_fkey foreign key (done_by) references principals(id) on delete set null;
alter table comments drop constraint comments_author_id_fkey;
alter table comments add constraint comments_author_id_fkey foreign key (author_id) references principals(id) on delete set null;
alter table device_requests drop constraint device_requests_approved_by_fkey;
alter table device_requests add constraint device_requests_approved_by_fkey foreign key (approved_by) references principals(id) on delete set null;

-- A pot owns its tasks: deleting the project takes them (and with them their checklist items, comments,
-- dependencies, comment replies) along. A sub-task whose parent goes away becomes a top-level task.
alter table tasks drop constraint tasks_project_id_fkey;
alter table tasks add constraint tasks_project_id_fkey foreign key (project_id) references projects(id) on delete cascade;
alter table tasks drop constraint tasks_parent_id_fkey;
alter table tasks add constraint tasks_parent_id_fkey foreign key (parent_id) references tasks(id) on delete set null;
alter table task_deps drop constraint task_deps_depends_on_fkey;
alter table task_deps add constraint task_deps_depends_on_fkey foreign key (depends_on) references tasks(id) on delete cascade;
