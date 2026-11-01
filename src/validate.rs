//! Input validation for everything that ends up in a generated nginx config,
//! a file name, or a certbot command line.
//!
//! Generated configs are assembled from strings, so every value must be
//! impossible to break out of: no whitespace, quotes, `;`, `{`, `}`, `#`, `$`
//! or control characters (which would let a value add directives), no `..`
//! (which would let a domain escape the config directory), and nothing that
//! starts with `-` (which certbot would read as an option).

use crate::env::Env;
use crate::nginx::template::{Backend, Spec, Ssl};
use anyhow::{bail, Result};
use std::net::IpAddr;
use std::path::Path;

/// A DNS name: labels of `a-z 0-9 -`, not starting or ending with `-`;
/// `*` only as the whole first label.
pub fn domain(d: &str) -> bool {
    if d.is_empty() || d.len() > 253 {
        return false;
    }
    d.split('.').enumerate().all(|(i, l)| {
        (i == 0 && l == "*" && d.len() > 2)
            || (!l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
    })
}

/// Upstream host: an IP address or a plain host name.
pub fn host(h: &str) -> bool {
    let bare = h.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(h);
    bare.parse::<IpAddr>().is_ok() || (!h.contains('*') && domain(h))
}

/// An absolute path made only of characters that can't change the meaning of
/// an nginx directive, with no `.`/`..` components.
pub fn path(p: &str) -> bool {
    p.starts_with('/')
        && p.len() < 4096
        && p.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+@=,~:".contains(c))
        && !p.split('/').any(|c| c == ".." || c == ".")
}

/// certbot DNS plugin name, e.g. `cloudflare` for certbot-dns-cloudflare.
pub fn dns_plugin(p: &str) -> bool {
    !p.is_empty() && p.len() <= 32 && !p.starts_with('-') && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// nginx size: `512k`, `50m`, `1g`, `0` (no limit).
pub fn size(s: &str) -> bool {
    let digits = s.trim_end_matches(['k', 'K', 'm', 'M', 'g', 'G']);
    !digits.is_empty() && digits.len() <= 6 && s.len() - digits.len() <= 1 && digits.chars().all(|c| c.is_ascii_digit())
}

/// nginx request rate: `10r/s`, `300r/m`.
pub fn rate(s: &str) -> bool {
    s.strip_suffix("r/s").or_else(|| s.strip_suffix("r/m")).is_some_and(|n| !n.is_empty() && n.len() <= 6 && n.chars().all(|c| c.is_ascii_digit()) && n != "0")
}

/// Location prefix for an extra route: `/api/`.
pub fn route_path(p: &str) -> bool {
    p.len() > 2
        && p.starts_with('/')
        && p.ends_with('/')
        && p.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c))
        && !p.split('/').any(|c| c == ".." || c == ".")
        && !p.starts_with("/.well-known/")
}

/// Health check path: `/health`, `/api/status`.
pub fn health_path(p: &str) -> bool {
    p.starts_with('/') && p.len() <= 256 && p.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)) && !p.split('/').any(|c| c == "..")
}

/// Notification URL: http(s), and nothing a unit file, crontab or shell would
/// interpret (no whitespace, quotes, `$`, `;`, backslash or backtick).
pub fn url(u: &str) -> bool {
    (u.starts_with("https://") || u.starts_with("http://"))
        && u.len() > 8
        && u.len() < 2048
        && u.chars().all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!&()*+,=%".contains(c))
}

/// Rough email check, mostly to keep it a single safe certbot argument.
pub fn email(e: &str) -> bool {
    let Some((user, dom)) = e.split_once('@') else { return false };
    !user.is_empty()
        && !user.starts_with('-')
        && user.chars().all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
        && domain(&dom.to_ascii_lowercase())
        && !dom.starts_with('*')
}

/// Directories that must never be published as a document root.
fn sensitive_root(root: &Path) -> Option<&'static str> {
    let env = Env::get();
    let r = root.to_string_lossy();
    let r = r.trim_end_matches('/');
    if r.is_empty() {
        return Some("is the whole filesystem");
    }
    for own in [&env.nginx_dir, &env.le_dir, &env.state_dir, &env.log_dir] {
        if root.starts_with(own) || own.starts_with(root) {
            return Some("contains nginx, certificate or ngx files");
        }
    }
    const UNDER: [&str; 17] = [
        "/etc", "/root", "/proc", "/sys", "/dev", "/boot", "/run", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/usr",
        "/var/lib", "/var/log", "/var/spool", "/var/backups",
    ];
    if UNDER.iter().any(|d| root.starts_with(d)) {
        return Some("is a system directory");
    }
    const EXACT: [&str; 7] = ["/home", "/var", "/opt", "/srv", "/mnt", "/media", "/tmp"];
    if EXACT.contains(&r) {
        return Some("is a top-level directory");
    }
    if root.parent() == Some(Path::new("/home")) {
        return Some("is a whole home directory (use a subdirectory, e.g. ~/www)");
    }
    None
}

fn port(p: u16, what: &str) -> Result<()> {
    if p == 0 {
        bail!("{what} port must be between 1 and 65535");
    }
    Ok(())
}

/// Check a complete spec before it's rendered and written.
pub fn spec(s: &Spec) -> Result<()> {
    for n in std::iter::once(&s.domain).chain(&s.aliases) {
        if !domain(n) {
            bail!("invalid domain name: {n:?} (letters, digits and '-' only; '*' only as the first label)");
        }
    }
    port(s.inbound, "inbound")?;
    let check_path = |p: &str, what: &str| -> Result<()> {
        if !path(p) {
            bail!("invalid {what}: {p:?} (must be an absolute path without spaces, quotes or ; {{ }} $ # characters)");
        }
        Ok(())
    };
    match &s.backend {
        Backend::Proxy { host: h, port: p, .. } => {
            if !host(h) {
                bail!("invalid upstream host: {h:?} (an IP address or host name)");
            }
            port(*p, "local")?;
        }
        Backend::Static { root } => check_path(root, "document root")?,
        Backend::Php { socket, root } => {
            check_path(socket, "php-fpm socket")?;
            check_path(root, "document root")?;
        }
    }
    if let Backend::Static { root } | Backend::Php { root, .. } = &s.backend
        && let Some(why) = sensitive_root(Path::new(root)) {
            bail!("refusing to publish {root} as a document root: it {why}");
        }
    match &s.ssl {
        Ssl::None => {}
        Ssl::LetsEncrypt => {
            if let Some(w) = std::iter::once(&s.domain).chain(&s.aliases).find(|n| n.starts_with('*')) {
                bail!("{w}: wildcard certificates need the DNS challenge: use --dns <plugin> (e.g. --dns cloudflare)");
            }
        }
        Ssl::LetsEncryptDns { plugin, credentials } => {
            if !dns_plugin(plugin) {
                bail!("invalid DNS plugin {plugin:?} (the part after certbot-dns-, e.g. cloudflare, route53, digitalocean)");
            }
            if let Some(c) = credentials {
                check_path(c, "DNS credentials path")?;
            }
        }
        Ssl::Existing { cert, key } => {
            check_path(cert, "certificate path")?;
            check_path(key, "private key path")?;
        }
    }
    for l in s.access_log.iter().chain(&s.error_log) {
        check_path(l, "log path")?;
    }
    let o = &s.opts;
    if let Some(b) = &o.max_body
        && !size(b) {
            bail!("invalid upload size {b:?} (e.g. 10m, 512k, 1g; 0 for no limit)");
        }
    if let Some(r) = &o.rate_limit
        && !rate(r) {
            bail!("invalid rate limit {r:?} (requests per client, e.g. 10r/s or 300r/m)");
        }
    if o.hsts && s.ssl == Ssl::None {
        bail!("HSTS needs HTTPS: choose an SSL mode or turn HSTS off");
    }
    if let Some(h) = &o.health {
        if !health_path(h) {
            bail!("invalid health check path {h:?} (e.g. /health)");
        }
        if !matches!(s.backend, Backend::Proxy { .. }) {
            bail!("health checks are for reverse proxy sites (they call the app directly)");
        }
    }
    let mut seen = Vec::new();
    for r in &o.routes {
        if !route_path(&r.path) {
            bail!("invalid path {:?} for an extra route (e.g. /api/: letters, digits and / . _ - only)", r.path);
        }
        if !host(&r.host) {
            bail!("invalid upstream host {:?} for {}", r.host, r.path);
        }
        port(r.port, "route")?;
        if seen.contains(&&r.path) {
            bail!("{} is routed twice", r.path);
        }
        seen.push(&r.path);
    }
    Ok(())
}

/// Warnings about a private key file that others can read.
pub fn key_warning(key: &str) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(Env::get().host(Path::new(key))).ok()?;
    if m.mode() & 0o004 != 0 {
        Some(format!("{key} is readable by every user on this machine — run `chmod 600 {key}`"))
    } else if m.uid() != 0 && !Env::get().sandboxed() {
        Some(format!("{key} is owned by uid {} (not root) — that user can read or replace the key", m.uid()))
    } else {
        None
    }
}

/// Make external text (logs, process output, config contents) safe to print to
/// a terminal: control characters and bidi overrides are shown escaped.
pub fn clean(s: &str) -> std::borrow::Cow<'_, str> {
    let bad = |c: char| (c.is_control() && c != '\t') || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    if !s.chars().any(bad) {
        return s.into();
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if c == '\t' {
            out.push(' ');
        } else if bad(c) {
            out.push_str(&format!("\\x{{{:x}}}", c as u32));
        } else {
            out.push(c);
        }
    }
    out.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains() {
        for ok in ["example.com", "a.b-c.example", "*.example.com", "localhost", "x1.io"] {
            assert!(domain(ok), "{ok}");
        }
        for bad in [
            "", "-x.com", "x-.com", "a..b", "a b.com", "x.com;", "evil.com; include /etc/shadow", "../../etc/x",
            "a/b", "*", "a.*.com", "x\n.com", "EXAMPLE.com", "x{.com",
        ] {
            assert!(!domain(bad), "{bad:?}");
        }
    }

    #[test]
    fn hosts_and_paths() {
        assert!(host("127.0.0.1") && host("[::1]") && host("::1") && host("backend.internal"));
        assert!(!host("1.2.3.4;") && !host("x y") && !host("*.x.com"));
        assert!(path("/var/www/site") && path("/run/php/php8.3-fpm.sock") && path("/etc/ssl/a@b.pem"));
        for bad in ["relative", "/a b", "/a;b", "/a{", "/a/../etc", "/a/./b", "/a\"", "/a'b", "/$x", "/a#b", "/a\nb", "/a\\b"] {
            assert!(!path(bad), "{bad:?}");
        }
        assert!(email("me@example.com") && !email("-x@example.com") && !email("a b@x.com") && !email("me"));
        assert!(size("50m") && size("0") && size("512k") && !size("m") && !size("5mb") && !size("1;"));
        assert!(rate("10r/s") && rate("300r/m") && !rate("0r/s") && !rate("r/s") && !rate("10r/h"));
        assert!(route_path("/api/") && !route_path("/") && !route_path("/a b/") && !route_path("/../") && !route_path("/.well-known/acme-challenge/"));
        assert!(dns_plugin("cloudflare") && !dns_plugin("-x") && !dns_plugin("a b"));
    }

    #[test]
    fn terminal_text() {
        assert_eq!(clean("plain"), "plain");
        assert_eq!(clean("a\x1b[31mb"), "a\\x{1b}[31mb");
        assert_eq!(clean("x\u{202e}y"), "x\\x{202e}y");
    }
}
