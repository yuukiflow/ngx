//! Safe configuration changes: back up, apply, `nginx -t`, roll back on
//! failure, reload on success. Every command's changes are recorded as one
//! entry in the change history (`ngx history`), which `ngx undo` can revert.

use crate::env::Env;
use crate::nginx::ctl;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How many history entries (backup directories) are kept.
pub const KEEP_BACKUPS: usize = 50;

enum Orig {
    Absent,
    File(Vec<u8>, u32),
    Link(PathBuf),
}

/// What a path looked like before a change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "was", rename_all = "lowercase")]
pub enum Before {
    /// Didn't exist: undo removes it.
    Absent,
    /// A regular file, saved as `backup` in the entry's directory.
    File { backup: String, mode: u32 },
    /// A symlink to `target`.
    Link { target: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Logical path (as nginx sees it).
    pub path: PathBuf,
    #[serde(flatten)]
    pub before: Before,
}

/// `manifest.json` in each backup directory.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    pub time: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    pub files: Vec<Entry>,
    /// Site ID moves made by the command (rename: old key → new key; rm: key → none),
    /// so undo can put IDs back without ever reusing one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<IdChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdChange {
    pub id: u32,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// Record an ID change in the current history entry (called by `state`).
pub fn note_id(id: u32, before: Option<&str>, after: Option<&str>) {
    let mut g = GROUP.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(gr) = g.as_mut() {
        gr.manifest.ids.push(IdChange { id, before: before.map(String::from), after: after.map(String::from) });
        if gr.dir.join("manifest.json").exists() {
            let _ = save_manifest(&gr.dir, &gr.manifest);
        }
    }
}

/// The history entry the current command writes to. All transactions of one
/// command (e.g. the HTTP bootstrap and the final HTTPS config of `ngx add`)
/// share it, so one `ngx undo` reverts the whole command.
struct Group {
    id: String,
    dir: PathBuf,
    manifest: Manifest,
    /// Paths recorded by a transaction that hasn't committed yet.
    pending: Vec<PathBuf>,
}

static GROUP: Mutex<Option<Group>> = Mutex::new(None);
static LABEL: Mutex<Option<String>> = Mutex::new(None);

fn backups_root() -> PathBuf {
    let env = Env::get();
    env.host(&env.state_dir.join("backups"))
}

/// Start a new history entry for the next change. The CLI runs one command per
/// process; the dashboard calls this before each action.
pub fn new_change(label: &str) {
    *GROUP.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *LABEL.lock().unwrap_or_else(|e| e.into_inner()) = Some(label.to_string());
}

fn command_line() -> String {
    if let Some(l) = LABEL.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return l;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "tui" { "dashboard".into() } else { format!("ngx {}", args.join(" ")) }
}

/// Write a file atomically (temp file + rename) with an explicit mode. The temp
/// file is created exclusively, so a pre-planted file or symlink is never followed.
fn write_atomic(host: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    let tmp = host.with_extension("ngx-tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(&tmp)?;
    f.set_permissions(std::fs::Permissions::from_mode(mode))?;
    f.write_all(data)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, host)
}

/// Backups can contain anything a config referenced: keep them root-only.
fn private_dir(dir: &Path) -> std::io::Result<()> {
    if let Some(parent) = dir.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    std::fs::DirBuilder::new().mode(0o700).create(dir)
}

fn save_manifest(dir: &Path, m: &Manifest) -> Result<()> {
    let json = serde_json::to_string_pretty(m)?;
    write_atomic(&dir.join("manifest.json"), json.as_bytes(), 0o600)?;
    Ok(())
}

impl Group {
    fn create() -> Result<Group> {
        let now = chrono::Local::now();
        let base = now.format("%Y%m%d-%H%M%S").to_string();
        let root = backups_root();
        // unique even when several commands run within the same second
        let mut n = 1;
        let (id, dir) = loop {
            let id = if n == 1 { base.clone() } else { format!("{base}-{n}") };
            let dir = root.join(&id);
            match private_dir(&dir) {
                Ok(()) => break (id, dir),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 1000 => n += 1,
                Err(e) => return Err(e).with_context(|| format!("creating {}", dir.display())),
            }
        };
        let user = std::env::var("SUDO_USER").ok().or_else(|| std::env::var("USER").ok());
        let manifest = Manifest { time: now.format("%d/%m/%y %H:%M:%S").to_string(), command: command_line(), user, files: Vec::new(), ids: Vec::new() };
        Ok(Group { id, dir, manifest, pending: Vec::new() })
    }
}

pub struct Tx {
    orig: Vec<(PathBuf, Orig)>,
    pub dry_run: bool,
    pub log: Vec<String>,
}

impl Tx {
    pub fn new(dry_run: bool) -> Tx {
        Tx { orig: Vec::new(), dry_run, log: Vec::new() }
    }

    /// Save the current state of `path` (for rollback, and for the history the
    /// first time this command touches it).
    fn remember(&mut self, path: &Path, host: &Path) -> Result<()> {
        if self.orig.iter().any(|(p, _)| p == host) {
            return Ok(());
        }
        let meta = std::fs::symlink_metadata(host);
        let orig = match &meta {
            Err(_) => Orig::Absent,
            Ok(m) if m.file_type().is_symlink() => Orig::Link(std::fs::read_link(host)?),
            Ok(m) => Orig::File(std::fs::read(host)?, m.permissions().mode() & 0o7777),
        };
        let mut g = GROUP.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            *g = Some(Group::create()?);
        }
        let g = g.as_mut().unwrap();
        if !g.manifest.files.iter().any(|e| e.path == path) {
            let before = match &orig {
                Orig::Absent => Before::Absent,
                Orig::Link(t) => Before::Link { target: t.clone() },
                Orig::File(data, mode) => {
                    let backup = path.to_string_lossy().trim_start_matches('/').replace('/', "%");
                    std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(g.dir.join(&backup))?.write_all(data)?;
                    Before::File { backup, mode: *mode }
                }
            };
            g.manifest.files.push(Entry { path: path.to_path_buf(), before });
            g.pending.push(path.to_path_buf());
        }
        self.orig.push((host.to_path_buf(), orig));
        Ok(())
    }

    pub fn write(&mut self, path: &Path, content: &str) -> Result<()> {
        self.write_bytes(path, content.as_bytes(), None)
    }

    /// Write a file; `mode` defaults to the existing file's mode, or 0644 for a
    /// new one (like nginx's own configs).
    pub fn write_bytes(&mut self, path: &Path, content: &[u8], mode: Option<u32>) -> Result<()> {
        let host = Env::get().host(path);
        self.log.push(format!("write   {}", path.display()));
        if self.dry_run {
            return Ok(());
        }
        self.remember(path, &host)?;
        if let Some(dir) = host.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mode = mode.unwrap_or_else(|| std::fs::metadata(&host).map(|m| m.permissions().mode() & 0o7777).unwrap_or(0o644));
        write_atomic(&host, content, mode)?;
        Ok(())
    }

    pub fn symlink(&mut self, target: &Path, link: &Path) -> Result<()> {
        let host = Env::get().host(link);
        self.log.push(format!("link    {} → {}", link.display(), target.display()));
        if self.dry_run {
            return Ok(());
        }
        self.remember(link, &host)?;
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
        self.remember(path, &host)?;
        if std::fs::symlink_metadata(&host).is_ok() {
            std::fs::remove_file(&host)?;
        }
        Ok(())
    }

    /// Move a file (keeping its mode).
    pub fn rename(&mut self, from: &Path, to: &Path) -> Result<()> {
        let host = Env::get().host(from);
        let data = std::fs::read(&host)?;
        let mode = std::fs::metadata(&host)?.permissions().mode() & 0o7777;
        self.write_bytes(to, &data, Some(mode))?;
        self.remove(from)
    }

    pub fn rollback(&mut self) {
        for (host, orig) in self.orig.drain(..).rev() {
            let _ = std::fs::remove_file(&host);
            match orig {
                Orig::Absent => {}
                Orig::File(d, mode) => {
                    let _ = write_atomic(&host, &d, mode);
                }
                Orig::Link(t) => {
                    let _ = std::os::unix::fs::symlink(t, &host);
                }
            }
        }
        // forget what this transaction recorded: nothing changed
        let mut g = GROUP.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(gr) = g.as_mut() {
            let pending = std::mem::take(&mut gr.pending);
            for e in gr.manifest.files.iter().filter(|e| pending.contains(&e.path)) {
                if let Before::File { backup, .. } = &e.before {
                    let _ = std::fs::remove_file(gr.dir.join(backup));
                }
            }
            gr.manifest.files.retain(|e| !pending.contains(&e.path));
            if gr.manifest.files.is_empty() {
                let _ = std::fs::remove_dir_all(&gr.dir);
                *g = None;
            }
        }
    }

    /// Record the change in the history and drop the oldest entries.
    fn record(&self) -> Option<String> {
        let mut g = GROUP.lock().unwrap_or_else(|e| e.into_inner());
        let gr = g.as_mut()?;
        gr.pending.clear();
        if let Err(e) = save_manifest(&gr.dir, &gr.manifest) {
            eprintln!("warning: could not save the change history: {e:#}");
        }
        prune(KEEP_BACKUPS);
        Some(gr.id.clone())
    }

    /// Commit files that aren't nginx configuration (timers…): recorded in the
    /// history, no nginx test or reload.
    pub fn commit_files(self) -> Result<()> {
        if self.dry_run {
            return Ok(());
        }
        if let Some(id) = self.record() {
            println!("backup: {id} (undo with `ngx undo {id}`)");
        }
        Ok(())
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
                    self.record();
                    println!("sandbox mode: skipped nginx -t and reload");
                    return Ok(());
                }
                // never leave a configuration in place that nginx hasn't validated
                self.rollback();
                bail!("cannot run nginx -t (nginx binary not found; set NGX_NGINX_BIN) — nothing was changed");
            }
            Some(t) if !t.ok => {
                self.rollback();
                let msgs: Vec<String> = if t.msgs.is_empty() { vec![t.raw] } else { t.msgs.iter().map(|m| m.to_string()).collect() };
                bail!("nginx -t failed, changes rolled back:\n  {}", msgs.join("\n  "));
            }
            Some(_) => {}
        }
        match ctl::reload() {
            Ok(msg) => {
                if let Some(id) = self.record() {
                    println!("backup: {id} (undo with `ngx undo {id}`)");
                }
                println!("{msg}");
                Ok(())
            }
            Err(e) => {
                // nginx accepted the config but didn't (or not in time) switch to it, e.g. a port
                // already in use. Never leave files on disk that differ from what nginx serves:
                // the next unrelated reload would apply them by surprise.
                self.rollback();
                let resync = match ctl::reload() {
                    Ok(_) => "nginx reloaded with the previous configuration".to_string(),
                    Err(e2) => format!("reloading the previous configuration failed too: {e2:#}"),
                };
                bail!("reload failed, changes rolled back ({resync}):\n  {e:#}");
            }
        }
    }
}

// ───────────────────────── history ─────────────────────────

pub struct HistoryEntry {
    pub id: String,
    pub dir: PathBuf,
    pub manifest: Manifest,
}

/// Change history, newest first. Entries made before manifests existed are
/// listed from their file names.
pub fn history() -> Vec<HistoryEntry> {
    let root = backups_root();
    let Ok(rd) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut v: Vec<HistoryEntry> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            let dir = e.path();
            let manifest = match std::fs::read_to_string(dir.join("manifest.json")) {
                Ok(s) => serde_json::from_str(&s).ok()?,
                Err(_) => legacy_manifest(&dir)?,
            };
            Some(HistoryEntry { id, dir, manifest })
        })
        .collect();
    // ids sort chronologically: YYYYmmdd-HHMMSS[-n]
    v.sort_by_key(|e| std::cmp::Reverse(sort_key(&e.id)));
    v
}

fn sort_key(id: &str) -> (String, u32) {
    match id.get(15..).and_then(|s| s.strip_prefix('-')).and_then(|n| n.parse().ok()) {
        Some(n) => (id[..15].to_string(), n),
        None => (id.to_string(), 1),
    }
}

fn legacy_manifest(dir: &Path) -> Option<Manifest> {
    let mut files = Vec::new();
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let backup = e.file_name().to_string_lossy().into_owned();
        let path = PathBuf::from(format!("/{}", backup.replace('%', "/")));
        files.push(Entry { path, before: Before::File { backup, mode: 0o644 } });
    }
    let id = dir.file_name()?.to_string_lossy().into_owned();
    let time = chrono::NaiveDateTime::parse_from_str(id.get(..15)?, "%Y%m%d-%H%M%S").ok()?.format("%d/%m/%y %H:%M:%S").to_string();
    Some(Manifest { time, command: "(recorded before ngx kept command history)".into(), user: None, files, ids: Vec::new() })
}

/// Keep the newest `keep` entries.
pub fn prune(keep: usize) {
    for e in history().into_iter().skip(keep) {
        let _ = std::fs::remove_dir_all(&e.dir);
    }
}

/// Undo a history entry: put every file it touched back as it was before.
/// Runs as a normal transaction (backed up, tested, rolled back on failure), so
/// an undo can itself be undone.
pub fn undo(entry: &HistoryEntry, dry_run: bool) -> Result<()> {
    let mut tx = Tx::new(dry_run);
    for f in &entry.manifest.files {
        match &f.before {
            Before::Absent => tx.remove(&f.path)?,
            Before::Link { target } => tx.symlink(target, &f.path)?,
            Before::File { backup, mode } => {
                let data = std::fs::read(entry.dir.join(backup)).with_context(|| format!("backup of {} is missing", f.path.display()))?;
                tx.write_bytes(&f.path, &data, Some(*mode))?;
            }
        }
    }
    tx.commit()?;
    // put the site IDs back (newest change first)
    for c in entry.manifest.ids.iter().rev() {
        match (&c.before, &c.after) {
            (Some(b), Some(a)) => crate::state::rekey(a, b),
            (Some(b), None) => crate::state::restore(b, c.id),
            (None, Some(a)) => crate::state::forget(a),
            (None, None) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn history_ids_sort_chronologically() {
        let mut ids = ["20261101-101500", "20261101-101500-10", "20261101-101500-2", "20261031-235959"];
        ids.sort_by_key(|id| super::sort_key(id));
        assert_eq!(ids, ["20261031-235959", "20261101-101500", "20261101-101500-2", "20261101-101500-10"]);
    }
}
