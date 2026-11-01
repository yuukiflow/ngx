//! Log viewer inside the dashboard: the last lines of a site's access or
//! error log, followed live, with status codes and levels colored.

use super::style;
use crate::env::Env;
use crate::nginx::site::Site;
use crate::render::Tone;
use crate::validate::clean;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

const KEEP: usize = 2000;
const INITIAL: u64 = 256 * 1024;

pub struct LogView {
    site: Site,
    pub error: bool,
    path: PathBuf,
    shared: bool,
    lines: Vec<String>,
    partial: String,
    pos: u64,
    /// Lines scrolled up from the bottom; 0 = following.
    back: usize,
    problem: Option<String>,
}

impl LogView {
    pub fn open(site: Site, error: bool) -> LogView {
        let (path, shared) = crate::cmd::log_path(&site, error);
        let mut v = LogView { site, error, path, shared, lines: Vec::new(), partial: String::new(), pos: 0, back: 0, problem: None };
        v.reload();
        v
    }

    fn reload(&mut self) {
        self.lines.clear();
        self.partial.clear();
        self.pos = 0;
        let host = Env::get().host(&self.path);
        match std::fs::metadata(&host) {
            Ok(m) => self.pos = m.len().saturating_sub(INITIAL),
            Err(e) => {
                self.problem = Some(format!("{}: {e}", self.path.display()));
                return;
            }
        }
        self.problem = None;
        self.poll();
        if self.pos > 0 && !self.lines.is_empty() {
            self.lines.remove(0); // probably cut in the middle
        }
    }

    /// Read what was appended since last time (handles rotation/truncation).
    pub fn poll(&mut self) {
        let host = Env::get().host(&self.path);
        let Ok(mut f) = std::fs::File::open(&host) else { return };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.pos {
            self.pos = 0; // rotated or truncated
        }
        if len == self.pos || f.seek(SeekFrom::Start(self.pos)).is_err() {
            return;
        }
        let mut buf = Vec::new();
        if f.take(4 * 1024 * 1024).read_to_end(&mut buf).is_err() {
            return;
        }
        self.pos += buf.len() as u64;
        let text = String::from_utf8_lossy(&buf);
        let mut chunk = std::mem::take(&mut self.partial);
        chunk.push_str(&text);
        let mut parts: Vec<&str> = chunk.split('\n').collect();
        let last = parts.pop().unwrap_or("").to_string();
        let names: Vec<String> = self.site.all_names().map(String::from).collect();
        for l in parts {
            // the shared error log: only this site's lines
            if self.shared && self.error && !names.iter().any(|n| l.contains(n.as_str())) {
                continue;
            }
            self.lines.push(l.to_string());
            if self.back > 0 {
                self.back += 1; // keep the view still while scrolled up
            }
        }
        self.partial = last;
        if self.lines.len() > KEEP {
            let cut = self.lines.len() - KEEP;
            self.lines.drain(..cut);
        }
    }

    /// Returns false when the viewer should close.
    pub fn key(&mut self, k: KeyEvent, page: usize) -> bool {
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return false,
            KeyCode::Up | KeyCode::Char('k') => self.back = (self.back + 1).min(self.lines.len().saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.back = self.back.saturating_sub(1),
            KeyCode::PageUp => self.back = (self.back + page).min(self.lines.len().saturating_sub(1)),
            KeyCode::PageDown => self.back = self.back.saturating_sub(page),
            KeyCode::Home => self.back = self.lines.len().saturating_sub(1),
            KeyCode::End | KeyCode::Char('f') => self.back = 0,
            KeyCode::Char('e') | KeyCode::Tab => {
                *self = LogView::open(self.site.clone(), !self.error);
            }
            _ => {}
        }
        true
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let h = area.height.saturating_sub(2) as usize;
        let end = self.lines.len().saturating_sub(self.back);
        let start = end.saturating_sub(h);
        let mut out: Vec<Line> = self.lines[start..end].iter().map(|l| colored(l, self.error)).collect();
        if let Some(p) = &self.problem {
            out = vec![Line::from(Span::styled(clean(p).into_owned(), style(Tone::Bad)))];
        } else if out.is_empty() {
            out.push(Line::from(Span::styled("(no lines yet — waiting for requests)", style(Tone::Dim))));
        }
        let state = if self.back == 0 { "following".to_string() } else { format!("{} lines up — End to follow", self.back) };
        let title = format!(
            " #{} {} · {} log{} · {} · {} ",
            self.site.id,
            self.site.domain,
            if self.error { "error" } else { "access" },
            if self.shared { " (shared)" } else { "" },
            Env::get().short(&self.path),
            state
        );
        f.render_widget(Paragraph::new(out).block(Block::default().borders(Borders::TOP).title(clean(&title).into_owned())), area);
    }
}

fn colored(l: &str, error: bool) -> Line<'static> {
    let text = clean(l).into_owned();
    let tone = if error {
        if ["[emerg]", "[alert]", "[crit]", "[error]"].iter().any(|t| text.contains(t)) {
            Tone::Bad
        } else if text.contains("[warn]") {
            Tone::Warn
        } else {
            Tone::Plain
        }
    } else {
        // combined format: the status follows the quoted request line
        match text.split('"').nth(2).and_then(|r| r.split_whitespace().next()).and_then(|c| c.parse::<u16>().ok()) {
            Some(500..) => Tone::Bad,
            Some(400..=499) => Tone::Warn,
            Some(300..=399) => Tone::Dim,
            _ => Tone::Plain,
        }
    };
    Line::from(Span::styled(text, style(tone)))
}
