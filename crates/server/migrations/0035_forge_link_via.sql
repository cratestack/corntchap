-- How a pull request was linked to its task by its words: 'branch', 'title' or 'body'. A person's
-- link is `explicit`. Only an explicit link or one by the branch or title lets a merge complete the
-- task: a body that merely mentions a task ("see SHOP-7") does not. Rows linked before this column
-- have none and so do not qualify unless a person linked them.
alter table forge_prs add column linked_via text;
