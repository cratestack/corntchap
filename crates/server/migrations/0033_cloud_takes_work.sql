-- Cloud cooks take new work by default; `on_call` opts out. The old opt-in flag is gone.
update wake_targets set config = config - 'takes_work' where config ? 'takes_work';
