//! Add/edit form inside the dashboard: every setting on one screen, checked
//! as you type. Saving hands the values to `ngx add` / `ngx edit` (which show
//! the config or diff and ask for confirmation), so the dashboard goes through
//! exactly the same validation and safe-change path as the command line.

use super::style;
use crate::cmd::{AddArgs, OnOff};
use crate::nginx::template::{Backend, Route, Spec, Ssl};
use crate::render::Tone;
use crate::validate;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

#[derive(Clone, Copy, PartialEq, Debug)]
enum K {
    Domain,
    Aliases,
    Type,
    Local,
    WebSocket,
    Root,
    Socket,
    Ssl,
    DnsPlugin,
    DnsCreds,
    Cert,
    Key,
    Email,
    Inbound,
    Redirect,
    MaxBody,
    Hsts,
    Auth,
    RateLimit,
    Canonical,
    Routes,
}

enum V {
    Text(String),
    Choice(&'static [&'static str], usize),
    Toggle(bool),
}

struct Field {
    k: K,
    label: &'static str,
    v: V,
    hint: &'static str,
}

const TYPES: &[&str] = &["reverse proxy", "static files", "PHP (php-fpm)"];
const SSLS: &[&str] = &["Let's Encrypt", "Let's Encrypt (DNS challenge)", "existing certificate", "none"];

pub struct Form {
    /// Site ID when editing.
    pub site: Option<String>,
    title: String,
    fields: Vec<Field>,
    sel: usize,
    error: Option<String>,
}

pub enum Outcome {
    Stay,
    Cancel,
    Submit(Option<String>, Box<AddArgs>),
}

fn text(k: K, label: &'static str, v: impl Into<String>, hint: &'static str) -> Field {
    Field { k, label, v: V::Text(v.into()), hint }
}

impl Form {
    pub fn new(site: Option<(String, Spec)>) -> Form {
        let editing = site.as_ref().map(|(id, _)| id.clone());
        let cur = site.map(|(_, s)| s);
        let c = cur.as_ref();
        let (kind, local, ws, root, socket) = match c.map(|c| &c.backend) {
            Some(Backend::Proxy { host, port, websocket }) => (0, if host == "127.0.0.1" { port.to_string() } else { format!("{host}:{port}") }, *websocket, String::new(), String::new()),
            Some(Backend::Static { root }) => (1, String::new(), false, root.clone(), String::new()),
            Some(Backend::Php { socket, root }) => (2, String::new(), false, root.clone(), socket.clone()),
            None => (0, String::new(), false, String::new(), crate::cmd::php_sockets().into_iter().next().unwrap_or_else(|| "/run/php/php-fpm.sock".into())),
        };
        let (ssl, plugin, creds, cert, key) = match c.map(|c| &c.ssl) {
            None | Some(Ssl::LetsEncrypt) => (0, String::new(), String::new(), String::new(), String::new()),
            Some(Ssl::LetsEncryptDns { plugin, credentials }) => (1, plugin.clone(), credentials.clone().unwrap_or_default(), String::new(), String::new()),
            Some(Ssl::Existing { cert, key }) => (2, String::new(), String::new(), cert.clone(), key.clone()),
            Some(Ssl::None) => (3, String::new(), String::new(), String::new(), String::new()),
        };
        let o = c.map(|c| c.opts.clone()).unwrap_or_default();
        let fields = vec![
            text(K::Domain, "Domain", c.map(|c| c.domain.clone()).unwrap_or_default(), "e.g. app.example.com"),
            text(K::Aliases, "Aliases", c.map(|c| c.aliases.join(" ")).unwrap_or_default(), "space-separated, e.g. www.example.com"),
            Field { k: K::Type, label: "Site type", v: V::Choice(TYPES, kind), hint: "←/→ to change" },
            text(K::Local, "Local port", local, "port of the app on 127.0.0.1, or HOST:PORT"),
            Field { k: K::WebSocket, label: "WebSocket", v: V::Toggle(ws), hint: "proxy WebSocket upgrades" },
            text(K::Root, "Document root", root, "default /var/www/<domain>"),
            text(K::Socket, "php-fpm socket", socket, "e.g. /run/php/php8.3-fpm.sock"),
            Field { k: K::Ssl, label: "SSL", v: V::Choice(SSLS, ssl), hint: "←/→ to change" },
            text(K::DnsPlugin, "DNS plugin", plugin, "certbot-dns-<plugin>: cloudflare, route53, digitalocean…"),
            text(K::DnsCreds, "DNS credentials", creds, "credentials file (empty if the plugin needs none)"),
            text(K::Cert, "Certificate", cert, "fullchain.pem path"),
            text(K::Key, "Private key", key, "privkey.pem path"),
            text(K::Email, "Account email", "", "optional; Let's Encrypt no longer sends expiry emails (use ngx check)"),
            text(K::Inbound, "Public port", c.map(|c| c.inbound).unwrap_or(443).to_string(), "443 for HTTPS, 80 for HTTP"),
            Field { k: K::Redirect, label: "Redirect HTTP", v: V::Toggle(c.map(|c| c.redirect_http || c.ssl == Ssl::None).unwrap_or(true)), hint: "port 80 → HTTPS" },
            text(K::MaxBody, "Max upload size", o.max_body.unwrap_or_default(), "e.g. 50m (empty: nginx default 1m)"),
            Field { k: K::Hsts, label: "HSTS", v: V::Toggle(o.hsts), hint: "browsers always use HTTPS for this domain" },
            Field { k: K::Auth, label: "Password", v: V::Toggle(o.auth), hint: "basic auth; users: ngx auth add <site> <user>" },
            text(K::RateLimit, "Rate limit", o.rate_limit.unwrap_or_default(), "per client, e.g. 10r/s (empty: none)"),
            Field { k: K::Canonical, label: "Aliases → domain", v: V::Toggle(o.canonical), hint: "redirect the aliases to the main domain" },
            text(K::Routes, "Extra routes", o.routes.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(" "), "e.g. /api=8080 /ws=10.0.0.5:9000"),
        ];
        let title = match &editing {
            Some(id) => format!(" Edit site #{id} "),
            None => " New site ".to_string(),
        };
        Form { site: editing, title, fields, sel: 0, error: None }
    }

    fn get(&self, k: K) -> &V {
        &self.fields.iter().find(|f| f.k == k).unwrap().v
    }
    fn txt(&self, k: K) -> String {
        match self.get(k) {
            V::Text(s) => s.trim().to_string(),
            _ => String::new(),
        }
    }
    fn choice(&self, k: K) -> usize {
        match self.get(k) {
            V::Choice(_, i) => *i,
            _ => 0,
        }
    }
    fn on(&self, k: K) -> bool {
        matches!(self.get(k), V::Toggle(true))
    }

    fn visible(&self, k: K) -> bool {
        let (kind, ssl) = (self.choice(K::Type), self.choice(K::Ssl));
        match k {
            K::Local | K::WebSocket => kind == 0,
            K::Root => kind != 0,
            K::Socket => kind == 2,
            K::DnsPlugin | K::DnsCreds => ssl == 1,
            K::Cert | K::Key => ssl == 2,
            K::Email => ssl <= 1 && self.site.is_none(),
            K::Redirect => ssl != 3 && self.txt(K::Inbound) != "80",
            K::Hsts => ssl != 3,
            K::Canonical => !self.txt(K::Aliases).is_empty(),
            _ => true,
        }
    }

    fn rows(&self) -> Vec<usize> {
        (0..self.fields.len()).filter(|&i| self.visible(self.fields[i].k)).collect()
    }

    /// Problem with a field's current value, if any.
    fn invalid(&self, k: K) -> Option<String> {
        let t = self.txt(k);
        let bad = |m: &str| Some(m.to_string());
        match k {
            K::Domain if !validate::domain(&t.to_lowercase()) => bad("not a valid domain name"),
            K::Aliases => t.split_whitespace().find(|a| !validate::domain(&a.to_lowercase())).map(|a| format!("{a} is not a valid domain name")),
            K::Local => {
                let ok = match t.rsplit_once(':') {
                    Some((h, p)) => validate::host(h) && p.parse::<u16>().is_ok_and(|p| p > 0),
                    None => t.parse::<u16>().is_ok_and(|p| p > 0),
                };
                (!ok).then(|| "a port (3000) or HOST:PORT".into())
            }
            K::Root if !t.is_empty() && !validate::path(&t) => bad("absolute path without spaces, quotes or ; { } $ #"),
            K::Socket | K::Cert | K::Key if !validate::path(&t) => bad("absolute path without spaces, quotes or ; { } $ #"),
            K::DnsCreds if !t.is_empty() && !validate::path(&t) => bad("absolute path"),
            K::DnsPlugin if !validate::dns_plugin(&t) => bad("plugin name, e.g. cloudflare"),
            K::Email if !t.is_empty() && !validate::email(&t) => bad("not a valid email address"),
            K::Inbound if !t.parse::<u16>().is_ok_and(|p| p > 0) => bad("a port number"),
            K::MaxBody if !t.is_empty() && !validate::size(&t) => bad("e.g. 10m, 512k, 1g"),
            K::RateLimit if !t.is_empty() && !validate::rate(&t) => bad("e.g. 10r/s or 300r/m"),
            K::Routes => t
                .split_whitespace()
                .find(|r| !Route::parse(r).is_some_and(|r| validate::route_path(&r.path) && validate::host(&r.host) && r.port > 0))
                .map(|r| format!("{r}: use PATH=PORT or PATH=HOST:PORT")),
            _ => None,
        }
    }

    fn to_args(&self) -> AddArgs {
        let opt = |s: String| Some(s).filter(|s| !s.is_empty());
        let onoff = |b: bool| Some(if b { OnOff::On } else { OnOff::Off });
        let kind = self.choice(K::Type);
        let ssl = self.choice(K::Ssl);
        let aliases: Vec<String> = self.txt(K::Aliases).split_whitespace().map(String::from).collect();
        let routes: Vec<String> = self.txt(K::Routes).split_whitespace().map(String::from).collect();
        AddArgs {
            domain: Some(self.txt(K::Domain).to_lowercase()),
            aliases: if aliases.is_empty() { vec!["none".into()] } else { aliases },
            kind: Some(["proxy", "static", "php"][kind].into()),
            local: (kind == 0).then(|| self.txt(K::Local)),
            root: if kind != 0 { opt(self.txt(K::Root)) } else { None },
            socket: (kind == 2).then(|| self.txt(K::Socket)),
            inbound: self.txt(K::Inbound).parse().ok(),
            ssl: Some(["letsencrypt", "dns", "existing", "none"][ssl].into()),
            cert: (ssl == 2).then(|| self.txt(K::Cert)),
            key: (ssl == 2).then(|| self.txt(K::Key)),
            email: if ssl <= 1 { opt(self.txt(K::Email)) } else { None },
            dns: (ssl == 1).then(|| self.txt(K::DnsPlugin)),
            dns_credentials: if ssl == 1 { opt(self.txt(K::DnsCreds)) } else { None },
            max_body: Some(opt(self.txt(K::MaxBody)).unwrap_or_else(|| "off".into())),
            hsts: onoff(self.on(K::Hsts) && ssl != 3),
            auth: onoff(self.on(K::Auth)),
            rate_limit: Some(opt(self.txt(K::RateLimit)).unwrap_or_else(|| "off".into())),
            canonical: onoff(self.on(K::Canonical)),
            routes: if routes.is_empty() { vec!["off".into()] } else { routes },
            websocket_set: (kind == 0).then(|| self.on(K::WebSocket)),
            redirect_set: Some(self.on(K::Redirect)),
            review: true,
            ..AddArgs::default()
        }
    }

    pub fn key(&mut self, k: KeyEvent) -> Outcome {
        let rows = self.rows();
        let pos = rows.iter().position(|&i| i == self.sel).unwrap_or(0);
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let idx = rows[pos];
        let kk = self.fields[idx].k;
        match k.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Char('s') if ctrl => return self.submit(),
            KeyCode::F(2) => return self.submit(),
            KeyCode::Down | KeyCode::Tab => self.sel = rows[(pos + 1) % rows.len()],
            KeyCode::Up | KeyCode::BackTab => self.sel = rows[(pos + rows.len() - 1) % rows.len()],
            KeyCode::Enter if pos + 1 == rows.len() => return self.submit(),
            KeyCode::Enter => self.sel = rows[pos + 1],
            _ => {
                let before_ssl = self.choice(K::Ssl);
                match (&mut self.fields[idx].v, k.code) {
                    (V::Choice(opts, i), KeyCode::Right | KeyCode::Char(' ')) => *i = (*i + 1) % opts.len(),
                    (V::Choice(opts, i), KeyCode::Left) => *i = (*i + opts.len() - 1) % opts.len(),
                    (V::Toggle(b), KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')) => *b = !*b,
                    (V::Text(s), KeyCode::Char('u')) if ctrl => s.clear(),
                    (V::Text(s), KeyCode::Char(c)) if !ctrl && !c.is_control() && s.len() < 512 => s.push(c),
                    (V::Text(s), KeyCode::Backspace) => {
                        s.pop();
                    }
                    _ => {}
                }
                // keep the public port sensible when SSL is switched on/off
                if kk == K::Ssl {
                    let after = self.choice(K::Ssl);
                    if let Some(f) = self.fields.iter_mut().find(|f| f.k == K::Inbound)
                        && let V::Text(p) = &mut f.v {
                            if after == 3 && before_ssl != 3 && p == "443" {
                                *p = "80".into();
                            } else if after != 3 && before_ssl == 3 && p == "80" {
                                *p = "443".into();
                            }
                        }
                }
            }
        }
        self.error = None;
        Outcome::Stay
    }

    fn submit(&mut self) -> Outcome {
        for i in self.rows() {
            if let Some(e) = self.invalid(self.fields[i].k) {
                self.sel = i;
                self.error = Some(format!("{}: {e}", self.fields[i].label));
                return Outcome::Stay;
            }
        }
        Outcome::Submit(self.site.clone(), Box::new(self.to_args()))
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let [body, foot] = Layout::vertical([Constraint::Fill(1), Constraint::Length(2)]).areas(area);
        let mut lines = Vec::new();
        for i in self.rows() {
            let fl = &self.fields[i];
            let selected = i == self.sel;
            let value = match &fl.v {
                V::Text(s) if s.is_empty() && !selected => Span::styled(fl.hint, style(Tone::Dim)),
                V::Text(s) => Span::raw(format!("{s}{}", if selected { "▏" } else { "" })),
                V::Choice(opts, i) => Span::raw(format!("‹ {} ›", opts[*i])),
                V::Toggle(b) => Span::styled(if *b { "[x] on" } else { "[ ] off" }, style(if *b { Tone::Good } else { Tone::Dim })),
            };
            let label_style = if selected { style(Tone::Accent).add_modifier(Modifier::BOLD) } else { style(Tone::Plain) };
            let mut l = vec![Span::styled(format!("{} {:<18}", if selected { "›" } else { " " }, fl.label), label_style), value];
            if let Some(e) = self.invalid(fl.k).filter(|_| !matches!(&fl.v, V::Text(s) if s.is_empty() && !selected)) {
                l.push(Span::styled(format!("   ✗ {e}"), style(Tone::Bad)));
            } else if selected && !matches!(fl.v, V::Text(_)) {
                l.push(Span::styled(format!("   {}", fl.hint), style(Tone::Dim)));
            }
            lines.push(Line::from(l));
        }
        f.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(self.title.as_str())), body);
        let help = Line::from(vec![
            Span::styled(" ↑↓", style(Tone::Accent)),
            Span::styled(" field ", style(Tone::Dim)),
            Span::styled(" ←→/space", style(Tone::Accent)),
            Span::styled(" change ", style(Tone::Dim)),
            Span::styled(" Ctrl-S/F2", style(Tone::Accent)),
            Span::styled(" review & save ", style(Tone::Dim)),
            Span::styled(" Esc", style(Tone::Accent)),
            Span::styled(" cancel", style(Tone::Dim)),
        ]);
        let mut foot_lines = vec![help];
        if let Some(e) = &self.error {
            foot_lines.insert(0, Line::from(Span::styled(format!(" {e}"), style(Tone::Bad))));
        }
        f.render_widget(Paragraph::new(foot_lines), foot);
    }
}
