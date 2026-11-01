//! Certificates: reading PEM files and wrapping certbot.

use crate::env::{self, Env};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct CertInfo {
    pub not_before: DateTime<Local>,
    pub not_after: DateTime<Local>,
    pub issuer: String,
    pub sans: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum CertState {
    None,
    Ok(CertInfo),
    Missing(PathBuf),
    Unreadable(String),
}

/// Warn this many days before expiry at most…
pub const WARN_DAYS: i64 = 30;

impl CertInfo {
    /// …or at a third of the certificate's lifetime, whichever is shorter. certbot
    /// renews at about a third, so short-lived certificates (Let's Encrypt is moving
    /// to 45 days and less) aren't flagged permanently, and a warning still means
    /// renewal is late.
    pub fn warn_days(&self) -> i64 {
        ((self.not_after - self.not_before).num_days() / 3).clamp(1, WARN_DAYS)
    }
}

impl CertState {
    pub fn info(&self) -> Option<&CertInfo> {
        match self {
            CertState::Ok(i) => Some(i),
            _ => None,
        }
    }
    pub fn days_left(&self) -> Option<i64> {
        self.info().map(|i| ((i.not_after - Local::now()).num_seconds() + 86_399).div_euclid(86_400))
    }
    pub fn expired(&self) -> bool {
        self.info().is_some_and(|i| i.not_after < Local::now())
    }
    pub fn expiring_soon(&self) -> bool {
        !self.expired() && self.info().zip(self.days_left()).is_some_and(|(i, d)| d < i.warn_days())
    }
}

pub fn read_cert(path: &Path) -> CertState {
    let data = match std::fs::read(Env::get().host(path)) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return CertState::Missing(path.into()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return CertState::Unreadable("permission denied (run as root)".into())
        }
        Err(e) => return CertState::Unreadable(e.to_string()),
    };
    match parse_pem(&data) {
        Some(i) => CertState::Ok(i),
        None => CertState::Unreadable("not a valid PEM certificate".into()),
    }
}

pub fn parse_pem(data: &[u8]) -> Option<CertInfo> {
    use x509_parser::prelude::*;
    let (_, pem) = x509_parser::pem::parse_x509_pem(data).ok()?;
    let cert = pem.parse_x509().ok()?;
    let not_after = Local.timestamp_opt(cert.validity().not_after.timestamp(), 0).single()?;
    let not_before = Local.timestamp_opt(cert.validity().not_before.timestamp(), 0).single()?;
    let issuer = cert.issuer();
    let org = issuer.iter_organization().next().and_then(|a| a.as_str().ok());
    let cn = issuer.iter_common_name().next().and_then(|a| a.as_str().ok());
    let issuer = match (org, cn) {
        (Some(o), Some(c)) if o != c => format!("{o} ({c})"),
        (Some(o), _) => o.to_string(),
        (None, Some(c)) => c.to_string(),
        _ => "unknown".into(),
    };
    let mut sans = Vec::new();
    if let Ok(Some(ext)) = cert.subject_alternative_name() {
        for n in &ext.value.general_names {
            if let GeneralName::DNSName(d) = n {
                sans.push(d.to_string());
            }
        }
    }
    if sans.is_empty()
        && let Some(cn) = cert.subject().iter_common_name().next().and_then(|a| a.as_str().ok()) {
            sans.push(cn.to_string());
        }
    Some(CertInfo { not_before, not_after, issuer, sans })
}

// ───────────────────────── certbot ─────────────────────────

/// Certbot lineage name from `/etc/letsencrypt/live/<name>/fullchain.pem`.
pub fn lineage(cert: &Path) -> Option<String> {
    let live = Env::get().le_dir.join("live");
    let rel = cert.strip_prefix(&live).ok()?;
    rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned())
}

fn renewal_conf(name: &str) -> PathBuf {
    Env::get().le_dir.join("renewal").join(format!("{name}.conf"))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Renew {
    None,
    /// `reloads`: is nginx reloaded after a renewal, so it serves the new certificate?
    Auto { next: Option<String>, reloads: bool },
    Manual(String),
}

#[derive(Debug, Clone)]
pub struct Timer {
    pub kind: String,
    pub next: Option<String>,
}

/// Is something scheduled to run `certbot renew`? (cached)
pub fn timer() -> Option<Timer> {
    static T: OnceLock<Option<Timer>> = OnceLock::new();
    T.get_or_init(|| {
        let env = Env::get();
        if !env.sandboxed() && env::which("systemctl").is_some() {
            for unit in ["certbot.timer", "snap.certbot.renew.timer", "certbot-renew.timer"] {
                if env::run("systemctl", &["is-active", "--quiet", unit]).is_some_and(|r| r.0) {
                    let next = env::run("systemctl", &["show", unit, "-p", "NextElapseUSecRealtime", "--value"])
                        .and_then(|(_, out)| fmt_systemd_time(&out));
                    return Some(Timer { kind: "certbot timer".into(), next });
                }
            }
        }
        for f in ["/etc/cron.d/certbot", "/etc/cron.daily/certbot"] {
            if env.host(Path::new(f)).exists() {
                return Some(Timer { kind: "certbot cron".into(), next: None });
            }
        }
        if !env.sandboxed() && env::is_root()
            && let Some((true, tab)) = env::run("crontab", &["-l"])
                && tab.lines().any(|l| !l.trim_start().starts_with('#') && l.contains("certbot")) {
                    return Some(Timer { kind: "crontab".into(), next: None });
                }
        None
    })
    .clone()
}

/// `Sun 2026-11-01 03:00:00 CET` → `01/11/26 03:00`
fn fmt_systemd_time(s: &str) -> Option<String> {
    let mut parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    parts.pop(); // timezone
    let t = NaiveDateTime::parse_from_str(&parts[1..].join(" "), "%Y-%m-%d %H:%M:%S").ok()?;
    Some(t.format("%d/%m/%y %H:%M").to_string())
}

fn conf_value(src: &str, key: &str) -> Option<String> {
    src.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().to_string())
    })
}

pub fn renewal(cert: Option<&Path>) -> Renew {
    let Some(cert) = cert else { return Renew::None };
    let Some(name) = lineage(cert) else {
        return Renew::Manual("not issued by certbot".into());
    };
    let Ok(src) = std::fs::read_to_string(Env::get().host(&renewal_conf(&name))) else {
        return Renew::Manual("no certbot renewal config".into());
    };
    if conf_value(&src, "autorenew").is_some_and(|v| v.eq_ignore_ascii_case("false")) {
        return Renew::Manual("autorenew disabled".into());
    }
    if conf_value(&src, "authenticator").as_deref() == Some("manual") && conf_value(&src, "manual_auth_hook").is_none() {
        return Renew::Manual("manual authenticator (DNS challenge)".into());
    }
    let reloads = reloads_nginx(&src);
    match timer() {
        Some(t) => Renew::Auto {
            next: Some(match t.next {
                Some(n) => format!("{}, next check {n}", t.kind),
                None => t.kind,
            }),
            reloads,
        },
        None => Renew::Manual("no certbot timer or cron job".into()),
    }
}

/// Will nginx pick up a renewed certificate? Only certbot's nginx installer
/// reloads it by itself; webroot/standalone certificates need a hook.
fn reloads_nginx(renewal_src: &str) -> bool {
    if conf_value(renewal_src, "installer").as_deref() == Some("nginx") {
        return true;
    }
    if ["renew_hook", "deploy_hook", "post_hook"].iter().any(|k| conf_value(renewal_src, k).is_some_and(|v| v.contains("nginx"))) {
        return true;
    }
    // global hooks: any executable in renewal-hooks/deploy or /post
    let env = Env::get();
    ["deploy", "post"].iter().any(|d| {
        std::fs::read_dir(env.host(&env.le_dir.join("renewal-hooks").join(d))).is_ok_and(|rd| {
            rd.flatten().any(|e| {
                use std::os::unix::fs::PermissionsExt;
                e.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            })
        })
    })
}

/// certbot lineages whose renewal wouldn't reload nginx.
pub fn lineages_without_reload() -> Vec<String> {
    let env = Env::get();
    let mut v: Vec<String> = std::fs::read_dir(env.host(&env.le_dir.join("renewal")))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().strip_suffix(".conf")?.to_string();
                    let src = std::fs::read_to_string(e.path()).ok()?;
                    (!reloads_nginx(&src)).then_some(name)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// The command certbot runs after a renewal so nginx serves the new certificate.
pub fn reload_hook() -> String {
    if env::which("systemctl").is_some() && Path::new("/run/systemd/system").exists() {
        "systemctl reload nginx".into()
    } else {
        let bin = crate::nginx::ctl::bin().map(|b| b.display().to_string()).unwrap_or_else(|| "nginx".into());
        format!("{bin} -s reload")
    }
}

pub fn certbot_bin() -> anyhow::Result<PathBuf> {
    env::which("certbot").ok_or_else(|| anyhow::anyhow!("certbot is not installed (e.g. `apt install certbot` / `pacman -S certbot`)"))
}

fn run_certbot(args: &[String]) -> anyhow::Result<()> {
    if Env::get().sandboxed() {
        println!("sandbox mode: would run certbot {}", args.join(" "));
        return Ok(());
    }
    let bin = certbot_bin()?;
    let st = std::process::Command::new(bin).args(args).status()?;
    anyhow::ensure!(st.success(), "certbot exited with {st}");
    Ok(())
}

/// Issue a certificate through the ACME webroot served by ngx-managed configs.
pub fn issue_webroot(names: &[String], email: Option<&str>, staging: bool) -> anyhow::Result<()> {
    let webroot = Env::get().host(&Env::get().acme_webroot);
    std::fs::create_dir_all(webroot.join(".well-known/acme-challenge"))?;
    let mut args: Vec<String> = vec!["certonly".into(), "--webroot".into(), "-w".into(), Env::get().acme_webroot.display().to_string()];
    common_args(&mut args, names, email, staging);
    run_certbot(&args)
}

/// Options shared by every issuance: lineage name, names, account, reload hook.
fn common_args(args: &mut Vec<String>, names: &[String], email: Option<&str>, staging: bool) {
    args.extend(["--cert-name".into(), crate::nginx::template::cert_name(&names[0]), "--non-interactive".into(), "--agree-tos".into(), "--keep-until-expiring".into(), "--expand".into()]);
    // saved in the renewal config: nginx is reloaded after every automatic renewal
    args.extend(["--deploy-hook".into(), reload_hook()]);
    for n in names {
        args.extend(["-d".into(), n.clone()]);
    }
    match email.filter(|e| !e.is_empty()) {
        Some(e) => args.extend(["-m".into(), e.to_string()]),
        None if !has_account() => args.push("--register-unsafely-without-email".into()),
        None => {}
    }
    if staging {
        args.push("--staging".into());
    }
}

/// Issue a certificate the way the site's SSL mode says (HTTP or DNS challenge).
pub fn issue_for(ssl: &crate::nginx::template::Ssl, names: &[String], email: Option<&str>, staging: bool) -> anyhow::Result<()> {
    use crate::nginx::template::Ssl;
    match ssl {
        Ssl::LetsEncryptDns { plugin, credentials } => issue_dns(names, plugin, credentials.as_deref(), email, staging),
        _ => issue_webroot(names, email, staging),
    }
}

/// Issue through a certbot DNS plugin (`certbot-dns-<plugin>` must be installed).
pub fn issue_dns(names: &[String], plugin: &str, credentials: Option<&str>, email: Option<&str>, staging: bool) -> anyhow::Result<()> {
    let mut args: Vec<String> = vec!["certonly".into(), "--authenticator".into(), format!("dns-{plugin}")];
    if let Some(c) = credentials {
        args.extend([format!("--dns-{plugin}-credentials"), c.to_string()]);
    }
    common_args(&mut args, names, email, staging);
    run_certbot(&args)
}

/// Issue with certbot's nginx plugin (for configs ngx didn't generate).
pub fn issue_nginx_plugin(names: &[String], email: Option<&str>, staging: bool) -> anyhow::Result<()> {
    let mut args: Vec<String> = vec!["--nginx".into(), "--cert-name".into(), names[0].clone(), "--non-interactive".into(), "--agree-tos".into(), "--redirect".into()];
    for n in names {
        args.extend(["-d".into(), n.clone()]);
    }
    match email.filter(|e| !e.is_empty()) {
        Some(e) => args.extend(["-m".into(), e.to_string()]),
        None if !has_account() => args.push("--register-unsafely-without-email".into()),
        None => {}
    }
    if staging {
        args.push("--staging".into());
    }
    run_certbot(&args)
}

pub fn renew(name: &str, force: bool) -> anyhow::Result<()> {
    let mut args = vec!["renew".to_string(), "--cert-name".into(), name.into(), "--non-interactive".into()];
    if force {
        args.push("--force-renewal".into());
    }
    run_certbot(&args)
}

pub fn delete(name: &str) -> anyhow::Result<()> {
    run_certbot(&["delete".into(), "--cert-name".into(), name.into(), "--non-interactive".into()])
}

pub fn has_account() -> bool {
    let dir = Env::get().host(&Env::get().le_dir.join("accounts"));
    std::fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(false)
}

/// Set `key = value` in the `[renewalparams]` section of a lineage's renewal config.
fn set_param(name: &str, key: &str, value: &str) -> anyhow::Result<()> {
    let path = Env::get().host(&renewal_conf(name));
    let src = std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    std::fs::write(&path, with_param(&src, key, value))?;
    Ok(())
}

fn with_param(src: &str, key: &str, value: &str) -> String {
    let line = format!("{key} = {value}");
    let mut out = Vec::new();
    let mut done = false;
    let mut in_params = false;
    for l in src.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            if in_params && !done {
                out.push(line.clone());
                done = true;
            }
            in_params = t == "[renewalparams]";
        }
        if in_params && t.split('=').next().is_some_and(|k| k.trim() == key) {
            if !done {
                out.push(line.clone());
                done = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !done {
        if !in_params {
            out.push("[renewalparams]".into());
        }
        out.push(line);
    }
    out.join("\n") + "\n"
}

/// Toggle `autorenew` in the lineage's renewal config.
pub fn set_autorenew(name: &str, on: bool) -> anyhow::Result<()> {
    set_param(name, "autorenew", if on { "True" } else { "False" })
}

/// Make certbot reload nginx after renewing this lineage. Returns false if it already does.
pub fn ensure_reload_hook(name: &str) -> anyhow::Result<bool> {
    let src = std::fs::read_to_string(Env::get().host(&renewal_conf(name)))?;
    if reloads_nginx(&src) {
        return Ok(false);
    }
    set_param(name, "renew_hook", &reload_hook())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn renewal_params() {
        let src = "version = 2.11\n[renewalparams]\nauthenticator = webroot\nautorenew = False\n[[webroot_map]]\na.com = /x\n";
        let out = super::with_param(src, "autorenew", "True");
        assert!(out.contains("autorenew = True") && !out.contains("False"));
        let out = super::with_param(&out, "renew_hook", "systemctl reload nginx");
        assert!(out.find("renew_hook").unwrap() < out.find("[[webroot_map]]").unwrap());
        assert!(!super::reloads_nginx(src) && super::reloads_nginx(&out));
        assert!(super::reloads_nginx("[renewalparams]\ninstaller = nginx\n"));
    }

    #[test]
    fn systemd_time() {
        assert_eq!(super::fmt_systemd_time("Sun 2026-11-01 03:00:00 CET").as_deref(), Some("01/11/26 03:00"));
        assert_eq!(super::fmt_systemd_time(""), None);
    }
}
