//! Filesystem layout and environment detection.
//!
//! All paths handled by ngx are *logical* paths, i.e. the absolute paths nginx
//! itself sees (`/etc/nginx/sites-enabled/x.conf`). `Env::host` maps a logical
//! path to the real location on disk, which differs only when `NGX_ROOT` is set
//! (sandbox mode, used for tests and demos).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub struct Env {
    /// Sandbox prefix (`NGX_ROOT`); `None` on a real system.
    pub root: Option<PathBuf>,
    pub nginx_dir: PathBuf,
    pub le_dir: PathBuf,
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
    pub acme_webroot: PathBuf,
}

static ENV: OnceLock<Env> = OnceLock::new();

fn var_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl Env {
    pub fn get() -> &'static Env {
        ENV.get_or_init(|| {
            // NGX_ROOT=/ would be the real system with nginx -t and reloads skipped: ignore it
            let root = var_path("NGX_ROOT").map(|r| std::fs::canonicalize(&r).unwrap_or(r)).filter(|r| r != Path::new("/"));
            let mut env = Env {
                root,
                nginx_dir: PathBuf::from("/etc/nginx"),
                le_dir: var_path("NGX_LETSENCRYPT_DIR").unwrap_or_else(|| "/etc/letsencrypt".into()),
                state_dir: var_path("NGX_STATE_DIR").unwrap_or_else(|| "/var/lib/ngx".into()),
                log_dir: PathBuf::from("/var/log/nginx"),
                acme_webroot: PathBuf::from("/var/lib/ngx/acme"),
            };
            env.nginx_dir = var_path("NGX_NGINX_DIR").unwrap_or_else(|| {
                ["/etc/nginx", "/usr/local/etc/nginx", "/usr/local/nginx/conf"]
                    .iter()
                    .map(PathBuf::from)
                    .find(|p| env.host(p).join("nginx.conf").exists())
                    .unwrap_or_else(|| "/etc/nginx".into())
            });
            env.acme_webroot = env.state_dir.join("acme");
            env
        })
    }

    /// Map a logical path to the on-disk path.
    pub fn host(&self, p: &Path) -> PathBuf {
        match &self.root {
            Some(r) if p.is_absolute() => r.join(p.strip_prefix("/").unwrap_or(p)),
            _ => p.to_path_buf(),
        }
    }

    pub fn sandboxed(&self) -> bool {
        self.root.is_some()
    }

    pub fn sites_available(&self) -> PathBuf {
        self.nginx_dir.join("sites-available")
    }
    pub fn sites_enabled(&self) -> PathBuf {
        self.nginx_dir.join("sites-enabled")
    }
    pub fn conf_d(&self) -> PathBuf {
        self.nginx_dir.join("conf.d")
    }
    pub fn nginx_conf(&self) -> PathBuf {
        self.nginx_dir.join("nginx.conf")
    }

    /// Debian-style sites-available / sites-enabled layout present?
    pub fn has_sites_layout(&self) -> bool {
        self.host(&self.sites_available()).is_dir() && self.host(&self.sites_enabled()).is_dir()
    }

    /// Path relative to the nginx dir for compact display (`sites-enabled/x.conf`).
    pub fn short(&self, p: &Path) -> String {
        p.strip_prefix(&self.nginx_dir).unwrap_or(p).display().to_string()
    }

    /// Resolve a symlink's target as a logical path.
    pub fn read_link(&self, link: &Path) -> Option<PathBuf> {
        let target = std::fs::read_link(self.host(link)).ok()?;
        Some(if target.is_absolute() {
            target
        } else {
            link.parent().unwrap_or(Path::new("/")).join(target)
        })
        .map(|p| normalize(&p))
    }
}

/// Lexically normalize `a/b/../c` → `a/c`.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Find an executable in PATH (plus sbin dirs, which are often missing for users).
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(["/usr/sbin", "/sbin", "/usr/local/sbin", "/snap/bin"].iter().map(PathBuf::from))
        .map(|d| d.join(name))
        .find(|p| {
            use std::os::unix::fs::PermissionsExt;
            p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
        })
}

/// Run a command and return trimmed stdout+stderr if it ran at all.
pub fn run(cmd: &str, args: &[&str]) -> Option<(bool, String)> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Some((out.status.success(), s.trim().to_string()))
}

/// Error out unless we can modify the system.
pub fn require_root() -> anyhow::Result<()> {
    if is_root() || Env::get().sandboxed() {
        Ok(())
    } else {
        anyhow::bail!("this command modifies nginx configuration and must run as root (try: sudo ngx ...)")
    }
}
