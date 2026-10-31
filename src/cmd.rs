//! Mutating and utility commands.

use crate::env::{self, require_root, Env};
use crate::nginx::site::{self, Kind, Site, Upstream, MANAGED_MARKER};
use crate::nginx::template::{self, Backend, Spec, Ssl};
use crate::nginx::{ctl, parser};
use crate::ops::Tx;
use crate::render::{self, Tone};
use crate::ssl;
use crate::status::Report;
use anyhow::{bail, Context, Result};
use inquire::validator::Validation;
use inquire::{Confirm, CustomType, Select, Text};
use std::path::{Path, PathBuf};

fn ok(msg: &str) {
    render::print(&[vec![render::seg("✓ ", Tone::Good), render::seg(msg, Tone::Plain)]]);
}

fn confirm(q: &str, default: bool, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    Ok(Confirm::new(q).with_default(default).prompt()?)
}

// ───────────────────────── layout ─────────────────────────

fn nginx_conf_includes_sites() -> bool {
    let env = Env::get();
    std::fs::read_to_string(env.host(&env.nginx_conf()))
        .map(|s| s.lines().any(|l| !l.trim_start().starts_with('#') && l.contains("sites-enabled")))
        .unwrap_or(false)
}

/// Make sure sites-available/sites-enabled exist and are included. Returns
/// the directory new configs should go to and whether they need a symlink.
pub fn ensure_layout(yes: bool, dry_run: bool) -> Result<(PathBuf, bool)> {
    let env = Env::get();
    if env.has_sites_layout() && nginx_conf_includes_sites() {
        return Ok((env.sites_available(), true));
    }
    let conf_host = env.host(&env.nginx_conf());
    if !conf_host.exists() {
        bail!("{} not found — is nginx installed? (set NGX_NGINX_DIR if it lives elsewhere)", env.nginx_conf().display());
    }
    let q = format!(
        "nginx is not using {}/{{sites-available,sites-enabled}}. Create them and include sites-enabled from nginx.conf?",
        env.nginx_dir.display()
    );
    if !confirm(&q, true, yes)? {
        println!("ok — writing configs to {} instead", env.conf_d().display());
        return Ok((env.conf_d(), false));
    }
    let mut tx = Tx::new(dry_run);
    if !dry_run {
        std::fs::create_dir_all(env.host(&env.sites_available()))?;
        std::fs::create_dir_all(env.host(&env.sites_enabled()))?;
    }
    if !nginx_conf_includes_sites() {
        let src = std::fs::read_to_string(&conf_host)?;
        let dirs = parser::parse(&src).map_err(|e| anyhow::anyhow!("cannot parse nginx.conf: {e}"))?;
        let http = dirs.iter().find(|d| d.name == "http").context("nginx.conf has no http {} block")?;
        let include = format!("    include {}/*;", env.sites_enabled().display());
        let mut lines: Vec<String> = src.lines().map(String::from).collect();
        let idx = http.end_line.saturating_sub(1).min(lines.len());
        lines.insert(idx, include);
        tx.write(&env.nginx_conf(), &(lines.join("\n") + "\n"))?;
    }
    tx.commit()?;
    ok("sites-available / sites-enabled layout ready");
    Ok((env.sites_available(), true))
}

pub fn init(yes: bool, dry_run: bool) -> Result<()> {
    require_root()?;
    ensure_layout(yes, dry_run)?;
    Ok(())
}

// ───────────────────────── add ─────────────────────────

#[derive(clap::Args, Debug, Default)]
pub struct AddArgs {
    /// Primary domain, e.g. app.example.com (with edit: the new domain)
    #[arg(long, short)]
    pub domain: Option<String>,
    /// Extra server name, repeatable: -a www.example.com -a example.org (with edit: replaces the list)
    #[arg(long = "alias", short = 'a', value_name = "NAME")]
    pub aliases: Vec<String>,
    /// Site type: proxy | static | php
    #[arg(long = "type", short = 't', value_name = "TYPE")]
    pub kind: Option<String>,
    /// Upstream for a proxy site: PORT (on 127.0.0.1) or HOST:PORT
    #[arg(long, short, value_name = "PORT|HOST:PORT")]
    pub local: Option<String>,
    /// Document root for static / php sites (default /var/www/<domain>, created if missing)
    #[arg(long, value_name = "DIR")]
    pub root: Option<String>,
    /// php-fpm socket for php sites (default: the one found in /run/php)
    #[arg(long, value_name = "PATH")]
    pub socket: Option<String>,
    /// Public port (default 443 with SSL, 80 without)
    #[arg(long = "in", short = 'i', value_name = "PORT")]
    pub inbound: Option<u16>,
    /// SSL: letsencrypt | existing | none
    #[arg(long, value_name = "MODE")]
    pub ssl: Option<String>,
    /// Certificate (fullchain) path, with --ssl existing
    #[arg(long, value_name = "PATH")]
    pub cert: Option<String>,
    /// Private key path, with --ssl existing
    #[arg(long, value_name = "PATH")]
    pub key: Option<String>,
    /// Email for Let's Encrypt expiry notices (asked once if no certbot account exists)
    #[arg(long)]
    pub email: Option<String>,
    /// Proxy WebSocket upgrades
    #[arg(long)]
    pub websocket: bool,
    /// Don't redirect HTTP (port 80) to HTTPS
    #[arg(long)]
    pub no_redirect: bool,
    /// Use the Let's Encrypt staging server (for testing; certificates are not trusted)
    #[arg(long)]
    pub staging: bool,
    /// No prompts: use flags, and current values / defaults for the rest
    #[arg(long, short)]
    pub yes: bool,
    /// Show the config and what would change without touching anything
    #[arg(long)]
    pub dry_run: bool,
}

fn valid_domain(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= 253
        && d.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '*'))
}

fn php_sockets() -> Vec<String> {
    let mut v = Vec::new();
    for dir in ["/run/php", "/run/php-fpm", "/var/run/php"] {
        if let Ok(rd) = std::fs::read_dir(Env::get().host(Path::new(dir))) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.ends_with(".sock") {
                    v.push(format!("{dir}/{n}"));
                }
            }
        }
    }
    v.sort();
    v
}

fn parse_local(s: &str) -> Result<(String, u16)> {
    match s.rsplit_once(':') {
        Some((h, p)) => Ok((h.to_string(), p.parse().context("invalid port")?)),
        None => Ok(("127.0.0.1".into(), s.parse().context("invalid port")?)),
    }
}

/// Ask for (or take from flags) every setting of a site. With `cur`, the
/// current settings are the defaults, so editing is "press Enter to keep".
fn build_spec(a: &AddArgs, existing: &[Site], cur: Option<&Spec>) -> Result<(Spec, Option<String>)> {
    let y = a.yes;
    let taken = |d: &str| existing.iter().find(|s| s.matches(d)).map(|s| format!("{} is already served by site #{} ({})", d, s.id, Env::get().short(&s.config)));
    let names: Vec<String> = existing.iter().flat_map(|s| s.all_names().map(String::from).collect::<Vec<_>>()).collect();

    let domain = match &a.domain {
        Some(d) => d.trim().to_lowercase(),
        None if y => match cur {
            Some(c) => c.domain.clone(),
            None => bail!("--domain is required with --yes"),
        },
        None => {
            let names = names.clone();
            let mut t = Text::new("Domain:").with_placeholder("app.example.com").with_validator(move |s: &str| {
                let s = s.trim().to_lowercase();
                Ok(if !valid_domain(&s) {
                    Validation::Invalid("not a valid domain name".into())
                } else if names.contains(&s) {
                    Validation::Invalid("this domain is already configured (see `ngx ls`)".into())
                } else {
                    Validation::Valid
                })
            });
            if let Some(c) = cur {
                t = t.with_initial_value(&c.domain);
            }
            t.prompt()?.trim().to_lowercase()
        }
    };
    if !valid_domain(&domain) {
        bail!("invalid domain: {domain}");
    }
    if let Some(t) = taken(&domain) {
        bail!(t);
    }

    let mut aliases: Vec<String> = if !a.aliases.is_empty() {
        a.aliases.iter().map(|s| s.trim().to_lowercase()).collect()
    } else if let Some(c) = cur {
        if y {
            c.aliases.clone()
        } else {
            let init = c.aliases.join(" ");
            Text::new("Aliases (space-separated, empty for none):")
                .with_initial_value(&init)
                .prompt()?
                .split_whitespace()
                .map(|s| s.to_lowercase())
                .collect()
        }
    } else {
        let www = format!("www.{domain}");
        if !y && domain.matches('.').count() == 1 && Confirm::new(&format!("Also serve {www}?")).with_default(true).prompt()? { vec![www] } else { vec![] }
    };
    aliases.retain(|al| *al != domain);
    aliases.dedup();
    for al in &aliases {
        if !valid_domain(al) {
            bail!("invalid alias: {al}");
        }
        if let Some(t) = taken(al) {
            bail!(t);
        }
    }

    // current backend values, used as defaults
    let (cur_kind, cur_host, cur_port, cur_ws, cur_root, cur_sock) = match cur.map(|c| &c.backend) {
        Some(Backend::Proxy { host, port, websocket }) => (Some(0), Some(host.clone()), Some(*port), *websocket, None, None),
        Some(Backend::Static { root }) => (Some(1), None, None, false, Some(root.clone()), None),
        Some(Backend::Php { socket, root }) => (Some(2), None, None, false, Some(root.clone()), Some(socket.clone())),
        None => (None, None, None, false, None, None),
    };

    const KINDS: [&str; 3] = ["reverse proxy (app on a local port)", "static files", "PHP (php-fpm)"];
    let kind = match a.kind.as_deref() {
        Some("proxy") => 0,
        Some("static") => 1,
        Some("php") => 2,
        Some(k) => bail!("unknown --type {k} (proxy | static | php)"),
        None if a.socket.is_some() => 2,
        None if a.root.is_some() && cur_kind != Some(2) => 1,
        None if a.local.is_some() => 0,
        None if y => cur_kind.unwrap_or(0),
        None => Select::new("Site type:", KINDS.to_vec()).with_starting_cursor(cur_kind.unwrap_or(0)).raw_prompt()?.index,
    };
    let default_root = cur_root.clone().unwrap_or_else(|| format!("/var/www/{domain}"));
    let ask_root = |a: &AddArgs| -> Result<String> {
        Ok(match &a.root {
            Some(r) => r.clone(),
            None if y => default_root.clone(),
            None => Text::new("Document root:").with_default(&default_root).prompt()?,
        })
    };
    let backend = match kind {
        0 => {
            let (host, port) = match (&a.local, cur_port) {
                (Some(l), _) => parse_local(l)?,
                (None, Some(p)) if y => (cur_host.clone().unwrap_or_else(|| "127.0.0.1".into()), p),
                (None, None) if y => bail!("--local <port> is required for a proxy site with --yes"),
                (None, cur_p) => {
                    let mut q = CustomType::<u16>::new("Local port (app listening on 127.0.0.1):").with_error_message("enter a port number (1-65535)");
                    q = match cur_p {
                        Some(p) => q.with_default(p),
                        None => q.with_placeholder("3000"),
                    };
                    (cur_host.clone().unwrap_or_else(|| "127.0.0.1".into()), q.prompt()?)
                }
            };
            let websocket = a.websocket || if y { cur_ws } else { Confirm::new("Proxy WebSocket connections?").with_default(cur_ws).prompt()? };
            Backend::Proxy { host, port, websocket }
        }
        1 => Backend::Static { root: ask_root(a)? },
        _ => {
            let root = ask_root(a)?;
            let socket = match (&a.socket, &cur_sock) {
                (Some(s), _) => s.clone(),
                (None, Some(c)) if y => c.clone(),
                (None, Some(c)) => Text::new("php-fpm socket:").with_default(c).prompt()?,
                (None, None) => {
                    let socks = php_sockets();
                    match (socks.len(), y) {
                        (0, true) => "/run/php/php-fpm.sock".into(),
                        (_, true) => socks[0].clone(),
                        (0, false) => Text::new("php-fpm socket:").with_default("/run/php/php-fpm.sock").prompt()?,
                        (1, false) => socks[0].clone(),
                        _ => Select::new("php-fpm socket:", socks).prompt()?,
                    }
                }
            };
            Backend::Php { socket, root }
        }
    };

    const SSLS: [&str; 3] = ["Let's Encrypt (certbot)", "existing certificate", "none (HTTP only)"];
    let cur_ssl = cur.map(|c| match c.ssl {
        Ssl::LetsEncrypt => 0,
        Ssl::Existing { .. } => 1,
        Ssl::None => 2,
    });
    let (cur_cert, cur_key) = match cur.map(|c| &c.ssl) {
        Some(Ssl::Existing { cert, key }) => (Some(cert.clone()), Some(key.clone())),
        _ => (None, None),
    };
    let ssl_choice = match a.ssl.as_deref() {
        Some("letsencrypt" | "le" | "certbot") => 0,
        Some("existing") => 1,
        Some("none" | "off" | "no") => 2,
        Some(s) => bail!("unknown --ssl {s} (letsencrypt | existing | none)"),
        None if a.cert.is_some() => 1,
        None if y => cur_ssl.unwrap_or(0),
        None => Select::new("SSL:", SSLS.to_vec()).with_starting_cursor(cur_ssl.unwrap_or(0)).raw_prompt()?.index,
    };
    let mut email = a.email.clone();
    let ask_path = |flag: &Option<String>, current: &Option<String>, label: &str, name: &str| -> Result<String> {
        Ok(match (flag, current) {
            (Some(v), _) => v.clone(),
            (None, Some(c)) if y => c.clone(),
            (None, None) if y => bail!("--{name} is required with --ssl existing"),
            (None, Some(c)) => Text::new(label).with_default(c).prompt()?,
            (None, None) => Text::new(label).prompt()?,
        })
    };
    let ssl = match ssl_choice {
        0 => {
            if email.is_none() && !y && !ssl::has_account() {
                let e = Text::new("Email for Let's Encrypt expiry notices (optional):").prompt()?;
                email = Some(e).filter(|e| !e.trim().is_empty());
            }
            Ssl::LetsEncrypt
        }
        1 => Ssl::Existing {
            cert: ask_path(&a.cert, &cur_cert, "Certificate (fullchain) path:", "cert")?,
            key: ask_path(&a.key, &cur_key, "Private key path:", "key")?,
        },
        _ => Ssl::None,
    };
    let default_in = match cur {
        Some(c) if (c.ssl == Ssl::None) == (ssl == Ssl::None) => c.inbound,
        _ if ssl == Ssl::None => 80,
        _ => 443,
    };
    let inbound = match a.inbound {
        Some(p) => p,
        None if y => default_in,
        None => CustomType::<u16>::new("Inbound (public) port:").with_default(default_in).prompt()?,
    };
    let redirect_http = if ssl == Ssl::None || inbound == 80 || a.no_redirect {
        false
    } else if let Some(c) = cur {
        let def = c.redirect_http || c.ssl == Ssl::None;
        if y { def } else { Confirm::new("Redirect HTTP (port 80) to HTTPS?").with_default(def).prompt()? }
    } else {
        true
    };
    let (access_log, error_log) = match cur {
        // keep custom log paths unless the domain (and thus the default name) changed
        Some(c) if c.domain == domain => (c.access_log.clone(), c.error_log.clone()),
        _ => (None, None),
    };
    Ok((Spec { domain, aliases, inbound, backend, ssl, redirect_http, access_log, error_log }, email))
}

fn ensure_log_format(tx: &mut Tx, dir: &Path, link: bool) -> Result<()> {
    let env = Env::get();
    let defined = std::fs::read_to_string(env.host(&env.nginx_conf())).map(|s| s.contains("log_format ngx ")).unwrap_or(false);
    let file = dir.join(template::LOG_FORMAT_FILE);
    if defined || env.host(&file).exists() {
        return Ok(());
    }
    tx.write(&file, template::LOG_FORMAT)?;
    if link {
        tx.symlink(&file, &env.sites_enabled().join(template::LOG_FORMAT_FILE))?;
    }
    Ok(())
}

fn prepare_dirs(spec: &Spec, dry_run: bool) -> Result<()> {
    if dry_run {
        return Ok(());
    }
    let env = Env::get();
    std::fs::create_dir_all(env.host(&env.acme_webroot))?;
    std::fs::create_dir_all(env.host(&env.log_dir)).ok();
    if let Backend::Static { root } | Backend::Php { root, .. } = &spec.backend {
        let host = env.host(Path::new(root));
        if !host.exists() {
            std::fs::create_dir_all(&host)?;
            if matches!(spec.backend, Backend::Static { .. }) {
                std::fs::write(host.join("index.html"), format!("<!doctype html><title>{0}</title><h1>{0}</h1><p>Served by nginx, configured by ngx.</p>\n", spec.domain))?;
            }
        }
    }
    Ok(())
}

/// Write (and enable) a managed site from a spec.
fn apply_spec(spec: &Spec, dir: &Path, link: bool, bootstrap: bool, dry_run: bool) -> Result<PathBuf> {
    let env = Env::get();
    let file = dir.join(spec.file_name());
    let mut tx = Tx::new(dry_run);
    ensure_log_format(&mut tx, dir, link)?;
    tx.write(&file, &template::render(spec, bootstrap))?;
    if link {
        tx.symlink(&file, &env.sites_enabled().join(spec.file_name()))?;
    }
    tx.commit()?;
    Ok(file)
}

pub fn add(a: AddArgs) -> Result<()> {
    if !a.dry_run {
        require_root()?;
    }
    let existing = site::load_all();
    let (spec, email) = build_spec(&a, &existing, None)?;
    let (dir, link) = if a.dry_run && !Env::get().has_sites_layout() { (Env::get().conf_d(), false) } else { ensure_layout(a.yes, a.dry_run)? };
    let file = dir.join(spec.file_name());
    if Env::get().host(&file).exists() {
        bail!("{} already exists", file.display());
    }
    let preview = template::render(&spec, false);
    println!();
    render::print(&[vec![render::seg(format!("── {} ", file.display()), Tone::Dim), render::seg("─".repeat(30), Tone::Dim)]]);
    print!("{preview}");
    render::print(&[vec![render::seg("─".repeat(60), Tone::Dim)]]);
    if !confirm("Create and enable this site?", true, a.yes)? {
        println!("aborted");
        return Ok(());
    }
    prepare_dirs(&spec, a.dry_run)?;

    if spec.ssl == Ssl::LetsEncrypt {
        let (cert, _) = spec.cert_paths().unwrap();
        let have_cert = Env::get().host(Path::new(&cert)).exists();
        if !have_cert {
            apply_spec(&spec, &dir, link, true, a.dry_run)?;
            ok(&format!("{} is live over HTTP; requesting certificate…", spec.domain));
            let names: Vec<String> = std::iter::once(spec.domain.clone()).chain(spec.aliases.clone()).collect();
            if a.dry_run {
                println!("dry run — would run certbot certonly --webroot for {}", names.join(", "));
            } else if let Err(e) = ssl::issue_webroot(&names, email.as_deref(), a.staging) {
                eprintln!("certificate issuance failed: {e:#}");
                eprintln!("the site stays on HTTP. Check DNS points here and port 80 is reachable, then run `ngx cert issue {}`", spec.domain);
                return Ok(());
            }
        }
    }
    apply_spec(&spec, &dir, link, false, a.dry_run)?;
    if a.dry_run {
        return Ok(());
    }
    let sites = site::load_all();
    let id = site::find(&sites, &spec.domain).map(|s| s.id).unwrap_or(0);
    ok(&format!("site #{id} {} created and enabled — see `ngx show {id}`", spec.domain));
    Ok(())
}

// ───────────────────────── enable / disable / rm ─────────────────────────

fn enabled_link(s: &Site) -> PathBuf {
    Env::get().sites_enabled().join(s.file.file_name().unwrap_or_default())
}

pub fn set_enabled(query: &str, on: bool, dry_run: bool) -> Result<()> {
    require_root()?;
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    if s.enabled == on {
        println!("site #{} {} is already {}", s.id, s.domain, if on { "enabled" } else { "disabled" });
        return Ok(());
    }
    let others: Vec<String> = sites.iter().filter(|o| o.file == s.file && o.key != s.key).map(|o| o.domain.clone()).collect();
    if !others.is_empty() {
        println!("note: {} also contains {} — they are toggled together", Env::get().short(&s.file), others.join(", "));
    }
    let mut tx = Tx::new(dry_run);
    let file = s.file.to_string_lossy().into_owned();
    if s.conf_d {
        let to = if on { PathBuf::from(file.trim_end_matches(".disabled")) } else { PathBuf::from(format!("{file}.disabled")) };
        tx.rename(&s.file, &to)?;
    } else if on {
        tx.symlink(&s.file, &enabled_link(s))?;
    } else if s.config == s.file {
        // a plain file in sites-enabled: move it to sites-available
        let to = Env::get().sites_available().join(s.file.file_name().unwrap_or_default());
        tx.rename(&s.file, &to)?;
    } else {
        tx.remove(&s.config)?;
    }
    tx.commit()?;
    ok(&format!("site #{} {} {}", s.id, s.domain, if on { "enabled" } else { "disabled" }));
    Ok(())
}

pub fn rm(query: &str, delete_cert: bool, yes: bool, dry_run: bool) -> Result<()> {
    require_root()?;
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    let others: Vec<&Site> = sites.iter().filter(|o| o.file == s.file && o.key != s.key).collect();
    let mut q = format!("Remove site #{} {} ({})?", s.id, s.domain, Env::get().short(&s.file));
    if !others.is_empty() {
        q = format!("{q} The file also defines {} which will be removed too.", others.iter().map(|o| o.domain.as_str()).collect::<Vec<_>>().join(", "));
    }
    if !confirm(&q, false, yes)? {
        return Ok(());
    }
    let mut tx = Tx::new(dry_run);
    if s.config != s.file {
        tx.remove(&s.config)?;
    }
    tx.remove(&s.file)?;
    tx.commit()?;
    for o in std::iter::once(s).chain(others) {
        crate::state::forget(&o.key);
    }
    ok(&format!("site #{} {} removed (backup kept under {})", s.id, s.domain, Env::get().state_dir.join("backups").display()));
    if delete_cert {
        if let Some(name) = s.ssl_cert().and_then(ssl::lineage) {
            ssl::delete(&name)?;
            ok(&format!("certificate {name} deleted"));
        }
    }
    Ok(())
}

// ───────────────────────── edit ─────────────────────────

/// Best-effort reading of a hand-written site into a spec, so it can be edited
/// with prompts like a generated one.
fn spec_from_site(s: &Site) -> Result<Spec> {
    let raw_hint = || format!("use `ngx edit {} --raw` to edit the file directly", s.id);
    let backend = match (s.kind(), s.upstream()) {
        (Kind::Proxy, Some(Upstream::Tcp { host, port })) => Backend::Proxy {
            host: host.clone(),
            port: *port,
            websocket: s.main().is_some_and(|b| b.locations.iter().any(|l| l.action.contains("WebSocket"))),
        },
        (Kind::FastCgi, Some(Upstream::Unix(sock))) => Backend::Php {
            socket: sock.display().to_string(),
            root: s.root().map(|r| r.display().to_string()).unwrap_or_else(|| format!("/var/www/{}", s.domain)),
        },
        (Kind::Static, _) if s.root().is_some() => Backend::Static { root: s.root().unwrap().display().to_string() },
        (k, u) => bail!(
            "site #{} is a {} ({}) — not a setup ngx can generate; {}",
            s.id,
            k.label(),
            u.map(|u| u.to_string()).unwrap_or_else(|| "no upstream".into()),
            raw_hint()
        ),
    };
    let ssl = match s.ssl_cert() {
        None => Ssl::None,
        Some(c) if ssl::lineage(c).as_deref() == Some(s.domain.as_str()) => Ssl::LetsEncrypt,
        Some(c) => Ssl::Existing {
            cert: c.display().to_string(),
            key: s.ssl_key().map(|k| k.display().to_string()).unwrap_or_default(),
        },
    };
    Ok(Spec {
        domain: s.domain.clone(),
        aliases: s.aliases.clone(),
        inbound: s.public_port().unwrap_or(80),
        backend,
        ssl,
        redirect_http: s.http_redirect().is_some(),
        access_log: s.main().and_then(|b| b.access_log.as_ref()).map(|p| p.display().to_string()),
        error_log: s.main().and_then(|b| b.error_log.as_ref()).map(|p| p.display().to_string()),
    })
}

/// Colored unified diff between the current and the new config.
fn print_diff(old: &str, new: &str, label: &str) {
    let dir = std::env::temp_dir();
    let (a, b) = (dir.join(format!("ngx-old-{}", std::process::id())), dir.join(format!("ngx-new-{}", std::process::id())));
    let out = std::fs::write(&a, old)
        .and_then(|_| std::fs::write(&b, new))
        .ok()
        .and_then(|_| std::process::Command::new("diff").args(["-u", "--label", &format!("{label} (current)"), "--label", &format!("{label} (new)")]).arg(&a).arg(&b).output().ok());
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
    let lines: Vec<render::Line> = match out {
        Some(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| {
                let tone = if l.starts_with("+++") || l.starts_with("---") {
                    Tone::Bold
                } else if l.starts_with('+') {
                    Tone::Good
                } else if l.starts_with('-') {
                    Tone::Bad
                } else if l.starts_with("@@") {
                    Tone::Accent
                } else {
                    Tone::Dim
                };
                vec![render::seg(l, tone)]
            })
            .collect(),
        None => new.lines().map(|l| vec![render::seg(l, Tone::Plain)]).collect(),
    };
    println!();
    render::print(&lines);
    println!();
}

/// Write a regenerated config for an existing site, renaming the file (and
/// its sites-enabled link) when the domain changed.
fn write_site(s: &Site, new_file: &Path, content: &str, dry_run: bool) -> Result<()> {
    let env = Env::get();
    let dir = new_file.parent().unwrap_or(Path::new("/"));
    let mut tx = Tx::new(dry_run);
    ensure_log_format(&mut tx, dir, dir == env.sites_available())?;
    tx.write(new_file, content)?;
    if new_file != s.file {
        tx.remove(&s.file)?;
        if s.config != s.file {
            tx.remove(&s.config)?;
            tx.symlink(new_file, &env.sites_enabled().join(new_file.file_name().unwrap_or_default()))?;
        }
    }
    tx.commit()
}

/// `ngx edit`: change a site's settings through the same questions as
/// `ngx add`, pre-filled with the current values. ngx rewrites the config.
pub fn edit(query: &str, a: AddArgs, raw: bool) -> Result<()> {
    if raw {
        return edit_raw(query);
    }
    if !a.dry_run {
        require_root()?;
    }
    let env = Env::get();
    let sites = site::load_all();
    let s = site::find(&sites, query)?.clone();
    let others: Vec<&str> = sites.iter().filter(|o| o.file == s.file && o.key != s.key).map(|o| o.domain.as_str()).collect();
    if !others.is_empty() {
        bail!("{} also defines {} — split it up or use `ngx edit {} --raw`", env.short(&s.file), others.join(", "), s.id);
    }
    let src = std::fs::read_to_string(env.host(&s.file))?;
    let (cur, managed) = match Spec::from_config(&src) {
        Some(sp) => (sp, true),
        None => (spec_from_site(&s)?, false),
    };
    render::print(&[vec![render::seg(format!("Editing site #{} {}", s.id, s.domain), Tone::Bold), render::seg("  (Enter keeps the current value)", Tone::Dim)]]);
    if !managed {
        render::print(&[vec![render::seg(
            format!("note: {} was not created by ngx — saving replaces it with a generated config (the original is backed up).", env.short(&s.file)),
            Tone::Warn,
        )]]);
    }
    let existing: Vec<Site> = sites.iter().filter(|o| o.file != s.file).cloned().collect();
    let (spec, email) = build_spec(&a, &existing, Some(&cur))?;
    if managed && spec == cur {
        println!("no changes");
        return Ok(());
    }

    // same directory, file named after the (possibly new) domain
    let dir = s.file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let suffix = if s.conf_d && !s.enabled { ".disabled" } else { "" };
    let new_file = dir.join(format!("{}{suffix}", spec.file_name()));
    if new_file != s.file && env.host(&new_file).exists() {
        bail!("{} already exists", new_file.display());
    }
    let final_cfg = template::render(&spec, false);
    print_diff(&src, &final_cfg, &env.short(&new_file));
    if !confirm("Apply these changes?", true, a.yes)? {
        println!("aborted — nothing changed");
        return Ok(());
    }
    prepare_dirs(&spec, a.dry_run)?;

    let names: Vec<String> = std::iter::once(spec.domain.clone()).chain(spec.aliases.clone()).collect();
    let mut need_issue = false;
    if spec.ssl == Ssl::LetsEncrypt {
        let (cert, _) = spec.cert_paths().unwrap();
        match ssl::read_cert(Path::new(&cert)) {
            ssl::CertState::Missing(_) => {
                // no certificate yet: serve the ACME challenge over HTTP first
                write_site(&s, &new_file, &template::render(&spec, true), a.dry_run)?;
                if a.dry_run {
                    println!("dry run — would run certbot for {}", names.join(", "));
                } else if let Err(e) = ssl::issue_webroot(&names, email.as_deref(), a.staging) {
                    eprintln!("certificate issuance failed: {e:#}");
                    eprintln!("the site is on HTTP for now; fix DNS / port 80 and run `ngx cert issue {}`", s.id);
                    return Ok(());
                }
            }
            ssl::CertState::Ok(info) => need_issue = names.iter().any(|n| !render::covers(&info.sans, n)),
            _ => {}
        }
    }
    let s_now = if new_file != s.file && env.host(&new_file).exists() {
        // bootstrap already moved it
        Site { file: new_file.clone(), config: if s.config != s.file { env.sites_enabled().join(new_file.file_name().unwrap_or_default()) } else { new_file.clone() }, ..s.clone() }
    } else {
        s.clone()
    };
    write_site(&s_now, &new_file, &final_cfg, a.dry_run)?;
    if need_issue && !a.dry_run {
        // the config already serves the new names over HTTP (ACME location), so certbot can expand the cert
        ssl::issue_webroot(&names, email.as_deref(), a.staging)?;
        println!("{}", ctl::reload()?);
    }
    if a.dry_run {
        return Ok(());
    }
    let base = new_file.file_name().map(|n| n.to_string_lossy().trim_end_matches(".disabled").to_string()).unwrap_or_default();
    let new_key = format!("{base}#{}", spec.domain);
    if new_key != s.key {
        crate::state::rekey(&s.key, &new_key);
    }
    ok(&format!("site #{} {} updated", s.id, spec.domain));
    Ok(())
}

/// Escape hatch: edit the raw nginx file in $EDITOR (validated before applying).
fn edit_raw(query: &str) -> Result<()> {
    require_root()?;
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    let env = Env::get();
    let orig = std::fs::read_to_string(env.host(&s.file))?;
    let tmp = std::env::temp_dir().join(format!("ngx-{}-{}", std::process::id(), s.file.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, &orig)?;
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| {
        if env::which("nano").is_some() { "nano".into() } else { "vi".into() }
    });
    let result = loop {
        let st = std::process::Command::new("sh").arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(&tmp).status()?;
        if !st.success() {
            break Err(anyhow::anyhow!("editor exited with {st}"));
        }
        let mut new = std::fs::read_to_string(&tmp)?;
        if new == orig {
            println!("no changes");
            break Ok(());
        }
        // hand-edited files are no longer regenerated by ngx
        if new.lines().next().is_some_and(|l| l.starts_with(MANAGED_MARKER)) {
            new = new
                .lines()
                .filter(|l| !l.starts_with("# ngx-spec: "))
                .map(|l| if l.starts_with(MANAGED_MARKER) { "# originally generated by ngx (hand-edited since)" } else { l })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
        }
        let err = match parser::parse(&new) {
            Err(e) => Some(format!("syntax error: {e}")),
            Ok(_) => {
                let mut tx = Tx::new(false);
                tx.write(&s.file, &new)?;
                tx.commit().err().map(|e| format!("{e:#}"))
            }
        };
        match err {
            None => {
                ok(&format!("{} updated", Env::get().short(&s.file)));
                break Ok(());
            }
            Some(e) => {
                eprintln!("{e}");
                if !Confirm::new("Edit again?").with_default(true).prompt()? {
                    break Err(anyhow::anyhow!("changes discarded"));
                }
            }
        }
    };
    let _ = std::fs::remove_file(&tmp);
    result
}

// ───────────────────────── certificates ─────────────────────────

#[derive(clap::Subcommand, Debug)]
pub enum CertCmd {
    /// Show certificate details: expiry, issuer, covered names, renewal
    Info {
        /// Site ID or domain
        site: String,
    },
    /// Obtain a Let's Encrypt certificate for a site and switch it to HTTPS
    #[command(after_help = "ngx-created sites use the ACME webroot (certbot certonly --webroot); \
        other sites use certbot's nginx plugin (certbot --nginx).\n\nExample:\n  sudo ngx cert issue 5 --email me@example.com")]
    Issue {
        /// Site ID or domain
        site: String,
        /// Email for expiry notices
        #[arg(long)]
        email: Option<String>,
        /// Use the Let's Encrypt staging server
        #[arg(long)]
        staging: bool,
    },
    /// Renew a site's certificate now and reload nginx
    Renew {
        /// Site ID or domain
        site: String,
        /// Renew even if it is not close to expiry
        #[arg(long)]
        force: bool,
    },
    /// Turn automatic renewal on or off (sets `autorenew` in the certbot renewal config)
    #[command(after_help = "Example:\n  sudo ngx cert auto 2 on")]
    Auto {
        /// Site ID or domain
        site: String,
        state: OnOff,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum OnOff {
    On,
    Off,
}

pub fn cert(c: CertCmd) -> Result<()> {
    match c {
        CertCmd::Info { site: q } => {
            let rep = Report::load(false);
            let i = rep.index_of(&q)?;
            let (s, st) = (&rep.sites[i], &rep.status[i]);
            let lines = render::detail(s, st);
            // header + SSL section only
            let mut out: Vec<render::Line> = lines.iter().take(2).cloned().collect();
            let mut on = false;
            for l in &lines {
                let text: String = l.iter().map(|s| s.0.as_str()).collect();
                if text == "SSL" {
                    on = true;
                } else if on && !text.is_empty() && !text.starts_with(' ') {
                    break;
                }
                if on {
                    out.push(l.clone());
                }
            }
            if !on {
                out.push(vec![render::seg("no SSL configured", Tone::Dim)]);
            }
            if let Some(p) = s.ssl_cert() {
                out.push(vec![render::seg(format!("  {:<14}{}", "File", p.display()), Tone::Plain)]);
            }
            render::print(&out);
            Ok(())
        }
        CertCmd::Issue { site: q, email, staging } => {
            require_root()?;
            let sites = site::load_all();
            let s = site::find(&sites, &q)?;
            let src = std::fs::read_to_string(Env::get().host(&s.file))?;
            let names: Vec<String> = s.all_names().map(String::from).collect();
            match Spec::from_config(&src) {
                Some(mut spec) => {
                    ssl::issue_webroot(&names, email.as_deref(), staging)?;
                    spec.ssl = Ssl::LetsEncrypt;
                    if spec.inbound == 80 {
                        spec.inbound = 443;
                        spec.redirect_http = true;
                    }
                    let mut tx = Tx::new(false);
                    tx.write(&s.file, &template::render(&spec, false))?;
                    tx.commit()?;
                }
                None => {
                    println!("{} is not managed by ngx — using certbot's nginx plugin", Env::get().short(&s.file));
                    ssl::issue_nginx_plugin(&names, email.as_deref(), staging)?;
                }
            }
            ok(&format!("certificate issued for {}", names.join(", ")));
            Ok(())
        }
        CertCmd::Renew { site: q, force } => {
            require_root()?;
            let sites = site::load_all();
            let s = site::find(&sites, &q)?;
            let name = s.ssl_cert().and_then(ssl::lineage).ok_or_else(|| anyhow::anyhow!("site #{} has no certbot-managed certificate", s.id))?;
            ssl::renew(&name, force)?;
            println!("{}", ctl::reload()?);
            ok(&format!("certificate {name} renewed"));
            Ok(())
        }
        CertCmd::Auto { site: q, state } => {
            require_root()?;
            let sites = site::load_all();
            let s = site::find(&sites, &q)?;
            let name = s.ssl_cert().and_then(ssl::lineage).ok_or_else(|| anyhow::anyhow!("site #{} has no certbot-managed certificate", s.id))?;
            let on = matches!(state, OnOff::On);
            ssl::set_autorenew(&name, on)?;
            ok(&format!("automatic renewal {} for {name}", if on { "enabled" } else { "disabled" }));
            if on && ssl::timer().is_none() {
                println!("warning: no certbot timer/cron found — enable it with `systemctl enable --now certbot.timer`");
            }
            Ok(())
        }
    }
}

// ───────────────────────── logs / test / reload / doctor ─────────────────────────

pub fn logs(query: &str, error: bool, follow: bool, lines: usize) -> Result<()> {
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    let env = Env::get();
    let b = s.main();
    let (path, shared) = if error {
        match b.and_then(|b| b.error_log.clone()) {
            Some(p) => (p, false),
            None => (env.log_dir.join("error.log"), true),
        }
    } else {
        match b.and_then(|b| b.access_log.clone()) {
            Some(p) => (p, false),
            None => (env.log_dir.join("access.log"), true),
        }
    };
    if shared {
        eprintln!("note: {} has no own log, showing the shared {}", s.domain, path.display());
    }
    let host = env.host(&path);
    let mut cmd = std::process::Command::new("tail");
    cmd.arg("-n").arg(lines.to_string());
    if follow {
        cmd.arg("-F");
    }
    let st = cmd.arg(&host).status().context("running tail")?;
    anyhow::ensure!(st.success(), "tail exited with {st}");
    Ok(())
}

pub fn test() -> Result<()> {
    match ctl::test() {
        None => bail!("nginx not found (or sandbox mode)"),
        Some(t) if t.ok => {
            for m in &t.msgs {
                render::print(&[vec![render::seg(m.to_string(), Tone::Warn)]]);
            }
            ok("configuration is valid");
            Ok(())
        }
        Some(t) => {
            for m in &t.msgs {
                render::print(&[vec![render::seg(m.to_string(), Tone::Bad)]]);
            }
            if t.msgs.is_empty() {
                eprintln!("{}", t.raw);
            }
            bail!("configuration test failed")
        }
    }
}

pub fn reload() -> Result<()> {
    require_root()?;
    test()?;
    println!("{}", ctl::reload()?);
    Ok(())
}

pub fn doctor() -> Result<()> {
    let env = Env::get();
    let mut rows: Vec<(bool, String, String)> = Vec::new();
    let mut check = |ok: bool, what: &str, detail: String| rows.push((ok, what.to_string(), detail));

    let bin = ctl::bin();
    check(bin.is_some(), "nginx installed", bin.as_ref().map(|b| format!("{} {}", b.display(), ctl::version().unwrap_or_default())).unwrap_or_else(|| "not found in PATH".into()));
    check(ctl::running(), "nginx running", if ctl::running() { "master process found".into() } else { "start with `systemctl start nginx`".into() });
    check(env.host(&env.nginx_conf()).exists(), "nginx.conf", env.nginx_conf().display().to_string());
    let layout = env.has_sites_layout() && nginx_conf_includes_sites();
    check(layout, "sites-enabled layout", if layout { "sites-available + sites-enabled included".into() } else { "run `sudo ngx init`".into() });
    if env::is_root() {
        if let Some(t) = ctl::test() {
            check(t.ok, "nginx -t", if t.ok { "configuration valid".into() } else { t.msgs.first().map(|m| m.to_string()).unwrap_or(t.raw) });
        }
    } else {
        check(true, "nginx -t", "skipped (needs root)".into());
    }
    let certbot = env::which("certbot");
    check(certbot.is_some(), "certbot installed", certbot.map(|p| p.display().to_string()).unwrap_or_else(|| "install certbot for Let's Encrypt support".into()));
    let timer = ssl::timer();
    check(timer.is_some(), "auto-renewal scheduler", timer.map(|t| format!("{}{}", t.kind, t.next.map(|n| format!(", next {n}")).unwrap_or_default())).unwrap_or_else(|| "enable with `systemctl enable --now certbot.timer`".into()));
    check(env::is_root(), "running as root", if env::is_root() { "full access".into() } else { "read-only: cert/process details may be hidden".into() });
    let state = env.host(&env.state_dir);
    check(true, "state dir", format!("{}{}", state.display(), if state.exists() { "" } else { " (created on first change)" }));

    let mut lines = Vec::new();
    for (good, what, detail) in rows {
        lines.push(vec![
            render::seg(if good { "✓ " } else { "✗ " }, if good { Tone::Good } else { Tone::Bad }),
            render::seg(format!("{what:<24}"), Tone::Plain),
            render::seg(detail, Tone::Dim),
        ]);
    }
    render::print(&lines);
    Ok(())
}
