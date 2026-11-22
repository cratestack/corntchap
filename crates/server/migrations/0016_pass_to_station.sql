-- A direct message to one session (station) rather than to a cook: it wakes exactly that session,
-- one of the sender's own sibling sessions included.
alter table pass_messages add column to_station_id uuid references stations(id) on delete set null;
