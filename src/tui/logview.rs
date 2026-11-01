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
    /// Case-insensitive text filter (`/`), and whether it's being typed.
    pub filter: String,
    pub typing: bool,
    /// Only 4xx/5xx (access log) or warnings and errors (error log).
    pub problems_only: bool,
}

impl LogView {
    pub fn open(site: Site, error: bool) -> LogView {
        let (path, shared) = crate::cmd::log_path(&site, error);
        let mut v = LogView { site, error, path, shared, lines: Vec::new(), partial: String::new(), pos: 0, back: 0, problem: None, filter: String::new(), typing: false, problems_only: false };
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
        let started_mid_file = self.pos > 0;
        self.poll();
        if started_mid_file && !self.lines.is_empty() {
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
            if self.back > 0 && self.shown(l) {
                self.back += 1; // keep the view still while scrolled up
            }
            self.lines.push(l.to_string());
        }
        self.partial = last;
        if self.lines.len() > KEEP {
            let cut = self.lines.len() - KEEP;
            self.lines.drain(..cut);
        }
    }

    fn shown(&self, l: &str) -> bool {
        (!self.problems_only || matches!(tone(l, self.error), Tone::Bad | Tone::Warn))
            && (self.filter.is_empty() || l.to_lowercase().contains(&self.filter.to_lowercase()))
    }

    fn visible(&self) -> Vec<&String> {
        self.lines.iter().filter(|l| self.shown(l)).collect()
    }

    /// Returns false when the viewer should close.
    pub fn key(&mut self, k: KeyEvent, page: usize) -> bool {
        if self.typing {
            match k.code {
                KeyCode::Esc => {
                    self.typing = false;
                    self.filter.clear();
                }
                KeyCode::Enter => self.typing = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) if !c.is_control() && self.filter.len() < 200 => self.filter.push(c),
                _ => {}
            }
            self.back = 0;
            return true;
        }
        let n = self.visible().len();
        match k.code {
            KeyCode::Esc if !self.filter.is_empty() || self.problems_only => {
                self.filter.clear();
                self.problems_only = false;
                self.back = 0;
            }
            KeyCode::Esc | KeyCode::Char('q') => return false,
            KeyCode::Char('/') => {
                self.typing = true;
                self.filter.clear();
            }
            KeyCode::Char('p') => {
                self.problems_only = !self.problems_only;
                self.back = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => self.back = (self.back + 1).min(n.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.back = self.back.saturating_sub(1),
            KeyCode::PageUp => self.back = (self.back + page).min(n.saturating_sub(1)),
            KeyCode::PageDown => self.back = self.back.saturating_sub(page),
            KeyCode::Home => self.back = n.saturating_sub(1),
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
        let vis = self.visible();
        let end = vis.len().saturating_sub(self.back);
        let start = end.saturating_sub(h);
        let mut out: Vec<Line> = vis[start..end].iter().map(|l| colored(l, self.error)).collect();
        if let Some(p) = &self.problem {
            out = vec![Line::from(Span::styled(clean(p).into_owned(), style(Tone::Bad)))];
        } else if out.is_empty() {
            let msg = if self.lines.is_empty() { "(no lines yet — waiting for requests)" } else { "(no line matches — Esc clears the filter)" };
            out.push(Line::from(Span::styled(msg, style(Tone::Dim))));
        }
        let mut state = if self.back == 0 { "following".to_string() } else { format!("{} lines up — End to follow", self.back) };
        if self.problems_only {
            state.push_str(if self.error { " · warnings and errors only" } else { " · 4xx/5xx only" });
        }
        if self.typing || !self.filter.is_empty() {
            state.push_str(&format!(" · search: {}{}", self.filter, if self.typing { "▏" } else { "" }));
        }
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
    Line::from(Span::styled(clean(l).into_owned(), style(tone(l, error))))
}

fn tone(text: &str, error: bool) -> Tone {
    if error {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }
    fn screen(v: &LogView) -> String {
        let mut t = Terminal::new(TestBackend::new(120, 12)).unwrap();
        t.draw(|f| v.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n").collect()
    }
    fn line(path: &str, status: u16) -> String {
        format!("1.2.3.4 - - [01/Nov/2026:10:00:00 +0100] \"GET {path} HTTP/1.1\" {status} 5 \"-\" \"curl\" rt=0.001\n")
    }

    #[test]
    fn follows_filters_and_survives_rotation() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("site.access.log");
        std::fs::write(&log, [line("/a", 200), line("/b", 502), line("/c", 404)].concat()).unwrap();
        let conf = dir.path().join("site.conf");
        std::fs::write(&conf, format!("server {{\n listen 80;\n server_name log.test;\n access_log {};\n root /srv;\n}}\n", log.display())).unwrap();
        let site = crate::nginx::site::parse_file(&conf, &conf, true, false).remove(0);
        let mut v = LogView::open(site, false);
        let s = screen(&v);
        assert!(s.contains("GET /a") && s.contains("GET /b") && s.contains("following"), "{s}");

        // new lines show up
        std::fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(line("/new", 200).as_bytes()).unwrap();
        v.poll();
        assert!(screen(&v).contains("GET /new"));

        // problems only: 4xx/5xx
        v.key(key(KeyCode::Char('p')), 5);
        let s = screen(&v);
        assert!(s.contains("GET /b") && s.contains("GET /c") && !s.contains("GET /a") && s.contains("4xx/5xx only"), "{s}");
        v.key(key(KeyCode::Esc), 5);

        // search
        v.key(key(KeyCode::Char('/')), 5);
        for c in "/C".chars() {
            v.key(key(KeyCode::Char(c)), 5);
        }
        v.key(key(KeyCode::Enter), 5);
        let s = screen(&v);
        assert!(s.contains("GET /c") && !s.contains("GET /b") && s.contains("search: /C"), "{s}");
        assert!(v.key(key(KeyCode::Esc), 5), "Esc first clears the search");
        assert!(!v.key(key(KeyCode::Esc), 5), "then closes");

        // rotation: the file starts over
        std::fs::write(&log, line("/after-rotation", 200)).unwrap();
        v.poll();
        assert!(screen(&v).contains("GET /after-rotation"));
    }

    #[test]
    fn escape_sequences_are_shown_not_sent() {
        let l = colored("GET /\u{1b}]0;pwned\u{7} HTTP/1.1", false);
        let text: String = l.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(!text.contains('\u{1b}') && text.contains("\\x{1b}"));
    }
}
