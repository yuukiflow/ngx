//! `ngx cert …`: certificate info, issuance, renewal.

#[allow(unused_imports)]
use super::*;

#[derive(clap::Subcommand, Debug)]
pub enum CertCmd {
    /// Show certificate details: expiry, issuer, covered names, renewal
    Info {
        /// Site ID or domain
        site: String,
    },
    /// Obtain a Let's Encrypt certificate for a site and switch it to HTTPS
    #[command(after_help = "ngx-created sites use the ACME webroot (certbot certonly --webroot); \
        other sites use certbot's nginx plugin (certbot --nginx).\n\nExample:\n  sudo ngx cert issue 5 --email me@example.com")]
    Issue {
        /// Site ID or domain
        site: String,
        /// Email for the Let's Encrypt account (if none exists yet)
        #[arg(long)]
        email: Option<String>,
        /// Use the Let's Encrypt staging server
        #[arg(long)]
        staging: bool,
        /// Use the DNS challenge with this certbot DNS plugin (e.g. cloudflare): wildcards, no port 80 needed
        #[arg(long, value_name = "PLUGIN")]
        dns: Option<String>,
        /// Credentials file for the DNS plugin (e.g. /root/.secrets/cloudflare.ini, mode 600)
        #[arg(long, value_name = "FILE")]
        dns_credentials: Option<String>,
    },
    /// Renew a site's certificate now and reload nginx
    Renew {
        /// Site ID or domain
        site: String,
        /// Renew even if it is not close to expiry
        #[arg(long)]
        force: bool,
    },
    /// Turn automatic renewal on or off (sets `autorenew` in the certbot renewal config)
    #[command(after_help = "Example:\n  sudo ngx cert auto 2 on")]
    Auto {
        /// Site ID or domain
        site: String,
        state: OnOff,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum OnOff {
    On,
    Off,
}

pub fn cert(c: CertCmd) -> Result<()> {
    match c {
        CertCmd::Info { site: q } => {
            let rep = Report::load(false);
            let i = rep.index_of(&q)?;
            let (s, st) = (&rep.sites[i], &rep.status[i]);
            let lines = render::detail(s, st);
            // header + SSL section only
            let mut out: Vec<render::Line> = lines.iter().take(2).cloned().collect();
            let mut on = false;
            for l in &lines {
                let text: String = l.iter().map(|s| s.0.as_str()).collect();
                if text == "SSL" {
                    on = true;
                } else if on && !text.is_empty() && !text.starts_with(' ') {
                    break;
                }
                if on {
                    out.push(l.clone());
                }
            }
            if !on {
                out.push(vec![render::seg("no SSL configured", Tone::Dim)]);
            }
            if let Some(p) = s.ssl_cert() {
                out.push(vec![render::seg(format!("  {:<14}{}", "File", p.display()), Tone::Plain)]);
            }
            render::print(&out);
            Ok(())
        }
        CertCmd::Issue { site: q, email, staging, dns, dns_credentials } => {
            let _lock = require_root()?;
            let sites = site::load_all();
            let s = site::find(&sites, &q)?;
            let src = std::fs::read_to_string(Env::get().host(&s.file))?;
            let names: Vec<String> = s.all_names().map(String::from).collect();
            check_email(email.as_deref())?;
            if let Some(bad) = names.iter().find(|n| !validate::domain(n) || (n.starts_with('*') && dns.is_none())) {
                bail!("cannot request a certificate for {bad:?}: only domain names can be validated (wildcards need --dns <plugin>)");
            }
            match Spec::from_config(&src) {
                Some(mut spec) => {
                    spec.ssl = match (dns, &spec.ssl) {
                        (Some(plugin), _) => Ssl::LetsEncryptDns { plugin, credentials: dns_credentials },
                        (None, Ssl::LetsEncryptDns { .. }) => spec.ssl.clone(),
                        _ => Ssl::LetsEncrypt,
                    };
                    if spec.inbound == 80 {
                        spec.inbound = 443;
                        spec.redirect_http = true;
                    }
                    validate::spec(&spec)?;
                    ssl::issue_for(&spec.ssl, &names, email.as_deref(), staging)?;
                    let mut tx = Tx::new(false);
                    tx.write(&s.file, &template::render(&spec, false))?;
                    tx.commit()?;
                }
                None => {
                    if dns.is_some() {
                        bail!("{} is not managed by ngx: run `ngx edit {}` first, or certbot with the DNS plugin yourself", Env::get().short(&s.file), s.id);
                    }
                    println!("{} is not managed by ngx — using certbot's nginx plugin", Env::get().short(&s.file));
                    ssl::issue_nginx_plugin(&names, email.as_deref(), staging)?;
                }
            }
            ok(&format!("certificate issued for {}", names.join(", ")));
            Ok(())
        }
        CertCmd::Renew { site: q, force } => {
            let _lock = require_root()?;
            let sites = site::load_all();
            let s = site::find(&sites, &q)?;
            let name = s.ssl_cert().and_then(ssl::lineage).ok_or_else(|| anyhow::anyhow!("site #{} has no certbot-managed certificate", s.id))?;
            ssl::renew(&name, force)?;
            println!("{}", ctl::reload()?);
            ok(&format!("certificate {name} renewed"));
            Ok(())
        }
        CertCmd::Auto { site: q, state } => {
            let _lock = require_root()?;
            let sites = site::load_all();
            let s = site::find(&sites, &q)?;
            let name = s.ssl_cert().and_then(ssl::lineage).ok_or_else(|| anyhow::anyhow!("site #{} has no certbot-managed certificate", s.id))?;
            let on = matches!(state, OnOff::On);
            ssl::set_autorenew(&name, on)?;
            ok(&format!("automatic renewal {} for {name}", if on { "enabled" } else { "disabled" }));
            if on && ssl::ensure_reload_hook(&name)? {
                ok(&format!("nginx will be reloaded after each renewal ({})", ssl::reload_hook()));
            }
            if on && ssl::timer().is_none() {
                println!("warning: no certbot timer/cron found — enable it with `systemctl enable --now certbot.timer`");
            }
            Ok(())
        }
    }
}
