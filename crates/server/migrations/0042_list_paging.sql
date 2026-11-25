-- The console pages every list (the *Page procedures): keyset on the sort each one names, under the
-- filter it takes. One index per sort so a page reads its rows and no more.

-- people and cooks and fires by name (userPage, cookPage, firePage, teamMemberPage)
create index if not exists principals_kind_name on principals (kind, name);
-- stations by pot or by cook, newest first (stationPage)
create index if not exists stations_project_page on stations (project_id, created_at desc, ref);
create index if not exists stations_agent_page on stations (agent_id, created_at desc, ref);
-- a pot's activity (activityPage) and its events by actor (eventPage)
create index if not exists events_activity_page on events (project_id, seq desc) where kind = 'activity';
create index if not exists events_actor_page on events (project_id, actor_id, seq desc);
-- a pot's files, newest first (filePage)
create index if not exists files_page on files (project_id, created_at desc, number desc);
-- one cook's usage over the days (usageCookPage)
create index if not exists usage_daily_principal on usage_daily (principal_id, day);
-- repositories that send events no pot follows, most recent first (forgeUnfollowedPage)
create index if not exists forge_unfollowed_page on forge_unfollowed (last_seen desc, provider, norm_url);

-- tasks by label, and what one person owns (taskPage, myWorkPage)
create index if not exists tasks_labels on tasks using gin (labels);
create index if not exists tasks_owner_page on tasks (owner_id, priority desc, number)
  where status in ('open','claimed','blocked');
-- menus (open first, then served) and saved views by position (menuPage, savedViewPage)
create index if not exists menus_page on menus (project_id, (status = 'served'), position, key);
create index if not exists pot_views_page on pot_views (project_id, position, key);
-- the direct messages a person wrote (passConversationPage)
create index if not exists pass_author_direct on pass_messages (project_id, author_id, seq desc)
  where to_principal is not null;
-- a task's comments and the questions one chef asked, newest first (commentPage, questionMinePage)
create index if not exists comments_page on comments (task_id, created_at desc, id desc);
create index if not exists questions_asked_page on questions (asked_by, created_at desc, id desc);
