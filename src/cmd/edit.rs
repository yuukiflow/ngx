//! `ngx edit`: prompt-based, and the `--raw` escape hatch.

#[allow(unused_imports)]
use super::*;

/// Best-effort reading of a hand-written site into a spec, so it can be edited
/// with prompts like a generated one.
pub(crate) fn spec_from_site(s: &Site) -> Result<Spec> {
    let raw_hint = || format!("use `ngx edit {} --raw` to edit the file directly", s.id);
    let backend = match (s.kind(), s.upstream()) {
        (Kind::Proxy, Some(Upstream::Tcp { host, port })) => Backend::Proxy {
            host: host.clone(),
            port: *port,
            websocket: s.main().is_some_and(|b| b.locations.iter().any(|l| l.action.contains("WebSocket"))),
        },
        (Kind::FastCgi, Some(Upstream::Unix(sock))) => Backend::Php {
            socket: sock.display().to_string(),
            root: s.root().map(|r| r.display().to_string()).unwrap_or_else(|| format!("/var/www/{}", s.domain)),
        },
        (Kind::Static, _) if s.root().is_some() => Backend::Static { root: s.root().unwrap().display().to_string() },
        (k, u) => bail!(
            "site #{} is a {} ({}) — not a setup ngx can generate; {}",
            s.id,
            k.label(),
            u.map(|u| u.to_string()).unwrap_or_else(|| "no upstream".into()),
            raw_hint()
        ),
    };
    let ssl = match s.ssl_cert() {
        None => Ssl::None,
        Some(c) if ssl::lineage(c).as_deref() == Some(template::cert_name(&s.domain).as_str()) => Ssl::LetsEncrypt,
        Some(c) => Ssl::Existing {
            cert: c.display().to_string(),
            key: s.ssl_key().map(|k| k.display().to_string()).unwrap_or_default(),
        },
    };
    Ok(Spec {
        domain: s.domain.clone(),
        aliases: s.aliases.clone(),
        inbound: s.public_port().unwrap_or(80),
        backend,
        ssl,
        redirect_http: s.http_redirect().is_some(),
        access_log: s.main().and_then(|b| b.access_log.as_ref()).map(|p| p.display().to_string()),
        error_log: s.main().and_then(|b| b.error_log.as_ref()).map(|p| p.display().to_string()),
        opts: template::Options::default(),
    })
}

/// Colored unified diff between the current and the new config.
pub(crate) fn print_diff(old: &str, new: &str, label: &str) {
    // private, randomly named directory (0700): nothing in /tmp can be pre-planted
    let out = tempfile::Builder::new().prefix("ngx-diff-").tempdir().ok().and_then(|dir| {
        let (a, b) = (dir.path().join("current"), dir.path().join("new"));
        std::fs::write(&a, old).and_then(|_| std::fs::write(&b, new)).ok()?;
        std::process::Command::new("diff").args(["-u", "--label", &format!("{label} (current)"), "--label", &format!("{label} (new)")]).arg(&a).arg(&b).output().ok()
    });
    let lines: Vec<render::Line> = match out {
        Some(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| {
                let tone = if l.starts_with("+++") || l.starts_with("---") {
                    Tone::Bold
                } else if l.starts_with('+') {
                    Tone::Good
                } else if l.starts_with('-') {
                    Tone::Bad
                } else if l.starts_with("@@") {
                    Tone::Accent
                } else {
                    Tone::Dim
                };
                vec![render::seg(l, tone)]
            })
            .collect(),
        None => new.lines().map(|l| vec![render::seg(l, Tone::Plain)]).collect(),
    };
    println!();
    render::print(&lines);
    println!();
}

/// Write a regenerated config for an existing site, renaming the file (and
/// its sites-enabled link) when the domain changed.
pub(crate) fn write_site(s: &Site, new_file: &Path, content: &str, dry_run: bool, rename: Option<(&str, &str)>) -> Result<()> {
    let env = Env::get();
    let dir = new_file.parent().unwrap_or(Path::new("/"));
    let mut tx = Tx::new(dry_run);
    // the domain changed: move the snippets and password file along
    if let Some((old, new)) = rename.filter(|(o, n)| o != n) {
        for f in site_files(old) {
            let to = template::site_dir(new).join(f.file_name().unwrap_or_default());
            if !env.host(&to).exists() {
                tx.rename(&f, &to)?;
            }
        }
    }
    ensure_log_format(&mut tx, dir, dir == env.sites_available())?;
    tx.write(new_file, content)?;
    if new_file != s.file {
        tx.remove(&s.file)?;
        if s.config != s.file {
            tx.remove(&s.config)?;
            tx.symlink(new_file, &env.sites_enabled().join(new_file.file_name().unwrap_or_default()))?;
        }
    }
    tx.commit()
}

/// The settings of a site as ngx would edit them (for the dashboard form).
pub fn current_spec(query: &str) -> Result<Spec> {
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    let src = std::fs::read_to_string(Env::get().host(&s.file))?;
    match Spec::from_config(&src) {
        Some(sp) => Ok(sp),
        None => spec_from_site(s),
    }
}

/// `ngx edit`: change a site's settings through the same questions as
/// `ngx add`, pre-filled with the current values. ngx rewrites the config.
pub fn edit(query: &str, a: AddArgs, raw: bool) -> Result<()> {
    if raw {
        return edit_raw(query);
    }
    let _lock = if a.dry_run { None } else { Some(require_root()?) };
    let env = Env::get();
    let sites = site::load_all();
    let s = site::find(&sites, query)?.clone();
    let others: Vec<&str> = sites.iter().filter(|o| o.file == s.file && o.key != s.key).map(|o| o.domain.as_str()).collect();
    if !others.is_empty() {
        bail!("{} also defines {} — split it up or use `ngx edit {} --raw`", env.short(&s.file), others.join(", "), s.id);
    }
    let src = std::fs::read_to_string(env.host(&s.file))?;
    let (cur, managed) = match Spec::from_config(&src) {
        Some(sp) => (sp, true),
        None => (spec_from_site(&s)?, false),
    };
    render::print(&[vec![render::seg(format!("Editing site #{} {}", s.id, s.domain), Tone::Bold), render::seg("  (Enter keeps the current value)", Tone::Dim)]]);
    if !managed {
        render::print(&[vec![render::seg(
            format!("note: {} was not created by ngx — saving replaces it with a generated config (the original is backed up).", env.short(&s.file)),
            Tone::Warn,
        )]]);
    }
    let existing: Vec<Site> = sites.iter().filter(|o| o.file != s.file).cloned().collect();
    let (spec, email) = build_spec(&a, &existing, Some(&cur))?;
    if managed && spec == cur {
        println!("no changes");
        return Ok(());
    }

    // same directory, file named after the (possibly new) domain
    let dir = s.file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let suffix = if s.conf_d && !s.enabled { ".disabled" } else { "" };
    let new_file = dir.join(format!("{}{suffix}", spec.file_name()));
    if new_file != s.file && env.host(&new_file).exists() {
        bail!("{} already exists", new_file.display());
    }
    let final_cfg = template::render(&spec, false);
    print_diff(&src, &final_cfg, &env.short(&new_file));
    if !confirm("Apply these changes?", true, a.yes && !a.review)? {
        println!("aborted — nothing changed");
        return Ok(());
    }
    prepare_dirs(&spec, a.dry_run)?;

    let names: Vec<String> = std::iter::once(spec.domain.clone()).chain(spec.aliases.clone()).collect();
    let mut need_issue = false;
    if spec.ssl.is_letsencrypt() {
        let (cert, _) = spec.cert_paths().unwrap();
        let dns = matches!(spec.ssl, Ssl::LetsEncryptDns { .. });
        match ssl::read_cert(Path::new(&cert)) {
            ssl::CertState::Missing(_) if dns => {
                if a.dry_run {
                    println!("dry run — would run certbot (DNS challenge) for {}", names.join(", "));
                } else if let Err(e) = ssl::issue_for(&spec.ssl, &names, email.as_deref(), a.staging) {
                    bail!("certificate issuance failed, nothing was changed: {e:#}");
                }
            }
            ssl::CertState::Missing(_) => {
                // no certificate yet: serve the ACME challenge over HTTP first
                write_site(&s, &new_file, &template::render(&spec, true), a.dry_run, Some((&cur.domain, &spec.domain)))?;
                if a.dry_run {
                    println!("dry run — would run certbot for {}", names.join(", "));
                } else if let Err(e) = ssl::issue_for(&spec.ssl, &names, email.as_deref(), a.staging) {
                    eprintln!("certificate issuance failed: {e:#}");
                    eprintln!("the site is on HTTP for now; fix DNS / port 80 and run `ngx cert issue {}`", s.id);
                    return Ok(());
                }
            }
            ssl::CertState::Ok(info) => need_issue = names.iter().any(|n| !render::covers(&info.sans, n)),
            _ => {}
        }
    }
    let s_now = if new_file != s.file && env.host(&new_file).exists() {
        // bootstrap already moved it
        Site { file: new_file.clone(), config: if s.config != s.file { env.sites_enabled().join(new_file.file_name().unwrap_or_default()) } else { new_file.clone() }, ..s.clone() }
    } else {
        s.clone()
    };
    write_site(&s_now, &new_file, &final_cfg, a.dry_run, Some((&cur.domain, &spec.domain)))?;
    if spec.opts.auth && !a.dry_run {
        ensure_htpasswd(&spec.domain, a.yes && !a.review)?;
    }
    if need_issue && !a.dry_run {
        // the config already serves the new names over HTTP (ACME location), so certbot can expand the cert
        ssl::issue_for(&spec.ssl, &names, email.as_deref(), a.staging)?;
        println!("{}", ctl::reload()?);
    }
    if a.dry_run {
        return Ok(());
    }
    let base = new_file.file_name().map(|n| n.to_string_lossy().trim_end_matches(".disabled").to_string()).unwrap_or_default();
    let new_key = format!("{base}#{}", spec.domain);
    if new_key != s.key {
        crate::state::rekey(&s.key, &new_key);
    }
    ok(&format!("site #{} {} updated", s.id, spec.domain));
    Ok(())
}

/// Escape hatch: edit the raw nginx file in $EDITOR (validated before applying).
pub(crate) fn edit_raw(query: &str) -> Result<()> {
    let _lock = require_root()?;
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    edit_file(&s.file, true)
}

/// `ngx edit <site> --snippet <name>`: edit `ngx.d/<domain>/<name>.conf`, the
/// site's own directives, which survive regeneration.
pub fn edit_snippet(query: &str, name: &str) -> Result<()> {
    let _lock = require_root()?;
    let sites = site::load_all();
    let s = site::find(&sites, query)?;
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') || name.is_empty() || name.len() > 64 {
        bail!("invalid snippet name {name:?} (letters, digits, - and _)");
    }
    let src = std::fs::read_to_string(Env::get().host(&s.file)).unwrap_or_default();
    if Spec::from_config(&src).is_none() {
        bail!("snippets work with sites generated by ngx; convert #{} first with `ngx edit {}`", s.id, s.id);
    }
    let path = template::site_dir(&s.domain).join(format!("{name}.conf"));
    if !src.contains(&template::site_dir(&s.domain).display().to_string()) {
        // generated by an older ngx: regenerate so the include is there
        println!("note: run `sudo ngx edit {} -y` once so the config includes {}/", s.id, template::site_dir(&s.domain).display());
    }
    edit_file(&path, false)
}

/// Edit a file in $EDITOR on a private copy; apply it only through a validated
/// transaction, and offer to edit again if nginx rejects it.
fn edit_file(path: &Path, site_config: bool) -> Result<()> {
    let env = Env::get();
    let existed = env.host(path).exists();
    let orig = if existed {
        std::fs::read_to_string(env.host(path))?
    } else {
        "# nginx directives for this site's server block (checked with nginx -t before they are applied)\n".to_string()
    };
    // edit a copy in a private, randomly named directory (0700, removed on return)
    let dir = tempfile::Builder::new().prefix("ngx-edit-").tempdir()?;
    let tmp = dir.path().join(path.file_name().unwrap_or_default());
    std::fs::write(&tmp, &orig)?;
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| {
        if env::which("nano").is_some() { "nano".into() } else { "vi".into() }
    });
    let result = loop {
        let st = std::process::Command::new("sh").arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(&tmp).status()?;
        if !st.success() {
            break Err(anyhow::anyhow!("editor exited with {st}"));
        }
        let mut new = std::fs::read_to_string(&tmp)?;
        if new == orig {
            println!("no changes");
            break Ok(());
        }
        // hand-edited files are no longer regenerated by ngx
        if site_config && new.lines().next().is_some_and(|l| l.starts_with(MANAGED_MARKER)) {
            new = new
                .lines()
                .filter(|l| !l.starts_with("# ngx-spec: "))
                .map(|l| if l.starts_with(MANAGED_MARKER) { "# originally generated by ngx (hand-edited since)" } else { l })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
        }
        let err = match parser::parse(&new) {
            Err(e) => Some(format!("syntax error: {e}")),
            Ok(_) => {
                let mut tx = Tx::new(false);
                tx.write(path, &new)?;
                tx.commit().err().map(|e| format!("{e:#}"))
            }
        };
        match err {
            None => {
                ok(&format!("{} updated", Env::get().short(path)));
                break Ok(());
            }
            Some(e) => {
                eprintln!("{e}");
                if !Confirm::new("Edit again?").with_default(true).prompt()? {
                    break Err(anyhow::anyhow!("changes discarded"));
                }
            }
        }
    };
    drop(dir);
    result
}
