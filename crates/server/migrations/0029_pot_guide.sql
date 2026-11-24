-- A pot's guide: "how we work here", written by people for agents (markdown). Returned by
-- what_now, and its start comes with register_station and whoami.
alter table projects add column guide text check (guide is null or char_length(guide) <= 20000);

-- A confined station is a session that cannot leave its folder (a sandbox, a harness locked to
-- one directory): claim_next hands it only tasks about its repository or about none.
alter table stations add column confined boolean not null default false;
