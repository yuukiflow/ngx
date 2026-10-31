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
    LetsEncrypt,
    Existing { cert: String, key: String },
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
            Ssl::LetsEncrypt => {
                let live = Env::get().le_dir.join("live").join(&self.domain);
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

fn body(spec: &Spec, out: &mut String) {
    let (access, error) = spec.log_paths();
    out.push_str(&format!("    access_log {} ngx;\n    error_log  {};\n\n", access.display(), error.display()));
    match &spec.backend {
        Backend::Proxy { host, port, websocket } => {
            out.push_str("    location / {\n");
            out.push_str(&format!("        proxy_pass http://{host}:{port};\n"));
            out.push_str("        proxy_http_version 1.1;\n");
            out.push_str("        proxy_set_header Host $host;\n");
            out.push_str("        proxy_set_header X-Real-IP $remote_addr;\n");
            out.push_str("        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n");
            out.push_str("        proxy_set_header X-Forwarded-Proto $scheme;\n");
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
            out.push_str("    location / {\n        try_files $uri $uri/ =404;\n    }\n");
        }
        Backend::Php { socket, root } => {
            out.push_str(&format!("    root {root};\n    index index.php index.html;\n\n"));
            out.push_str("    location / {\n        try_files $uri $uri/ /index.php?$query_string;\n    }\n\n");
            out.push_str("    location ~ \\.php$ {\n");
            out.push_str("        include fastcgi_params;\n");
            out.push_str("        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n");
            out.push_str(&format!("        fastcgi_pass unix:{socket};\n"));
            out.push_str("    }\n\n");
            out.push_str("    location ~ /\\.(?!well-known) {\n        deny all;\n    }\n");
        }
    }
}

fn acme(out: &mut String) {
    out.push_str(&format!(
        "    location ^~ /.well-known/acme-challenge/ {{\n        root {};\n        default_type text/plain;\n    }}\n\n",
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
    let names = spec.names();
    let certs = if bootstrap { None } else { spec.cert_paths() };

    match certs {
        None => {
            let port = if spec.cert_paths().is_some() { 80 } else { spec.inbound };
            o.push_str(&format!("server {{\n    listen {port};\n    listen [::]:{port};\n    server_name {names};\n\n"));
            acme(&mut o);
            body(spec, &mut o);
            o.push_str("}\n");
        }
        Some((cert, key)) => {
            let port = spec.inbound;
            if port != 80 {
                o.push_str(&format!("server {{\n    listen 80;\n    listen [::]:80;\n    server_name {names};\n\n"));
                acme(&mut o);
                if spec.redirect_http {
                    let target = if port == 443 { "https://$host$request_uri".to_string() } else { format!("https://$host:{port}$request_uri") };
                    o.push_str(&format!("    location / {{\n        return 301 {target};\n    }}\n"));
                } else {
                    body(spec, &mut o);
                }
                o.push_str("}\n\n");
            }
            o.push_str(&format!("server {{\n    listen {port} ssl;\n    listen [::]:{port} ssl;\n    server_name {names};\n\n"));
            o.push_str(&format!("    ssl_certificate     {cert};\n    ssl_certificate_key {key};\n"));
            o.push_str("    ssl_protocols TLSv1.2 TLSv1.3;\n    ssl_session_cache shared:ngx_ssl:10m;\n    ssl_session_timeout 1d;\n\n");
            body(spec, &mut o);
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
    fn bootstrap_is_http_only() {
        let src = render(&spec(Ssl::LetsEncrypt), true);
        assert!(!src.contains("ssl_certificate"));
        assert!(src.contains("acme-challenge"));
        assert!(parser::parse(&src).is_ok());
        let plain = render(&Spec { inbound: 8080, ..spec(Ssl::None) }, false);
        assert!(plain.contains("listen 8080;"));
    }
}
