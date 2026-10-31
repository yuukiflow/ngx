//! Safe configuration changes: back up, apply, `nginx -t`, roll back on
//! failure, reload on success.

use crate::env::Env;
use crate::nginx::ctl;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

enum Orig {
    Absent,
    File(Vec<u8>),
    Link(PathBuf),
}

pub struct Tx {
    orig: Vec<(PathBuf, Orig)>,
    backup_dir: PathBuf,
    pub dry_run: bool,
    pub log: Vec<String>,
}

impl Tx {
    pub fn new(dry_run: bool) -> Tx {
        let env = Env::get();
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        Tx { orig: Vec::new(), backup_dir: env.host(&env.state_dir.join("backups").join(ts.to_string())), dry_run, log: Vec::new() }
    }

    fn remember(&mut self, host: &Path) -> Result<()> {
        if self.orig.iter().any(|(p, _)| p == host) {
            return Ok(());
        }
        let orig = match std::fs::symlink_metadata(host) {
            Err(_) => Orig::Absent,
            Ok(m) if m.file_type().is_symlink() => Orig::Link(std::fs::read_link(host)?),
            Ok(_) => {
                let data = std::fs::read(host)?;
                if !self.dry_run {
                    std::fs::create_dir_all(&self.backup_dir)?;
                    let name = host.to_string_lossy().trim_start_matches('/').replace('/', "%");
                    std::fs::write(self.backup_dir.join(name), &data)?;
                }
                Orig::File(data)
            }
        };
        self.orig.push((host.to_path_buf(), orig));
        Ok(())
    }

    pub fn write(&mut self, path: &Path, content: &str) -> Result<()> {
        let host = Env::get().host(path);
        self.log.push(format!("write   {}", path.display()));
        if self.dry_run {
            return Ok(());
        }
        self.remember(&host)?;
        if let Some(dir) = host.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = host.with_extension("ngx-tmp");
        std::fs::write(&tmp, content)?;
        std::fs::rename(&tmp, &host)?;
        Ok(())
    }

    pub fn symlink(&mut self, target: &Path, link: &Path) -> Result<()> {
        let host = Env::get().host(link);
        self.log.push(format!("link    {} → {}", link.display(), target.display()));
        if self.dry_run {
            return Ok(());
        }
        self.remember(&host)?;
        if std::fs::symlink_metadata(&host).is_ok() {
            std::fs::remove_file(&host)?;
        }
        if let Some(dir) = host.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::os::unix::fs::symlink(target, &host)?;
        Ok(())
    }

    pub fn remove(&mut self, path: &Path) -> Result<()> {
        let host = Env::get().host(path);
        self.log.push(format!("remove  {}", path.display()));
        if self.dry_run {
            return Ok(());
        }
        self.remember(&host)?;
        if std::fs::symlink_metadata(&host).is_ok() {
            std::fs::remove_file(&host)?;
        }
        Ok(())
    }

    pub fn rename(&mut self, from: &Path, to: &Path) -> Result<()> {
        let data = std::fs::read_to_string(Env::get().host(from))?;
        self.write(to, &data)?;
        self.remove(from)
    }

    pub fn rollback(&mut self) {
        for (host, orig) in self.orig.drain(..).rev() {
            let _ = std::fs::remove_file(&host);
            match orig {
                Orig::Absent => {}
                Orig::File(d) => {
                    let _ = std::fs::write(&host, d);
                }
                Orig::Link(t) => {
                    let _ = std::os::unix::fs::symlink(t, &host);
                }
            }
        }
    }

    /// Validate with `nginx -t`; roll back on failure, reload on success.
    pub fn commit(mut self) -> Result<()> {
        if self.dry_run {
            println!("dry run — would apply:");
            for l in &self.log {
                println!("  {l}");
            }
            return Ok(());
        }
        match ctl::test() {
            None => {
                if Env::get().sandboxed() {
                    println!("sandbox mode: skipped nginx -t and reload");
                } else {
                    println!("warning: nginx binary not found — configuration was written but not tested");
                }
                return Ok(());
            }
            Some(t) if !t.ok => {
                self.rollback();
                let msgs: Vec<String> = if t.msgs.is_empty() { vec![t.raw] } else { t.msgs.iter().map(|m| m.to_string()).collect() };
                bail!("nginx -t failed, changes rolled back:\n  {}", msgs.join("\n  "));
            }
            Some(_) => {}
        }
        if !self.orig.is_empty() && self.backup_dir.exists() {
            println!("backup: {}", self.backup_dir.display());
        }
        println!("{}", ctl::reload()?);
        Ok(())
    }
}
