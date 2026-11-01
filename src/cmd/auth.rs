//! `ngx auth`: users of a password-protected site (basic auth).
//!
//! The password file is `<nginx dir>/ngx.d/<domain>/htpasswd`, with SHA-512
//! crypt hashes (`$6$`, which nginx reads through the system's crypt()), mode
//! 0640 and owned by nginx's worker group so other local users can't read it.

#[allow(unused_imports)]
use super::*;
use sha_crypt::{PasswordHasher, ShaCrypt};

#[derive(clap::Subcommand, Debug)]
pub enum AuthCmd {
    /// List the users who can log in
    List {
        /// Site ID or domain
        site: String,
    },
    /// Add a user, or change their password (asked twice, or read from stdin with --password-stdin)
    #[command(after_help = "Examples:\n  sudo ngx auth add 4 alice\n  echo \"$PASS\" | sudo ngx auth add 4 ci --password-stdin")]
    Add {
        /// Site ID or domain
        site: String,
        user: String,
        /// Read the password from the first line of stdin
        #[arg(long)]
        password_stdin: bool,
    },
    /// Remove a user
    Rm {
        /// Site ID or domain
        site: String,
        user: String,
    },
}

pub fn valid_user(u: &str) -> bool {
    !u.is_empty() && u.len() <= 64 && u.chars().all(|c| c.is_ascii_alphanumeric() || "._-@".contains(c))
}

fn random_salt() -> Result<[u8; 12]> {
    use std::io::Read;
    let mut salt = [0u8; 12];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut salt)?;
    Ok(salt)
}

pub fn hash_password(pw: &str) -> Result<String> {
    let h = ShaCrypt::default().hash_password_with_salt(pw.as_bytes(), &random_salt()?).map_err(|e| anyhow::anyhow!("hashing failed: {e}"))?;
    Ok(h.to_string())
}

fn read_users(domain: &str) -> Vec<(String, String)> {
    std::fs::read_to_string(Env::get().host(&template::htpasswd_path(domain)))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once(':').map(|(u, h)| (u.to_string(), h.to_string())))
        .collect()
}

/// Group nginx workers run as (so they can read the password file).
fn worker_gid() -> Option<u32> {
    for e in std::fs::read_dir("/proc").ok()?.flatten() {
        let cmd = std::fs::read(e.path().join("cmdline")).unwrap_or_default();
        if cmd.starts_with(b"nginx: worker") {
            let st = std::fs::read_to_string(e.path().join("status")).ok()?;
            return st.lines().find_map(|l| l.strip_prefix("Gid:"))?.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

fn write_users(domain: &str, users: &[(String, String)]) -> Result<()> {
    let env = Env::get();
    let path = template::htpasswd_path(domain);
    let body: String = users.iter().map(|(u, h)| format!("{u}:{h}\n")).collect();
    let gid = if env.sandboxed() { None } else { worker_gid() };
    let mut tx = Tx::new(false);
    tx.write_bytes(&path, body.as_bytes(), Some(if gid.is_some() { 0o640 } else { 0o644 }))?;
    if let Some(g) = gid {
        std::os::unix::fs::chown(env.host(&path), Some(0), Some(g))?;
    }
    tx.commit()
}

fn ask_password(user: &str, stdin: bool) -> Result<String> {
    let pw = if stdin {
        let mut l = String::new();
        std::io::stdin().read_line(&mut l)?;
        l.trim_end_matches(['\n', '\r']).to_string()
    } else {
        inquire::Password::new(&format!("Password for {user}:"))
            .with_display_mode(inquire::PasswordDisplayMode::Masked)
            .with_custom_confirmation_message("Again:")
            .prompt()?
    };
    if pw.len() < 8 {
        bail!("the password must have at least 8 characters");
    }
    Ok(pw)
}

/// After password protection is turned on: make sure the password file
/// exists, and offer to add the first user.
pub(crate) fn ensure_htpasswd(domain: &str, yes: bool) -> Result<()> {
    if !read_users(domain).is_empty() {
        return Ok(());
    }
    if yes {
        if !Env::get().host(&template::htpasswd_path(domain)).exists() {
            write_users(domain, &[])?;
        }
        render::print(&[vec![render::seg(format!("note: nobody can log in to {domain} yet — add a user with `sudo ngx auth add {domain} <user>`"), Tone::Warn)]]);
        return Ok(());
    }
    let user = Text::new("First user:")
        .with_validator(|u: &str| Ok(if valid_user(u.trim()) { Validation::Valid } else { Validation::Invalid("letters, digits and . _ - @".into()) }))
        .prompt()?
        .trim()
        .to_string();
    let pw = ask_password(&user, false)?;
    write_users(domain, &[(user.clone(), hash_password(&pw)?)])?;
    ok(&format!("{user} can now log in to {domain}"));
    Ok(())
}

pub fn auth(c: AuthCmd) -> Result<()> {
    let q = match &c {
        AuthCmd::List { site } | AuthCmd::Add { site, .. } | AuthCmd::Rm { site, .. } => site.clone(),
    };
    let sites = site::load_all();
    let s = site::find(&sites, &q)?;
    let src = std::fs::read_to_string(Env::get().host(&s.file)).unwrap_or_default();
    let protected = Spec::from_config(&src).is_some_and(|sp| sp.opts.auth);
    match c {
        AuthCmd::List { .. } => {
            let users = read_users(&s.domain);
            if !protected {
                println!("site #{} {} is not password-protected (turn it on with `sudo ngx edit {} --auth on`)", s.id, s.domain, s.id);
            }
            if users.is_empty() {
                println!("no users");
            }
            for (u, _) in users {
                println!("{u}");
            }
            Ok(())
        }
        AuthCmd::Add { user, password_stdin, .. } => {
            let _lock = require_root()?;
            if !valid_user(&user) {
                bail!("invalid user name {user:?} (letters, digits and . _ - @)");
            }
            let pw = ask_password(&user, password_stdin)?;
            let mut users = read_users(&s.domain);
            let hash = hash_password(&pw)?;
            let existed = users.iter().any(|(u, _)| *u == user);
            match users.iter_mut().find(|(u, _)| *u == user) {
                Some(e) => e.1 = hash,
                None => users.push((user.clone(), hash)),
            }
            write_users(&s.domain, &users)?;
            ok(&format!("{} {user} for {}", if existed { "password changed for" } else { "added" }, s.domain));
            if !protected {
                println!("note: the site isn't password-protected yet — turn it on with `sudo ngx edit {} --auth on`", s.id);
            }
            Ok(())
        }
        AuthCmd::Rm { user, .. } => {
            let _lock = require_root()?;
            let mut users = read_users(&s.domain);
            let before = users.len();
            users.retain(|(u, _)| *u != user);
            if users.len() == before {
                bail!("{user} is not a user of {}", s.domain);
            }
            write_users(&s.domain, &users)?;
            ok(&format!("removed {user} from {}", s.domain));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sha512_crypt_format() {
        let h = super::hash_password("correct horse").unwrap();
        assert!(h.starts_with("$6$"), "{h}");
        assert!(!super::valid_user("a:b") && super::valid_user("alice"));
    }
}
