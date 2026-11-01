//! `ngx enable`, `ngx disable`, `ngx rm`.

#[allow(unused_imports)]
use super::*;

pub(crate) fn enabled_link(s: &Site) -> PathBuf {
    Env::get().sites_enabled().join(s.file.file_name().unwrap_or_default())
}

pub fn set_enabled(query: &str, on: bool, dry_run: bool) -> Result<()> {
    let _lock = require_root()?;
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    if s.enabled == on {
        println!("site #{} {} is already {}", s.id, s.domain, if on { "enabled" } else { "disabled" });
        return Ok(());
    }
    let others: Vec<String> = sites.iter().filter(|o| o.file == s.file && o.key != s.key).map(|o| o.domain.clone()).collect();
    if !others.is_empty() {
        println!("note: {} also contains {} — they are toggled together", Env::get().short(&s.file), others.join(", "));
    }
    let mut tx = Tx::new(dry_run);
    let file = s.file.to_string_lossy().into_owned();
    if s.conf_d {
        let to = if on { PathBuf::from(file.trim_end_matches(".disabled")) } else { PathBuf::from(format!("{file}.disabled")) };
        tx.rename(&s.file, &to)?;
    } else if on {
        tx.symlink(&s.file, &enabled_link(s))?;
    } else if s.config == s.file {
        // a plain file in sites-enabled: move it to sites-available
        let to = Env::get().sites_available().join(s.file.file_name().unwrap_or_default());
        tx.rename(&s.file, &to)?;
    } else {
        tx.remove(&s.config)?;
    }
    tx.commit()?;
    ok(&format!("site #{} {} {}", s.id, s.domain, if on { "enabled" } else { "disabled" }));
    Ok(())
}

pub fn rm(query: &str, delete_cert: bool, yes: bool, dry_run: bool) -> Result<()> {
    let _lock = require_root()?;
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    let others: Vec<&Site> = sites.iter().filter(|o| o.file == s.file && o.key != s.key).collect();
    let mut q = format!("Remove site #{} {} ({})?", s.id, s.domain, Env::get().short(&s.file));
    if !others.is_empty() {
        q = format!("{q} The file also defines {} which will be removed too.", others.iter().map(|o| o.domain.as_str()).collect::<Vec<_>>().join(", "));
    }
    if !confirm(&q, false, yes)? {
        return Ok(());
    }
    let managed = Spec::from_config(&std::fs::read_to_string(Env::get().host(&s.file)).unwrap_or_default()).is_some();
    let mut tx = Tx::new(dry_run);
    if s.config != s.file {
        tx.remove(&s.config)?;
    }
    tx.remove(&s.file)?;
    // the site's own snippets and password file go too (restored by `ngx undo`)
    if managed {
        for f in site_files(&s.domain) {
            tx.remove(&f)?;
        }
    }
    tx.commit()?;
    for o in std::iter::once(s).chain(others) {
        crate::state::forget(&o.key);
    }
    ok(&format!("site #{} {} removed (backup kept under {})", s.id, s.domain, Env::get().state_dir.join("backups").display()));
    if delete_cert
        && let Some(name) = s.ssl_cert().and_then(ssl::lineage) {
            ssl::delete(&name)?;
            ok(&format!("certificate {name} deleted"));
        }
    Ok(())
}
