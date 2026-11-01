//! `ngx check`: one pass over every enabled site for cron jobs and monitoring
//! (Nagios-style exit codes). Let's Encrypt no longer emails about expiring
//! certificates, so this is what tells you.

#[allow(unused_imports)]
use super::*;
use crate::ssl::{CertState, Renew};
use crate::status::State;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warning,
    Critical,
}

#[derive(serde::Serialize)]
pub struct Problem {
    pub level: Level,
    pub site: Option<u32>,
    pub domain: Option<String>,
    pub message: String,
}

pub fn problems(rep: &Report) -> Vec<Problem> {
    let mut out = Vec::new();
    for g in &rep.global {
        out.push(Problem { level: Level::Critical, site: None, domain: None, message: format!("nginx: {g}") });
    }
    for (s, st) in rep.sites.iter().zip(&rep.status) {
        if st.state == State::Disabled {
            continue;
        }
        let mut p = |level, message: String| out.push(Problem { level, site: Some(s.id), domain: Some(s.domain.clone()), message });
        if st.state == State::Down {
            p(Level::Critical, format!("down: {}", st.error.clone().unwrap_or_else(|| "unknown error".into())));
        }
        match &st.cert {
            CertState::Ok(i) => {
                let days = st.cert.days_left().unwrap_or(0);
                if st.cert.expired() {
                    p(Level::Critical, format!("certificate expired {}", render::date(i.not_after)));
                } else if st.cert.expiring_soon() {
                    let why = match &st.renew {
                        Renew::Auto { .. } => "automatic renewal is failing — run `sudo certbot renew --dry-run`".to_string(),
                        Renew::Manual(r) => format!("renewal is manual ({r})"),
                        Renew::None => String::new(),
                    };
                    p(Level::Warning, format!("certificate expires in {days} day{} ({}) · {why}", if days == 1 { "" } else { "s" }, render::date(i.not_after)));
                }
                let missing: Vec<&str> = s.all_names().filter(|n| !render::covers(&i.sans, n)).collect();
                if !missing.is_empty() {
                    p(Level::Warning, format!("certificate doesn't cover {}", missing.join(", ")));
                }
            }
            CertState::Missing(path) => p(Level::Critical, format!("certificate file missing: {}", path.display())),
            CertState::Unreadable(e) => p(Level::Warning, format!("certificate unreadable: {e}")),
            CertState::None => {}
        }
        if let Renew::Auto { reloads: false, .. } = st.renew {
            p(Level::Warning, format!("nginx isn't reloaded after renewal, renewed certificates won't be served (fix: sudo ngx cert auto {} on)", s.id));
        }
    }
    out
}

/// Exit code: 0 everything fine, 1 warnings, 2 something is down or expired.
pub fn check(json: bool, quiet: bool) -> Result<i32> {
    let rep = Report::load(true);
    let probs = problems(&rep);
    let worst = probs.iter().map(|p| p.level).fold(Level::Ok, |a, b| if b > a { b } else { a });
    let code = match worst {
        Level::Ok => 0,
        Level::Warning => 1,
        Level::Critical => 2,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "status": worst, "sites": rep.sites.len(), "problems": probs }))?);
    } else if !quiet {
        let enabled = rep.status.iter().filter(|s| s.state != State::Disabled).count();
        let mut out = Vec::new();
        for p in &probs {
            let (tag, tone) = match p.level {
                Level::Critical => ("CRIT", Tone::Bad),
                Level::Warning => ("WARN", Tone::Warn),
                Level::Ok => ("OK", Tone::Good),
            };
            let who = match (p.site, &p.domain) {
                (Some(id), Some(d)) => format!("#{id} {d}: "),
                _ => String::new(),
            };
            out.push(vec![render::seg(format!("{tag:<5}"), tone), render::seg(who, Tone::Bold), render::seg(p.message.clone(), Tone::Plain)]);
        }
        if probs.is_empty() {
            out.push(vec![render::seg("OK   ", Tone::Good), render::seg(format!("{enabled} site{} up, certificates valid and renewing", if enabled == 1 { "" } else { "s" }), Tone::Plain)]);
        }
        render::print(&out);
    }
    Ok(code)
}
