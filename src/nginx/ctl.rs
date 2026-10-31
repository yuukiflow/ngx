//! Talking to the nginx binary: config test, reload, version, running state.

use crate::env::{self, Env};
use std::path::PathBuf;

pub fn bin() -> Option<PathBuf> {
    std::env::var_os("NGX_NGINX_BIN").map(PathBuf::from).or_else(|| env::which("nginx"))
}

#[derive(Debug, Clone)]
pub struct Msg {
    pub level: String,
    pub text: String,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
}

impl std::fmt::Display for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.level, self.text)?;
        if let (Some(file), Some(line)) = (&self.file, self.line) {
            write!(f, " ({}:{line})", Env::get().short(file))?;
        }
        Ok(())
    }
}

pub struct TestResult {
    pub ok: bool,
    pub msgs: Vec<Msg>,
    pub raw: String,
}

/// Parse `nginx: [emerg] message in /path/file.conf:12` lines.
pub fn parse_output(out: &str) -> Vec<Msg> {
    out.lines()
        .filter_map(|l| {
            // `nginx: [emerg] msg` or `2026/10/31 19:49:19 [emerg] 92#92: msg`
            let rest = &l[l.find('[')? + 1..];
            let (level, text) = rest.split_once("] ")?;
            if !matches!(level, "emerg" | "alert" | "crit" | "error" | "warn") {
                return None;
            }
            let text = match text.split_once(": ") {
                Some((pid, t)) if pid.split('#').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) => t,
                _ => text,
            };
            let (mut text, mut file, mut line) = (text.to_string(), None, None);
            if let Some((t, loc)) = text.clone().rsplit_once(" in ") {
                if let Some((f, n)) = loc.rsplit_once(':') {
                    if let Ok(n) = n.parse() {
                        text = t.to_string();
                        file = Some(PathBuf::from(f));
                        line = Some(n);
                    }
                }
            }
            Some(Msg { level: level.to_string(), text, file, line })
        })
        .collect()
}

/// Run `nginx -t`. `None` when nginx can't be tested here (missing / sandbox).
pub fn test() -> Option<TestResult> {
    if Env::get().sandboxed() {
        return None;
    }
    let bin = bin()?;
    let (ok, raw) = env::run(bin.to_str()?, &["-t", "-q"])?;
    Some(TestResult { ok, msgs: parse_output(&raw), raw })
}

pub fn version() -> Option<String> {
    let (_, out) = env::run(bin()?.to_str()?, &["-v"])?;
    out.split('/').nth(1).map(|v| v.split_whitespace().next().unwrap_or(v).to_string())
}

fn procs_matching(prefix: &str) -> Vec<u32> {
    std::fs::read_dir("/proc")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let pid: u32 = e.file_name().to_str()?.parse().ok()?;
                    let c = std::fs::read_to_string(e.path().join("cmdline")).ok()?;
                    c.starts_with(prefix).then_some(pid)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Is an nginx master process running?
pub fn running() -> bool {
    Env::get().sandboxed() || !procs_matching("nginx: master").is_empty()
}

/// Reload signals are handled asynchronously: wait until fresh worker
/// processes exist, which means the master accepted the new configuration.
fn wait_for_new_workers(before: &[u32]) -> anyhow::Result<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
        // new workers are up, and every old one is gone or "is shutting down" (no longer accepting)
        let fresh = procs_matching("nginx: worker").iter().any(|p| !before.contains(p));
        let old_active = procs_matching("nginx: worker process").iter().any(|p| {
            before.contains(p)
                && std::fs::read_to_string(format!("/proc/{p}/cmdline")).is_ok_and(|c| !c.contains("shutting down"))
        });
        if fresh && !old_active {
            return Ok("nginx reloaded".into());
        }
    }
    anyhow::bail!("nginx did not pick up the new configuration within 5s — check {}", Env::get().log_dir.join("error.log").display())
}

/// Reload nginx; returns a human-readable description of what happened.
pub fn reload() -> anyhow::Result<String> {
    if Env::get().sandboxed() {
        return Ok("sandbox mode: reload skipped".into());
    }
    if !running() {
        return Ok("nginx is not running — start it with `systemctl start nginx`".into());
    }
    let before = procs_matching("nginx: worker");
    if env::which("systemctl").is_some()
        && env::run("systemctl", &["is-active", "--quiet", "nginx"]).is_some_and(|r| r.0)
    {
        let (ok, out) = env::run("systemctl", &["reload", "nginx"]).unwrap_or((false, String::new()));
        anyhow::ensure!(ok, "systemctl reload nginx failed: {out}");
        return wait_for_new_workers(&before);
    }
    let bin = bin().ok_or_else(|| anyhow::anyhow!("nginx binary not found"))?;
    let (ok, out) = env::run(bin.to_str().unwrap_or("nginx"), &["-s", "reload"]).unwrap_or((false, String::new()));
    anyhow::ensure!(ok, "nginx -s reload failed: {out}");
    wait_for_new_workers(&before)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_test_output() {
        let m = super::parse_output(
            "nginx: [emerg] unknown directive \"proxy_pas\" in /etc/nginx/sites-enabled/a.conf:12\n\
             nginx: [warn] conflicting server name \"x.com\" on 0.0.0.0:80, ignored\n\
             2026/10/31 19:49:19 [emerg] 92#92: unknown directive \"bogus\" in /etc/nginx/sites-enabled/bad.conf:1\n\
             nginx: configuration file /etc/nginx/nginx.conf test failed",
        );
        assert_eq!(m.len(), 3);
        assert_eq!(m[2].text, "unknown directive \"bogus\"");
        assert_eq!(m[2].line, Some(1));
        assert_eq!(m[0].level, "emerg");
        assert_eq!(m[0].line, Some(12));
        assert_eq!(m[0].text, "unknown directive \"proxy_pas\"");
        assert_eq!(m[1].file, None);
    }
}
