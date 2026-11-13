//! Site model: groups of `server` blocks serving the same domain, plus loading
//! them from the nginx configuration directories.

use super::parser::{self, Directive};
use crate::env::Env;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Upstream {
    Tcp { host: String, port: u16 },
    Unix(PathBuf),
    /// Something we can't resolve statically (variables, unknown upstream name).
    Other(String),
}

impl fmt::Display for Upstream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Upstream::Tcp { host, port } => write!(f, "{host}:{port}"),
            Upstream::Unix(p) => write!(f, "unix:{}", p.display()),
            Upstream::Other(s) => write!(f, "{s}"),
        }
    }
}

impl Upstream {
    /// Parse a `proxy_pass` / `fastcgi_pass` target.
    pub fn parse(raw: &str, upstreams: &HashMap<String, String>) -> Upstream {
        let mut s = raw;
        let mut default_port = 80;
        for (scheme, port) in [("http://", 80), ("https://", 443), ("grpc://", 80), ("grpcs://", 443)] {
            if let Some(rest) = s.strip_prefix(scheme) {
                s = rest;
                default_port = port;
            }
        }
        if let Some(rest) = s.strip_prefix("unix:") {
            // http://unix:/path.sock:/uri
            let path = rest.split(':').next().unwrap_or(rest);
            return Upstream::Unix(PathBuf::from(path));
        }
        let hostport = s.split('/').next().unwrap_or(s);
        if hostport.contains('$') {
            return Upstream::Other(raw.to_string());
        }
        if let Some(target) = upstreams.get(hostport) {
            return Upstream::parse(target, &HashMap::new());
        }
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) if !h.ends_with(':') => match p.parse() {
                Ok(p) => (h.to_string(), p),
                Err(_) => (hostport.to_string(), default_port),
            },
            _ => (hostport.to_string(), default_port),
        };
        if host.is_empty() {
            return Upstream::Other(raw.to_string());
        }
        Upstream::Tcp { host, port }
    }

    pub fn short(&self) -> String {
        match self {
            Upstream::Tcp { port, .. } => port.to_string(),
            Upstream::Unix(p) => p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            Upstream::Other(_) => "?".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Proxy,
    FastCgi,
    Static,
    Redirect,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Proxy => "reverse proxy",
            Kind::FastCgi => "FastCGI",
            Kind::Static => "static files",
            Kind::Redirect => "redirect",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Listen {
    pub port: u16,
    pub ssl: bool,
}

#[derive(Debug, Clone)]
pub struct Location {
    pub matcher: String,
    pub action: String,
    pub upstream: Option<Upstream>,
}

#[derive(Debug, Clone)]
pub struct ServerBlock {
    pub line: usize,
    pub end_line: usize,
    pub names: Vec<String>,
    pub names_line: usize,
    pub listens: Vec<Listen>,
    /// `Some(target)` when the whole block just redirects.
    pub redirect: Option<String>,
    pub ssl_cert: Option<PathBuf>,
    pub ssl_key: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub access_log: Option<PathBuf>,
    pub error_log: Option<PathBuf>,
    pub locations: Vec<Location>,
}

impl ServerBlock {
    pub fn is_ssl(&self) -> bool {
        self.listens.iter().any(|l| l.ssl)
    }
    pub fn upstream(&self) -> Option<&Upstream> {
        self.locations.iter().find(|l| l.matcher == "/").and_then(|l| l.upstream.as_ref())
            .or_else(|| self.locations.iter().find_map(|l| l.upstream.as_ref()))
    }
    fn has_fastcgi(&self) -> bool {
        self.locations.iter().any(|l| l.action.starts_with("fastcgi"))
    }
}

#[derive(Debug, Clone)]
pub struct Site {
    /// Stable key for ID assignment: `<file name>#<domain>`.
    pub key: String,
    pub id: u32,
    pub domain: String,
    pub aliases: Vec<String>,
    /// Path shown to the user (the sites-enabled entry when enabled).
    pub config: PathBuf,
    /// Where the content actually lives (symlink target).
    pub file: PathBuf,
    pub enabled: bool,
    /// conf.d style site (toggle by renaming to `.disabled`).
    pub conf_d: bool,
    pub mtime: Option<SystemTime>,
    pub blocks: Vec<ServerBlock>,
    pub main: usize,
    pub managed: bool,
    /// Suspended with `ngx suspend` (managed sites only).
    pub suspended: Option<crate::nginx::template::Suspension>,
    pub parse_error: Option<String>,
}

impl Site {
    pub fn main(&self) -> Option<&ServerBlock> {
        self.blocks.get(self.main)
    }
    pub fn ssl(&self) -> bool {
        self.blocks.iter().any(|b| b.is_ssl())
    }
    pub fn public_port(&self) -> Option<u16> {
        let b = self.main()?;
        b.listens.iter().find(|l| l.ssl).or(b.listens.first()).map(|l| l.port)
    }
    pub fn http_redirect(&self) -> Option<&ServerBlock> {
        self.blocks.iter().find(|b| {
            !b.is_ssl() && b.redirect.as_deref().is_some_and(|t| t.starts_with("https://"))
        })
    }
    pub fn ssl_cert(&self) -> Option<&Path> {
        self.main().and_then(|b| b.ssl_cert.as_deref())
            .or_else(|| self.blocks.iter().find_map(|b| b.ssl_cert.as_deref()))
    }
    pub fn ssl_key(&self) -> Option<&Path> {
        self.main().and_then(|b| b.ssl_key.as_deref())
            .or_else(|| self.blocks.iter().find_map(|b| b.ssl_key.as_deref()))
    }
    pub fn kind(&self) -> Kind {
        let Some(b) = self.main() else { return Kind::Static };
        if b.redirect.is_some() {
            Kind::Redirect
        } else if b.has_fastcgi() {
            Kind::FastCgi
        } else if b.upstream().is_some() {
            Kind::Proxy
        } else {
            Kind::Static
        }
    }
    pub fn upstream(&self) -> Option<&Upstream> {
        self.main().and_then(|b| b.upstream())
    }
    pub fn root(&self) -> Option<&Path> {
        self.main().and_then(|b| b.root.as_deref())
    }
    pub fn line(&self) -> usize {
        self.main().map(|b| b.line).unwrap_or(1)
    }
    pub fn all_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.domain.as_str()).chain(self.aliases.iter().map(String::as_str))
    }
    pub fn matches(&self, query: &str) -> bool {
        self.all_names().any(|n| n.eq_ignore_ascii_case(query))
    }
}

// ───────────────────────── extraction ─────────────────────────

fn parse_listen(d: &Directive) -> Listen {
    let addr = d.arg(0).unwrap_or("80");
    let port = if addr.starts_with("unix:") {
        0
    } else {
        let tail = addr.rsplit_once("]:").map(|(_, p)| p).or_else(|| {
            if addr.starts_with('[') { None } else { Some(addr.rsplit(':').next().unwrap_or(addr)) }
        });
        tail.and_then(|p| p.parse().ok()).unwrap_or(80)
    };
    Listen { port, ssl: d.args.iter().skip(1).any(|a| a == "ssl" || a == "quic") }
}

fn redirect_target(block: &Directive) -> Option<String> {
    let ret = |d: &Directive| -> Option<String> {
        let code = d.arg(0)?;
        if matches!(code, "301" | "302" | "307" | "308") { d.arg(1).map(String::from) } else { None }
    };
    // `return 301 https://...;` at server level
    if let Some(t) = block.all("return").find_map(ret) {
        return Some(t);
    }
    // certbot style: `if ($host = x) { return 301 https://...; } return 404;`
    let has_content = block.children().iter().any(|d| {
        matches!(d.name.as_str(), "root" | "proxy_pass" | "fastcgi_pass")
            || (d.name == "location" && d.first("return").is_none())
    });
    if has_content {
        // `location / { return 301 ... }` with only acme locations besides
        let locs: Vec<_> = block.all("location").collect();
        let main = locs.iter().find(|l| l.arg(0) == Some("/"))?;
        let others_acme = locs.iter().all(|l| l.arg(0) == Some("/") || l.args.iter().any(|a| a.contains("acme-challenge")));
        if others_acme && block.first("proxy_pass").is_none() {
            return main.all("return").find_map(ret);
        }
        return None;
    }
    block.all("if").find_map(|i| i.all("return").find_map(ret))
}

fn location_info(loc: &Directive, server_root: Option<&str>, ups: &HashMap<String, String>) -> Location {
    let matcher = loc.args.join(" ");
    let mut upstream = None;
    let action = if let Some(p) = loc.first_arg("proxy_pass") {
        let u = Upstream::parse(p, ups);
        let ws = loc.all("proxy_set_header").any(|h| {
            h.arg(0).is_some_and(|n| n.eq_ignore_ascii_case("upgrade"))
                || (h.arg(0).is_some_and(|n| n.eq_ignore_ascii_case("connection"))
                    && h.arg(1).is_some_and(|v| v.to_ascii_lowercase().contains("upgrade")))
        });
        let s = format!("proxy {u}{}", if ws { "  (WebSocket upgrade)" } else { "" });
        upstream = Some(u);
        s
    } else if let Some(p) = loc.first_arg("fastcgi_pass") {
        let u = Upstream::parse(p, ups);
        let s = format!("fastcgi {}", match &u { Upstream::Unix(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), _ => u.to_string() });
        upstream = Some(u);
        s
    } else if let Some(p) = loc.first_arg("uwsgi_pass").or(loc.first_arg("grpc_pass")) {
        let u = Upstream::parse(p, ups);
        let s = format!("{} {u}", loc.first("uwsgi_pass").map(|_| "uwsgi").unwrap_or("grpc"));
        upstream = Some(u);
        s
    } else if let Some(r) = loc.first("return") {
        format!("return {}", r.args.join(" "))
    } else if let Some(a) = loc.first_arg("alias") {
        format!("files {a}")
    } else if let Some(r) = loc.first_arg("root") {
        format!("files {r}")
    } else if let Some(t) = loc.first("try_files") {
        format!("try_files {}", t.args.join(" "))
    } else if let Some(r) = server_root {
        format!("files {r}")
    } else {
        "—".into()
    };
    Location { matcher, action, upstream }
}

fn server_block(d: &Directive, ups: &HashMap<String, String>) -> ServerBlock {
    let path = |name: &str| d.first_arg(name).filter(|v| *v != "off").map(PathBuf::from);
    let root = d.first_arg("root");
    let mut locations: Vec<Location> = Vec::new();
    // server-level proxy_pass is invalid nginx, but `location` nesting is common
    fn collect(d: &Directive, root: Option<&str>, ups: &HashMap<String, String>, out: &mut Vec<Location>) {
        for loc in d.all("location") {
            if loc.args.iter().any(|a| a.contains("acme-challenge")) {
                continue;
            }
            out.push(location_info(loc, root, ups));
            collect(loc, loc.first_arg("root").or(root), ups, out);
        }
    }
    collect(d, root, ups, &mut locations);
    let listens: Vec<Listen> = d.all("listen").map(parse_listen).collect();
    ServerBlock {
        line: d.line,
        end_line: d.end_line,
        names_line: d.first("server_name").map(|n| n.line).unwrap_or(d.line),
        names: d.all("server_name").flat_map(|s| s.args.iter().cloned()).collect(),
        listens: if listens.is_empty() { vec![Listen { port: 80, ssl: false }] } else { listens },
        redirect: redirect_target(d),
        ssl_cert: path("ssl_certificate"),
        ssl_key: path("ssl_certificate_key"),
        root: root.map(PathBuf::from),
        access_log: path("access_log"),
        error_log: path("error_log"),
        locations,
    }
}

/// Named `upstream { }` blocks (first server of each), at top level or in `http { }`.
fn upstreams_in(dirs: &[Directive], out: &mut HashMap<String, String>) {
    for d in dirs {
        match d.name.as_str() {
            "upstream" => {
                if let (Some(name), Some(server)) = (d.arg(0), d.first_arg("server")) {
                    out.insert(name.to_string(), server.to_string());
                }
            }
            "http" => upstreams_in(d.block.as_deref().unwrap_or(&[]), out),
            _ => {}
        }
    }
}

/// Group server blocks of one file into sites.
pub fn extract(dirs: &[Directive]) -> Vec<(String, Vec<String>, Vec<ServerBlock>, usize)> {
    extract_with(dirs, &HashMap::new())
}

/// Like `extract`, also resolving upstreams defined in other files (nginx shares
/// them across the whole `http` context).
pub fn extract_with(dirs: &[Directive], global: &HashMap<String, String>) -> Vec<(String, Vec<String>, Vec<ServerBlock>, usize)> {
    let mut ups = global.clone();
    upstreams_in(dirs, &mut ups);
    // servers may be at top level (sites-enabled) or inside http {} (full configs)
    let mut servers = Vec::new();
    for d in dirs {
        match d.name.as_str() {
            "server" => servers.push(server_block(d, &ups)),
            "http" => servers.extend(d.all("server").map(|s| server_block(s, &ups))),
            _ => {}
        }
    }

    let mut groups: Vec<Vec<ServerBlock>> = Vec::new();
    for b in servers {
        let names: Vec<&String> = b.names.iter().filter(|n| *n != "_" && !n.is_empty()).collect();
        let found = groups.iter_mut().find(|g| {
            g.iter().any(|o| o.names.iter().any(|n| names.contains(&n)))
        });
        match found {
            Some(g) if !names.is_empty() => g.push(b),
            _ => groups.push(vec![b]),
        }
    }

    groups
        .into_iter()
        .map(|blocks| {
            let score = |b: &ServerBlock| (b.redirect.is_none() as u8) * 2 + b.is_ssl() as u8;
            let main = (0..blocks.len()).max_by_key(|&i| (score(&blocks[i]), usize::MAX - i)).unwrap_or(0);
            let mut names: Vec<String> = Vec::new();
            for n in blocks[main].names.iter().chain(blocks.iter().flat_map(|b| &b.names)) {
                if n != "_" && !n.is_empty() && !names.contains(n) {
                    names.push(n.clone());
                }
            }
            if names.is_empty() {
                names.push("_".into());
            }
            let domain = names.remove(0);
            (domain, names, blocks, main)
        })
        .collect()
}

// ───────────────────────── loading ─────────────────────────

fn skip_name(name: &str) -> bool {
    name.starts_with('.')
        || name.ends_with('~')
        || [".bak", ".orig", ".swp", ".dpkg-old", ".dpkg-dist", ".dpkg-new", ".save"].iter().any(|s| name.ends_with(s))
}

fn list_dir(dir: &Path) -> Vec<PathBuf> {
    let env = Env::get();
    let mut v: Vec<PathBuf> = std::fs::read_dir(env.host(dir))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !skip_name(n))
                .map(|n| dir.join(n))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

struct Entry {
    config: PathBuf,
    file: PathBuf,
    enabled: bool,
    conf_d: bool,
}

fn entries() -> Vec<Entry> {
    let env = Env::get();
    let mut out = Vec::new();
    let mut targets = Vec::new();
    for link in list_dir(&env.sites_enabled()) {
        let file = env.read_link(&link).unwrap_or_else(|| link.clone());
        targets.push(file.clone());
        out.push(Entry { config: link, file, enabled: true, conf_d: false });
    }
    for f in list_dir(&env.sites_available()) {
        if !targets.contains(&f) && env.host(&f).is_file() {
            out.push(Entry { config: f.clone(), file: f, enabled: false, conf_d: false });
        }
    }
    for f in list_dir(&env.conf_d()) {
        let name = f.to_string_lossy();
        if name.ends_with(".conf") {
            out.push(Entry { config: f.clone(), file: f, enabled: true, conf_d: true });
        } else if name.ends_with(".conf.disabled") {
            out.push(Entry { config: f.clone(), file: f, enabled: false, conf_d: true });
        }
    }
    out
}

pub const MANAGED_MARKER: &str = "# managed by ngx";

pub fn parse_file(config: &Path, file: &Path, enabled: bool, conf_d: bool) -> Vec<Site> {
    parse_file_with(config, file, enabled, conf_d, &HashMap::new())
}

fn parse_file_with(config: &Path, file: &Path, enabled: bool, conf_d: bool, global: &HashMap<String, String>) -> Vec<Site> {
    let env = Env::get();
    let host = env.host(file);
    let base = file
        .file_name()
        .map(|n| n.to_string_lossy().trim_end_matches(".disabled").to_string())
        .unwrap_or_default();
    let mtime = std::fs::metadata(&host).and_then(|m| m.modified()).ok();
    let mk = |domain: String, aliases, blocks, main, err: Option<String>, managed| Site {
        key: format!("{base}#{domain}"),
        id: 0,
        domain,
        aliases,
        config: config.to_path_buf(),
        file: file.to_path_buf(),
        enabled,
        conf_d,
        mtime,
        blocks,
        main,
        managed,
        suspended: None,
        parse_error: err,
    };
    let src = match std::fs::read_to_string(&host) {
        Ok(s) => s,
        Err(e) => {
            let stem = base.trim_end_matches(".conf").to_string();
            return vec![mk(stem, vec![], vec![], 0, Some(format!("cannot read: {e}")), false)];
        }
    };
    let managed = src.lines().take(3).any(|l| l.starts_with(MANAGED_MARKER));
    let suspended = if managed { crate::nginx::template::Spec::from_config(&src).and_then(|s| s.opts.suspended) } else { None };
    match parser::parse(&src) {
        Ok(dirs) => extract_with(&dirs, global)
            .into_iter()
            .map(|(d, a, b, m)| Site { suspended: suspended.clone(), ..mk(d, a, b, m, None, managed) })
            .collect(),
        Err(e) => {
            let stem = base.trim_end_matches(".conf").to_string();
            vec![mk(stem, vec![], vec![], 0, Some(e.to_string()), managed)]
        }
    }
}

/// Load every site (enabled and disabled) with stable IDs assigned.
pub fn load_all() -> Vec<Site> {
    let env = Env::get();
    let entries = entries();
    // upstreams are shared by everything nginx loads: nginx.conf and the enabled files
    let mut global = HashMap::new();
    for f in std::iter::once(env.nginx_conf()).chain(entries.iter().filter(|e| e.enabled).map(|e| e.file.clone())) {
        if let Ok(dirs) = std::fs::read_to_string(env.host(&f)).map_err(|_| ()).and_then(|t| parser::parse(&t).map_err(|_| ())) {
            upstreams_in(&dirs, &mut global);
        }
    }
    let mut sites: Vec<Site> = entries
        .iter()
        .flat_map(|e| parse_file_with(&e.config, &e.file, e.enabled, e.conf_d, &global))
        .collect();
    let keys: Vec<String> = sites.iter().map(|s| s.key.clone()).collect();
    let ids = crate::state::assign_ids(&keys);
    for (s, id) in sites.iter_mut().zip(ids) {
        s.id = id;
    }
    sites.sort_by_key(|s| s.id);
    sites
}

/// Find a site by numeric ID or domain name.
pub fn find<'a>(sites: &'a [Site], query: &str) -> anyhow::Result<&'a Site> {
    let q = query.trim().trim_start_matches('#');
    if let Ok(id) = q.parse::<u32>()
        && let Some(s) = sites.iter().find(|s| s.id == id) {
            return Ok(s);
        }
    sites
        .iter()
        .find(|s| s.matches(q))
        .ok_or_else(|| anyhow::anyhow!("no site matching \"{query}\" (see `ngx ls --all`)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sites(src: &str) -> Vec<(String, Vec<String>, Vec<ServerBlock>, usize)> {
        extract(&parser::parse(src).unwrap())
    }

    #[test]
    fn groups_redirect_and_ssl_blocks() {
        let s = sites(
            r#"
upstream app { server 127.0.0.1:4000; }
server {
    listen 80; listen [::]:80;
    server_name staging.example.com www.staging.example.com;
    return 301 https://$host$request_uri;
}
server {
    listen 443 ssl; listen [::]:443 ssl;
    server_name staging.example.com www.staging.example.com;
    ssl_certificate /etc/letsencrypt/live/staging.example.com/fullchain.pem;
    root /var/www/staging;
    location / { proxy_pass http://app; }
    location /static/ { alias /var/www/staging/public; }
    location /ws { proxy_pass http://127.0.0.1:4000; proxy_set_header Upgrade $http_upgrade; }
}
"#,
        );
        assert_eq!(s.len(), 1);
        let (domain, aliases, blocks, main) = &s[0];
        assert_eq!(domain, "staging.example.com");
        assert_eq!(aliases, &vec!["www.staging.example.com".to_string()]);
        assert_eq!(*main, 1);
        let b = &blocks[*main];
        assert_eq!(b.listens[0], Listen { port: 443, ssl: true });
        assert_eq!(b.upstream(), Some(&Upstream::Tcp { host: "127.0.0.1".into(), port: 4000 }));
        assert_eq!(b.locations[1].action, "files /var/www/staging/public");
        assert!(b.locations[2].action.ends_with("(WebSocket upgrade)"));
        assert_eq!(blocks[0].redirect.as_deref(), Some("https://$host$request_uri"));
    }

    #[test]
    fn certbot_style_redirect_and_fastcgi() {
        let s = sites(
            r#"
server {
    server_name blog.example.com;
    root /var/www/blog;
    location / { try_files $uri $uri/ /index.php; }
    location ~ \.php$ { fastcgi_pass unix:/run/php/php8.3-fpm.sock; }
    listen 443 ssl;
}
server {
    if ($host = blog.example.com) { return 301 https://$host$request_uri; }
    listen 80;
    server_name blog.example.com;
    return 404;
}
"#,
        );
        assert_eq!(s.len(), 1);
        let (_, _, blocks, main) = &s[0];
        assert_eq!(*main, 0);
        assert!(blocks[1].redirect.is_some());
        assert_eq!(blocks[0].locations[1].action, "fastcgi php8.3-fpm.sock");
        assert_eq!(blocks[0].upstream(), Some(&Upstream::Unix("/run/php/php8.3-fpm.sock".into())));
    }

    #[test]
    fn upstream_parsing() {
        let m = HashMap::new();
        assert_eq!(Upstream::parse("http://localhost:3000/api", &m), Upstream::Tcp { host: "localhost".into(), port: 3000 });
        assert_eq!(Upstream::parse("https://backend.internal", &m), Upstream::Tcp { host: "backend.internal".into(), port: 443 });
        assert_eq!(Upstream::parse("http://unix:/tmp/app.sock:/", &m), Upstream::Unix("/tmp/app.sock".into()));
        assert_eq!(Upstream::parse("127.0.0.1:9000", &m), Upstream::Tcp { host: "127.0.0.1".into(), port: 9000 });
        assert!(matches!(Upstream::parse("http://$backend", &m), Upstream::Other(_)));
    }
}
