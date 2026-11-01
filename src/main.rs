mod cmd;
mod env;
mod logs;
mod nginx;
mod ops;
mod probe;
mod render;
mod ssl;
mod state;
mod status;
mod tui;
mod validate;

use clap::{Parser, Subcommand};
use status::{Report, State};

const SITE_HELP: &str = "Site ID (from `ngx ls`) or any of its domain names";

#[derive(Parser)]
#[command(
    name = "ngx",
    version,
    about = "Manage nginx sites and Let's Encrypt certificates",
    long_about = "Manage nginx sites and Let's Encrypt certificates.\n\n\
        Run without a command to open the full-screen dashboard. Every change is backed up, \
        validated with `nginx -t`, rolled back on failure and followed by a reload that waits \
        until nginx has picked up the new configuration.",
    after_help = "Examples:\n  ngx                      open the dashboard\n  ngx ls                   list sites\n  \
        ngx show 4               details of site #4\n  sudo ngx add             create a site (interactive)\n  \
        sudo ngx edit api.example.com --local 8081 -y\n\nDocs: docs/README.md · man ngx"
)]
pub struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// List sites with status, SSL, renewal, ports and errors
    #[command(alias = "list", after_help = "Examples:\n  ngx ls\n  ngx ls --all          include disabled sites\n  ngx ls --json | jq '.[] | select(.status == \"down\") | .domain'")]
    Ls {
        /// Include disabled sites
        #[arg(long, short)]
        all: bool,
        /// Print JSON instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Show everything about one site: SSL, routing, backend process, locations, logs, traffic
    #[command(after_help = "Examples:\n  ngx show 4\n  ngx show blog.example.com\n  ngx show 4 --json")]
    Show {
        #[arg(help = SITE_HELP)]
        site: String,
        /// Print JSON instead of the detail view
        #[arg(long)]
        json: bool,
    },
    /// Create and enable a new site (asks for anything not given as a flag)
    #[command(after_help = "Examples:\n  sudo ngx add                                    interactive\
        \n  sudo ngx add -y -d app.example.com -l 3000      proxy to :3000 + Let's Encrypt\
        \n  sudo ngx add -y -d app.example.com -l 3000 --websocket --email me@example.com\
        \n  sudo ngx add -y -d docs.example.com --root /var/www/docs --ssl none  static, HTTP only\
        \n  sudo ngx add -y -d blog.example.com -t php --root /var/www/blog\
        \n  ngx add -y -d app.example.com -l 3000 --dry-run  preview only")]
    Add(cmd::AddArgs),
    /// Change a site's settings: same questions as `add`, pre-filled with the current values
    #[command(after_help = "Without flags every question is asked again with the current value as default \
        (press Enter to keep it). With -y, only the given flags change.\n\n\
        Examples:\n  sudo ngx edit 3\
        \n  sudo ngx edit 3 -y --local 4000         move the upstream port\
        \n  sudo ngx edit 3 -y -d new.example.com   rename (ID is kept)\
        \n  sudo ngx edit 3 -y -a www.example.com   set aliases (adds them to the certificate)\
        \n  sudo ngx edit 3 -y --ssl letsencrypt    switch to HTTPS\
        \n  sudo ngx edit 3 --raw                   open the nginx file in $EDITOR instead")]
    Edit {
        #[arg(help = SITE_HELP)]
        site: String,
        /// Open the raw nginx file in $EDITOR (validated with nginx -t before it is applied)
        #[arg(long)]
        raw: bool,
        #[command(flatten)]
        args: cmd::AddArgs,
    },
    /// Remove a site's configuration (a backup is kept)
    #[command(alias = "remove", after_help = "Examples:\n  sudo ngx rm 5\n  sudo ngx rm old.example.com --delete-cert -y")]
    Rm {
        #[arg(help = SITE_HELP)]
        site: String,
        /// Also delete its certbot certificate (`certbot delete`)
        #[arg(long)]
        delete_cert: bool,
        /// Don't ask for confirmation
        #[arg(long, short)]
        yes: bool,
        /// Show what would change without touching anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Enable a disabled site
    Enable {
        #[arg(help = SITE_HELP)]
        site: String,
        /// Show what would change without touching anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Disable a site without deleting it (`ngx enable` brings it back with the same ID)
    Disable {
        #[arg(help = SITE_HELP)]
        site: String,
        /// Show what would change without touching anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Certificates: info, issue, renew, auto-renewal on/off (wraps certbot)
    #[command(subcommand)]
    Cert(cmd::CertCmd),
    /// Show a site's access log (or error log with -e)
    #[command(after_help = "Examples:\n  ngx logs 4 -f\n  ngx logs 4 -e -n 200")]
    Logs {
        #[arg(help = SITE_HELP)]
        site: String,
        /// Error log instead of access log
        #[arg(long, short)]
        error: bool,
        /// Keep following new lines (Ctrl-C to stop)
        #[arg(long, short)]
        follow: bool,
        /// Number of lines to show
        #[arg(long, short = 'n', default_value_t = 50)]
        lines: usize,
    },
    /// Validate the nginx configuration (`nginx -t`)
    Test,
    /// Validate, then reload nginx (waits until the new config is live)
    Reload,
    /// Check the environment: nginx, certbot, layout, renewal timer, permissions
    Doctor,
    /// Create sites-available/ + sites-enabled/ and include them from nginx.conf (Arch, RHEL, …)
    Init {
        /// Don't ask for confirmation
        #[arg(long, short)]
        yes: bool,
        /// Show what would change without touching anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Full-screen dashboard (also the default when no command is given)
    Tui,
    /// Generate man pages into a directory
    #[command(hide = true)]
    Man {
        #[arg(default_value = "man")]
        dir: std::path::PathBuf,
    },
}

/// Write ngx.1 plus one page per subcommand (ngx-add.1, ngx-cert-issue.1, …).
fn man(dir: &std::path::Path) -> anyhow::Result<()> {
    use clap::CommandFactory;
    std::fs::create_dir_all(dir)?;
    fn walk(cmd: &clap::Command, name: &str, dir: &std::path::Path, out: &mut Vec<String>) -> anyhow::Result<()> {
        let cmd = cmd.clone().name(name.to_string());
        let mut buf = Vec::new();
        clap_mangen::Man::new(cmd.clone()).render(&mut buf)?;
        let file = dir.join(format!("{name}.1"));
        std::fs::write(&file, buf)?;
        out.push(file.display().to_string());
        for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set() && s.get_name() != "help") {
            walk(sub, &format!("{name}-{}", sub.get_name()), dir, out)?;
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(&Cli::command(), "ngx", dir, &mut out)?;
    println!("wrote {} man pages to {}", out.len(), dir.display());
    Ok(())
}

fn ls(all: bool, json: bool) -> anyhow::Result<()> {
    let rep = Report::load(true);
    let idx: Vec<usize> = (0..rep.sites.len()).filter(|&i| all || rep.status[i].state != State::Disabled).collect();
    if json {
        let v: Vec<_> = idx.iter().map(|&i| render::json(&rep.sites[i], &rep.status[i])).collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if idx.is_empty() {
        println!("no sites found in {} — create one with `ngx add`", env::Env::get().nginx_dir.display());
        return Ok(());
    }
    let rows: Vec<_> = idx.iter().map(|&i| render::row(&rep.sites[i], &rep.status[i])).collect();
    let mut out = vec![vec![]];
    out.extend(render::table(&rows));
    out.push(vec![]);
    out.push(render::summary(&rep, &idx));
    let hidden = rep.sites.len() - idx.len();
    if hidden > 0 {
        out.push(vec![render::seg(format!("{hidden} disabled site{} hidden (ngx ls --all)", if hidden == 1 { "" } else { "s" }), render::Tone::Dim)]);
    }
    for g in &rep.global {
        out.push(vec![render::seg(format!("nginx: {g}"), render::Tone::Bad)]);
    }
    render::print(&out);
    Ok(())
}

fn show(q: &str, json: bool) -> anyhow::Result<()> {
    let rep = Report::load(true);
    let i = rep.index_of(q)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&render::json(&rep.sites[i], &rep.status[i]))?);
        return Ok(());
    }
    let mut out = vec![vec![]];
    out.extend(render::detail(&rep.sites[i], &rep.status[i]));
    render::print(&out);
    Ok(())
}

fn main() {
    // Files and directories ngx creates (configs, document roots, backups) must
    // never be group/world-writable, whatever umask root's shell has.
    unsafe { libc::umask(0o022) };
    let cli = Cli::parse();
    let res = match cli.cmd {
        None | Some(Cmd::Tui) => tui::run(),
        Some(Cmd::Ls { all, json }) => ls(all, json),
        Some(Cmd::Show { site, json }) => show(&site, json),
        Some(Cmd::Add(a)) => cmd::add(a),
        Some(Cmd::Rm { site, delete_cert, yes, dry_run }) => cmd::rm(&site, delete_cert, yes, dry_run),
        Some(Cmd::Enable { site, dry_run }) => cmd::set_enabled(&site, true, dry_run),
        Some(Cmd::Disable { site, dry_run }) => cmd::set_enabled(&site, false, dry_run),
        Some(Cmd::Edit { site, raw, args }) => cmd::edit(&site, args, raw),
        Some(Cmd::Cert(c)) => cmd::cert(c),
        Some(Cmd::Logs { site, error, follow, lines }) => cmd::logs(&site, error, follow, lines),
        Some(Cmd::Test) => cmd::test(),
        Some(Cmd::Reload) => cmd::reload(),
        Some(Cmd::Doctor) => cmd::doctor(),
        Some(Cmd::Init { yes, dry_run }) => cmd::init(yes, dry_run),
        Some(Cmd::Man { dir }) => man(&dir),
    };
    if let Err(e) = res {
        if let Some(inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted) = e.downcast_ref() {
            eprintln!("cancelled");
            std::process::exit(130);
        }
        let msg = format!("{e:#}");
        eprintln!("error: {}", msg.lines().map(|l| validate::clean(l).into_owned()).collect::<Vec<_>>().join("\n"));
        std::process::exit(1);
    }
}
