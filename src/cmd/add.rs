//! `ngx add` and the questions shared with `ngx edit`.

#[allow(unused_imports)]
use super::*;

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
    /// Email for the Let's Encrypt account (asked once if none exists). Let's Encrypt no longer sends expiry emails: use `ngx check` to monitor
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
    /// Let's Encrypt with the DNS challenge through this certbot plugin (cloudflare, route53, digitalocean…): wildcards, no port 80 needed
    #[arg(long, value_name = "PLUGIN")]
    pub dns: Option<String>,
    /// Credentials file for the DNS plugin (e.g. /root/.secrets/cloudflare.ini, mode 600)
    #[arg(long, value_name = "FILE")]
    pub dns_credentials: Option<String>,
    /// Maximum upload (request body) size, e.g. 50m; `off` for nginx's default (1m)
    #[arg(long, value_name = "SIZE")]
    pub max_body: Option<String>,
    /// HSTS: tell browsers to always use HTTPS for this domain (on | off)
    #[arg(long, value_name = "on|off")]
    pub hsts: Option<OnOff>,
    /// Password-protect the site (users: `ngx auth add <site> <user>`) (on | off)
    #[arg(long, value_name = "on|off")]
    pub auth: Option<OnOff>,
    /// Limit requests per client, e.g. 10r/s or 300r/m; `off` to remove
    #[arg(long, value_name = "RATE")]
    pub rate_limit: Option<String>,
    /// Redirect the aliases (e.g. www.) to the main domain (on | off)
    #[arg(long, value_name = "on|off")]
    pub canonical: Option<OnOff>,
    /// Security headers: X-Content-Type-Options, X-Frame-Options, Referrer-Policy (on | off)
    #[arg(long, value_name = "on|off")]
    pub headers: Option<OnOff>,
    /// Path the backend answers health checks on, e.g. /health (`ngx check` and the status use it); `off` to remove
    #[arg(long, value_name = "PATH")]
    pub health: Option<String>,
    /// Proxy a path to another local app, repeatable: --route /api=8080 --route /ws=10.0.0.5:9000 (with edit: replaces the list; `--route off` removes all)
    #[arg(long = "route", value_name = "PATH=PORT")]
    pub routes: Vec<String>,
    /// No prompts: use flags, and current values / defaults for the rest
    #[arg(long, short)]
    pub yes: bool,
    /// Show the config and what would change without touching anything
    #[arg(long)]
    pub dry_run: bool,
    /// Set by the dashboard form: every value is given, ask nothing but the
    /// final confirmation (after showing the config / diff).
    #[arg(skip)]
    pub review: bool,
    /// Explicit WebSocket / HTTP-redirect choices (the dashboard form can turn them off).
    #[arg(skip)]
    pub websocket_set: Option<bool>,
    #[arg(skip)]
    pub redirect_set: Option<bool>,
}

impl AddArgs {
    /// No questions about values (flags, current values and defaults decide).
    fn no_questions(&self) -> bool {
        self.yes || self.review
    }
}


pub(crate) fn valid_domain(d: &str) -> bool {
    validate::domain(d)
}

/// Prompt validator for absolute paths that go into the config.
pub(crate) fn path_validator(s: &str) -> std::result::Result<Validation, inquire::CustomUserError> {
    Ok(if validate::path(s.trim()) {
        Validation::Valid
    } else {
        Validation::Invalid("absolute path, without spaces, quotes or ; { } $ # characters".into())
    })
}

pub(crate) fn php_sockets() -> Vec<String> {
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

pub(crate) fn parse_local(s: &str) -> Result<(String, u16)> {
    match s.rsplit_once(':') {
        Some((h, p)) => {
            if !validate::host(h) {
                bail!("invalid upstream host {h:?}: use an IP address or host name");
            }
            Ok((h.to_string(), p.parse().context("invalid port")?))
        }
        None => Ok(("127.0.0.1".into(), s.parse().context("invalid port")?)),
    }
}

/// Ask for (or take from flags) every setting of a site. With `cur`, the
/// current settings are the defaults, so editing is "press Enter to keep".
pub(crate) fn build_spec(a: &AddArgs, existing: &[Site], cur: Option<&Spec>) -> Result<(Spec, Option<String>)> {
    let y = a.no_questions();
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

    let mut aliases: Vec<String> = if a.aliases.iter().any(|s| s == "none") {
        Vec::new()
    } else if !a.aliases.is_empty() {
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
            None => Text::new("Document root:").with_default(&default_root).with_validator(path_validator).prompt()?.trim().to_string(),
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
            let websocket = match a.websocket_set {
                Some(v) => v,
                None if a.websocket => true,
                None if y => cur_ws,
                None => Confirm::new("Proxy WebSocket connections?").with_default(cur_ws).prompt()?,
            };
            Backend::Proxy { host, port, websocket }
        }
        1 => Backend::Static { root: ask_root(a)? },
        _ => {
            let root = ask_root(a)?;
            let socket = match (&a.socket, &cur_sock) {
                (Some(s), _) => s.clone(),
                (None, Some(c)) if y => c.clone(),
                (None, Some(c)) => Text::new("php-fpm socket:").with_default(c).with_validator(path_validator).prompt()?.trim().to_string(),
                (None, None) => {
                    let socks = php_sockets();
                    match (socks.len(), y) {
                        (0, true) => "/run/php/php-fpm.sock".into(),
                        (_, true) => socks[0].clone(),
                        (0, false) => Text::new("php-fpm socket:").with_default("/run/php/php-fpm.sock").with_validator(path_validator).prompt()?.trim().to_string(),
                        (1, false) => socks[0].clone(),
                        _ => Select::new("php-fpm socket:", socks).prompt()?,
                    }
                }
            };
            Backend::Php { socket, root }
        }
    };

    const SSLS: [&str; 4] = [
        "Let's Encrypt (certbot, HTTP challenge)",
        "Let's Encrypt with DNS challenge (wildcards, or port 80 not reachable)",
        "existing certificate",
        "none (HTTP only)",
    ];
    let cur_ssl = cur.map(|c| match c.ssl {
        Ssl::LetsEncrypt => 0,
        Ssl::LetsEncryptDns { .. } => 1,
        Ssl::Existing { .. } => 2,
        Ssl::None => 3,
    });
    let (cur_plugin, cur_creds) = match cur.map(|c| &c.ssl) {
        Some(Ssl::LetsEncryptDns { plugin, credentials }) => (Some(plugin.clone()), credentials.clone()),
        _ => (None, None),
    };
    let (cur_cert, cur_key) = match cur.map(|c| &c.ssl) {
        Some(Ssl::Existing { cert, key }) => (Some(cert.clone()), Some(key.clone())),
        _ => (None, None),
    };
    let ssl_choice = match a.ssl.as_deref() {
        Some("letsencrypt" | "le" | "certbot") if a.dns.is_some() => 1,
        Some("letsencrypt" | "le" | "certbot") => 0,
        Some("dns") => 1,
        Some("existing") => 2,
        Some("none" | "off" | "no") => 3,
        Some(s) => bail!("unknown --ssl {s} (letsencrypt | dns | existing | none)"),
        None if a.dns.is_some() => 1,
        None if a.cert.is_some() => 2,
        None if y => cur_ssl.unwrap_or(0),
        None => Select::new("SSL:", SSLS.to_vec()).with_starting_cursor(cur_ssl.unwrap_or(0)).raw_prompt()?.index,
    };
    let mut email = a.email.clone();
    let ask_path = |flag: &Option<String>, current: &Option<String>, label: &str, name: &str| -> Result<String> {
        Ok(match (flag, current) {
            (Some(v), _) => v.clone(),
            (None, Some(c)) if y => c.clone(),
            (None, None) if y => bail!("--{name} is required with --ssl existing"),
            (None, Some(c)) => Text::new(label).with_default(c).with_validator(path_validator).prompt()?.trim().to_string(),
            (None, None) => Text::new(label).with_validator(path_validator).prompt()?.trim().to_string(),
        })
    };
    let ssl = match ssl_choice {
        0 | 1 => {
            if email.is_none() && !y && !ssl::has_account() {
                let e = Text::new("Email for the Let's Encrypt account (optional, for account and policy notices):")
                    .with_help_message("Let's Encrypt no longer emails about expiring certificates: `ngx check` monitors them")
                    .with_validator(|e: &str| {
                        Ok(if e.trim().is_empty() || validate::email(e.trim()) { Validation::Valid } else { Validation::Invalid("not a valid email address".into()) })
                    })
                    .prompt()?
                    .trim()
                    .to_string();
                email = Some(e).filter(|e| !e.trim().is_empty());
            }
            if ssl_choice == 0 {
                Ssl::LetsEncrypt
            } else {
                let plugin = match (&a.dns, &cur_plugin) {
                    (Some(p), _) => p.clone(),
                    (None, Some(p)) if y => p.clone(),
                    (None, None) if y => bail!("--dns <plugin> is required for the DNS challenge"),
                    (None, c) => {
                        let mut t = Text::new("certbot DNS plugin:")
                            .with_help_message("the part after certbot-dns-: cloudflare, route53, digitalocean, ovh, linode, rfc2136… (install it first)")
                            .with_validator(|p: &str| Ok(if validate::dns_plugin(p.trim()) { Validation::Valid } else { Validation::Invalid("lowercase plugin name, e.g. cloudflare".into()) }));
                        if let Some(c) = c {
                            t = t.with_default(c);
                        }
                        t.prompt()?.trim().to_string()
                    }
                };
                let credentials = match (&a.dns_credentials, &cur_creds) {
                    (Some(c), _) => Some(c.clone()),
                    (None, c) if y => c.clone(),
                    (None, c) => {
                        let mut t = Text::new("DNS credentials file (empty if the plugin needs none, e.g. route53):").with_validator(|s: &str| {
                            Ok(if s.trim().is_empty() || validate::path(s.trim()) { Validation::Valid } else { Validation::Invalid("absolute path".into()) })
                        });
                        if let Some(c) = c {
                            t = t.with_default(c);
                        }
                        Some(t.prompt()?.trim().to_string()).filter(|s| !s.is_empty())
                    }
                };
                Ssl::LetsEncryptDns { plugin, credentials }
            }
        }
        2 => Ssl::Existing {
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
    } else if let Some(r) = a.redirect_set {
        r
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
    let opts = build_options(a, cur.map(|c| &c.opts), &domain, &aliases, ssl != Ssl::None, matches!(backend, Backend::Proxy { .. }))?;
    let spec = Spec { domain, aliases, inbound, backend, ssl, redirect_http, access_log, error_log, opts };
    validate::spec(&spec)?;
    check_email(email.as_deref())?;
    let secret = match &spec.ssl {
        Ssl::Existing { key, .. } => Some(key),
        Ssl::LetsEncryptDns { credentials: Some(c), .. } => Some(c),
        _ => None,
    };
    if let Some(w) = secret.and_then(|k| validate::key_warning(k)) {
        render::print(&[vec![render::seg(format!("warning: {w}"), Tone::Warn)]]);
    }
    Ok((spec, email))
}

const OPTION_LABELS: [&str; 8] = [
    "Max upload size",
    "HSTS (browsers always use HTTPS)",
    "Password protection",
    "Rate limit per client",
    "Redirect aliases to the main domain",
    "Extra paths proxied to other apps (/api → :8080)",
    "Security headers (nosniff, frame and referrer policy)",
    "Health check path (ngx check calls it)",
];

fn on_off(v: Option<OnOff>) -> Option<bool> {
    v.map(|v| matches!(v, OnOff::On))
}

/// The optional settings. Flags win; with -y the rest keep their current
/// value; otherwise a checklist (pre-ticked with the current options) asks
/// which to use, then their values.
fn build_options(a: &AddArgs, cur: Option<&template::Options>, domain: &str, aliases: &[String], tls: bool, proxy: bool) -> Result<template::Options> {
    let c = cur.cloned().unwrap_or_default();
    let off = |s: &str| matches!(s, "off" | "none" | "no");
    let flag_routes: Option<Vec<template::Route>> = if a.routes.is_empty() {
        None
    } else if a.routes.iter().any(|r| off(r)) {
        Some(Vec::new())
    } else {
        Some(a.routes.iter().map(|r| template::Route::parse(r).ok_or_else(|| anyhow::anyhow!("invalid --route {r:?} (PATH=PORT or PATH=HOST:PORT, e.g. /api=8080)"))).collect::<Result<_>>()?)
    };
    let mut o = template::Options {
        max_body: a.max_body.as_ref().map(|v| (!off(v)).then(|| v.clone())).unwrap_or(c.max_body.clone()),
        hsts: on_off(a.hsts).unwrap_or(c.hsts) && tls,
        auth: on_off(a.auth).unwrap_or(c.auth),
        rate_limit: a.rate_limit.as_ref().map(|v| (!off(v)).then(|| v.clone())).unwrap_or(c.rate_limit.clone()),
        canonical: on_off(a.canonical).unwrap_or(c.canonical) && !aliases.is_empty(),
        routes: flag_routes.clone().unwrap_or(c.routes.clone()),
        headers: on_off(a.headers).unwrap_or(c.headers),
        health: a.health.as_ref().map(|v| (!off(v)).then(|| v.clone())).unwrap_or(c.health.clone()).filter(|_| proxy),
        // `ngx edit` keeps a suspension; only `ngx resume` lifts it
        suspended: c.suspended.clone(),
    };
    if a.no_questions() {
        return Ok(o);
    }
    // interactive: which options? (only the ones not given as flags)
    let given = [a.max_body.is_some(), a.hsts.is_some(), a.auth.is_some(), a.rate_limit.is_some(), a.canonical.is_some(), flag_routes.is_some(), a.headers.is_some(), a.health.is_some()];
    let usable = [true, tls, true, true, !aliases.is_empty(), true, true, proxy];
    let on = [o.max_body.is_some(), o.hsts, o.auth, o.rate_limit.is_some(), o.canonical, !o.routes.is_empty(), o.headers, o.health.is_some()];
    let idx: Vec<usize> = (0..8).filter(|&i| usable[i] && !given[i]).collect();
    if idx.is_empty() {
        return Ok(o);
    }
    let labels: Vec<String> = idx.iter().map(|&i| if i == 4 { format!("Redirect aliases to {domain}") } else { OPTION_LABELS[i].to_string() }).collect();
    let defaults: Vec<usize> = idx.iter().enumerate().filter(|(_, i)| on[**i]).map(|(n, _)| n).collect();
    let picked: Vec<usize> = inquire::MultiSelect::new("Options (space toggles, enter confirms):", labels)
        .with_default(&defaults)
        .raw_prompt()?
        .into_iter()
        .map(|l| idx[l.index])
        .collect();
    let has = |i: usize| picked.contains(&i);
    if !given[0] {
        o.max_body = if has(0) {
            let d = o.max_body.clone().unwrap_or_else(|| "50m".into());
            Some(Text::new("Max upload size:").with_default(&d).with_validator(|s: &str| Ok(if validate::size(s.trim()) { Validation::Valid } else { Validation::Invalid("e.g. 10m, 512k, 1g".into()) })).prompt()?.trim().to_string())
        } else {
            None
        };
    }
    if !given[1] {
        o.hsts = has(1);
    }
    if !given[2] {
        o.auth = has(2);
    }
    if !given[3] {
        o.rate_limit = if has(3) {
            let d = o.rate_limit.clone().unwrap_or_else(|| "10r/s".into());
            Some(Text::new("Requests per client:").with_default(&d).with_help_message("e.g. 10r/s, 300r/m (short bursts are allowed)").with_validator(|s: &str| Ok(if validate::rate(s.trim()) { Validation::Valid } else { Validation::Invalid("e.g. 10r/s or 300r/m".into()) })).prompt()?.trim().to_string())
        } else {
            None
        };
    }
    if !given[4] {
        o.canonical = has(4);
    }
    if !given[6] {
        o.headers = has(6);
    }
    if !given[7] {
        o.health = if has(7) {
            let d = o.health.clone().unwrap_or_else(|| "/health".into());
            Some(Text::new("Health check path:").with_default(&d).with_help_message("the backend should answer 2xx/3xx; ngx check reports anything else").with_validator(|s: &str| Ok(if validate::health_path(s.trim()) { Validation::Valid } else { Validation::Invalid("a path like /health".into()) })).prompt()?.trim().to_string())
        } else {
            None
        };
    }
    if !given[5] {
        o.routes = if has(5) {
            let init = o.routes.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(" ");
            let v = Text::new("Paths (space-separated PATH=PORT or PATH=HOST:PORT):").with_initial_value(&init).with_placeholder("/api=8080").prompt()?;
            v.split_whitespace().map(|r| template::Route::parse(r).ok_or_else(|| anyhow::anyhow!("invalid route {r:?} (e.g. /api=8080)"))).collect::<Result<_>>()?
        } else {
            Vec::new()
        };
    }
    Ok(o)
}

pub(crate) fn check_email(e: Option<&str>) -> Result<()> {
    match e {
        Some(e) if !e.is_empty() && !validate::email(e) => bail!("invalid email address: {e:?}"),
        _ => Ok(()),
    }
}

pub(crate) fn ensure_log_format(tx: &mut Tx, dir: &Path, link: bool) -> Result<()> {
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

pub(crate) fn prepare_dirs(spec: &Spec, dry_run: bool) -> Result<()> {
    if dry_run {
        return Ok(());
    }
    let env = Env::get();
    std::fs::create_dir_all(env.host(&env.acme_webroot))?;
    std::fs::create_dir_all(env.host(&env.log_dir)).ok();
    // per-site directory for the user's own snippets (and the password file)
    let sd = env.host(&template::site_dir(&spec.domain));
    if !sd.exists() {
        std::fs::create_dir_all(&sd)?;
        std::fs::write(
            sd.join("README"),
            format!(
                "Your own nginx directives for {0}.\n\n\
                 Every *.conf file here is included inside the site's server block, and is kept\n\
                 when ngx regenerates the site's config. Changes are checked with nginx -t:\n\n  \
                 sudo ngx edit {0} --snippet <name>\n\n\
                 Examples: extra `location` blocks, `add_header`, `proxy_read_timeout`.\n\
                 Don't add `location /` (the generated config already has it).\n",
                spec.domain
            ),
        )?;
    }
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

/// Files in a site's `ngx.d/<domain>/` directory (logical paths), for moving
/// them along when the domain changes and removing them with the site.
pub(crate) fn site_files(domain: &str) -> Vec<PathBuf> {
    let dir = template::site_dir(domain);
    let mut v: Vec<PathBuf> = std::fs::read_dir(Env::get().host(&dir))
        .map(|rd| rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| dir.join(e.file_name())).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Write (and enable) a managed site from a spec.
pub(crate) fn apply_spec(spec: &Spec, dir: &Path, link: bool, bootstrap: bool, dry_run: bool) -> Result<PathBuf> {
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
    let _lock = if a.dry_run { None } else { Some(require_root()?) };
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
    if !confirm("Create and enable this site?", true, a.yes && !a.review)? {
        return Err(Aborted.into());
    }
    prepare_dirs(&spec, a.dry_run)?;

    if spec.ssl.is_letsencrypt() {
        let (cert, _) = spec.cert_paths().unwrap();
        let have_cert = Env::get().host(Path::new(&cert)).exists();
        let dns = matches!(spec.ssl, Ssl::LetsEncryptDns { .. });
        if !have_cert {
            let names: Vec<String> = std::iter::once(spec.domain.clone()).chain(spec.aliases.clone()).collect();
            if dns {
                // DNS challenge: nothing needs to be served, get the certificate first
                ok("requesting certificate (DNS challenge)…");
            } else {
                apply_spec(&spec, &dir, link, true, a.dry_run)?;
                ok(&format!("{} is live over HTTP; requesting certificate…", spec.domain));
            }
            if a.dry_run {
                println!("dry run — would run certbot for {}", names.join(", "));
            } else if let Err(e) = ssl::issue_for(&spec.ssl, &names, email.as_deref(), a.staging) {
                eprintln!("certificate issuance failed: {e:#}");
                if dns {
                    bail!("nothing was changed. Check the DNS plugin is installed (certbot plugins) and its credentials, then run `ngx add` again");
                }
                eprintln!("the site stays on HTTP. Check DNS points here and port 80 is reachable, then run `ngx cert issue {}`", spec.domain);
                return Ok(());
            }
        }
    }
    apply_spec(&spec, &dir, link, false, a.dry_run)?;
    if a.dry_run {
        return Ok(());
    }
    if spec.opts.auth {
        ensure_htpasswd(&spec.domain, a.yes && !a.review)?;
    }
    let sites = site::load_all();
    let id = site::find(&sites, &spec.domain).map(|s| s.id).unwrap_or(0);
    ok(&format!("site #{id} {} created and enabled — see `ngx show {id}`", spec.domain));
    Ok(())
}
