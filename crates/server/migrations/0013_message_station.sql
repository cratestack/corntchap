-- Which session (station) wrote a pass message or a comment: a cook can run several sessions,
-- and people need to tell them apart.
alter table pass_messages add column station_id uuid references stations(id) on delete set null;
alter table comments add column station_id uuid references stations(id) on delete set null;
