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

/// Load the spec of a managed site for `suspend` / `resume`.
fn managed_spec(s: &Site) -> Result<Spec> {
    let src = std::fs::read_to_string(Env::get().host(&s.file))?;
    let Some(spec) = Spec::from_config(&src) else {
        bail!("#{} {} was not created by ngx: convert it first with `ngx edit {}`", s.id, s.domain, s.id);
    };
    validate::spec(&spec).with_context(|| format!("fix it first with `ngx edit {}`", s.id))?;
    Ok(spec)
}

/// Write a managed site's config, staying on the HTTP-only config while a
/// Let's Encrypt certificate doesn't exist yet (like `ngx regen`).
fn write_spec(tx: &mut Tx, s: &Site, spec: &Spec) -> Result<()> {
    let waiting = spec.ssl.is_letsencrypt() && spec.cert_paths().is_some_and(|(c, _)| !Env::get().host(Path::new(&c)).exists());
    tx.write(&s.file, &template::render(spec, waiting))
}

/// `ngx suspend`: answer every request with 503 and a notice page, keeping the
/// site's settings for `ngx resume`. Running it again only updates the page or
/// the reason.
pub fn suspend(query: &str, page: Option<&Path>, reason: Option<&str>, dry_run: bool) -> Result<()> {
    let _lock = if dry_run { None } else { Some(require_root()?) };
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    let mut spec = managed_spec(s)?;
    if let Some(r) = reason
        && (r.chars().count() > 200 || r.chars().any(char::is_control)) {
            bail!("--reason must be one line of at most 200 characters");
        }
    let html = match page {
        Some(p) => {
            let data = std::fs::read(p).with_context(|| format!("cannot read {}", p.display()))?;
            if data.len() > 1024 * 1024 {
                bail!("{} is larger than 1 MB", p.display());
            }
            data
        }
        None => match &spec.opts.suspended {
            // already suspended: keep the page that is there
            Some(_) if Env::get().host(&template::suspended_page(&spec.domain)).exists() => std::fs::read(Env::get().host(&template::suspended_page(&spec.domain)))?,
            _ => template::SUSPENDED_HTML.as_bytes().to_vec(),
        },
    };
    let since = match &spec.opts.suspended {
        Some(old) => old.since.clone(),
        None => chrono::Local::now().format("%Y-%m-%d").to_string(),
    };
    let reason = reason.map(str::to_string).or_else(|| spec.opts.suspended.as_ref().and_then(|o| o.reason.clone()));
    let was = spec.opts.suspended.is_some();
    spec.opts.suspended = Some(template::Suspension { since, reason });
    crate::cmd::prepare_dirs(&spec, dry_run)?;
    let mut tx = Tx::new(dry_run);
    tx.write_bytes(&template::suspended_page(&spec.domain), &html, Some(0o644))?;
    write_spec(&mut tx, s, &spec)?;
    tx.commit()?;
    if !dry_run {
        ok(&format!("site #{} {} {}", s.id, s.domain, if was { "is still suspended (updated)" } else { "suspended" }));
    }
    Ok(())
}

/// `ngx resume`: lift a suspension; the site comes back exactly as it was.
pub fn resume(query: &str, dry_run: bool) -> Result<()> {
    let _lock = if dry_run { None } else { Some(require_root()?) };
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    let mut spec = managed_spec(s)?;
    if spec.opts.suspended.is_none() {
        println!("site #{} {} is not suspended", s.id, s.domain);
        return Ok(());
    }
    spec.opts.suspended = None;
    let mut tx = Tx::new(dry_run);
    write_spec(&mut tx, s, &spec)?;
    tx.remove(&template::suspended_page(&spec.domain))?;
    tx.commit()?;
    if !dry_run {
        ok(&format!("site #{} {} resumed", s.id, s.domain));
    }
    Ok(())
}
