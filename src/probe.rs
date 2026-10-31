//! Runtime probing: is the upstream reachable, and which process serves it?

use crate::env::{self, Env};
use crate::nginx::site::Upstream;
use chrono::{DateTime, Local, TimeZone};
use std::collections::HashSet;
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Try to connect to the upstream. `Err` carries a short reason.
pub fn check_upstream(u: &Upstream) -> Result<(), String> {
    let reason = |e: std::io::Error| match e.kind() {
        ErrorKind::ConnectionRefused => "refused connection".to_string(),
        ErrorKind::TimedOut | ErrorKind::WouldBlock => "timed out".to_string(),
        ErrorKind::NotFound => "socket does not exist".to_string(),
        ErrorKind::PermissionDenied => "permission denied".to_string(),
        _ => e.to_string(),
    };
    if Env::get().sandboxed() {
        return sandbox_probe(u);
    }
    match u {
        Upstream::Tcp { host, port } => {
            let addrs: Vec<_> = (host.as_str(), *port)
                .to_socket_addrs()
                .map_err(|_| format!("cannot resolve {host}"))?
                .collect();
            let mut last = None;
            for a in addrs {
                match TcpStream::connect_timeout(&a, Duration::from_millis(1500)) {
                    Ok(_) => return Ok(()),
                    Err(e) => last = Some(e),
                }
            }
            Err(last.map(reason).unwrap_or_else(|| format!("cannot resolve {host}")))
        }
        Upstream::Unix(p) => std::os::unix::net::UnixStream::connect(p).map(|_| ()).map_err(reason),
        Upstream::Other(_) => Ok(()),
    }
}

/// Sandbox: `$NGX_ROOT/probe/down` lists upstreams that should appear down.
fn sandbox_probe(u: &Upstream) -> Result<(), String> {
    let down = std::fs::read_to_string(Env::get().host(Path::new("/probe/down"))).unwrap_or_default();
    if down.lines().any(|l| l.trim() == u.to_string()) { Err("refused connection".into()) } else { Ok(()) }
}

// ───────────────────────── processes ─────────────────────────

#[derive(Debug, Clone)]
pub enum Manager {
    Systemd(String),
    Pm2(String),
    Docker(String),
    Supervisor,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ProcInfo {
    pub cwd: Option<PathBuf>,
    pub started: Option<DateTime<Local>>,
    pub manager: Manager,
    pub runtime: Option<String>,
    /// e.g. `pool "www", 6 workers`
    pub workers: Option<String>,
}

fn listening_inodes(port: u16) -> HashSet<u64> {
    let mut out = HashSet::new();
    for f in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(s) = std::fs::read_to_string(f) else { continue };
        for l in s.lines().skip(1) {
            let c: Vec<&str> = l.split_whitespace().collect();
            if c.len() < 10 || c[3] != "0A" {
                continue;
            }
            let p = c[1].rsplit(':').next().and_then(|h| u16::from_str_radix(h, 16).ok());
            if p == Some(port) {
                if let Ok(i) = c[9].parse() {
                    out.insert(i);
                }
            }
        }
    }
    out
}

fn unix_inodes(path: &Path) -> HashSet<u64> {
    let mut out = HashSet::new();
    if let Ok(s) = std::fs::read_to_string("/proc/net/unix") {
        for l in s.lines().skip(1) {
            let c: Vec<&str> = l.split_whitespace().collect();
            if c.len() >= 8 && Path::new(c[7]) == path {
                if let Ok(i) = c[6].parse() {
                    out.insert(i);
                }
            }
        }
    }
    out
}

fn pids() -> impl Iterator<Item = u32> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
}

fn owner_of(inodes: &HashSet<u64>) -> Option<u32> {
    if inodes.is_empty() {
        return None;
    }
    let mut found: Vec<u32> = pids()
        .filter(|pid| {
            std::fs::read_dir(format!("/proc/{pid}/fd"))
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .any(|e| {
                    std::fs::read_link(e.path())
                        .ok()
                        .and_then(|t| t.to_str()?.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok())
                        .is_some_and(|i: u64| inodes.contains(&i))
                })
        })
        .collect();
    found.sort();
    found.first().copied()
}

/// Something is listening on the upstream address, but can we see who?
pub enum Listener {
    Pid(u32),
    /// Listening socket exists but its owner is hidden (not root).
    Hidden,
    None,
}

pub fn listener(u: &Upstream) -> Listener {
    if Env::get().sandboxed() {
        return if sandbox_probe(u).is_ok() { Listener::Hidden } else { Listener::None };
    }
    let inodes = match u {
        Upstream::Tcp { port, .. } => listening_inodes(*port),
        Upstream::Unix(p) => unix_inodes(p),
        Upstream::Other(_) => return Listener::None,
    };
    if inodes.is_empty() {
        return Listener::None;
    }
    owner_of(&inodes).map(Listener::Pid).unwrap_or(Listener::Hidden)
}

/// (ppid, starttime ticks) from /proc/pid/stat
fn stat(pid: u32) -> Option<(u32, u64)> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = &s[s.rfind(')')? + 2..];
    let f: Vec<&str> = after.split_whitespace().collect();
    Some((f.get(1)?.parse().ok()?, f.get(19)?.parse().ok()?))
}

fn cmdline(pid: u32) -> Vec<String> {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|b| b.split(|c| *c == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect())
        .unwrap_or_default()
}

fn environ(pid: u32, key: &str) -> Option<String> {
    let b = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    b.split(|c| *c == 0).find_map(|kv| {
        let kv = String::from_utf8_lossy(kv);
        kv.strip_prefix(key).and_then(|r| r.strip_prefix('=')).map(String::from)
    })
}

fn start_time(ticks: u64) -> Option<DateTime<Local>> {
    let btime: i64 = std::fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())?;
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
    Local.timestamp_opt(btime + (ticks / hz) as i64, 0).single()
}

fn manager(pid: u32) -> Manager {
    // pm2: an ancestor is the "PM2 vX: God Daemon"
    let mut p = pid;
    for _ in 0..8 {
        let Some((ppid, _)) = stat(p) else { break };
        if ppid <= 1 {
            break;
        }
        if cmdline(ppid).first().is_some_and(|c| c.starts_with("PM2")) {
            let name = environ(pid, "name").or_else(|| environ(pid, "pm_id")).unwrap_or_else(|| "?".into());
            return Manager::Pm2(name);
        }
        if cmdline(ppid).iter().any(|c| c.contains("supervisord")) {
            return Manager::Supervisor;
        }
        p = ppid;
    }
    let cg = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default();
    let path = cg.lines().find_map(|l| l.strip_prefix("0::")).unwrap_or("");
    if path.contains("docker") || path.contains("containerd") || path.contains("libpod") {
        let id = path.rsplit(['-', '/']).next().unwrap_or("").trim_end_matches(".scope");
        return Manager::Docker(id.chars().take(12).collect());
    }
    if let Some(unit) = path.rsplit('/').find(|s| s.ends_with(".service")) {
        return Manager::Systemd(unit.to_string());
    }
    Manager::Unknown
}

fn first_line(cmd: &Path, arg: &str) -> Option<String> {
    let (_, out) = env::run(cmd.to_str()?, &[arg])?;
    out.lines().next().map(|s| s.trim().to_string())
}

fn runtime(pid: u32, exe: &Path) -> Option<String> {
    let name = exe.file_name()?.to_string_lossy().into_owned();
    let proc_exe = PathBuf::from(format!("/proc/{pid}/exe"));
    let args = cmdline(pid);
    if name == "node" || name.starts_with("node") {
        let v = first_line(&proc_exe, "--version")?;
        return Some(format!("Node.js {}", v.trim_start_matches('v')));
    }
    if name == "bun" || name == "deno" {
        let v = first_line(&proc_exe, "--version")?;
        return Some(v);
    }
    if name.starts_with("php-fpm") {
        let v = first_line(&proc_exe, "-v")?;
        let ver = v.split_whitespace().nth(1)?;
        return Some(format!("PHP {ver} (php-fpm)"));
    }
    if name.starts_with("python") {
        let v = first_line(&proc_exe, "--version")?;
        let server = args.iter().find_map(|a| {
            ["gunicorn", "uvicorn", "uwsgi", "hypercorn", "daphne", "flask", "manage.py"]
                .into_iter()
                .find(|s| a.contains(s))
        });
        return Some(match server {
            Some(s) => format!("{v} ({s})"),
            None => v,
        });
    }
    if name == "java" {
        let (_, out) = env::run(proc_exe.to_str()?, &["-version"])?;
        return out.lines().next().map(String::from);
    }
    Some(name)
}

pub fn proc_info(pid: u32) -> ProcInfo {
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
    let cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).ok();
    let started = stat(pid).and_then(|(_, t)| start_time(t));
    let runtime = exe.as_deref().and_then(|e| runtime(pid, e));
    let workers = if cmdline(pid).first().is_some_and(|c| c.starts_with("php-fpm: master")) {
        let mut pools: Vec<String> = Vec::new();
        let mut n = 0;
        for p in pids() {
            if stat(p).is_some_and(|(pp, _)| pp == pid) {
                n += 1;
                if let Some(pool) = cmdline(p).first().and_then(|c| c.strip_prefix("php-fpm: pool ").map(|s| s.trim().to_string())) {
                    if !pools.contains(&pool) {
                        pools.push(pool);
                    }
                }
            }
        }
        let pools = pools.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(", ");
        Some(format!("pool {pools}, {n} workers"))
    } else {
        None
    };
    ProcInfo { cwd, started, manager: manager(pid), runtime, workers }
}

/// When nothing is listening: try to find what *should* be serving it.
#[derive(Debug, Clone, Default)]
pub struct Stopped {
    pub manager: Option<Manager>,
    pub detail: String,
    pub cwd: Option<String>,
}

pub fn stopped_backend(u: &Upstream, roots: &[&Path]) -> Stopped {
    if Env::get().sandboxed() {
        return Stopped { detail: "not running".into(), ..Default::default() };
    }
    // pm2 apps matched by PORT env or working directory
    if env::which("pm2").is_some() {
        if let Some((true, out)) = env::run("pm2", &["jlist"]) {
            let json = out.find('[').map(|i| &out[i..]).unwrap_or("[]");
            if let Ok(serde_json::Value::Array(apps)) = serde_json::from_str::<serde_json::Value>(json) {
                for app in apps {
                    let e = &app["pm2_env"];
                    let port = e["env"]["PORT"].as_str().map(String::from).or_else(|| e["env"]["PORT"].as_u64().map(|p| p.to_string()));
                    let cwd = e["pm_cwd"].as_str().unwrap_or("");
                    let port_match = matches!(u, Upstream::Tcp { port: p, .. } if port.as_deref() == Some(&p.to_string()));
                    let cwd_match = !cwd.is_empty() && roots.iter().any(|r| r.starts_with(cwd) || Path::new(cwd).starts_with(r));
                    if port_match || cwd_match {
                        let name = app["name"].as_str().unwrap_or("?").to_string();
                        let status = e["status"].as_str().unwrap_or("stopped");
                        let code = e["exit_code"].as_i64().map(|c| format!(", code {c}")).unwrap_or_default();
                        let when = e["pm_uptime"].as_i64().and_then(|ms| Local.timestamp_millis_opt(ms).single())
                            .map(|t| format!(" · last exit {}", t.format("%d/%m/%y %H:%M"))).unwrap_or_default();
                        return Stopped {
                            manager: Some(Manager::Pm2(name)),
                            detail: format!("not running ({status}){when}{code}"),
                            cwd: Some(cwd.to_string()).filter(|c| !c.is_empty()),
                        };
                    }
                }
            }
        }
    }
    // php-fpm style sockets: guess the systemd unit from the socket name
    if let Upstream::Unix(p) = u {
        if let Some(stem) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) {
            let unit = format!("{stem}.service");
            if let Some((_, out)) = env::run("systemctl", &["show", &unit, "-p", "LoadState,ActiveState,SubState,ExecMainStatus,ExecMainExitTimestamp"]) {
                let get = |k: &str| out.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap_or("").to_string();
                if get("LoadState") == "loaded" {
                    let ts = get("ExecMainExitTimestamp");
                    let when = if ts.is_empty() { String::new() } else { format!(" · last exit {ts}") };
                    return Stopped {
                        manager: Some(Manager::Systemd(unit)),
                        detail: format!("not running ({}/{}){when}, code {}", get("ActiveState"), get("SubState"), get("ExecMainStatus")),
                        cwd: None,
                    };
                }
            }
        }
    }
    let what = match u {
        Upstream::Tcp { port, .. } => format!("nothing listening on :{port}"),
        Upstream::Unix(p) => format!("no socket at {}", p.display()),
        Upstream::Other(s) => s.clone(),
    };
    Stopped { manager: None, detail: format!("not running ({what})"), cwd: None }
}

pub fn fmt_uptime(since: DateTime<Local>) -> String {
    let d = Local::now() - since;
    let (days, hours, mins) = (d.num_days(), d.num_hours() % 24, d.num_minutes() % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}
