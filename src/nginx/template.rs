//! Config generation for sites created by `ngx add`.
//!
//! Generated files start with a marker line and the spec as JSON, so ngx can
//! regenerate them later (e.g. when a certificate is issued).

use super::site::MANAGED_MARKER;
use crate::env::Env;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Backend {
    Proxy { host: String, port: u16, websocket: bool },
    Static { root: String },
    Php { socket: String, root: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Ssl {
    None,
    /// Let's Encrypt with the HTTP challenge (served from the ACME webroot).
    LetsEncrypt,
    /// Let's Encrypt with a DNS challenge through a certbot DNS plugin
    /// (`certbot-dns-<plugin>`): works for wildcards and for servers that
    /// aren't reachable on port 80.
    #[serde(rename = "letsencrypt-dns")]
    LetsEncryptDns {
        plugin: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credentials: Option<String>,
    },
    Existing { cert: String, key: String },
}

impl Ssl {
    pub fn is_letsencrypt(&self) -> bool {
        matches!(self, Ssl::LetsEncrypt | Ssl::LetsEncryptDns { .. })
    }
}

/// certbot lineage name for a domain (`*.example.com` → `wildcard.example.com`,
/// so no `*` ends up in paths).
pub fn cert_name(domain: &str) -> String {
    domain.replace('*', "wildcard")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Spec {
    pub domain: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub inbound: u16,
    pub backend: Backend,
    pub ssl: Ssl,
    #[serde(default = "yes")]
    pub redirect_http: bool,
    /// Custom log paths (kept when converting a hand-written config).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_log: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_log: Option<String>,
    #[serde(flatten)]
    pub opts: Options,
}

/// Optional site settings. All default to off, so specs written by older
/// versions read the same.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Options {
    /// `client_max_body_size`, e.g. `50m` (nginx's default is 1m).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_body: Option<String>,
    /// Send `Strict-Transport-Security` (HTTPS sites).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hsts: bool,
    /// Password protection with `ngx.d/<domain>/htpasswd` (users: `ngx auth`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub auth: bool,
    /// Per-client request rate, e.g. `10r/s`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<String>,
    /// Redirect the aliases (e.g. www.) to the main domain.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub canonical: bool,
    /// Common security headers (nosniff, frame and referrer policy).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub headers: bool,
    /// Path the backend answers health checks on (e.g. `/health`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
    /// Extra paths proxied to other local apps (`/api/` → 127.0.0.1:8080).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<Route>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Route {
    /// Normalized to start and end with `/`.
    pub path: String,
    pub host: String,
    pub port: u16,
}

impl Route {
    /// `/api=8080`, `/api=10.0.0.5:8080`
    pub fn parse(s: &str) -> Option<Route> {
        let (path, target) = s.split_once('=')?;
        let (host, port) = match target.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().ok()?),
            None => ("127.0.0.1".to_string(), target.parse().ok()?),
        };
        let mut path = path.trim().to_string();
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
        if !path.ends_with('/') {
            path.push('/');
        }
        Some(Route { path, host, port })
    }
}

impl std::fmt::Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.host == "127.0.0.1" { write!(f, "{}={}", self.path, self.port) } else { write!(f, "{}={}:{}", self.path, self.host, self.port) }
    }
}

/// Per-site directory for snippets and the password file:
/// `<nginx dir>/ngx.d/<domain>/`.
pub fn site_dir(domain: &str) -> PathBuf {
    Env::get().nginx_dir.join("ngx.d").join(domain)
}

pub fn htpasswd_path(domain: &str) -> PathBuf {
    site_dir(domain).join("htpasswd")
}

/// `http2 on;` exists since nginx 1.25.1; older versions take `listen … ssl http2`.
/// Sandbox (tests, demo): the current syntax.
fn http2_directive() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        if Env::get().sandboxed() {
            return true;
        }
        let Some(v) = crate::nginx::ctl::version() else { return false };
        let n: Vec<u32> = v.split('.').filter_map(|p| p.parse().ok()).collect();
        (n.first().copied().unwrap_or(0), n.get(1).copied().unwrap_or(0), n.get(2).copied().unwrap_or(0)) >= (1, 25, 1)
    })
}

fn zone_name(domain: &str) -> String {
    format!("ngx_{}", domain.replace('*', "wildcard"))
}

fn rate_burst(rate: &str) -> u32 {
    rate.split('r').next().and_then(|n| n.parse::<u32>().ok()).map(|n| (n * 2).max(10)).unwrap_or(10)
}

fn yes() -> bool {
    true
}

pub const LOG_FORMAT_FILE: &str = "00-ngx-logformat.conf";
pub const LOG_FORMAT: &str = "# managed by ngx — shared log format with request timing\n\
log_format ngx '$remote_addr - $remote_user [$time_local] \"$request\" $status $body_bytes_sent \"$http_referer\" \"$http_user_agent\" rt=$request_time';\n";

const SPEC_PREFIX: &str = "# ngx-spec: ";

impl Spec {
    pub fn file_name(&self) -> String {
        format!("{}.conf", self.domain)
    }
    pub fn names(&self) -> String {
        std::iter::once(&self.domain).chain(&self.aliases).cloned().collect::<Vec<_>>().join(" ")
    }
    pub fn cert_paths(&self) -> Option<(String, String)> {
        match &self.ssl {
            Ssl::None => None,
            Ssl::LetsEncrypt | Ssl::LetsEncryptDns { .. } => {
                let live = Env::get().le_dir.join("live").join(cert_name(&self.domain));
                Some((
                    live.join("fullchain.pem").display().to_string(),
                    live.join("privkey.pem").display().to_string(),
                ))
            }
            Ssl::Existing { cert, key } => Some((cert.clone(), key.clone())),
        }
    }
    /// Read the spec embedded in a managed config file.
    pub fn from_config(src: &str) -> Option<Spec> {
        src.lines().take(5).find_map(|l| l.strip_prefix(SPEC_PREFIX)).and_then(|j| serde_json::from_str(j).ok())
    }
    pub fn log_paths(&self) -> (PathBuf, PathBuf) {
        let d = &Env::get().log_dir;
        (
            self.access_log.as_ref().map(PathBuf::from).unwrap_or_else(|| d.join(format!("{}.access.log", self.domain))),
            self.error_log.as_ref().map(PathBuf::from).unwrap_or_else(|| d.join(format!("{}.error.log", self.domain))),
        )
    }
}

fn proxy_headers(out: &mut String, indent: &str) {
    for h in ["Host $host", "X-Real-IP $remote_addr", "X-Forwarded-For $proxy_add_x_forwarded_for", "X-Forwarded-Proto $scheme"] {
        out.push_str(&format!("{indent}proxy_set_header {h};\n"));
    }
}

/// The server body shared by the HTTP and HTTPS blocks. `tls`: inside the
/// HTTPS block; `port`: the block's public port.
fn body(spec: &Spec, out: &mut String, tls: bool, port: u16) {
    let o = &spec.opts;
    if tls && o.hsts {
        out.push_str("    add_header Strict-Transport-Security \"max-age=31536000\" always;\n");
    }
    if o.headers {
        out.push_str("    add_header X-Content-Type-Options \"nosniff\" always;\n");
        out.push_str("    add_header X-Frame-Options \"SAMEORIGIN\" always;\n");
        out.push_str("    add_header Referrer-Policy \"strict-origin-when-cross-origin\" always;\n");
    }
    if (tls && o.hsts) || o.headers {
        out.push('\n');
    }
    if o.canonical && !spec.aliases.is_empty() {
        let scheme = if tls { "https" } else { "http" };
        let default = if tls { 443 } else { 80 };
        let p = if port == default { String::new() } else { format!(":{port}") };
        out.push_str(&format!("    # aliases redirect to the main domain\n    if ($host != {}) {{\n        return 301 {scheme}://{}{p}$request_uri;\n    }}\n\n", spec.domain, spec.domain));
    }
    let (access, error) = spec.log_paths();
    out.push_str(&format!("    access_log {} ngx;\n    error_log  {};\n\n", access.display(), error.display()));
    let mut extra = false;
    if let Some(b) = &o.max_body {
        out.push_str(&format!("    client_max_body_size {b};\n"));
        extra = true;
    }
    if let Some(r) = &o.rate_limit {
        out.push_str(&format!("    limit_req zone={} burst={} nodelay;\n    limit_req_status 429;\n", zone_name(&spec.domain), rate_burst(r)));
        extra = true;
    }
    if o.auth {
        out.push_str(&format!("    auth_basic \"Restricted\";\n    auth_basic_user_file {};\n", htpasswd_path(&spec.domain).display()));
        extra = true;
    }
    if extra {
        out.push('\n');
    }
    out.push_str(&format!("    # your own directives: {}/*.conf (kept when ngx regenerates this file)\n", site_dir(&spec.domain).display()));
    out.push_str(&format!("    include {}/*.conf;\n\n", site_dir(&spec.domain).display()));
    for r in &o.routes {
        out.push_str(&format!("    location ^~ {} {{\n        proxy_pass http://{}:{};\n        proxy_http_version 1.1;\n", r.path, r.host, r.port));
        proxy_headers(out, "        ");
        out.push_str("        proxy_set_header Connection \"\";\n    }\n\n");
    }
    match &spec.backend {
        Backend::Proxy { host, port, websocket } => {
            out.push_str("    location / {\n");
            out.push_str(&format!("        proxy_pass http://{host}:{port};\n"));
            out.push_str("        proxy_http_version 1.1;\n");
            proxy_headers(out, "        ");
            if *websocket {
                out.push_str("        proxy_set_header Upgrade $http_upgrade;\n");
                out.push_str("        proxy_set_header Connection \"upgrade\";\n");
                out.push_str("        proxy_read_timeout 3600s;\n");
            } else {
                out.push_str("        proxy_set_header Connection \"\";\n");
            }
            out.push_str("    }\n");
        }
        Backend::Static { root } => {
            out.push_str(&format!("    root {root};\n    index index.html index.htm;\n\n"));
            deny_hidden(out);
            out.push_str("    location / {\n        try_files $uri $uri/ =404;\n    }\n");
        }
        Backend::Php { socket, root } => {
            out.push_str(&format!("    root {root};\n    index index.php index.html;\n\n"));
            // before the PHP location: regex locations match in order
            deny_hidden(out);
            out.push_str("    location / {\n        try_files $uri $uri/ /index.php?$query_string;\n    }\n\n");
            out.push_str("    location ~ \\.php$ {\n");
            // only run scripts that exist, so /upload.jpg/x.php can't execute an upload
            out.push_str("        try_files $fastcgi_script_name =404;\n");
            out.push_str("        include fastcgi_params;\n");
            out.push_str("        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n");
            out.push_str(&format!("        fastcgi_pass unix:{socket};\n"));
            out.push_str("    }\n");
        }
    }
}

/// Hidden files (.git, .env, .htpasswd…) are never served.
fn deny_hidden(out: &mut String) {
    out.push_str("    location ~ /\\.(?!well-known/) {\n        deny all;\n    }\n\n");
}

fn acme(out: &mut String) {
    out.push_str(&format!(
        "    location ^~ /.well-known/acme-challenge/ {{\n        root {};\n        default_type text/plain;\n        auth_basic off;\n    }}\n\n",
        Env::get().acme_webroot.display()
    ));
}

/// Render the config. With `bootstrap`, a Let's Encrypt site gets an HTTP-only
/// config so the ACME challenge can be served before the certificate exists.
pub fn render(spec: &Spec, bootstrap: bool) -> String {
    let now = chrono::Local::now().format("%d/%m/%y %H:%M");
    let mut o = format!("{MANAGED_MARKER} — generated {now}. Edit with `ngx edit`.\n");
    o.push_str(SPEC_PREFIX);
    o.push_str(&serde_json::to_string(spec).unwrap_or_default());
    o.push_str("\n\n");
    if let Some(r) = &spec.opts.rate_limit {
        o.push_str(&format!("limit_req_zone $binary_remote_addr zone={}:10m rate={r};\n\n", zone_name(&spec.domain)));
    }
    let names = spec.names();
    let certs = if bootstrap { None } else { spec.cert_paths() };

    match certs {
        None => {
            let port = if spec.cert_paths().is_some() { 80 } else { spec.inbound };
            o.push_str(&format!("server {{\n    listen {port};\n    listen [::]:{port};\n    server_name {names};\n\n"));
            acme(&mut o);
            body(spec, &mut o, false, port);
            o.push_str("}\n");
        }
        Some((cert, key)) => {
            let port = spec.inbound;
            if port != 80 {
                o.push_str(&format!("server {{\n    listen 80;\n    listen [::]:80;\n    server_name {names};\n\n"));
                acme(&mut o);
                if spec.redirect_http {
                    let host = if spec.opts.canonical { spec.domain.as_str() } else { "$host" };
                    let target = if port == 443 { format!("https://{host}$request_uri") } else { format!("https://{host}:{port}$request_uri") };
                    o.push_str(&format!("    location / {{\n        return 301 {target};\n    }}\n"));
                } else {
                    body(spec, &mut o, false, 80);
                }
                o.push_str("}\n\n");
            }
            if http2_directive() {
                o.push_str(&format!("server {{\n    listen {port} ssl;\n    listen [::]:{port} ssl;\n    http2 on;\n    server_name {names};\n\n"));
            } else {
                // nginx before 1.25.1 has no `http2` directive
                o.push_str(&format!("server {{\n    listen {port} ssl http2;\n    listen [::]:{port} ssl http2;\n    server_name {names};\n\n"));
            }
            o.push_str(&format!("    ssl_certificate     {cert};\n    ssl_certificate_key {key};\n"));
            o.push_str("    ssl_protocols TLSv1.2 TLSv1.3;\n    ssl_session_cache shared:ngx_ssl:10m;\n    ssl_session_timeout 1d;\n\n");
            body(spec, &mut o, true, port);
            o.push_str("}\n");
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nginx::{parser, site};

    fn spec(ssl: Ssl) -> Spec {
        Spec {
            domain: "app.example.com".into(),
            aliases: vec!["www.app.example.com".into()],
            inbound: 443,
            backend: Backend::Proxy { host: "127.0.0.1".into(), port: 3000, websocket: true },
            ssl,
            redirect_http: true,
            access_log: None,
            error_log: None,
            opts: Options::default(),
        }
    }

    #[test]
    fn generated_config_round_trips() {
        let s = spec(Ssl::LetsEncrypt);
        let src = render(&s, false);
        assert_eq!(Spec::from_config(&src), Some(s.clone()));
        let sites = site::extract(&parser::parse(&src).unwrap());
        assert_eq!(sites.len(), 1);
        let (domain, aliases, blocks, main) = &sites[0];
        assert_eq!(domain, "app.example.com");
        assert_eq!(aliases.len(), 1);
        assert!(blocks[*main].is_ssl());
        assert!(blocks.iter().any(|b| b.redirect.is_some()));
        assert_eq!(blocks[*main].upstream().unwrap().to_string(), "127.0.0.1:3000");
    }

    #[test]
    fn php_and_static_are_hardened() {
        let php = render(&Spec { backend: Backend::Php { socket: "/run/php/php-fpm.sock".into(), root: "/var/www/x".into() }, ..spec(Ssl::None) }, false);
        let (deny, fpm) = (php.find("deny all").unwrap(), php.find("location ~ \\.php$").unwrap());
        assert!(deny < fpm, "hidden files must be denied before the PHP location");
        assert!(php.contains("try_files $fastcgi_script_name =404;"));
        let st = render(&Spec { backend: Backend::Static { root: "/var/www/x".into() }, ..spec(Ssl::None) }, false);
        assert!(st.contains("deny all"));
    }

    #[test]
    fn injection_is_rejected() {
        use crate::validate;
        let base = spec(Ssl::None);
        let bad = [
            Spec { domain: "x.com; include /etc/shadow".into(), ..base.clone() },
            Spec { domain: "../../etc/cron.d/x".into(), ..base.clone() },
            Spec { aliases: vec!["a.com;\n location / { root / ; }".into()], ..base.clone() },
            Spec { backend: Backend::Static { root: "/var/www/x; autoindex on".into() }, ..base.clone() },
            Spec { backend: Backend::Static { root: "/".into() }, ..base.clone() },
            Spec { backend: Backend::Static { root: "/etc/nginx".into() }, ..base.clone() },
            Spec { backend: Backend::Proxy { host: "127.0.0.1;".into(), port: 1, websocket: false }, ..base.clone() },
            Spec { backend: Backend::Php { socket: "/run/x.sock;".into(), root: "/var/www/x".into() }, ..base.clone() },
            Spec { ssl: Ssl::Existing { cert: "/a b".into(), key: "/k".into() }, ..base.clone() },
            Spec { access_log: Some("/var/log/x;".into()), ..base.clone() },
            Spec { aliases: vec!["*.x.com".into()], ..spec(Ssl::LetsEncrypt) },
        ];
        for s in &bad {
            assert!(validate::spec(s).is_err(), "{s:?}");
        }
        assert!(validate::spec(&base).is_ok());
    }

    #[test]
    fn bootstrap_is_http_only() {
        let src = render(&spec(Ssl::LetsEncrypt), true);
        assert!(!src.contains("ssl_certificate"));
        assert!(src.contains("acme-challenge"));
        assert!(parser::parse(&src).is_ok());
        let plain = render(&Spec { inbound: 8080, ..spec(Ssl::None) }, false);
        assert!(plain.contains("listen 8080;"));
    }
}
