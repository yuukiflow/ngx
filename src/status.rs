//! Combine config, certificates and probes into per-site status.

use crate::env::{self, Env};
use crate::nginx::ctl;
use crate::nginx::site::{Kind, Site, Upstream};
use crate::ssl::{self, CertState, Renew};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    Up,
    Down,
    Disabled,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub state: State,
    pub error: Option<String>,
    pub cert: CertState,
    pub renew: Renew,
}

pub struct Report {
    pub sites: Vec<Site>,
    pub status: Vec<Status>,
    /// nginx -t problems not attributable to one site.
    pub global: Vec<String>,
}

/// Static duplicate-server_name detection across enabled sites (nginx keeps
/// the first definition and ignores the rest).
fn duplicates(sites: &[Site]) -> HashMap<usize, String> {
    let mut seen: HashMap<(u16, String), usize> = HashMap::new();
    let mut out = HashMap::new();
    for (i, s) in sites.iter().enumerate().filter(|(_, s)| s.enabled) {
        for b in &s.blocks {
            for n in b.names.iter().filter(|n| *n != "_") {
                for l in &b.listens {
                    if let Some(&j) = seen.get(&(l.port, n.to_lowercase())) {
                        if j != i && !out.contains_key(&i) {
                            out.insert(i, format!("config error: duplicate server_name {n} (line {})", b.names_line));
                        }
                    } else {
                        seen.insert((l.port, n.to_lowercase()), i);
                    }
                }
            }
        }
    }
    out
}

fn same_file(a: &PathBuf, s: &Site) -> bool {
    *a == s.config || *a == s.file
}

pub fn evaluate(sites: Vec<Site>, run_nginx_test: bool) -> Report {
    let env = Env::get();
    let mut errors: HashMap<usize, String> = HashMap::new();
    let mut global = Vec::new();

    for (i, s) in sites.iter().enumerate() {
        if let Some(e) = &s.parse_error {
            errors.insert(i, format!("config error: {e}"));
        }
    }
    for (i, e) in duplicates(&sites) {
        errors.entry(i).or_insert(e);
    }
    // nginx -t needs root to open certs/logs; only trust it then.
    if run_nginx_test && (env::is_root() || env.sandboxed())
        && let Some(t) = ctl::test() {
            for m in t.msgs.iter().filter(|m| m.level != "warn") {
                let hit = m.file.as_ref().and_then(|f| {
                    sites.iter().enumerate().filter(|(_, s)| same_file(f, s)).min_by_key(|(_, s)| {
                        let line = m.line.unwrap_or(0);
                        let inside = s.blocks.iter().any(|b| b.line <= line && line <= b.end_line);
                        (!inside, s.line())
                    }).map(|(i, _)| i)
                });
                let line = m.line.map(|l| format!(" (line {l})")).unwrap_or_default();
                match hit {
                    Some(i) => {
                        errors.entry(i).or_insert(format!("config error: {}{line}", m.text));
                    }
                    None => global.push(m.to_string()),
                }
            }
            if !t.ok && t.msgs.is_empty() {
                global.push(t.raw.clone());
            }
        }
    let nginx_up = ctl::running();

    let status: Vec<Status> = std::thread::scope(|scope| {
        let handles: Vec<_> = sites
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let err = errors.get(&i).cloned();
                scope.spawn(move || site_status(s, err, nginx_up))
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("status thread")).collect()
    });
    Report { sites, status, global }
}

fn site_status(s: &Site, config_err: Option<String>, nginx_up: bool) -> Status {
    let cert = s.ssl_cert().map(ssl::read_cert).unwrap_or(CertState::None);
    let renew = ssl::renewal(s.ssl_cert());
    let (state, error) = if !s.enabled {
        (State::Disabled, None)
    } else if let Some(e) = config_err {
        (State::Down, Some(e))
    } else if !nginx_up {
        (State::Down, Some("nginx is not running".into()))
    } else {
        match backend_error(s) {
            Some(e) => (State::Down, Some(e)),
            None => (State::Up, None),
        }
    };
    Status { state, error, cert, renew }
}

fn backend_error(s: &Site) -> Option<String> {
    match s.kind() {
        Kind::Proxy | Kind::FastCgi => {
            let u = s.upstream()?;
            if let Upstream::Other(_) = u {
                return None;
            }
            if let Some(e) = ssl_free_probe(u) {
                return Some(e);
            }
            // ngx-managed proxy sites can have an HTTP health check
            if let (true, Upstream::Tcp { host, port }) = (s.managed, u) {
                let spec = std::fs::read_to_string(Env::get().host(&s.file)).ok().and_then(|src| crate::nginx::template::Spec::from_config(&src));
                if let Some(path) = spec.and_then(|sp| sp.opts.health)
                    && let Err(why) = crate::probe::check_health(host, *port, &path, &s.domain) {
                        return Some(format!("health check {path} {why}"));
                    }
            }
            None
        }
        Kind::Static => {
            let root = s.root()?;
            if Env::get().host(root).is_dir() { None } else { Some(format!("404 Not Found: root {} does not exist", root.display())) }
        }
        Kind::Redirect => None,
    }
}

fn ssl_free_probe(u: &Upstream) -> Option<String> {
    crate::probe::check_upstream(u).err().map(|why| format!("502 Bad Gateway: upstream {u} {why}"))
}

impl Report {
    pub fn load(run_nginx_test: bool) -> Report {
        evaluate(crate::nginx::site::load_all(), run_nginx_test)
    }
    pub fn index_of(&self, query: &str) -> anyhow::Result<usize> {
        let s = crate::nginx::site::find(&self.sites, query)?;
        Ok(self.sites.iter().position(|x| x.key == s.key).unwrap_or(0))
    }
}
