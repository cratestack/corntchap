-- A cook with cloud targets only is fire and forget: ready work and its owner's direct messages
-- start it, nothing else. The `on_call` opt-out is gone.
update wake_targets set config = config - 'on_call' where config ? 'on_call';
