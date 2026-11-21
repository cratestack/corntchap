//! `corntchap-cli kitchen`: a live board in the terminal (ratatui + crossterm).
//!
//! `App` is pure state: `update(Msg) -> Vec<Action>` and `draw(frame, &App)` never touch the network,
//! so both are unit-tested with ratatui's `TestBackend`. `run` is the thin async shell that executes
//! actions against the REST API and feeds the results back as messages.

use std::{
    io::IsTerminal,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use ratatui::{
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::api::Api;

const POLL: Duration = Duration::from_millis(2500);
/// Display order of status groups in the task pane.
const GROUPS: [&str; 5] = ["claimed", "blocked", "open", "done", "cancelled"];

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Projects,
    Tasks,
    Detail,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddField {
    Title,
    Items,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Mode {
    Normal,
    Help,
    Add {
        title: String,
        items: String,
        field: AddField,
    },
    Comment(String),
    Confirm {
        prompt: String,
        action: Action,
    },
}

/// Work for the async shell.
#[derive(Clone, PartialEq, Debug)]
pub enum Action {
    Quit,
    Refresh,
    LoadTasks(String),
    LoadDetail(String),
    Check {
        id: String,
        pos: i64,
        done: bool,
    },
    AddTask {
        project: String,
        title: String,
        items: Vec<String>,
    },
    Comment {
        id: String,
        body: String,
    },
    Done(String),
    Unblock(String),
    Cancel(String),
}

/// Events for `App::update`.
pub enum Msg {
    Key(KeyEvent),
    Tick,
    Me(Result<String, String>),
    Projects(Result<Value, String>),
    Tasks(String, Result<Value, String>),
    Detail(String, Result<Value, String>),
    Acted(String, Result<(), String>),
}

pub struct App {
    pub server: String,
    pub me: String,
    pub projects: Vec<Value>,
    pub sel_project: usize,
    pub tasks: Vec<Value>,
    /// Project key the `tasks` list belongs to.
    pub tasks_for: Option<String>,
    pub sel_task: usize,
    pub detail: Option<Value>,
    pub item_sel: Option<usize>,
    pub focus: Pane,
    pub mode: Mode,
    pub notice: Option<String>,
    pub error: Option<String>,
    pub last_refresh: Option<Instant>,
    pub color: bool,
    pub quit: bool,
}

impl App {
    pub fn new(server: &str) -> App {
        App {
            server: server.to_string(),
            me: "?".into(),
            projects: vec![],
            sel_project: 0,
            tasks: vec![],
            tasks_for: None,
            sel_task: 0,
            detail: None,
            item_sel: None,
            focus: Pane::Projects,
            mode: Mode::Normal,
            notice: None,
            error: None,
            last_refresh: None,
            color: crate::ui::color_on(),
            quit: false,
        }
    }

    pub fn project_key(&self) -> Option<String> {
        self.projects
            .get(self.sel_project)
            .and_then(|p| p["key"].as_str())
            .map(String::from)
    }

    pub fn task_id(&self) -> Option<String> {
        self.tasks
            .get(self.sel_task)
            .and_then(|t| t["id"].as_str())
            .map(String::from)
    }

    fn detail_items(&self) -> &[Value] {
        self.detail
            .as_ref()
            .filter(|d| d["id"].as_str() == self.task_id().as_deref())
            .and_then(|d| d["items"].as_array())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Item the cursor is on: the one chosen with j/k, else the first unchecked one.
    pub fn cur_item(&self) -> Option<usize> {
        let items = self.detail_items();
        if items.is_empty() {
            return None;
        }
        let i = self
            .item_sel
            .unwrap_or_else(|| items.iter().position(|i| i["done"] != true).unwrap_or(0));
        Some(i.min(items.len() - 1))
    }

    pub fn update(&mut self, msg: Msg) -> Vec<Action> {
        match msg {
            Msg::Key(k) => self.key(k),
            Msg::Tick => vec![Action::Refresh],
            Msg::Me(r) => {
                match r {
                    Ok(n) => self.me = n,
                    Err(e) => self.error = Some(e),
                }
                vec![]
            }
            Msg::Projects(r) => match r {
                Ok(v) => {
                    self.last_refresh = Some(Instant::now());
                    self.error = None;
                    let keep = self.project_key();
                    self.projects = v.as_array().cloned().unwrap_or_default();
                    self.sel_project = keep
                        .and_then(|k| self.projects.iter().position(|p| p["key"] == k.as_str()))
                        .unwrap_or(0)
                        .min(self.projects.len().saturating_sub(1));
                    match self.project_key() {
                        Some(k) if self.tasks_for.as_deref() != Some(&k) => {
                            vec![Action::LoadTasks(k)]
                        }
                        _ => vec![],
                    }
                }
                Err(e) => {
                    self.error = Some(e);
                    vec![]
                }
            },
            Msg::Tasks(key, r) => match r {
                Ok(v) => {
                    if self.project_key().as_deref() != Some(&key) {
                        return vec![]; // stale answer for a project we left
                    }
                    self.error = None;
                    let keep = self.task_id();
                    let mut tasks = v.as_array().cloned().unwrap_or_default();
                    tasks.sort_by_key(|t| {
                        GROUPS
                            .iter()
                            .position(|g| t["status"] == *g)
                            .unwrap_or(GROUPS.len())
                    });
                    self.tasks = tasks;
                    self.tasks_for = Some(key);
                    self.sel_task = keep
                        .and_then(|id| self.tasks.iter().position(|t| t["id"] == id.as_str()))
                        .unwrap_or(self.sel_task)
                        .min(self.tasks.len().saturating_sub(1));
                    match self.task_id() {
                        Some(id)
                            if self
                                .detail
                                .as_ref()
                                .map(|d| d["id"] != id.as_str())
                                .unwrap_or(true) =>
                        {
                            vec![Action::LoadDetail(id)]
                        }
                        None => {
                            self.detail = None;
                            vec![]
                        }
                        _ => vec![],
                    }
                }
                Err(e) => {
                    self.error = Some(e);
                    vec![]
                }
            },
            Msg::Detail(id, r) => {
                match r {
                    Ok(v) if self.task_id().as_deref() == Some(&id) => {
                        self.error = None;
                        self.detail = Some(v);
                    }
                    Ok(_) => {}
                    Err(e) => self.error = Some(e),
                }
                vec![]
            }
            Msg::Acted(label, r) => match r {
                Ok(()) => {
                    self.notice = Some(label);
                    vec![Action::Refresh]
                }
                Err(e) => {
                    self.error = Some(format!("{label}: {e}"));
                    vec![]
                }
            },
        }
    }

    fn select_task(&mut self, i: usize) -> Vec<Action> {
        if self.tasks.is_empty() {
            return vec![];
        }
        let i = i.min(self.tasks.len() - 1);
        if i == self.sel_task {
            return vec![];
        }
        self.sel_task = i;
        self.item_sel = None;
        self.task_id().map(Action::LoadDetail).into_iter().collect()
    }

    fn select_project(&mut self, i: usize) -> Vec<Action> {
        if self.projects.is_empty() {
            return vec![];
        }
        let i = i.min(self.projects.len() - 1);
        if i == self.sel_project {
            return vec![];
        }
        self.sel_project = i;
        self.sel_task = 0;
        self.item_sel = None;
        self.tasks.clear();
        self.tasks_for = None;
        self.detail = None;
        self.project_key()
            .map(Action::LoadTasks)
            .into_iter()
            .collect()
    }

    fn step(&mut self, delta: i64) -> Vec<Action> {
        let mv = |cur: usize, len: usize| {
            (cur as i64 + delta).clamp(0, len.saturating_sub(1) as i64) as usize
        };
        match self.focus {
            Pane::Projects => self.select_project(mv(self.sel_project, self.projects.len())),
            Pane::Tasks => self.select_task(mv(self.sel_task, self.tasks.len())),
            Pane::Detail => {
                if let Some(c) = self.cur_item() {
                    self.item_sel = Some(mv(c, self.detail_items().len()));
                }
                vec![]
            }
        }
    }

    fn cycle(&mut self, back: bool) {
        let order = [Pane::Projects, Pane::Tasks, Pane::Detail];
        let i = order.iter().position(|p| *p == self.focus).unwrap_or(0);
        self.focus = order[(i + if back { 2 } else { 1 }) % 3];
    }

    fn key(&mut self, k: KeyEvent) -> Vec<Action> {
        if k.kind == KeyEventKind::Release {
            return vec![];
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            self.quit = true;
            return vec![Action::Quit];
        }
        self.notice = None;
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.key_normal(k),
            Mode::Help => vec![], // any key closes
            Mode::Confirm { prompt, action } => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => vec![action],
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => vec![],
                _ => {
                    self.mode = Mode::Confirm { prompt, action };
                    vec![]
                }
            },
            Mode::Comment(mut text) => match k.code {
                KeyCode::Esc => vec![],
                KeyCode::Enter => {
                    let body = text.trim().to_string();
                    match (self.task_id(), body.is_empty()) {
                        (Some(id), false) => vec![Action::Comment { id, body }],
                        _ => vec![],
                    }
                }
                KeyCode::Backspace => {
                    text.pop();
                    self.mode = Mode::Comment(text);
                    vec![]
                }
                KeyCode::Char(c) if !ctrl => {
                    text.push(c);
                    self.mode = Mode::Comment(text);
                    vec![]
                }
                _ => {
                    self.mode = Mode::Comment(text);
                    vec![]
                }
            },
            Mode::Add {
                mut title,
                mut items,
                mut field,
            } => {
                let submit = |app: &App, title: &str, items: &str| -> Vec<Action> {
                    let title = title.trim().to_string();
                    match (app.project_key(), title.is_empty()) {
                        (Some(project), false) => vec![Action::AddTask {
                            project,
                            title,
                            items: items
                                .lines()
                                .map(|l| l.trim().to_string())
                                .filter(|l| !l.is_empty())
                                .collect(),
                        }],
                        _ => vec![],
                    }
                };
                match k.code {
                    KeyCode::Esc => return vec![],
                    KeyCode::Char('s') if ctrl => return submit(self, &title, &items),
                    KeyCode::Tab | KeyCode::BackTab => {
                        field = if field == AddField::Title {
                            AddField::Items
                        } else {
                            AddField::Title
                        };
                    }
                    KeyCode::Enter => match field {
                        AddField::Title => field = AddField::Items,
                        AddField::Items => {
                            // A blank line ends the list and saves.
                            if items.is_empty() || items.ends_with('\n') {
                                return submit(self, &title, &items);
                            }
                            items.push('\n');
                        }
                    },
                    KeyCode::Backspace => {
                        match field {
                            AddField::Title => title.pop(),
                            AddField::Items => items.pop(),
                        };
                    }
                    KeyCode::Char(c) if !ctrl => match field {
                        AddField::Title => title.push(c),
                        AddField::Items => items.push(c),
                    },
                    _ => {}
                }
                self.mode = Mode::Add {
                    title,
                    items,
                    field,
                };
                vec![]
            }
        }
    }

    fn key_normal(&mut self, k: KeyEvent) -> Vec<Action> {
        match k.code {
            KeyCode::Char('q') => {
                self.quit = true;
                vec![Action::Quit]
            }
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                vec![]
            }
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                self.cycle(false);
                vec![]
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.cycle(true);
                vec![]
            }
            KeyCode::Esc => {
                if self.focus != Pane::Projects {
                    self.cycle(true);
                }
                vec![]
            }
            KeyCode::Enter => {
                match self.focus {
                    Pane::Projects => self.focus = Pane::Tasks,
                    Pane::Tasks => self.focus = Pane::Detail,
                    Pane::Detail => {
                        return self.toggle_item();
                    }
                }
                vec![]
            }
            KeyCode::Char('r') => vec![Action::Refresh],
            KeyCode::Char('a') => {
                if self.project_key().is_some() {
                    self.mode = Mode::Add {
                        title: String::new(),
                        items: String::new(),
                        field: AddField::Title,
                    };
                } else {
                    self.error = Some("no project selected".into());
                }
                vec![]
            }
            KeyCode::Char('c') => {
                if self.task_id().is_some() {
                    self.mode = Mode::Comment(String::new());
                }
                vec![]
            }
            KeyCode::Char('x') | KeyCode::Char(' ') => self.toggle_item(),
            KeyCode::Char('d') => self.task_id().map(Action::Done).into_iter().collect(),
            KeyCode::Char('u') => self.task_id().map(Action::Unblock).into_iter().collect(),
            KeyCode::Char('X') => {
                if let Some(id) = self.task_id() {
                    self.mode = Mode::Confirm {
                        prompt: format!("Cancel {id}? (y/n)"),
                        action: Action::Cancel(id),
                    };
                }
                vec![]
            }
            _ => vec![],
        }
    }

    fn toggle_item(&mut self) -> Vec<Action> {
        let (Some(id), Some(i)) = (self.task_id(), self.cur_item()) else {
            return vec![];
        };
        let item = &self.detail_items()[i];
        vec![Action::Check {
            id,
            pos: item["position"].as_i64().unwrap_or(i as i64 + 1),
            done: item["done"] != true,
        }]
    }
}

// ---------------------------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------------------------

fn status_color(status: &str) -> Color {
    match status {
        "open" => Color::Cyan,
        "claimed" => Color::Yellow,
        "blocked" => Color::Red,
        "done" => Color::Green,
        _ => Color::DarkGray,
    }
}

struct Theme {
    color: bool,
}

impl Theme {
    fn fg(&self, c: Color) -> Style {
        if self.color {
            Style::new().fg(c)
        } else {
            Style::new()
        }
    }
    fn focus(&self, on: bool) -> Style {
        if on {
            self.fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            self.fg(Color::DarkGray)
        }
    }
    fn selected(&self, focused: bool) -> Style {
        if focused {
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::BOLD)
        }
    }
}

fn pane_block<'a>(th: &Theme, title: String, on: bool) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(th.focus(on))
        .title(Span::styled(format!(" {title} "), th.focus(on)))
}

pub fn draw(f: &mut Frame, app: &App) {
    let th = Theme { color: app.color };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(2)])
        .split(f.area());
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(24),
            Constraint::Percentage(42),
            Constraint::Min(20),
        ])
        .split(rows[0]);
    draw_projects(f, app, &th, cols[0]);
    draw_tasks(f, app, &th, cols[1]);
    draw_detail(f, app, &th, cols[2]);
    draw_footer(f, app, &th, rows[1]);
    match &app.mode {
        Mode::Help => draw_help(f, &th),
        Mode::Add {
            title,
            items,
            field,
        } => draw_add(f, &th, title, items, *field),
        Mode::Comment(text) => draw_comment(f, &th, text, app.task_id().unwrap_or_default()),
        Mode::Confirm { prompt, .. } => {
            draw_popup(f, &th, "Confirm", vec![Line::from(prompt.clone())], 50, 5)
        }
        Mode::Normal => {}
    }
}

fn draw_projects(f: &mut Frame, app: &App, th: &Theme, area: Rect) {
    let on = app.focus == Pane::Projects;
    let items: Vec<ListItem> = app
        .projects
        .iter()
        .map(|p| {
            let n = |k: &str| p[k].as_i64().unwrap_or(0);
            ListItem::new(vec![
                Line::from(Span::styled(
                    format!(
                        "{}  {}",
                        p["key"].as_str().unwrap_or("?"),
                        p["name"].as_str().unwrap_or("")
                    ),
                    Style::new().add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(
                        "  {} ready  {} claimed  {} done",
                        n("ready"),
                        n("claimed"),
                        n("done")
                    ),
                    th.fg(Color::DarkGray),
                )),
            ])
        })
        .collect();
    let mut state =
        ListState::default().with_selected((!items.is_empty()).then_some(app.sel_project));
    let list = List::new(items)
        .block(pane_block(th, "Projects".into(), on))
        .highlight_style(th.selected(on));
    f.render_stateful_widget(list, area, &mut state);
    if app.projects.is_empty() {
        let inner = area.inner(ratatui::layout::Margin::new(2, 1));
        f.render_widget(
            Paragraph::new(if app.last_refresh.is_some() {
                "no projects yet"
            } else {
                "loading…"
            })
            .style(th.fg(Color::DarkGray)),
            inner,
        );
    }
}

fn progress(t: &Value) -> String {
    let total = t["total_items"].as_i64().unwrap_or(0);
    if total == 0 {
        String::new()
    } else {
        format!(
            " [{}/{}]",
            total - t["open_items"].as_i64().unwrap_or(0),
            total
        )
    }
}

fn draw_tasks(f: &mut Frame, app: &App, th: &Theme, area: Rect) {
    let on = app.focus == Pane::Tasks;
    let title = match app.project_key() {
        Some(k) => format!("Tasks · {k}"),
        None => "Tasks".into(),
    };
    let mut items: Vec<ListItem> = vec![];
    let mut sel_row = None;
    let mut last_group = "";
    for (i, t) in app.tasks.iter().enumerate() {
        let status = t["status"].as_str().unwrap_or("?");
        if status != last_group {
            let n = app.tasks.iter().filter(|x| x["status"] == status).count();
            items.push(ListItem::new(Line::from(Span::styled(
                format!("● {status} ({n})"),
                th.fg(status_color(status)).add_modifier(Modifier::BOLD),
            ))));
            last_group = status;
        }
        if i == app.sel_task {
            sel_row = Some(items.len());
        }
        let mut spans = vec![
            Span::raw(format!("  {} ", t["id"].as_str().unwrap_or("?"))),
            Span::raw(t["title"].as_str().unwrap_or("").to_string()),
            Span::styled(progress(t), th.fg(Color::Green)),
        ];
        if let Some(h) = t["claimed_by"].as_str() {
            spans.push(Span::styled(format!("  @{h}"), th.fg(Color::Yellow)));
        }
        if t["ready"] == true {
            spans.push(Span::styled("  ready", th.fg(Color::Cyan)));
        }
        items.push(ListItem::new(Line::from(spans)));
    }
    let mut state = ListState::default().with_selected(sel_row);
    let list = List::new(items)
        .block(pane_block(th, title, on))
        .highlight_style(th.selected(on));
    f.render_stateful_widget(list, area, &mut state);
    if app.tasks.is_empty() && app.project_key().is_some() && app.tasks_for.is_some() {
        f.render_widget(
            Paragraph::new("no tasks yet: press a to add one").style(th.fg(Color::DarkGray)),
            area.inner(ratatui::layout::Margin::new(2, 1)),
        );
    }
}

/// Estimated rows a line takes when wrapped to `width`.
fn wrapped_rows(text: &str, width: u16) -> usize {
    let w = width.max(1) as usize;
    text.chars().count().div_ceil(w).max(1)
}

fn draw_detail(f: &mut Frame, app: &App, th: &Theme, area: Rect) {
    let on = app.focus == Pane::Detail;
    let block = pane_block(th, "Task".into(), on);
    let inner_w = area.width.saturating_sub(2);
    let inner_h = area.height.saturating_sub(2) as usize;
    let Some(d) = app
        .detail
        .as_ref()
        .filter(|d| d["id"].as_str() == app.task_id().as_deref())
    else {
        let msg = if app.task_id().is_some() {
            "loading…"
        } else {
            "select a task"
        };
        f.render_widget(
            Paragraph::new(msg)
                .style(th.fg(Color::DarkGray))
                .block(block),
            area,
        );
        return;
    };
    let s = |k: &str| d[k].as_str().unwrap_or("").to_string();
    let status = s("status");
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(s("id"), Style::new().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(s("title"), Style::new().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(status.clone(), th.fg(status_color(&status))),
            Span::raw(format!("  priority {}", d["priority"])),
            Span::styled(
                d["claimed_by"]
                    .as_str()
                    .map(|h| format!("  held by {h}"))
                    .unwrap_or_default(),
                th.fg(Color::Yellow),
            ),
        ]),
    ];
    let mut rows_before_sel = 0usize;
    let mut used: usize = 2;
    let body = s("body");
    if !body.trim().is_empty() {
        lines.push(Line::raw(""));
        used += 1;
        for l in body.lines() {
            used += wrapped_rows(l, inner_w);
            lines.push(Line::raw(l.to_string()));
        }
    }
    let items = d["items"].as_array().cloned().unwrap_or_default();
    let cur = if on || !items.is_empty() {
        app.cur_item()
    } else {
        None
    };
    if !items.is_empty() {
        lines.push(Line::raw(""));
        used += 1;
        lines.push(Line::from(Span::styled(
            "Checklist",
            Style::new().add_modifier(Modifier::BOLD),
        )));
        used += 1;
        for (i, it) in items.iter().enumerate() {
            let done = it["done"] == true;
            let text = format!(
                "{} {}",
                if done { "[x]" } else { "[ ]" },
                it["text"].as_str().unwrap_or("")
            );
            if Some(i) == cur {
                rows_before_sel = used;
            }
            used += wrapped_rows(&text, inner_w);
            let mut style = if done {
                th.fg(Color::DarkGray)
            } else {
                Style::new()
            };
            if Some(i) == cur {
                style = style.add_modifier(Modifier::REVERSED);
            }
            lines.push(Line::from(Span::styled(text, style)));
        }
    }
    if let Some(deps) = d["dependencies"].as_array().filter(|x| !x.is_empty()) {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "Depends on",
            Style::new().add_modifier(Modifier::BOLD),
        )));
        for x in deps {
            let st = x["status"].as_str().unwrap_or("?");
            lines.push(Line::from(vec![
                Span::raw(format!("  {} ", x["id"].as_str().unwrap_or("?"))),
                Span::styled(format!("[{st}] "), th.fg(status_color(st))),
                Span::raw(x["title"].as_str().unwrap_or("").to_string()),
            ]));
        }
    }
    if let Some(cs) = d["comments"].as_array().filter(|x| !x.is_empty()) {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "Comments",
            Style::new().add_modifier(Modifier::BOLD),
        )));
        for c in cs {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}: ", c["author"].as_str().unwrap_or("?")),
                    th.fg(Color::Cyan),
                ),
                Span::raw(c["body"].as_str().unwrap_or("").to_string()),
            ]));
        }
    }
    if let Some(r) = d["result"].as_str().filter(|r| !r.is_empty()) {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "Result",
            Style::new().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::raw(r.to_string()));
    }
    // Keep the item cursor on screen.
    let scroll = if rows_before_sel >= inner_h.saturating_sub(1) {
        (rows_before_sel + 2).saturating_sub(inner_h) as u16
    } else {
        0
    };
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(block),
        area,
    );
}

fn ago(t: Option<Instant>) -> String {
    match t {
        None => "not refreshed yet".into(),
        Some(t) => format!("refreshed {}s ago", t.elapsed().as_secs()),
    }
}

fn draw_footer(f: &mut Frame, app: &App, th: &Theme, area: Rect) {
    let hints = match app.mode {
        Mode::Normal => {
            "j/k move  Tab pane  Enter open  a add  c comment  x tick  d done  u unblock  X cancel  r refresh  ? help  q quit"
        }
        Mode::Help => "any key closes help",
        Mode::Add { .. } => {
            "Enter next/newline  Enter on a blank line or Ctrl-S saves  Tab field  Esc cancel"
        }
        Mode::Comment(_) => "Enter send  Esc cancel",
        Mode::Confirm { .. } => "y confirm  n cancel",
    };
    let mut status = vec![
        Span::styled(format!(" {}", app.server), th.fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(format!("as {}", app.me), th.fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(ago(app.last_refresh), th.fg(Color::DarkGray)),
    ];
    if let Some(n) = &app.notice {
        status.push(Span::styled(format!("  {n}"), th.fg(Color::Green)));
    }
    if let Some(e) = &app.error {
        status.push(Span::styled(
            format!("  error: {e}"),
            th.fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(" {hints}"),
                Style::new().add_modifier(Modifier::DIM),
            )),
            Line::from(status),
        ]),
        area,
    );
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn draw_popup(f: &mut Frame, th: &Theme, title: &str, lines: Vec<Line>, w: u16, h: u16) {
    let area = centered(f.area(), w, h);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(pane_block(th, title.to_string(), true)),
        area,
    );
}

fn draw_help(f: &mut Frame, th: &Theme) {
    let rows = [
        ("j / k, arrows", "move in the focused pane"),
        ("Tab / Shift-Tab, h / l", "switch pane"),
        (
            "Enter",
            "open (projects > tasks > detail); in detail: tick item",
        ),
        ("x or Space", "tick / untick the highlighted checklist item"),
        ("a", "add a task (title, then items one per line)"),
        ("c", "comment on the task"),
        ("d", "mark done (claims, then completes)"),
        ("u", "unblock"),
        ("X", "cancel the task (asks first)"),
        ("r", "refresh now"),
        ("q / Ctrl-C", "quit"),
    ];
    let lines = rows
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!("{k:<26}"), th.fg(Color::Cyan)),
                Span::raw(d.to_string()),
            ])
        })
        .collect();
    draw_popup(f, th, "Help", lines, 84, rows.len() as u16 + 2);
}

fn draw_add(f: &mut Frame, th: &Theme, title: &str, items: &str, field: AddField) {
    let cursor = |on: bool| if on { "█" } else { "" };
    let mut lines = vec![
        Line::from(Span::styled("Title", th.focus(field == AddField::Title))),
        Line::from(format!("{title}{}", cursor(field == AddField::Title))),
        Line::raw(""),
        Line::from(Span::styled(
            "Checklist items (one per line)",
            th.focus(field == AddField::Items),
        )),
    ];
    let mut it: Vec<String> = items.split('\n').map(String::from).collect();
    if let (Some(l), true) = (it.last_mut(), field == AddField::Items) {
        l.push_str(cursor(true));
    }
    lines.extend(it.into_iter().map(Line::from));
    draw_popup(f, th, "Add task", lines, 70, 16);
}

fn draw_comment(f: &mut Frame, th: &Theme, text: &str, id: String) {
    draw_popup(
        f,
        th,
        &format!("Comment on {id}"),
        vec![Line::from(format!("{text}█"))],
        70,
        5,
    );
}

// ---------------------------------------------------------------------------------------------
// Async shell
// ---------------------------------------------------------------------------------------------

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

fn dispatch(api: &Api, app: &App, tx: &UnboundedSender<Msg>, action: Action) {
    let api = api.clone();
    let tx = tx.clone();
    let is_unblock = matches!(action, Action::Unblock(_));
    match action {
        Action::Quit => {}
        Action::Refresh => {
            let (pk, tid) = (app.project_key(), app.task_id());
            tokio::spawn(async move {
                let _ = tx.send(Msg::Projects(api.get("/projects").await.map_err(err)));
                if let Some(k) = pk {
                    let r = api.get(&format!("/projects/{k}/tasks")).await.map_err(err);
                    let _ = tx.send(Msg::Tasks(k, r));
                }
                if let Some(id) = tid {
                    let r = api.get(&format!("/tasks/{id}")).await.map_err(err);
                    let _ = tx.send(Msg::Detail(id, r));
                }
            });
        }
        Action::LoadTasks(k) => {
            tokio::spawn(async move {
                let r = api.get(&format!("/projects/{k}/tasks")).await.map_err(err);
                let _ = tx.send(Msg::Tasks(k, r));
            });
        }
        Action::LoadDetail(id) => {
            tokio::spawn(async move {
                let r = api.get(&format!("/tasks/{id}")).await.map_err(err);
                let _ = tx.send(Msg::Detail(id, r));
            });
        }
        Action::Check { id, pos, done } => {
            tokio::spawn(async move {
                let r = api
                    .post(
                        &format!("/tasks/{id}/check"),
                        json!({"position": pos, "done": done}),
                    )
                    .await
                    .map(|_| ())
                    .map_err(err);
                let label = format!(
                    "{id} item {pos} {}",
                    if done { "checked" } else { "unchecked" }
                );
                let _ = tx.send(Msg::Acted(label, r));
            });
        }
        Action::AddTask {
            project,
            title,
            items,
        } => {
            tokio::spawn(async move {
                let r = api
                    .post(
                        &format!("/projects/{project}/tasks"),
                        json!({"title": title, "items": items}),
                    )
                    .await;
                let (label, r) = match r {
                    Ok(v) => (
                        format!("created {}", v["id"].as_str().unwrap_or("task")),
                        Ok(()),
                    ),
                    Err(e) => ("add task".to_string(), Err(err(e))),
                };
                let _ = tx.send(Msg::Acted(label, r));
            });
        }
        Action::Comment { id, body } => {
            tokio::spawn(async move {
                let r = api
                    .post(&format!("/tasks/{id}/comment"), json!({"body": body}))
                    .await
                    .map(|_| ())
                    .map_err(err);
                let _ = tx.send(Msg::Acted(format!("commented on {id}"), r));
            });
        }
        Action::Done(id) => {
            tokio::spawn(async move {
                let r = async {
                    let c = api.post(&format!("/tasks/{id}/claim"), json!({})).await?;
                    let epoch = c["lease"]["epoch"].as_i64().unwrap_or(0);
                    api.post(
                        &format!("/tasks/{id}/complete"),
                        json!({"epoch": epoch, "result": ""}),
                    )
                    .await?;
                    anyhow::Ok(())
                }
                .await
                .map_err(err);
                let _ = tx.send(Msg::Acted(format!("{id} done"), r));
            });
        }
        Action::Unblock(id) | Action::Cancel(id) => {
            let verb = if is_unblock { "unblock" } else { "cancel" };
            tokio::spawn(async move {
                let r = api
                    .post(&format!("/tasks/{id}/{verb}"), json!({}))
                    .await
                    .map(|_| ())
                    .map_err(err);
                let past = if verb == "unblock" {
                    "unblocked"
                } else {
                    "cancelled"
                };
                let _ = tx.send(Msg::Acted(format!("{id} {past}"), r));
            });
        }
    }
}

pub async fn run(api: Api) -> Result<()> {
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        bail!("`tui` needs an interactive terminal (use `task ls --json` in scripts)");
    }
    let mut app = App::new(&api.server);
    let (tx, mut rx) = unbounded_channel::<Msg>();

    // Keys come from a blocking thread; it stops when `stop` is set.
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let (tx, stop) = (tx.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                    match event::read() {
                        Ok(Event::Key(k)) => {
                            if tx.send(Msg::Key(k)).is_err() {
                                return;
                            }
                        }
                        Ok(Event::Resize(..)) => {
                            if tx.send(Msg::Tick).is_err() {
                                return;
                            }
                        }
                        Ok(_) => {}
                        Err(_) => return,
                    }
                }
            }
        })
    };

    let mut terminal = ratatui::init(); // also installs a panic hook that restores the terminal
    let result = event_loop(&api, &mut app, &mut terminal, &tx, &mut rx).await;
    stop.store(true, Ordering::Relaxed);
    ratatui::restore();
    let _ = reader.join();
    result
}

async fn event_loop(
    api: &Api,
    app: &mut App,
    terminal: &mut ratatui::DefaultTerminal,
    tx: &UnboundedSender<Msg>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Msg>,
) -> Result<()> {
    {
        let (api, tx) = (api.clone(), tx.clone());
        tokio::spawn(async move {
            let r = api
                .get("/me")
                .await
                .map(|v| v["name"].as_str().unwrap_or("?").to_string());
            let _ = tx.send(Msg::Me(r.map_err(err)));
        });
    }
    dispatch(api, app, tx, Action::Refresh);
    let mut tick = tokio::time::interval(POLL);
    tick.tick().await; // first tick is immediate; Refresh already sent
    loop {
        terminal.draw(|f| draw(f, app))?;
        let msg = tokio::select! {
            m = rx.recv() => m,
            _ = tick.tick() => Some(Msg::Tick),
        };
        let Some(msg) = msg else { break };
        // Drain whatever else is queued so a burst of keys costs one redraw.
        let mut batch = vec![msg];
        while let Ok(m) = rx.try_recv() {
            batch.push(m);
        }
        for m in batch {
            for a in app.update(m) {
                dispatch(api, app, tx, a);
            }
        }
        if app.quit {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    fn key(c: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
    }

    fn render(app: &App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, app)).unwrap();
        buf_text(t.backend().buffer())
    }

    fn buf_text(b: &Buffer) -> String {
        (0..b.area.height)
            .map(|y| {
                (0..b.area.width)
                    .map(|x| b[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn fixture() -> App {
        let mut app = App::new("http://localhost:1");
        app.me = "octo".into();
        app.update(Msg::Projects(Ok(json!([
            {"key":"PAY","name":"Payments","ready":1,"claimed":1,"done":0},
            {"key":"WEB","name":"Website","ready":0,"claimed":0,"done":3}
        ]))));
        app.update(Msg::Tasks(
            "PAY".into(),
            Ok(json!([
                {"id":"PAY-1","title":"Open one","status":"open","ready":true,"total_items":2,"open_items":2,"claimed_by":null},
                {"id":"PAY-2","title":"Busy one","status":"claimed","total_items":2,"open_items":1,"claimed_by":"bot"}
            ])),
        ));
        app
    }

    fn with_detail(app: &mut App, id: &str) {
        app.update(Msg::Detail(
            id.into(),
            Ok(json!({"id":id,"title":"Busy one","status":"claimed","priority":2,"claimed_by":"bot",
                "body":"Do the thing","items":[
                    {"position":1,"text":"first step","done":true},
                    {"position":2,"text":"second step","done":false}],
                "comments":[{"author":"ann","body":"looks good"}],
                "dependencies":[{"id":"PAY-0","status":"done","title":"base"}]})),
        ));
    }

    #[test]
    fn groups_tasks_by_status_and_shows_progress_and_holder() {
        let app = fixture();
        // claimed sorts before open
        assert_eq!(app.tasks[0]["id"], "PAY-2");
        let s = render(&app, 120, 24);
        assert!(
            s.contains("Projects") && s.contains("PAY") && s.contains("Payments"),
            "{s}"
        );
        assert!(s.contains("claimed (1)") && s.contains("open (1)"), "{s}");
        assert!(
            s.contains("PAY-2") && s.contains("[1/2]") && s.contains("@bot"),
            "{s}"
        );
        assert!(
            s.contains("as octo") && s.contains("not refreshed yet") || s.contains("refreshed"),
            "{s}"
        );
    }

    #[test]
    fn detail_pane_shows_body_items_comments_and_deps() {
        let mut app = fixture();
        with_detail(&mut app, "PAY-2");
        let s = render(&app, 140, 30);
        assert!(s.contains("Do the thing"), "{s}");
        assert!(
            s.contains("[x] first step") && s.contains("[ ] second step"),
            "{s}"
        );
        assert!(s.contains("ann: looks good") && s.contains("PAY-0"), "{s}");
    }

    #[test]
    fn navigation_loads_detail_and_x_ticks_first_open_item() {
        let mut app = fixture();
        app.focus = Pane::Tasks;
        let acts = app.update(key(KeyCode::Char('j')));
        assert_eq!(acts, vec![Action::LoadDetail("PAY-1".into())]);
        let acts = app.update(key(KeyCode::Char('k')));
        assert_eq!(acts, vec![Action::LoadDetail("PAY-2".into())]);
        with_detail(&mut app, "PAY-2");
        // cursor starts on the first unchecked item (position 2)
        let acts = app.update(key(KeyCode::Char('x')));
        assert_eq!(
            acts,
            vec![Action::Check {
                id: "PAY-2".into(),
                pos: 2,
                done: true
            }]
        );
    }

    #[test]
    fn tab_cycles_panes_and_enter_opens() {
        let mut app = fixture();
        assert_eq!(app.focus, Pane::Projects);
        app.update(key(KeyCode::Tab));
        assert_eq!(app.focus, Pane::Tasks);
        app.update(key(KeyCode::Enter));
        assert_eq!(app.focus, Pane::Detail);
        app.update(key(KeyCode::BackTab));
        assert_eq!(app.focus, Pane::Tasks);
        // moving the project selection loads its tasks
        app.focus = Pane::Projects;
        let acts = app.update(key(KeyCode::Down));
        assert_eq!(acts, vec![Action::LoadTasks("WEB".into())]);
    }

    #[test]
    fn add_task_modal_collects_title_and_items() {
        let mut app = fixture();
        app.update(key(KeyCode::Char('a')));
        for c in "Ship it".chars() {
            app.update(key(KeyCode::Char(c)));
        }
        app.update(key(KeyCode::Enter)); // to items
        for c in "one".chars() {
            app.update(key(KeyCode::Char(c)));
        }
        app.update(key(KeyCode::Enter));
        for c in "two".chars() {
            app.update(key(KeyCode::Char(c)));
        }
        let s = render(&app, 120, 30);
        assert!(
            s.contains("Add task") && s.contains("Ship it") && s.contains("two"),
            "{s}"
        );
        app.update(key(KeyCode::Enter));
        let acts = app.update(key(KeyCode::Enter)); // blank line saves
        assert_eq!(
            acts,
            vec![Action::AddTask {
                project: "PAY".into(),
                title: "Ship it".into(),
                items: vec!["one".into(), "two".into()]
            }]
        );
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn typing_q_in_a_modal_does_not_quit_and_esc_cancels() {
        let mut app = fixture();
        app.update(key(KeyCode::Char('c')));
        app.update(key(KeyCode::Char('q')));
        assert!(!app.quit);
        assert_eq!(app.mode, Mode::Comment("q".into()));
        app.update(key(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn cancel_asks_for_confirmation() {
        let mut app = fixture();
        assert!(app.update(key(KeyCode::Char('X'))).is_empty());
        let s = render(&app, 100, 24);
        assert!(s.contains("Cancel PAY-2? (y/n)"), "{s}");
        assert_eq!(
            app.update(key(KeyCode::Char('y'))),
            vec![Action::Cancel("PAY-2".into())]
        );
    }

    #[test]
    fn errors_show_in_footer_and_clear_on_next_success() {
        let mut app = fixture();
        app.update(Msg::Projects(Err("connection refused".into())));
        assert!(render(&app, 120, 24).contains("error: connection refused"));
        app.update(Msg::Projects(Ok(json!([{"key":"PAY","name":"Payments"}]))));
        assert!(!render(&app, 120, 24).contains("error:"));
    }

    #[test]
    fn help_overlay_lists_keys() {
        let mut app = fixture();
        app.update(key(KeyCode::Char('?')));
        let s = render(&app, 120, 30);
        assert!(s.contains("Help") && s.contains("tick / untick"), "{s}");
        app.update(key(KeyCode::Char('z')));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let mut app = fixture();
        with_detail(&mut app, "PAY-2");
        app.update(key(KeyCode::Char('?')));
        let _ = render(&app, 10, 4);
        app.mode = Mode::Add {
            title: "x".into(),
            items: String::new(),
            field: AddField::Title,
        };
        let _ = render(&app, 30, 8);
    }
}
