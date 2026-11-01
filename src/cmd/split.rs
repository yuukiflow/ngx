//! `ngx split`: a file that defines several sites becomes one file per site,
//! so each can be edited, disabled or removed on its own. Server blocks are
//! moved verbatim; everything else in the file (upstreams, maps, zones) stays
//! where it was. IDs follow their site.

#[allow(unused_imports)]
use super::*;

pub fn split(query: &str, yes: bool, dry_run: bool) -> Result<()> {
    let _lock = if dry_run { None } else { Some(require_root()?) };
    let env = Env::get();
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    let file = s.file.clone();
    let group: Vec<&Site> = sites.iter().filter(|o| o.file == file).collect();
    if group.len() < 2 {
        println!("{} only defines {} — nothing to split", env.short(&file), s.domain);
        return Ok(());
    }
    let src = std::fs::read_to_string(env.host(&file))?;
    let lines: Vec<&str> = src.lines().collect();
    let dir = file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let disabled_suffix = if s.conf_d && !s.enabled { ".disabled" } else { "" };
    let orig_name = file.file_name().unwrap_or_default().to_string_lossy().trim_end_matches(".disabled").to_string();

    // which lines go where
    let mut moved = vec![false; lines.len()];
    let mut plan: Vec<(&Site, PathBuf, String)> = Vec::new();
    let mut stays_in_orig: Option<&Site> = None;
    for (n, o) in group.iter().enumerate() {
        let name = if validate::domain(&o.domain) { format!("{}.conf", o.domain) } else { format!("{}-{}.conf", orig_name.trim_end_matches(".conf"), n + 1) };
        if name == orig_name {
            stays_in_orig = Some(o); // already named after it: it keeps the file
            continue;
        }
        let target = dir.join(format!("{name}{disabled_suffix}"));
        if env.host(&target).exists() || plan.iter().any(|(_, t, _)| *t == target) {
            bail!("{} already exists; rename it first", env.short(&target));
        }
        let mut text = format!("# split from {} by `ngx split`\n", file.display());
        for b in &o.blocks {
            let (from, to) = (b.line.saturating_sub(1), b.end_line.min(lines.len()));
            // keep the comment lines right above the block with it
            let mut start = from;
            while start > 0 && lines[start - 1].trim_start().starts_with('#') && !moved[start - 1] {
                start -= 1;
            }
            text.push('\n');
            for (i, l) in lines.iter().enumerate().take(to).skip(start) {
                text.push_str(l);
                text.push('\n');
                moved[i] = true;
            }
        }
        plan.push((o, target, text));
    }
    let rest: Vec<&str> = lines.iter().enumerate().filter(|(i, _)| !moved[*i]).map(|(_, l)| *l).collect();
    let rest_has_content = rest.iter().any(|l| {
        let t = l.trim();
        !t.is_empty() && !t.starts_with('#')
    });
    let mut rest_text = rest.join("\n");
    while rest_text.contains("\n\n\n") {
        rest_text = rest_text.replace("\n\n\n", "\n\n");
    }
    let keep_orig = stays_in_orig.is_some() || rest_has_content;

    // show the plan
    println!("{} defines {} sites:", env.short(&file), group.len());
    for (o, target, _) in &plan {
        println!("  #{} {} → {}", o.id, o.domain, env.short(target));
    }
    match (stays_in_orig, rest_has_content) {
        (Some(o), _) => println!("  #{} {} stays in {}{}", o.id, o.domain, env.short(&file), if rest_has_content { " (with the shared directives)" } else { "" }),
        (None, true) => println!("  the other directives (upstreams, maps…) stay in {}", env.short(&file)),
        (None, false) => println!("  {} is removed (nothing else in it)", env.short(&file)),
    }
    if !confirm("Split it?", true, yes)? {
        return Err(Aborted.into());
    }

    let linked = s.config != s.file;
    let mut tx = Tx::new(dry_run);
    for (_, target, text) in &plan {
        tx.write(target, text)?;
        if linked {
            tx.symlink(target, &env.sites_enabled().join(target.file_name().unwrap_or_default()))?;
        }
    }
    if keep_orig {
        tx.write(&file, &(rest_text.trim_end().to_string() + "\n"))?;
    } else {
        if linked {
            tx.remove(&s.config)?;
        }
        tx.remove(&file)?;
    }
    tx.commit()?;
    if dry_run {
        return Ok(());
    }
    for (o, target, _) in &plan {
        let base = target.file_name().map(|n| n.to_string_lossy().trim_end_matches(".disabled").to_string()).unwrap_or_default();
        crate::state::rekey(&o.key, &format!("{base}#{}", o.domain));
    }
    ok(&format!("split into {} files — each site can now be edited on its own", plan.len() + usize::from(stays_in_orig.is_some())));
    Ok(())
}
