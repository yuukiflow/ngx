//! Output rendering shared by the CLI (ANSI) and the TUI (ratatui spans).

use crate::env::Env;
use crate::logs;
use crate::nginx::site::{Kind, Site, Upstream};
use crate::probe::{self, Listener, Manager};
use crate::ssl::{CertState, Renew};
use crate::status::{Report, State, Status};
use chrono::{DateTime, Local};
use std::io::IsTerminal;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone {
    Plain,
    Dim,
    Good,
    Warn,
    Bad,
    Bold,
    Accent,
}

#[derive(Debug, Clone)]
pub struct Seg(pub String, pub Tone);
pub type Line = Vec<Seg>;

pub fn seg(s: impl Into<String>, t: Tone) -> Seg {
    Seg(s.into(), t)
}
fn plain(s: impl Into<String>) -> Seg {
    Seg(s.into(), Tone::Plain)
}

pub fn width(l: &[Seg]) -> usize {
    l.iter().map(|s| s.0.width()).sum()
}

pub fn color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

pub fn ansi(lines: &[Line]) -> String {
    let color = color_enabled();
    let mut out = String::new();
    for l in lines {
        for Seg(s, t) in l {
            let code = match t {
                Tone::Plain => "",
                Tone::Dim => "2",
                Tone::Good => "32",
                Tone::Warn => "33",
                Tone::Bad => "31",
                Tone::Bold => "1",
                Tone::Accent => "36",
            };
            if color && !code.is_empty() {
                out.push_str(&format!("\x1b[{code}m{s}\x1b[0m"));
            } else {
                out.push_str(s);
            }
        }
        // trim trailing padding
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }
    out
}

pub fn print(lines: &[Line]) {
    print!("{}", ansi(lines));
}

pub fn date(t: DateTime<Local>) -> String {
    t.format("%d/%m/%y").to_string()
}
pub fn datetime(t: DateTime<Local>) -> String {
    t.format("%d/%m/%y %H:%M").to_string()
}

/// `1284` → `1 284`
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

// ───────────────────────── ls table ─────────────────────────

pub fn state_cell(st: State) -> Line {
    match st {
        State::Up => vec![seg("●", Tone::Good), plain(" up")],
        State::Down => vec![seg("○", Tone::Bad), seg(" down", Tone::Bad)],
        State::Disabled => vec![seg("◌ off", Tone::Dim)],
    }
}

pub fn ssl_cell(st: &Status) -> Line {
    match &st.cert {
        CertState::None => vec![seg("none", Tone::Dim)],
        CertState::Missing(_) => vec![seg("missing", Tone::Bad)],
        CertState::Unreadable(_) => vec![seg("unreadable", Tone::Dim)],
        CertState::Ok(i) => {
            if st.cert.expired() {
                vec![seg(format!("expired {}", date(i.not_after)), Tone::Bad)]
            } else if st.cert.expiring_soon() {
                vec![seg(format!("valid until {} ⚠", date(i.not_after)), Tone::Warn)]
            } else {
                vec![plain(format!("valid until {}", date(i.not_after)))]
            }
        }
    }
}

pub fn renew_cell(st: &Status) -> Line {
    match &st.renew {
        Renew::None => vec![seg("—", Tone::Dim)],
        Renew::Auto { .. } => vec![plain("auto")],
        Renew::Manual(_) => {
            let t = if st.cert.expired() || st.cert.expiring_soon() { Tone::Warn } else { Tone::Plain };
            vec![seg("manual", t)]
        }
    }
}

pub fn local_cell(s: &Site) -> String {
    match s.kind() {
        Kind::Proxy | Kind::FastCgi => s.upstream().map(|u| u.short()).unwrap_or_else(|| "—".into()),
        Kind::Static => "files".into(),
        Kind::Redirect => "→".into(),
    }
}

pub fn row(s: &Site, st: &Status) -> Vec<Line> {
    vec![
        vec![seg(s.id.to_string(), Tone::Dim)],
        state_cell(st.state),
        vec![seg(s.domain.clone(), if st.state == State::Disabled { Tone::Dim } else { Tone::Plain })],
        ssl_cell(st),
        renew_cell(st),
        vec![plain(s.public_port().map(|p| p.to_string()).unwrap_or_else(|| "—".into()))],
        vec![plain(local_cell(s))],
        vec![seg(Env::get().short(&s.config), Tone::Dim)],
        match &st.error {
            Some(e) => vec![seg(e.clone(), Tone::Bad)],
            None => vec![seg("—", Tone::Dim)],
        },
    ]
}

pub const HEADERS: [&str; 9] = ["ID", "STATUS", "DOMAIN", "SSL", "RENEW", "IN", "LOCAL", "CONFIG", "ERROR"];

pub fn table(rows: &[Vec<Line>]) -> Vec<Line> {
    let mut widths: Vec<usize> = HEADERS.iter().map(|h| h.width()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(width(c));
        }
    }
    let mut out = Vec::new();
    let mut line_of = |cells: Vec<Line>| {
        let mut l: Line = Vec::new();
        let n = cells.len();
        for (i, c) in cells.into_iter().enumerate() {
            let w = width(&c);
            l.extend(c);
            if i + 1 < n {
                l.push(plain(" ".repeat(widths[i] - w + 2)));
            }
        }
        out.push(l);
    };
    line_of(HEADERS.iter().map(|h| vec![seg(*h, Tone::Bold)]).collect());
    for r in rows {
        line_of(r.clone());
    }
    out
}

pub fn summary(rep: &Report, idx: &[usize]) -> Line {
    let n = idx.len();
    let mut up = 0;
    let mut down = 0;
    let mut off = 0;
    let mut expired = 0;
    let mut soon = 0;
    let mut soon_manual = 0;
    for &i in idx {
        let st = &rep.status[i];
        match st.state {
            State::Up => up += 1,
            State::Down => down += 1,
            State::Disabled => off += 1,
        }
        if st.cert.expired() {
            expired += 1;
        }
        if st.cert.expiring_soon() {
            soon += 1;
            if matches!(st.renew, Renew::Manual(_)) {
                soon_manual += 1;
            }
        }
    }
    let mut l = vec![plain(format!("{n} domain{}", if n == 1 { "" } else { "s" }))];
    let dot = || plain(" · ");
    l.extend([dot(), seg(format!("{up} up"), Tone::Good)]);
    l.extend([dot(), seg(format!("{down} down"), if down > 0 { Tone::Bad } else { Tone::Plain })]);
    if off > 0 {
        l.extend([dot(), seg(format!("{off} disabled"), Tone::Dim)]);
    }
    if expired > 0 {
        l.extend([dot(), seg(format!("{expired} cert{} expired", if expired == 1 { "" } else { "s" }), Tone::Bad)]);
    }
    if soon > 0 {
        let note = if soon_manual == soon { " (manual renewal)" } else if soon_manual > 0 { " (some manual)" } else { "" };
        l.extend([dot(), seg(format!("{soon} expiring soon{note}"), Tone::Warn)]);
    }
    l
}

// ───────────────────────── show detail ─────────────────────────

pub const DETAIL_WIDTH: usize = 74;

struct Doc(Vec<Line>);

impl Doc {
    fn top(&mut self, label: &str, value: Line) {
        let mut l = vec![plain(format!("{label:<16}"))];
        if label.len() >= 16 {
            l.push(plain(" "));
        }
        l.extend(value);
        self.0.push(l);
    }
    fn item(&mut self, label: &str, value: Line) {
        let pad = 14.max(label.width() + 1);
        let mut l = vec![plain("  "), plain(format!("{label:<pad$}"))];
        l.extend(value);
        self.0.push(l);
    }
    fn section(&mut self, title: &str) {
        self.0.push(vec![]);
        self.0.push(vec![seg(title, Tone::Bold)]);
    }
}

fn joined(parts: Vec<Line>) -> Line {
    let mut out = Vec::new();
    for (i, p) in parts.into_iter().enumerate() {
        if i > 0 {
            out.push(seg("  ·  ", Tone::Dim));
        }
        out.extend(p);
    }
    out
}

fn manager_label(m: &Manager) -> String {
    match m {
        Manager::Systemd(u) => format!("systemd  ({u})"),
        Manager::Pm2(n) => format!("pm2  (app \"{n}\")"),
        Manager::Docker(id) => format!("docker  (container {id})"),
        Manager::Supervisor => "supervisord".into(),
        Manager::Unknown => "—".into(),
    }
}

/// Build the `ngx show` view. Probes processes and reads logs, so it may take
/// a moment.
pub fn detail(s: &Site, st: &Status) -> Vec<Line> {
    let env = Env::get();
    let mut d = Doc(Vec::new());
    let badge = match st.state {
        State::Up => vec![seg("● UP", Tone::Good)],
        State::Down => vec![seg("○ DOWN", Tone::Bad)],
        State::Disabled => vec![seg("◌ DISABLED", Tone::Dim)],
    };
    let gap = DETAIL_WIDTH.saturating_sub(s.domain.width() + width(&badge)).max(2);
    let mut head = vec![seg(s.domain.clone(), Tone::Bold), plain(" ".repeat(gap))];
    head.extend(badge);
    d.0.push(head);
    d.0.push(vec![seg("─".repeat(DETAIL_WIDTH), Tone::Dim)]);

    d.top("ID", vec![plain(s.id.to_string())]);
    if !s.aliases.is_empty() {
        d.top("Aliases", vec![plain(s.aliases.join(", "))]);
    }
    let mut cfg = vec![plain(s.config.display().to_string())];
    let mut notes = Vec::new();
    if let Some(m) = s.mtime {
        notes.push(format!("modified {}", datetime(m.into())));
    }
    if !s.enabled {
        notes.push("disabled".into());
    }
    if s.managed {
        notes.push("managed by ngx".into());
    }
    if !notes.is_empty() {
        cfg.push(seg(format!("  ({})", notes.join(", ")), Tone::Dim));
    }
    d.top("Config", cfg);
    if let Some(e) = &st.error {
        d.top("Problem", vec![seg(e.clone(), Tone::Bad)]);
    }

    // SSL
    if s.ssl_cert().is_some() || s.ssl() {
        d.section("SSL");
        let status = match &st.cert {
            CertState::None => vec![seg("no certificate configured", Tone::Bad)],
            CertState::Missing(p) => vec![seg(format!("certificate file missing: {}", p.display()), Tone::Bad)],
            CertState::Unreadable(why) => vec![seg(format!("cannot read certificate: {why}"), Tone::Dim)],
            CertState::Ok(i) => {
                let days = st.cert.days_left().unwrap_or(0);
                if st.cert.expired() {
                    let ago = (Local::now() - i.not_after).num_days();
                    vec![seg(format!("expired {}", date(i.not_after)), Tone::Bad), seg(format!("  ({ago} day{} ago)", if ago == 1 { "" } else { "s" }), Tone::Dim)]
                } else {
                    let t = if st.cert.expiring_soon() { Tone::Warn } else { Tone::Plain };
                    vec![seg(format!("valid until {}", date(i.not_after)), t), seg(format!("  ({days} days left)"), Tone::Dim)]
                }
            }
        };
        d.item("Status", status);
        if let Some(i) = st.cert.info() {
            d.item("Issuer", vec![plain(i.issuer.clone())]);
        }
        let renewal = match &st.renew {
            Renew::None => vec![seg("—", Tone::Dim)],
            Renew::Auto { next } => {
                let mut l = vec![plain("auto")];
                if let Some(n) = next {
                    l.push(seg(format!("  ·  {n}"), Tone::Dim));
                }
                l
            }
            Renew::Manual(why) => vec![seg("manual", Tone::Warn), seg(format!("  ·  {why}"), Tone::Dim)],
        };
        d.item("Renewal", renewal);
        if let Some(i) = st.cert.info() {
            if i.sans.len() > 1 || i.sans.first() != Some(&s.domain) {
                d.item("Covers", vec![plain(i.sans.join(", "))]);
            }
            let missing: Vec<&str> = s.all_names().filter(|n| !covers(&i.sans, n)).collect();
            if !missing.is_empty() {
                d.item("Not covered", vec![seg(missing.join(", "), Tone::Bad)]);
            }
        }
    }

    // ROUTING
    d.section("ROUTING");
    let mut inbound: Vec<Line> = Vec::new();
    if let Some(b) = s.main() {
        let mut ports: Vec<(u16, bool)> = b.listens.iter().map(|l| (l.port, l.ssl)).collect();
        ports.dedup();
        for (p, ssl) in ports {
            inbound.push(vec![plain(format!(":{p} ({})", if ssl { "HTTPS" } else { "HTTP" }))]);
        }
    }
    if let Some(r) = s.http_redirect() {
        let p = r.listens.first().map(|l| l.port).unwrap_or(80);
        inbound.push(vec![plain(format!(":{p} → redirect to HTTPS"))]);
    }
    d.item("Inbound", joined(inbound));
    d.item("Type", vec![plain(s.kind().label())]);
    match s.kind() {
        Kind::Proxy | Kind::FastCgi => {
            if let Some(u) = s.upstream() {
                d.item("Upstream", vec![plain(u.to_string())]);
            }
        }
        Kind::Static => {
            if let Some(r) = s.root() {
                d.item("Root", vec![plain(r.display().to_string())]);
            }
        }
        Kind::Redirect => {
            if let Some(t) = s.main().and_then(|b| b.redirect.clone()) {
                d.item("Target", vec![plain(t)]);
            }
        }
    }

    // BACKEND
    if let (Kind::Proxy | Kind::FastCgi, Some(u)) = (s.kind(), s.upstream()) {
        if !matches!(u, Upstream::Other(_)) {
            backend(&mut d, s, u);
        }
    }

    // LOCATIONS
    if let Some(b) = s.main() {
        if !b.locations.is_empty() {
            d.section("LOCATIONS");
            for l in &b.locations {
                d.item(&l.matcher, vec![seg("→ ", Tone::Dim), plain(l.action.clone())]);
            }
        }
    }

    // LOGS
    let main = s.main();
    let access = main.and_then(|b| b.access_log.clone());
    let error = main.and_then(|b| b.error_log.clone());
    if access.is_some() || error.is_some() {
        d.section("LOGS");
        if let Some(a) = &access {
            d.item("Access", vec![plain(a.display().to_string())]);
        }
        if let Some(e) = &error {
            d.item("Error", vec![plain(e.display().to_string())]);
        }
    }
    let (err_path, filter) = match &error {
        Some(e) => (e.clone(), None),
        None => (env.log_dir.join("error.log"), Some(s.domain.as_str())),
    };
    let errs = logs::recent_errors(&err_path, filter, 3);
    if !errs.is_empty() {
        d.section(&format!("RECENT ERRORS (last {})", errs.len()));
        for e in errs {
            let code = e.code.map(|c| c.to_string()).unwrap_or_else(|| "—".into());
            d.0.push(vec![
                plain("  "),
                seg(e.time.format("%d/%m/%y %H:%M").to_string(), Tone::Dim),
                plain("  "),
                seg(format!("{code:<3}"), Tone::Bad),
                plain("  "),
                plain(e.msg),
            ]);
        }
    }
    if let Some(t) = access.as_deref().and_then(logs::traffic_24h) {
        d.section("TRAFFIC (24h)");
        let pct = if t.requests > 0 { t.errors_5xx as f64 * 100.0 / t.requests as f64 } else { 0.0 };
        let tone = if pct >= 5.0 { Tone::Bad } else if pct > 0.5 { Tone::Warn } else { Tone::Plain };
        let lat = t.avg_ms.map(|m| format!("{m:.0} ms")).unwrap_or_else(|| "—".into());
        d.item("Requests", joined(vec![
            vec![plain(thousands(t.requests))],
            vec![seg(format!("5xx {pct:.1}%"), tone)],
            vec![plain(format!("avg latency {lat}"))],
        ]));
    }
    d.0
}

pub fn covers(sans: &[String], name: &str) -> bool {
    sans.iter().any(|s| {
        s.eq_ignore_ascii_case(name)
            || s.strip_prefix("*.").is_some_and(|base| {
                name.split_once('.').is_some_and(|(_, rest)| rest.eq_ignore_ascii_case(base))
            })
    })
}

fn backend(d: &mut Doc, s: &Site, u: &Upstream) {
    d.section("BACKEND");
    let roots: Vec<&std::path::Path> = s.root().into_iter().collect();
    match probe::listener(u) {
        Listener::Pid(pid) => {
            let p = probe::proc_info(pid);
            d.item("Runtime", vec![plain(p.runtime.clone().unwrap_or_else(|| "unknown".into()))]);
            d.item("Managed by", vec![plain(manager_label(&p.manager))]);
            let mut parts = vec![vec![seg("running", Tone::Good)]];
            parts.push(vec![plain(format!("pid {pid}"))]);
            if let Some(w) = p.workers {
                parts.push(vec![plain(w)]);
            }
            if let Some(t) = p.started {
                parts.push(vec![plain(format!("up {}", probe::fmt_uptime(t)))]);
            }
            d.item("Process", joined(parts));
            if s.kind() == Kind::FastCgi || u.is_unix() {
                if let Some(r) = s.root() {
                    d.item("Document root", vec![plain(r.display().to_string())]);
                }
            } else if let Some(c) = p.cwd {
                d.item("Working dir", vec![plain(c.display().to_string())]);
            }
        }
        Listener::Hidden => {
            d.item("Process", joined(vec![vec![seg("running", Tone::Good)], vec![seg(if Env::get().sandboxed() { "no process details in sandbox" } else { "details need root" }, Tone::Dim)]]));
            if let Some(r) = s.root() {
                d.item("Document root", vec![plain(r.display().to_string())]);
            }
        }
        Listener::None => {
            let st = probe::stopped_backend(u, &roots);
            if let Some(m) = &st.manager {
                d.item("Managed by", vec![plain(manager_label(m))]);
            }
            d.item("Process", vec![seg(st.detail, Tone::Bad)]);
            if let Some(c) = st.cwd {
                d.item("Working dir", vec![plain(c)]);
            } else if let Some(r) = s.root() {
                d.item("Document root", vec![plain(r.display().to_string())]);
            }
        }
    }
}

impl Upstream {
    pub fn is_unix(&self) -> bool {
        matches!(self, Upstream::Unix(_))
    }
}

// ───────────────────────── JSON ─────────────────────────

pub fn json(s: &Site, st: &Status) -> serde_json::Value {
    let cert = st.cert.info();
    serde_json::json!({
        "id": s.id,
        "domain": s.domain,
        "aliases": s.aliases,
        "status": match st.state { State::Up => "up", State::Down => "down", State::Disabled => "disabled" },
        "error": st.error,
        "enabled": s.enabled,
        "managed": s.managed,
        "config": s.config,
        "file": s.file,
        "kind": s.kind().label(),
        "inbound": s.public_port(),
        "upstream": s.upstream().map(|u| u.to_string()),
        "root": s.root(),
        "ssl": {
            "certificate": s.ssl_cert(),
            "expires": cert.map(|c| c.not_after.to_rfc3339()),
            "issuer": cert.map(|c| c.issuer.clone()),
            "names": cert.map(|c| c.sans.clone()),
            "expired": st.cert.expired(),
            "expiring_soon": st.cert.expiring_soon(),
            "renewal": match &st.renew { Renew::None => "none", Renew::Auto { .. } => "auto", Renew::Manual(_) => "manual" },
        },
        "locations": s.main().map(|b| b.locations.iter().map(|l| serde_json::json!({"match": l.matcher, "action": l.action})).collect::<Vec<_>>()),
    })
}
