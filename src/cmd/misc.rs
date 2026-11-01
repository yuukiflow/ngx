//! `ngx logs`, `ngx test`, `ngx reload`, `ngx doctor`.

#[allow(unused_imports)]
use super::*;

/// The access or error log of a site (logical path), and whether it's the shared one.
pub fn log_path(s: &Site, error: bool) -> (PathBuf, bool) {
    let env = Env::get();
    let b = s.main();
    let own = if error { b.and_then(|b| b.error_log.clone()) } else { b.and_then(|b| b.access_log.clone()) };
    match own {
        Some(p) => (p, false),
        None => (env.log_dir.join(if error { "error.log" } else { "access.log" }), true),
    }
}

pub fn logs(query: &str, error: bool, follow: bool, lines: usize) -> Result<()> {
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    let env = Env::get();
    let (path, shared) = log_path(s, error);
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
    let _lock = require_root()?;
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
    let no_reload = ssl::lineages_without_reload();
    check(
        no_reload.is_empty(),
        "nginx reload after renewal",
        if no_reload.is_empty() {
            "every certbot certificate reloads nginx when renewed".into()
        } else {
            format!("not for {}: fix with `sudo ngx cert auto <site> on`", no_reload.join(", "))
        },
    );
    check(env::is_root(), "running as root", if env::is_root() { "full access".into() } else { "read-only: cert/process details may be hidden".into() });
    let state = env.host(&env.state_dir);
    check(true, "state dir", format!("{}{}", state.display(), if state.exists() { "" } else { " (created on first change)" }));

    let mut lines = Vec::new();
    for (good, what, detail) in rows {
        lines.push(vec![
            render::seg(if good { "✓ " } else { "✗ " }, if good { Tone::Good } else { Tone::Bad }),
            render::seg(format!("{what:<28}"), Tone::Plain),
            render::seg(detail, Tone::Dim),
        ]);
    }
    render::print(&lines);
    Ok(())
}
