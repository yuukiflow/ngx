//! Detecting and setting up the sites-available / sites-enabled layout.

#[allow(unused_imports)]
use super::*;

pub(crate) fn nginx_conf_includes_sites() -> bool {
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
    let _lock = require_root()?;
    ensure_layout(yes, dry_run)?;
    Ok(())
}
