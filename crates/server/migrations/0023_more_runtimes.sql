-- Best-effort harnesses: GitHub Copilot (CLI, desktop app, VS Code agent mode) and Google Antigravity
-- (IDE and the `agy` CLI) join OpenCode, Claude Code and Codex as station runtimes.
alter table stations drop constraint stations_runtime_check;
alter table stations add constraint stations_runtime_check
  check (runtime in ('opencode', 'claude-code', 'codex', 'copilot', 'antigravity', 'other'));
