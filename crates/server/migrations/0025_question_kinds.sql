-- Questions with a title, a description, a kind and structured choices; answers that name the
-- chosen choices. `body` stays (title + blank line + description) and `answer` stays the readable
-- rendering, so older readers keep working.
alter table questions
  add column title text,
  add column description text not null default '',
  add column kind text not null default 'free' check (kind in ('single', 'multiple', 'free', 'cards')),
  add column choices jsonb not null default '[]',
  add column allow_other boolean not null default false,
  add column min_choices integer check (min_choices >= 0),
  add column max_choices integer check (max_choices >= 1),
  add column answer_choices text[] not null default '{}',
  add column answer_text text;

-- Backfill: the title is the first line of the body (cut near 120 characters at a word boundary),
-- the description is the rest.
with parts as (
  select id,
         btrim(body, E' \n\r\t') as b,
         split_part(btrim(body, E' \n\r\t'), E'\n', 1) as first
  from questions
), cut as (
  select id, b, first,
         case when char_length(first) <= 120 then first
              else coalesce(nullif(regexp_replace(left(first, 120), '\s+\S*$', ''), ''), left(first, 120))
         end as head
  from parts
)
update questions q
set title = btrim(c.head, E' \r\t') || case when char_length(c.first) > char_length(c.head) then '…' else '' end,
    description = btrim(
      substr(c.first, char_length(c.head) + 1) || substr(c.b, char_length(c.first) + 1),
      E' \n\r\t')
from cut c
where c.id = q.id;
update questions set title = 'Question' where title is null or title = '';
alter table questions
  alter column title set not null,
  add constraint questions_title_len check (char_length(title) between 1 and 300);

-- Choices from the old options: a slug per label (numbered when empty or repeated).
update questions q
set choices = s.c,
    kind = 'single'
from (
  select id, jsonb_agg(jsonb_build_object('id', cid, 'label', opt) order by ord) as c
  from (
    select id, opt, ord,
           case when slug = '' or count(*) over (partition by id, slug) > 1
                then coalesce(nullif(slug, ''), 'choice') || '-' || ord
                else slug end as cid
    from (
      select q2.id, o.opt, o.ord,
             left(btrim(lower(regexp_replace(o.opt, '[^a-zA-Z0-9]+', '-', 'g')), '-'), 24) as slug
      from questions q2, unnest(q2.options) with ordinality as o(opt, ord)
    ) x
  ) y
  group by id
) s
where s.id = q.id;

-- Answers: a stored answer that equals a label names that choice, else it is the typed text.
update questions q
set answer_choices = coalesce(
      (select array_agg(c->>'id') from jsonb_array_elements(q.choices) c where c->>'label' = q.answer), '{}')
where q.status = 'answered' and q.answer is not null;
update questions
set answer_text = answer
where status = 'answered' and answer is not null and cardinality(answer_choices) = 0;
