//! `ngx check`: one pass over every enabled site for cron jobs and monitoring
//! (Nagios-style exit codes). Let's Encrypt no longer emails about expiring
//! certificates, so this is what tells you.

#[allow(unused_imports)]
use super::*;
use crate::ssl::{CertState, Renew};
use crate::status::State;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warning,
    Critical,
}

#[derive(serde::Serialize)]
pub struct Problem {
    pub level: Level,
    pub site: Option<u32>,
    pub domain: Option<String>,
    pub message: String,
}

pub fn problems(rep: &Report, max_5xx: f64) -> Vec<Problem> {
    let mut out = Vec::new();
    for g in &rep.global {
        out.push(Problem { level: Level::Critical, site: None, domain: None, message: format!("nginx: {g}") });
    }
    for (s, st) in rep.sites.iter().zip(&rep.status) {
        if st.state == State::Disabled {
            continue;
        }
        let mut p = |level, message: String| out.push(Problem { level, site: Some(s.id), domain: Some(s.domain.clone()), message });
        if st.state == State::Down {
            p(Level::Critical, format!("down: {}", st.error.clone().unwrap_or_else(|| "unknown error".into())));
        }
        match &st.cert {
            CertState::Ok(i) => {
                let days = st.cert.days_left().unwrap_or(0);
                if st.cert.expired() {
                    p(Level::Critical, format!("certificate expired {}", render::date(i.not_after)));
                } else if st.cert.expiring_soon() {
                    let why = match &st.renew {
                        Renew::Auto { .. } => "automatic renewal is failing — run `sudo certbot renew --dry-run`".to_string(),
                        Renew::Manual(r) => format!("renewal is manual ({r})"),
                        Renew::None => String::new(),
                    };
                    p(Level::Warning, format!("certificate expires in {days} day{} ({}) · {why}", if days == 1 { "" } else { "s" }, render::date(i.not_after)));
                }
                let missing: Vec<&str> = s.all_names().filter(|n| !render::covers(&i.sans, n)).collect();
                if !missing.is_empty() {
                    p(Level::Warning, format!("certificate doesn't cover {}", missing.join(", ")));
                }
            }
            CertState::Missing(path) => p(Level::Critical, format!("certificate file missing: {}", path.display())),
            CertState::Unreadable(e) => p(Level::Warning, format!("certificate unreadable: {e}")),
            CertState::None => {}
        }
        // a high share of server errors in the site's own access log
        if st.state == State::Up
            && let Some(t) = s.main().and_then(|b| b.access_log.clone()).and_then(|p| crate::logs::traffic_24h(&p))
            && t.requests >= 20
        {
            let pct = t.errors_5xx as f64 * 100.0 / t.requests as f64;
            if pct > max_5xx {
                p(Level::Warning, format!("{pct:.1}% of {} requests in the last 24h failed with 5xx", t.requests));
            }
        }
        if let Renew::Auto { reloads: false, .. } = st.renew {
            p(Level::Warning, format!("nginx isn't reloaded after renewal, renewed certificates won't be served (fix: sudo ngx cert auto {} on)", s.id));
        }
    }
    out
}

#[derive(clap::Args, Debug, Default, Clone)]
pub struct CheckArgs {
    /// Print JSON
    #[arg(long)]
    pub json: bool,
    /// No output, only the exit status
    #[arg(long, short)]
    pub quiet: bool,
    /// Warn when more than this percentage of a site's requests failed with 5xx in the last 24h
    #[arg(long, value_name = "PERCENT", default_value_t = 5.0)]
    pub max_5xx: f64,
    /// POST a plain-text report to this URL (ntfy-style: Title/Priority headers). Repeatable
    #[arg(long, value_name = "URL")]
    pub notify: Vec<String>,
    /// POST a JSON report ({"text", "content", "status", "problems"}: Slack, Mattermost, Discord…). Repeatable
    #[arg(long, value_name = "URL")]
    pub notify_json: Vec<String>,
    /// Email the report through the local sendmail. Repeatable
    #[arg(long, value_name = "ADDRESS")]
    pub mail: Vec<String>,
    /// Notify on every run with problems (default: only when the problems change, and once when all is resolved)
    #[arg(long)]
    pub always: bool,
    /// Install a systemd timer (or cron job) that runs this check with the same options
    #[arg(long, conflicts_with = "remove_timer")]
    pub install_timer: bool,
    /// How often the timer runs: 15m, 30m, 1h, 6h, 1d…
    #[arg(long, value_name = "INTERVAL", default_value = "1h", requires = "install_timer")]
    pub every: String,
    /// Remove the timer installed with --install-timer
    #[arg(long)]
    pub remove_timer: bool,
}

/// Exit code: 0 everything fine, 1 warnings, 2 something is down or expired.
pub fn check(a: &CheckArgs) -> Result<i32> {
    if a.install_timer {
        install_timer(a)?;
        return Ok(0);
    }
    if a.remove_timer {
        remove_timer()?;
        return Ok(0);
    }
    for u in a.notify.iter().chain(&a.notify_json) {
        if !validate::url(u) {
            bail!("invalid URL {u:?} (http(s)://…, without spaces, quotes, $ or ;)");
        }
    }
    for m in &a.mail {
        if !validate::email(m) {
            bail!("invalid email address {m:?}");
        }
    }
    let rep = Report::load(true);
    let probs = problems(&rep, a.max_5xx);
    let worst = probs.iter().map(|p| p.level).fold(Level::Ok, |a, b| if b > a { b } else { a });
    let code = match worst {
        Level::Ok => 0,
        Level::Warning => 1,
        Level::Critical => 2,
    };
    if a.json {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "status": worst, "sites": rep.sites.len(), "problems": probs }))?);
    } else if !a.quiet {
        let enabled = rep.status.iter().filter(|s| s.state != State::Disabled).count();
        let mut out = Vec::new();
        for p in &probs {
            let (tag, tone) = match p.level {
                Level::Critical => ("CRIT", Tone::Bad),
                Level::Warning => ("WARN", Tone::Warn),
                Level::Ok => ("OK", Tone::Good),
            };
            let who = match (p.site, &p.domain) {
                (Some(id), Some(d)) => format!("#{id} {d}: "),
                _ => String::new(),
            };
            out.push(vec![render::seg(format!("{tag:<5}"), tone), render::seg(who, Tone::Bold), render::seg(p.message.clone(), Tone::Plain)]);
        }
        if probs.is_empty() {
            out.push(vec![render::seg("OK   ", Tone::Good), render::seg(format!("{enabled} site{} up, certificates valid and renewing", if enabled == 1 { "" } else { "s" }), Tone::Plain)]);
        }
        render::print(&out);
    }
    if !a.notify.is_empty() || !a.notify_json.is_empty() || !a.mail.is_empty() {
        notify(a, &probs, worst)?;
    }
    Ok(code)
}

// ───────────────────────── notifications ─────────────────────────

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname").map(|h| h.trim().to_string()).unwrap_or_else(|_| "server".into())
}

fn report_text(probs: &[Problem]) -> String {
    if probs.is_empty() {
        return "All problems are resolved: every site is up and every certificate is valid and renewing.".into();
    }
    probs
        .iter()
        .map(|p| {
            let tag = if p.level == Level::Critical { "CRIT" } else { "WARN" };
            match (p.site, &p.domain) {
                (Some(id), Some(d)) => format!("{tag} #{id} {d}: {}", p.message),
                _ => format!("{tag} {}", p.message),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Problems seen by the previous run, to notify only on changes.
fn state_path() -> std::path::PathBuf {
    let env = Env::get();
    env.host(&env.state_dir.join("check-state.json"))
}

fn notify(a: &CheckArgs, probs: &[Problem], worst: Level) -> Result<()> {
    let now: Vec<String> = probs.iter().map(|p| format!("{:?} {:?} {}", p.level, p.site, p.message)).collect();
    let before: Vec<String> = std::fs::read_to_string(state_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let changed = now != before;
    let _ = std::fs::write(state_path(), serde_json::to_string(&now)?);
    if !(changed || (a.always && !probs.is_empty())) || (probs.is_empty() && before.is_empty()) {
        return Ok(());
    }
    let host = hostname();
    let title = if probs.is_empty() {
        format!("ngx on {host}: all clear")
    } else {
        let crit = probs.iter().filter(|p| p.level == Level::Critical).count();
        format!("ngx on {host}: {} problem{}{}", probs.len(), if probs.len() == 1 { "" } else { "s" }, if crit > 0 { format!(" ({crit} critical)") } else { String::new() })
    };
    let text = report_text(probs);
    let mut failures = Vec::new();
    for u in &a.notify {
        let priority = match worst {
            Level::Critical => "high",
            _ => "default",
        };
        if let Err(e) = post(u, "text/plain; charset=utf-8", &[("Title", &title), ("Priority", priority), ("Tags", "ngx")], &text) {
            failures.push(format!("{u}: {e}"));
        }
    }
    for u in &a.notify_json {
        let full = format!("{title}\n{text}");
        let body = serde_json::json!({ "text": full, "content": full, "status": worst, "host": host, "problems": probs }).to_string();
        if let Err(e) = post(u, "application/json", &[], &body) {
            failures.push(format!("{u}: {e}"));
        }
    }
    if !a.mail.is_empty()
        && let Err(e) = sendmail(&a.mail, &title, &text) {
            failures.push(format!("mail: {e}"));
        }
    if !failures.is_empty() {
        // notify again next time
        let _ = std::fs::remove_file(state_path());
        bail!("notification failed: {}", failures.join("; "));
    }
    Ok(())
}

/// POST with curl (argument list, body on stdin: nothing goes through a shell).
fn post(url: &str, ctype: &str, headers: &[(&str, &str)], body: &str) -> Result<()> {
    use std::io::Write;
    if Env::get().sandboxed() {
        println!("sandbox mode: would POST to {url}:\n{body}");
        return Ok(());
    }
    let curl = env::which("curl").ok_or_else(|| anyhow::anyhow!("curl is needed to send notifications"))?;
    let mut cmd = std::process::Command::new(curl);
    cmd.args(["-fsS", "--max-time", "15", "-X", "POST", "-H", &format!("Content-Type: {ctype}")]);
    for (k, v) in headers {
        // header values are ours (title, priority): keep them on one line
        cmd.arg("-H").arg(format!("{k}: {}", v.replace(['\r', '\n'], " ")));
    }
    cmd.args(["--data-binary", "@-", "--", url]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null());
    let mut child = cmd.spawn()?;
    child.stdin.take().unwrap().write_all(body.as_bytes())?;
    let st = child.wait()?;
    anyhow::ensure!(st.success(), "curl exited with {st}");
    Ok(())
}

fn sendmail(to: &[String], subject: &str, text: &str) -> Result<()> {
    use std::io::Write;
    if Env::get().sandboxed() {
        println!("sandbox mode: would mail {}: {subject}", to.join(", "));
        return Ok(());
    }
    let bin = env::which("sendmail").ok_or_else(|| anyhow::anyhow!("no sendmail found (install an MTA, e.g. msmtp-mta or postfix)"))?;
    let mut child = std::process::Command::new(bin).args(["-t", "-oi"]).stdin(std::process::Stdio::piped()).spawn()?;
    let msg = format!("To: {}\nSubject: {subject}\nContent-Type: text/plain; charset=utf-8\n\n{text}\n", to.join(", "));
    child.stdin.take().unwrap().write_all(msg.as_bytes())?;
    let st = child.wait()?;
    anyhow::ensure!(st.success(), "sendmail exited with {st}");
    Ok(())
}

// ───────────────────────── timer ─────────────────────────

const UNIT: &str = "/etc/systemd/system/ngx-check.service";
const TIMER: &str = "/etc/systemd/system/ngx-check.timer";
const CRON: &str = "/etc/cron.d/ngx-check";

fn has_systemd() -> bool {
    !Env::get().sandboxed() && Path::new("/run/systemd/system").exists() && env::which("systemctl").is_some()
}

/// The binary a root timer will run must not be replaceable by anyone else.
fn trusted_exe() -> Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let exe = std::env::current_exe()?;
    if Env::get().sandboxed() {
        return Ok(exe);
    }
    let mut p = Some(exe.as_path());
    while let Some(cur) = p {
        let m = std::fs::metadata(cur)?;
        if m.uid() != 0 || m.mode() & 0o022 != 0 {
            bail!(
                "{} is not owned by root or writable by others; a root timer must not run it. Install ngx system-wide first: sudo install -m755 {} /usr/local/bin/ngx",
                cur.display(),
                exe.display()
            );
        }
        p = cur.parent();
    }
    Ok(exe)
}

fn check_line(a: &CheckArgs) -> Vec<String> {
    let mut v = vec!["check".to_string(), "-q".into()];
    if a.max_5xx != 5.0 {
        v.extend(["--max-5xx".into(), a.max_5xx.to_string()]);
    }
    for u in &a.notify {
        v.extend(["--notify".into(), u.clone()]);
    }
    for u in &a.notify_json {
        v.extend(["--notify-json".into(), u.clone()]);
    }
    for m in &a.mail {
        v.extend(["--mail".into(), m.clone()]);
    }
    if a.always {
        v.push("--always".into());
    }
    v
}

fn interval(every: &str) -> Option<(u32, char)> {
    let unit = every.trim_start_matches(|c: char| c.is_ascii_digit());
    let n: u32 = every[..every.len() - unit.len()].parse().ok()?;
    let u = match unit {
        "m" | "min" => 'm',
        "h" => 'h',
        "d" => 'd',
        _ => return None,
    };
    (n > 0 && (u != 'm' || n >= 5)).then_some((n, u))
}

fn install_timer(a: &CheckArgs) -> Result<()> {
    let _lock = require_root()?;
    if a.notify.is_empty() && a.notify_json.is_empty() && a.mail.is_empty() {
        bail!("give at least one --notify, --notify-json or --mail, or nobody hears about problems");
    }
    for u in a.notify.iter().chain(&a.notify_json) {
        if !validate::url(u) {
            bail!("invalid URL {u:?}");
        }
    }
    for m in &a.mail {
        if !validate::email(m) {
            bail!("invalid email address {m:?}");
        }
    }
    let (n, unit) = interval(&a.every).ok_or_else(|| anyhow::anyhow!("invalid interval {:?} (e.g. 15m, 1h, 1d; at least 5m)", a.every))?;
    let exe = trusted_exe()?;
    let args = check_line(a);
    let mut tx = Tx::new(false);
    // the files hold notification URLs, which often are secrets (ntfy topics, webhook tokens): root-only
    if has_systemd() {
        let exec = std::iter::once(exe.display().to_string()).chain(args).map(|s| s.replace('%', "%%")).collect::<Vec<_>>().join(" ");
        let span = format!("{n}{}", match unit { 'm' => "min", 'h' => "h", _ => "d" });
        tx.write_bytes(Path::new(UNIT), format!("# installed by `ngx check --install-timer`\n[Unit]\nDescription=ngx check: sites and certificates\nAfter=network-online.target\n\n[Service]\nType=oneshot\nExecStart={exec}\n# 1 = warnings, 2 = critical: reported through the notifications, not a unit failure\nSuccessExitStatus=1 2\n").as_bytes(), Some(0o600))?;
        tx.write_bytes(Path::new(TIMER), format!("# installed by `ngx check --install-timer`\n[Unit]\nDescription=Run ngx check every {}\n\n[Timer]\nOnBootSec=5min\nOnUnitActiveSec={span}\n\n[Install]\nWantedBy=timers.target\n", a.every).as_bytes(), Some(0o600))?;
        tx.commit_files()?;
        for c in [&["daemon-reload"][..], &["enable", "--now", "ngx-check.timer"]] {
            let (ok, out) = env::run("systemctl", c).unwrap_or((false, String::new()));
            anyhow::ensure!(ok, "systemctl {} failed: {out}", c.join(" "));
        }
        ok(&format!("ngx check runs every {} (systemctl list-timers ngx-check.timer; logs: journalctl -u ngx-check)", a.every));
    } else {
        let sched = match unit {
            'm' => format!("*/{n} * * * *"),
            'h' => format!("0 */{n} * * *"),
            _ => "0 8 * * *".into(),
        };
        let line = std::iter::once(exe.display().to_string()).chain(args).map(|s| s.replace('%', "\\%")).collect::<Vec<_>>().join(" ");
        tx.write_bytes(Path::new(CRON), format!("# installed by `ngx check --install-timer`\n{sched} root {line}\n").as_bytes(), Some(0o600))?;
        tx.commit_files()?;
        ok(&format!("ngx check runs from cron ({CRON}, {sched})"));
    }
    Ok(())
}

fn remove_timer() -> Result<()> {
    let _lock = require_root()?;
    let env = Env::get();
    if has_systemd() && env.host(Path::new(TIMER)).exists() {
        let _ = env::run("systemctl", &["disable", "--now", "ngx-check.timer"]);
    }
    let mut tx = Tx::new(false);
    let mut any = false;
    for f in [UNIT, TIMER, CRON] {
        if env.host(Path::new(f)).exists() {
            tx.remove(Path::new(f))?;
            any = true;
        }
    }
    if !any {
        println!("no ngx check timer installed");
        return Ok(());
    }
    tx.commit_files()?;
    if has_systemd() {
        let _ = env::run("systemctl", &["daemon-reload"]);
    }
    ok("ngx check timer removed");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn intervals() {
        assert_eq!(super::interval("15m"), Some((15, 'm')));
        assert_eq!(super::interval("1h"), Some((1, 'h')));
        assert_eq!(super::interval("1d"), Some((1, 'd')));
        assert_eq!(super::interval("1m"), None);
        assert_eq!(super::interval("h"), None);
        assert_eq!(super::interval("10x"), None);
    }
}
