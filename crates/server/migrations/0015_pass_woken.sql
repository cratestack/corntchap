-- Who posting a pass message woke ([{cook, station, label}]), kept with the message so every
-- reader sees it, not only the answer to the post.
alter table pass_messages add column woken jsonb not null default '[]';
