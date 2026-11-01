//! Full-screen dashboard (`ngx` with no arguments).

mod form;
mod logview;

use crate::cmd;
use crate::env::Env;
use crate::render::{self, Tone};
use crate::status::{Report, State};
use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::io::Write;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

const REFRESH: Duration = Duration::from_secs(5);

pub(crate) fn style(t: Tone) -> Style {
    match t {
        Tone::Plain => Style::default(),
        Tone::Dim => Style::default().fg(Color::DarkGray),
        Tone::Good => Style::default().fg(Color::Green),
        Tone::Warn => Style::default().fg(Color::Yellow),
        Tone::Bad => Style::default().fg(Color::Red),
        Tone::Bold => Style::default().add_modifier(Modifier::BOLD),
        Tone::Accent => Style::default().fg(Color::Cyan),
    }
}

fn to_line(l: &render::Line) -> Line<'static> {
    Line::from(l.iter().map(|s| Span::styled(s.0.clone(), style(s.1))).collect::<Vec<_>>())
}

struct App {
    report: Option<Report>,
    table: TableState,
    filter: String,
    filtering: bool,
    zoom: bool,
    detail: Option<(String, Vec<render::Line>)>,
    detail_pending: Option<String>,
    scroll: u16,
    message: Option<(String, Tone)>,
    refresh_tx: Sender<()>,
    report_rx: Receiver<Report>,
    detail_tx: Sender<(String, Vec<render::Line>)>,
    detail_rx: Receiver<(String, Vec<render::Line>)>,
    form: Option<form::Form>,
    logs: Option<logview::LogView>,
}

enum Action {
    Add,
    Edit,
    Toggle,
    Remove,
    Renew,
    Undo,
    Issue,
    Logs(bool),
    Test,
    Reload,
    /// The dashboard form was submitted: add (no site) or edit a site.
    Save(Option<String>, Box<cmd::AddArgs>),
}

impl Action {
    fn label(&self, id: &str) -> String {
        match self {
            Action::Add => "add".into(),
            Action::Edit => format!("edit #{id}"),
            Action::Toggle => format!("enable/disable #{id}"),
            Action::Remove => format!("remove #{id}"),
            Action::Renew => format!("renew #{id}"),
            Action::Undo => "undo".into(),
            Action::Issue => format!("issue certificate #{id}"),
            Action::Logs(_) => format!("logs #{id}"),
            Action::Test => "test".into(),
            Action::Reload => "reload".into(),
            Action::Save(None, _) => "add".into(),
            Action::Save(Some(id), _) => format!("edit #{id}"),
        }
    }
}

impl App {
    fn visible(&self) -> Vec<usize> {
        let Some(r) = &self.report else { return vec![] };
        let f = self.filter.to_lowercase();
        (0..r.sites.len())
            .filter(|&i| f.is_empty() || r.sites[i].all_names().any(|n| n.contains(&f)) || r.sites[i].id.to_string() == f)
            .collect()
    }

    fn selected(&self) -> Option<usize> {
        let v = self.visible();
        self.table.selected().and_then(|i| v.get(i).copied())
    }

    fn selected_key(&self) -> Option<String> {
        let r = self.report.as_ref()?;
        self.selected().map(|i| r.sites[i].key.clone())
    }

    fn request_detail(&mut self) {
        let (Some(r), Some(i)) = (&self.report, self.selected()) else { return };
        let key = r.sites[i].key.clone();
        if self.detail.as_ref().is_some_and(|d| d.0 == key) || self.detail_pending.as_deref() == Some(&key) {
            return;
        }
        self.detail_pending = Some(key.clone());
        let (site, st, tx) = (r.sites[i].clone(), r.status[i].clone(), self.detail_tx.clone());
        std::thread::spawn(move || {
            let _ = tx.send((key, render::detail(&site, &st)));
        });
    }

    fn set_report(&mut self, rep: Report) {
        let keep = self.selected_key();
        self.report = Some(rep);
        let v = self.visible();
        let r = self.report.as_ref().unwrap();
        let pos = keep.and_then(|k| v.iter().position(|&i| r.sites[i].key == k));
        self.table.select(if v.is_empty() { None } else { Some(pos.unwrap_or(0)) });
        // re-probe the selected site's details with the new status
        self.detail_pending = None;
        if let Some(d) = &mut self.detail {
            d.0.push('\u{0}'); // mark stale, keep showing until replaced
        }
        self.request_detail();
    }

    fn move_by(&mut self, delta: i64) {
        let n = self.visible().len() as i64;
        if n == 0 {
            return;
        }
        let cur = self.table.selected().unwrap_or(0) as i64;
        self.table.select(Some((cur + delta).rem_euclid(n) as usize));
        self.scroll = 0;
        self.request_detail();
    }
}

pub fn run() -> Result<()> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        anyhow::bail!("the dashboard needs a terminal — try `ngx ls`");
    }
    let (refresh_tx, refresh_rx) = mpsc::channel::<()>();
    let (report_tx, report_rx) = mpsc::channel();
    std::thread::spawn(move || loop {
        if report_tx.send(Report::load(true)).is_err() {
            return;
        }
        match refresh_rx.recv_timeout(REFRESH) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {
                while refresh_rx.try_recv().is_ok() {}
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    });
    let (detail_tx, detail_rx) = mpsc::channel();
    let mut app = App {
        report: None,
        table: TableState::default(),
        filter: String::new(),
        filtering: false,
        zoom: false,
        detail: None,
        detail_pending: None,
        scroll: 0,
        message: None,
        refresh_tx,
        report_rx,
        detail_tx,
        detail_rx,
        form: None,
        logs: None,
    };
    let mut term = ratatui::init();
    let res = main_loop(&mut term, &mut app);
    ratatui::restore();
    res
}

fn main_loop(term: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        while let Ok(rep) = app.report_rx.try_recv() {
            app.set_report(rep);
        }
        while let Ok((key, lines)) = app.detail_rx.try_recv() {
            if app.detail_pending.as_deref() == Some(&key) {
                app.detail_pending = None;
            }
            if app.selected_key().as_deref() == Some(&key) {
                app.detail = Some((key, lines));
            }
        }
        if let Some(l) = &mut app.logs {
            l.poll();
        }
        term.draw(|f| draw(f, app))?;
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(k) = event::read()? else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        if let Some(fm) = &mut app.form {
            match fm.key(k) {
                form::Outcome::Stay => {}
                form::Outcome::Cancel => app.form = None,
                form::Outcome::Submit(site, args) => {
                    run_action(term, app, Action::Save(site, args))?;
                    // back to the form if it was cancelled or failed, so nothing typed is lost
                    if matches!(app.message, Some((_, Tone::Good))) {
                        app.form = None;
                    }
                }
            }
            continue;
        }
        if let Some(l) = &mut app.logs {
            let page = term.size().map(|s| s.height.saturating_sub(4) as usize).unwrap_or(20);
            if !l.key(k, page) {
                app.logs = None;
            }
            continue;
        }
        if app.filtering {
            match k.code {
                KeyCode::Esc => {
                    app.filtering = false;
                    app.filter.clear();
                }
                KeyCode::Enter => app.filtering = false,
                KeyCode::Backspace => {
                    app.filter.pop();
                }
                KeyCode::Char(c) => app.filter.push(c),
                _ => {}
            }
            app.table.select(if app.visible().is_empty() { None } else { Some(0) });
            app.request_detail();
            continue;
        }
        if let Some(action) = key_action(app, k)? {
            if matches!(action, Flow::Quit) {
                return Ok(());
            }
            if let Flow::Run(a) = action {
                run_action(term, app, a)?;
            }
        }
    }
}

enum Flow {
    Quit,
    Run(Action),
    Handled,
}

fn key_action(app: &mut App, k: KeyEvent) -> Result<Option<Flow>> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    Ok(Some(match k.code {
        KeyCode::Char('q') => Flow::Quit,
        KeyCode::Char('c') if ctrl => Flow::Quit,
        KeyCode::Esc if app.zoom => {
            app.zoom = false;
            Flow::Handled
        }
        KeyCode::Esc if !app.filter.is_empty() => {
            app.filter.clear();
            app.move_by(0);
            Flow::Handled
        }
        KeyCode::Esc => Flow::Quit,
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_by(1);
            Flow::Handled
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_by(-1);
            Flow::Handled
        }
        KeyCode::Home => {
            app.table.select(Some(0));
            app.move_by(0);
            Flow::Handled
        }
        KeyCode::End => {
            let n = app.visible().len();
            app.table.select(n.checked_sub(1));
            app.move_by(0);
            Flow::Handled
        }
        KeyCode::PageDown | KeyCode::Char('J') => {
            app.scroll = app.scroll.saturating_add(5);
            Flow::Handled
        }
        KeyCode::PageUp | KeyCode::Char('K') => {
            app.scroll = app.scroll.saturating_sub(5);
            Flow::Handled
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            app.zoom = !app.zoom;
            app.scroll = 0;
            Flow::Handled
        }
        KeyCode::Char('/') => {
            app.filtering = true;
            Flow::Handled
        }
        KeyCode::Char('g') => {
            app.message = Some(("refreshing…".into(), Tone::Dim));
            let _ = app.refresh_tx.send(());
            Flow::Handled
        }
        KeyCode::Char('a') => Flow::Run(Action::Add),
        KeyCode::Char('e') => Flow::Run(Action::Edit),
        KeyCode::Char('d') => Flow::Run(Action::Toggle),
        KeyCode::Char('x') | KeyCode::Delete => Flow::Run(Action::Remove),
        KeyCode::Char('r') => Flow::Run(Action::Renew),
        KeyCode::Char('u') => Flow::Run(Action::Undo),
        KeyCode::Char('i') => Flow::Run(Action::Issue),
        KeyCode::Char('l') => Flow::Run(Action::Logs(false)),
        KeyCode::Char('L') => Flow::Run(Action::Logs(true)),
        KeyCode::Char('t') => Flow::Run(Action::Test),
        KeyCode::Char('R') => Flow::Run(Action::Reload),
        _ => return Ok(None),
    }))
}

extern "C" fn ignore_sigint(_: libc::c_int) {}

/// Leave the alternate screen, run a CLI command, wait for Enter, come back.
fn run_action(term: &mut DefaultTerminal, app: &mut App, a: Action) -> Result<()> {
    let site = app.selected().and_then(|i| app.report.as_ref().map(|r| r.sites[i].id.to_string()));
    let needs_site = !matches!(a, Action::Add | Action::Test | Action::Reload | Action::Undo | Action::Save(..));
    let Some(id) = site.clone().or_else(|| (!needs_site).then(String::new)) else {
        app.message = Some(("no site selected".into(), Tone::Warn));
        return Ok(());
    };
    // these stay inside the dashboard
    match a {
        Action::Add => {
            app.form = Some(form::Form::new(None));
            return Ok(());
        }
        Action::Edit => {
            match cmd::current_spec(&id) {
                Ok(spec) => app.form = Some(form::Form::new(Some((id, spec)))),
                Err(e) => app.message = Some((e.to_string().lines().next().unwrap_or("").to_string(), Tone::Bad)),
            }
            return Ok(());
        }
        Action::Logs(err) => {
            if let Some(s) = app.selected().and_then(|i| app.report.as_ref().map(|r| r.sites[i].clone())) {
                app.logs = Some(logview::LogView::open(s, err));
            }
            return Ok(());
        }
        _ => {}
    }
    ratatui::restore();
    println!();
    // let Ctrl-C stop child processes (tail -F, editors, certbot) without killing the dashboard
    let prev = unsafe { libc::signal(libc::SIGINT, ignore_sigint as *const () as libc::sighandler_t) };
    crate::ops::new_change(&format!("dashboard: {}", a.label(&id)));
    let res = match a {
        Action::Undo => cmd::undo(None, false, false),
        Action::Add | Action::Edit | Action::Logs(_) => unreachable!("handled in the dashboard"),
        Action::Save(None, args) => cmd::add(*args),
        Action::Save(Some(sid), args) => cmd::edit(&sid, *args, false),
        Action::Toggle => {
            let enabled = app.selected().and_then(|i| app.report.as_ref().map(|r| r.status[i].state != State::Disabled)).unwrap_or(true);
            cmd::set_enabled(&id, !enabled, false)
        }
        Action::Remove => cmd::rm(&id, false, false, false),
        Action::Renew => cmd::cert(cmd::CertCmd::Renew { site: id.clone(), force: false }),
        Action::Issue => cmd::cert(cmd::CertCmd::Issue { site: id.clone(), email: None, staging: false, dns: None, dns_credentials: None }),
        Action::Test => cmd::test(),
        Action::Reload => cmd::reload(),
    };
    let msg = match &res {
        Ok(()) => ("done".to_string(), Tone::Good),
        Err(e) => {
            let cancelled = matches!(
                e.downcast_ref::<inquire::InquireError>(),
                Some(inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted)
            );
            if cancelled {
                ("cancelled".to_string(), Tone::Dim)
            } else {
                eprintln!("\nerror: {e:#}");
                (format!("error: {}", e.to_string().lines().next().unwrap_or("")), Tone::Bad)
            }
        }
    };
    if !matches!(msg.1, Tone::Dim) {
        print!("\npress Enter to return to the dashboard… ");
        let _ = std::io::stdout().flush();
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
    }
    unsafe { libc::signal(libc::SIGINT, prev) };
    *term = ratatui::init();
    term.clear()?;
    app.message = Some(msg);
    app.detail = None;
    app.detail_pending = None;
    let _ = app.refresh_tx.send(());
    Ok(())
}

fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let [top, body, foot] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)]).areas(area);

    // title bar
    let env = Env::get();
    let mut title = vec![Span::styled(" ngx ", Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)), Span::raw(" ")];
    if let Some(r) = &app.report {
        let idx: Vec<usize> = (0..r.sites.len()).collect();
        title.extend(to_line(&render::summary(r, &idx)).spans);
        if !r.global.is_empty() {
            title.push(Span::styled(format!("  ·  nginx: {}", crate::validate::clean(&r.global[0])), style(Tone::Bad)));
        }
    } else {
        title.push(Span::styled("loading…", style(Tone::Dim)));
    }
    if env.sandboxed() {
        title.push(Span::styled("  [sandbox]", style(Tone::Warn)));
    }
    f.render_widget(Paragraph::new(Line::from(title)), top);

    if let Some(fm) = &app.form {
        fm.draw(f, body);
    } else if let Some(l) = &app.logs {
        l.draw(f, body);
    } else if app.zoom {
        draw_detail(f, app, body);
    } else {
        let rows = app.visible().len() as u16;
        let list_h = (rows + 3).clamp(5, body.height / 2 + 2);
        let [list, detail] = Layout::vertical([Constraint::Length(list_h), Constraint::Fill(1)]).areas(body);
        draw_table(f, app, list);
        draw_detail(f, app, detail);
    }

    // footer
    let footer: Line = if app.form.is_some() {
        Line::from(Span::styled(" changes are shown for review before anything is written", style(Tone::Dim)))
    } else if app.logs.is_some() {
        let keys = [("↑↓ PgUp PgDn", "scroll"), ("End", "follow"), ("Home", "oldest"), ("e", "access/error"), ("Esc", "close")];
        let mut spans = Vec::new();
        for (k, d) in keys {
            spans.push(Span::styled(format!(" {k}"), style(Tone::Accent)));
            spans.push(Span::styled(format!(" {d} "), style(Tone::Dim)));
        }
        Line::from(spans)
    } else if app.filtering {
        Line::from(vec![Span::styled(" filter: ", style(Tone::Accent)), Span::raw(crate::validate::clean(&app.filter).into_owned()), Span::styled("▏", style(Tone::Accent))])
    } else {
        let keys = [("↑↓", "select"), ("⏎", "zoom"), ("/", "filter"), ("a", "add"), ("e", "edit"), ("d", "on/off"), ("x", "remove"), ("i", "issue cert"), ("r", "renew"), ("l/L", "logs"), ("t", "test"), ("R", "reload"), ("u", "undo"), ("g", "refresh"), ("q", "quit")];
        let mut spans = Vec::new();
        if let Some((m, t)) = &app.message {
            spans.push(Span::styled(format!(" {} ", crate::validate::clean(m)), style(*t)));
            spans.push(Span::raw(" "));
        }
        for (k, d) in keys {
            spans.push(Span::styled(format!(" {k}"), style(Tone::Accent)));
            spans.push(Span::styled(format!(" {d} "), style(Tone::Dim)));
        }
        Line::from(spans)
    };
    f.render_widget(Paragraph::new(footer), foot);
}

fn draw_table(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(r) = &app.report else {
        f.render_widget(Paragraph::new("  scanning nginx configuration…").block(Block::bordered()), area);
        return;
    };
    let vis = app.visible();
    let cells: Vec<Vec<render::Line>> = vis.iter().map(|&i| render::row(&r.sites[i], &r.status[i])).collect();
    let mut widths: Vec<u16> = render::HEADERS.iter().map(|h| h.width() as u16).collect();
    for row in &cells {
        for (i, c) in row.iter().enumerate() {
            widths[i] = widths[i].max(render::width(c) as u16);
        }
    }
    let constraints: Vec<Constraint> = widths
        .iter()
        .enumerate()
        .map(|(i, w)| if i + 1 == widths.len() { Constraint::Fill(1) } else { Constraint::Length(*w) })
        .collect();
    let header = Row::new(render::HEADERS.iter().map(|h| Span::styled(*h, style(Tone::Bold)))).height(1);
    let rows: Vec<Row> = cells.iter().map(|c| Row::new(c.iter().map(to_line))).collect();
    let title = if app.filter.is_empty() { " sites ".to_string() } else { format!(" sites matching \"{}\" ", app.filter) };
    let table = Table::new(rows, constraints)
        .header(header)
        .column_spacing(2)
        .block(Block::bordered().title(title))
        .row_highlight_style(Style::default().bg(Color::Rgb(40, 44, 52)).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(table, area, &mut app.table);
}

fn draw_detail(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" details ");
    let lines: Vec<Line> = match (&app.detail, app.selected_key()) {
        (Some((k, lines)), Some(sel)) if k.trim_end_matches('\u{0}') == sel => lines.iter().map(to_line).collect(),
        (_, None) => vec![Line::styled("  no sites — press a to add one", style(Tone::Dim))],
        _ => vec![Line::styled("  probing…", style(Tone::Dim))],
    };
    let max = (lines.len() as u16).saturating_sub(area.height.saturating_sub(2));
    app.scroll = app.scroll.min(max);
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }).scroll((app.scroll, 0)), area);
}
