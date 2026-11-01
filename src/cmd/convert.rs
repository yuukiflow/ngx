//! Converting a hand-written site into an ngx-managed one without losing what
//! the generator doesn't produce: known settings become options, everything
//! else is moved verbatim into a snippet (`ngx.d/<domain>/converted.conf`),
//! and what can't be kept (directives inside `location /`) is listed.

#[allow(unused_imports)]
use super::*;
use crate::nginx::parser::Directive;
use crate::nginx::site::ServerBlock;
use crate::nginx::template::{Options, Route};

#[derive(Debug, Default)]
pub(crate) struct Conversion {
    pub opts: Options,
    /// Server-level directives and locations to keep, as they were written.
    pub snippet: Vec<String>,
    /// What can't be kept, for a warning.
    pub lost: Vec<String>,
}

impl Conversion {
    pub fn snippet_text(&self, file: &Path) -> Option<String> {
        (!self.snippet.is_empty()).then(|| {
            format!(
                "# Moved here by `ngx edit` from {} when it was converted to a generated config:\n# the directives ngx doesn't generate itself. Edit with `ngx edit <site> --snippet converted`.\n\n{}\n",
                file.display(),
                self.snippet.join("\n\n")
            )
        })
    }
}

/// Server-level directives the generated config already covers.
const COVERED: &[&str] = &[
    "listen", "server_name", "ssl_certificate", "ssl_certificate_key", "ssl_protocols", "ssl_session_cache", "ssl_session_timeout",
    "ssl_session_tickets", "ssl_ciphers", "ssl_prefer_server_ciphers", "ssl_dhparam", "http2", "access_log", "error_log", "root",
    "index",
];

/// Directives in `location /` (or the PHP location) that the generated one has too.
const LOCATION_COVERED: &[&str] = &[
    "proxy_pass", "proxy_http_version", "proxy_set_header", "proxy_read_timeout", "try_files", "root", "index", "include",
    "fastcgi_pass", "fastcgi_param", "fastcgi_index", "fastcgi_split_path_info",
];

fn find_server(dirs: &[Directive], line: usize) -> Option<&Directive> {
    for d in dirs {
        if d.name == "server" && d.line == line {
            return Some(d);
        }
        if let Some(b) = &d.block
            && let Some(f) = find_server(b, line) {
                return Some(f);
            }
    }
    None
}

fn text(lines: &[&str], d: &Directive) -> String {
    let from = d.line.saturating_sub(1);
    let to = d.end_line.min(lines.len());
    let chunk = &lines[from..to.max(from + 1).min(lines.len())];
    // dedent to the directive's own indentation
    let indent = chunk.first().map(|l| l.len() - l.trim_start().len()).unwrap_or(0);
    chunk.iter().map(|l| if l.len() >= indent && l[..indent].trim().is_empty() { &l[indent..] } else { l.trim_start() }).collect::<Vec<_>>().join("\n")
}

fn matcher(d: &Directive) -> String {
    d.args.join(" ")
}

/// A location that only proxies to `http://host:port` (plus the usual headers) becomes a route.
fn as_route(d: &Directive) -> Option<Route> {
    let path = match d.args.as_slice() {
        [p] | [_, p] if d.args.len() == 1 || d.args[0] == "^~" => p.clone(),
        _ => return None,
    };
    let body = d.block.as_ref()?;
    if !body.iter().all(|c| matches!(c.name.as_str(), "proxy_pass" | "proxy_set_header" | "proxy_http_version")) {
        return None;
    }
    let target = body.iter().find(|c| c.name == "proxy_pass")?.args.first()?.strip_prefix("http://")?;
    if target.contains('/') && target.find('/') != Some(target.len() - 1) {
        return None; // proxy_pass with a URI rewrites paths: keep it verbatim
    }
    let r = Route::parse(&format!("{path}={}", target.trim_end_matches('/')))?;
    (validate::route_path(&r.path) && validate::host(&r.host) && r.port > 0 && path.ends_with('/')).then_some(r)
}

pub(crate) fn convert(src: &str, main: &ServerBlock) -> Conversion {
    let mut c = Conversion::default();
    let Ok(dirs) = crate::nginx::parser::parse(src) else { return c };
    let Some(server) = find_server(&dirs, main.line) else { return c };
    let lines: Vec<&str> = src.lines().collect();
    for d in server.block.as_deref().unwrap_or(&[]) {
        let name = d.name.as_str();
        if COVERED.contains(&name) {
            continue;
        }
        match name {
            "client_max_body_size" if d.args.len() == 1 && validate::size(&d.args[0]) => c.opts.max_body = Some(d.args[0].clone()),
            "add_header" if d.args.first().is_some_and(|h| h.eq_ignore_ascii_case("Strict-Transport-Security")) => c.opts.hsts = true,
            // certbot's own includes, regenerated with the TLS settings
            "include" if d.args.first().is_some_and(|a| a.contains("letsencrypt/options-ssl-nginx") || a.contains("fastcgi")) => {}
            "location" => {
                let m = matcher(d);
                let body = d.block.as_deref().unwrap_or(&[]);
                if m.contains("acme-challenge") || (m.contains("/\\.") && body.iter().any(|x| x.name == "deny")) {
                    continue; // generated too
                }
                if m == "/" || m.contains("\\.php") {
                    for x in body.iter().filter(|x| !LOCATION_COVERED.contains(&x.name.as_str())) {
                        c.lost.push(format!("`{}` inside `location {m}` (line {})", x.name, x.line));
                    }
                    continue;
                }
                match as_route(d) {
                    Some(r) if !c.opts.routes.iter().any(|x| x.path == r.path) => c.opts.routes.push(r),
                    _ => c.snippet.push(text(&lines, d)),
                }
            }
            _ => c.snippet.push(text(&lines, d)),
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_what_the_generator_does_not_make() {
        let src = "server {\n    listen 80;\n    server_name a.test;\n    client_max_body_size 20m;\n    gzip on;\n    add_header X-Test 1;\n    location / {\n        proxy_pass http://127.0.0.1:3000;\n        proxy_buffering off;\n    }\n    location /api/ {\n        proxy_pass http://127.0.0.1:8080;\n        proxy_set_header Host $host;\n    }\n    location /static/ {\n        alias /srv/static/;\n        expires 7d;\n    }\n}\n";
        let sites = crate::nginx::site::extract(&crate::nginx::parser::parse(src).unwrap());
        let (_, _, blocks, main) = &sites[0];
        let c = convert(src, &blocks[*main]);
        assert_eq!(c.opts.max_body.as_deref(), Some("20m"));
        assert_eq!(c.opts.routes.len(), 1);
        assert_eq!(c.opts.routes[0].port, 8080);
        assert_eq!(c.snippet.len(), 3, "{:?}", c.snippet);
        assert!(c.snippet[2].starts_with("location /static/ {") && c.snippet[2].contains("expires 7d;"));
        assert_eq!(c.lost.len(), 1);
        assert!(c.lost[0].contains("proxy_buffering"));
    }
}
