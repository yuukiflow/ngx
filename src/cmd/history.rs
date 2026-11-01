//! `ngx history` and `ngx undo`.

#[allow(unused_imports)]
use super::*;
use crate::ops::{self, Before, HistoryEntry};

fn touches_site(e: &HistoryEntry, s: &Site) -> bool {
    let stem = |p: &Path| p.file_name().map(|n| n.to_string_lossy().trim_end_matches(".disabled").to_string());
    let names: Vec<String> = s.all_names().map(|n| format!("{n}.conf")).collect();
    e.manifest.files.iter().any(|f| {
        f.path == s.file || f.path == s.config || stem(&f.path).is_some_and(|n| Some(&n) == stem(&s.file).as_ref() || names.contains(&n))
    })
}

fn summary(e: &HistoryEntry) -> String {
    let env = Env::get();
    e.manifest
        .files
        .iter()
        .map(|f| {
            let what = match f.before {
                Before::Absent => "+",
                Before::File { .. } => "~",
                Before::Link { .. } => "→",
            };
            format!("{what}{}", env.short(&f.path))
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// `ngx history [site]`: recorded changes, newest first.
pub fn history(site: Option<&str>, limit: usize, json: bool) -> Result<()> {
    let mut entries = ops::history();
    if let Some(q) = site {
        let sites = site::load_all();
        let s = site::find(&sites, q)?;
        entries.retain(|e| touches_site(e, s));
    }
    entries.truncate(limit);
    if json {
        let v: Vec<_> = entries.iter().map(|e| serde_json::json!({ "id": e.id, "time": e.manifest.time, "user": e.manifest.user, "command": e.manifest.command, "files": e.manifest.files })).collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if entries.is_empty() {
        println!("no recorded changes");
        return Ok(());
    }
    let rows: Vec<Vec<String>> = entries
        .iter()
        .map(|e| vec![e.id.clone(), e.manifest.time.clone(), e.manifest.user.clone().unwrap_or_else(|| "—".into()), e.manifest.command.clone(), summary(e)])
        .collect();
    let mut out = vec![vec![]];
    let header = ["ID", "TIME", "USER", "COMMAND", "FILES (+ created, ~ changed, → link)"];
    let w: Vec<usize> = (0..4).map(|i| rows.iter().map(|r| r[i].chars().count()).chain([header[i].len()]).max().unwrap_or(0)).collect();
    let line = |cells: &[String], tone: Tone| {
        let mut l = Vec::new();
        for (i, c) in cells.iter().enumerate() {
            let text = if i < 4 { format!("{:<width$}  ", c, width = w[i]) } else { c.clone() };
            l.push(render::seg(text, if i == 0 && tone == Tone::Plain { Tone::Accent } else { tone }));
        }
        l
    };
    out.push(line(&header.map(String::from), Tone::Bold));
    for r in &rows {
        out.push(line(r, Tone::Plain));
    }
    out.push(vec![]);
    out.push(vec![render::seg(format!("undo one with `sudo ngx undo <ID>` · the last {} changes are kept", ops::KEEP_BACKUPS), Tone::Dim)]);
    render::print(&out);
    Ok(())
}

/// `ngx undo [id]`: restore every file a change touched (default: the latest).
pub fn undo(id: Option<&str>, yes: bool, dry_run: bool) -> Result<()> {
    let _lock = if dry_run { None } else { Some(require_root()?) };
    let entries = ops::history();
    let pos = match id {
        Some(id) => entries.iter().position(|e| e.id == id).ok_or_else(|| anyhow::anyhow!("no change {id} in the history (see `ngx history`)"))?,
        None if entries.is_empty() => bail!("no recorded changes to undo"),
        None => 0,
    };
    let e = &entries[pos];
    let env = Env::get();
    render::print(&[vec![
        render::seg(format!("Undo {} ", e.id), Tone::Bold),
        render::seg(format!("({}, {}{})", e.manifest.time, e.manifest.command, e.manifest.user.as_ref().map(|u| format!(", by {u}")).unwrap_or_default()), Tone::Dim),
    ]]);
    for f in &e.manifest.files {
        let host = env.host(&f.path);
        match &f.before {
            Before::Absent => println!("  remove   {}  (created by this change)", f.path.display()),
            Before::Link { target } => println!("  relink   {} → {}", f.path.display(), target.display()),
            Before::File { backup, .. } => {
                let old = std::fs::read_to_string(e.dir.join(backup)).unwrap_or_default();
                let cur = std::fs::read_to_string(&host).unwrap_or_default();
                if old == cur {
                    println!("  keep     {}  (already as it was)", f.path.display());
                } else {
                    println!("  restore  {}", f.path.display());
                    print_diff(&cur, &old, &env.short(&f.path));
                }
            }
        }
    }
    // later changes to the same files would be overwritten
    let later: Vec<&HistoryEntry> = entries[..pos].iter().filter(|n| n.manifest.files.iter().any(|f| e.manifest.files.iter().any(|g| g.path == f.path))).collect();
    if !later.is_empty() {
        render::print(&[vec![render::seg(
            format!("warning: later changes touched the same files and will be lost: {}", later.iter().map(|l| format!("{} ({})", l.id, l.manifest.command)).collect::<Vec<_>>().join(", ")),
            Tone::Warn,
        )]]);
    }
    if !confirm("Undo this change?", later.is_empty(), yes)? {
        return Err(Aborted.into());
    }
    ops::undo(e, dry_run)?;
    if !dry_run {
        ok(&format!("change {} undone (this undo can be undone too: `ngx history`)", e.id));
    }
    Ok(())
}
