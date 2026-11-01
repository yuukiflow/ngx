//! `ngx regen`: rewrite ngx-managed sites with the current template (after an
//! upgrade), as one change: every diff shown, one confirmation, one `nginx -t`
//! for all of them, one history entry.

#[allow(unused_imports)]
use super::*;

/// Generated body without the first line (it holds the generation time).
fn body(cfg: &str) -> &str {
    cfg.split_once('\n').map(|(_, b)| b).unwrap_or(cfg)
}

pub fn regen(queries: &[String], all: bool, yes: bool, dry_run: bool) -> Result<()> {
    if queries.is_empty() && !all {
        bail!("name the sites to regenerate, or use --all");
    }
    let _lock = if dry_run { None } else { Some(require_root()?) };
    let env = Env::get();
    let sites = site::load_all();
    let targets: Vec<&Site> = if all {
        sites.iter().collect()
    } else {
        queries.iter().map(|q| site::find(&sites, q)).collect::<Result<_>>()?
    };
    let mut changes: Vec<(&Site, Spec, String, String)> = Vec::new();
    let mut seen_files = Vec::new();
    for s in targets {
        if seen_files.contains(&&s.file) {
            continue;
        }
        seen_files.push(&s.file);
        let src = std::fs::read_to_string(env.host(&s.file))?;
        let Some(spec) = Spec::from_config(&src) else {
            if !all {
                println!("skip #{} {}: not created by ngx (convert it with `ngx edit {}`)", s.id, s.domain, s.id);
            }
            continue;
        };
        if let Err(e) = validate::spec(&spec) {
            println!("skip #{} {}: {e:#} — fix it with `ngx edit {}`", s.id, s.domain, s.id);
            continue;
        }
        // a Let's Encrypt site still waiting for its certificate stays on the HTTP-only config
        let waiting = spec.ssl.is_letsencrypt() && spec.cert_paths().is_some_and(|(c, _)| !env.host(Path::new(&c)).exists());
        let new = template::render(&spec, waiting);
        if body(&new) != body(&src) {
            changes.push((s, spec, src, new));
        }
    }
    if changes.is_empty() {
        ok("every ngx-managed site already matches the current template");
        return Ok(());
    }
    for (s, _, src, new) in &changes {
        render::print(&[vec![render::seg(format!("#{} {}", s.id, s.domain), Tone::Bold)]]);
        print_diff(src, new, &env.short(&s.file));
    }
    let n = changes.len();
    if !confirm(&format!("Regenerate {n} site{}?", if n == 1 { "" } else { "s" }), true, yes)? {
        return Err(Aborted.into());
    }
    let mut tx = Tx::new(dry_run);
    for (s, spec, _, new) in &changes {
        prepare_dirs(spec, dry_run)?;
        tx.write(&s.file, new)?;
    }
    tx.commit()?;
    if !dry_run {
        ok(&format!("{n} site{} regenerated", if n == 1 { "" } else { "s" }));
    }
    Ok(())
}
