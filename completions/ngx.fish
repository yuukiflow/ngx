# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_ngx_global_optspecs
    string join \n h/help V/version
end

function __fish_ngx_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_ngx_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_ngx_using_subcommand
    set -l cmd (__fish_ngx_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c ngx -n "__fish_ngx_needs_command" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ngx -n "__fish_ngx_needs_command" -s V -l version -d 'Print version'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "ls" -d 'List sites with status, SSL, renewal, ports and errors'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "show" -d 'Show everything about one site: SSL, routing, backend process, locations, logs, traffic'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "add" -d 'Create and enable a new site (asks for anything not given as a flag)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "edit" -d 'Change a site\'s settings: same questions as `add`, pre-filled with the current values'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "rm" -d 'Remove a site\'s configuration (a backup is kept)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "enable" -d 'Enable a disabled site'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "disable" -d 'Disable a site without deleting it (`ngx enable` brings it back with the same ID)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "suspend" -d 'Suspend a site: every request gets a 503 and a notice page (`ngx resume` brings it back as it was)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "resume" -d 'Bring a suspended site back'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "cert" -d 'Certificates: info, issue, renew, auto-renewal on/off (wraps certbot)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "logs" -d 'Show a site\'s access log (or error log with -e)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "test" -d 'Validate the nginx configuration (`nginx -t`)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "reload" -d 'Validate, then reload nginx (waits until the new config is live)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "doctor" -d 'Check the environment: nginx, certbot, layout, renewal timer, permissions'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "regen" -d 'Rewrite ngx-managed sites with the current template (after an upgrade): diffs, one confirmation, one nginx -t'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "split" -d 'Split a file that defines several sites into one file per site (so each can be edited on its own)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "auth" -d 'Users of password-protected sites (basic auth)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "check" -d 'Check every site for monitoring/cron: down sites, expired or expiring certificates, renewal problems'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "history" -d 'List recorded changes (newest first), optionally only those touching one site'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "undo" -d 'Undo a change: restore every file it touched (default: the latest change)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "init" -d 'Create sites-available/ + sites-enabled/ and include them from nginx.conf (Arch, RHEL, …)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "tui" -d 'Full-screen dashboard (also the default when no command is given)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "completions" -d 'Print a shell completion script (bash, zsh, fish, elvish, powershell)'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "man" -d 'Generate man pages into a directory'
complete -c ngx -n "__fish_ngx_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ngx -n "__fish_ngx_using_subcommand ls" -s a -l all -d 'Include disabled sites'
complete -c ngx -n "__fish_ngx_using_subcommand ls" -l json -d 'Print JSON instead of a table'
complete -c ngx -n "__fish_ngx_using_subcommand ls" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand show" -l json -d 'Print JSON instead of the detail view'
complete -c ngx -n "__fish_ngx_using_subcommand show" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand add" -s d -l domain -d 'Primary domain, e.g. app.example.com (with edit: the new domain)' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -s a -l alias -d 'Extra server name, repeatable: -a www.example.com -a example.org (with edit: replaces the list)' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -s t -l type -d 'Site type: proxy | static | php' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -s l -l local -d 'Upstream for a proxy site: PORT (on 127.0.0.1) or HOST:PORT' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l root -d 'Document root for static / php sites (default /var/www/<domain>, created if missing)' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l socket -d 'php-fpm socket for php sites (default: the one found in /run/php)' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -s i -l in -d 'Public port (default 443 with SSL, 80 without)' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l ssl -d 'SSL: letsencrypt | existing | none' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l cert -d 'Certificate (fullchain) path, with --ssl existing' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l key -d 'Private key path, with --ssl existing' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l email -d 'Email for the Let\'s Encrypt account (asked once if none exists). Let\'s Encrypt no longer sends expiry emails: use `ngx check` to monitor' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l dns -d 'Let\'s Encrypt with the DNS challenge through this certbot plugin (cloudflare, route53, digitalocean…): wildcards, no port 80 needed' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l dns-credentials -d 'Credentials file for the DNS plugin (e.g. /root/.secrets/cloudflare.ini, mode 600)' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l max-body -d 'Maximum upload (request body) size, e.g. 50m; `off` for nginx\'s default (1m)' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l hsts -d 'HSTS: tell browsers to always use HTTPS for this domain (on | off)' -r -f -a "on\t''
off\t''"
complete -c ngx -n "__fish_ngx_using_subcommand add" -l auth -d 'Password-protect the site (users: `ngx auth add <site> <user>`) (on | off)' -r -f -a "on\t''
off\t''"
complete -c ngx -n "__fish_ngx_using_subcommand add" -l rate-limit -d 'Limit requests per client, e.g. 10r/s or 300r/m; `off` to remove' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l canonical -d 'Redirect the aliases (e.g. www.) to the main domain (on | off)' -r -f -a "on\t''
off\t''"
complete -c ngx -n "__fish_ngx_using_subcommand add" -l headers -d 'Security headers: X-Content-Type-Options, X-Frame-Options, Referrer-Policy (on | off)' -r -f -a "on\t''
off\t''"
complete -c ngx -n "__fish_ngx_using_subcommand add" -l health -d 'Path the backend answers health checks on, e.g. /health (`ngx check` and the status use it); `off` to remove' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l route -d 'Proxy a path to another local app, repeatable: --route /api=8080 --route /ws=10.0.0.5:9000 (with edit: replaces the list; `--route off` removes all)' -r
complete -c ngx -n "__fish_ngx_using_subcommand add" -l websocket -d 'Proxy WebSocket upgrades'
complete -c ngx -n "__fish_ngx_using_subcommand add" -l no-redirect -d 'Don\'t redirect HTTP (port 80) to HTTPS'
complete -c ngx -n "__fish_ngx_using_subcommand add" -l staging -d 'Use the Let\'s Encrypt staging server (for testing; certificates are not trusted)'
complete -c ngx -n "__fish_ngx_using_subcommand add" -s y -l yes -d 'No prompts: use flags, and current values / defaults for the rest'
complete -c ngx -n "__fish_ngx_using_subcommand add" -l dry-run -d 'Show the config and what would change without touching anything'
complete -c ngx -n "__fish_ngx_using_subcommand add" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l snippet -d 'Edit your own directives for the site in $EDITOR: ngx.d/<domain>/<NAME>.conf, kept when ngx regenerates the config' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -s d -l domain -d 'Primary domain, e.g. app.example.com (with edit: the new domain)' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -s a -l alias -d 'Extra server name, repeatable: -a www.example.com -a example.org (with edit: replaces the list)' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -s t -l type -d 'Site type: proxy | static | php' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -s l -l local -d 'Upstream for a proxy site: PORT (on 127.0.0.1) or HOST:PORT' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l root -d 'Document root for static / php sites (default /var/www/<domain>, created if missing)' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l socket -d 'php-fpm socket for php sites (default: the one found in /run/php)' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -s i -l in -d 'Public port (default 443 with SSL, 80 without)' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l ssl -d 'SSL: letsencrypt | existing | none' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l cert -d 'Certificate (fullchain) path, with --ssl existing' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l key -d 'Private key path, with --ssl existing' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l email -d 'Email for the Let\'s Encrypt account (asked once if none exists). Let\'s Encrypt no longer sends expiry emails: use `ngx check` to monitor' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l dns -d 'Let\'s Encrypt with the DNS challenge through this certbot plugin (cloudflare, route53, digitalocean…): wildcards, no port 80 needed' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l dns-credentials -d 'Credentials file for the DNS plugin (e.g. /root/.secrets/cloudflare.ini, mode 600)' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l max-body -d 'Maximum upload (request body) size, e.g. 50m; `off` for nginx\'s default (1m)' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l hsts -d 'HSTS: tell browsers to always use HTTPS for this domain (on | off)' -r -f -a "on\t''
off\t''"
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l auth -d 'Password-protect the site (users: `ngx auth add <site> <user>`) (on | off)' -r -f -a "on\t''
off\t''"
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l rate-limit -d 'Limit requests per client, e.g. 10r/s or 300r/m; `off` to remove' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l canonical -d 'Redirect the aliases (e.g. www.) to the main domain (on | off)' -r -f -a "on\t''
off\t''"
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l headers -d 'Security headers: X-Content-Type-Options, X-Frame-Options, Referrer-Policy (on | off)' -r -f -a "on\t''
off\t''"
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l health -d 'Path the backend answers health checks on, e.g. /health (`ngx check` and the status use it); `off` to remove' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l route -d 'Proxy a path to another local app, repeatable: --route /api=8080 --route /ws=10.0.0.5:9000 (with edit: replaces the list; `--route off` removes all)' -r
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l raw -d 'Open the raw nginx file in $EDITOR (validated with nginx -t before it is applied)'
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l websocket -d 'Proxy WebSocket upgrades'
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l no-redirect -d 'Don\'t redirect HTTP (port 80) to HTTPS'
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l staging -d 'Use the Let\'s Encrypt staging server (for testing; certificates are not trusted)'
complete -c ngx -n "__fish_ngx_using_subcommand edit" -s y -l yes -d 'No prompts: use flags, and current values / defaults for the rest'
complete -c ngx -n "__fish_ngx_using_subcommand edit" -l dry-run -d 'Show the config and what would change without touching anything'
complete -c ngx -n "__fish_ngx_using_subcommand edit" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand rm" -l delete-cert -d 'Also delete its certbot certificate (`certbot delete`)'
complete -c ngx -n "__fish_ngx_using_subcommand rm" -s y -l yes -d 'Don\'t ask for confirmation'
complete -c ngx -n "__fish_ngx_using_subcommand rm" -l dry-run -d 'Show what would change without touching anything'
complete -c ngx -n "__fish_ngx_using_subcommand rm" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand enable" -l dry-run -d 'Show what would change without touching anything'
complete -c ngx -n "__fish_ngx_using_subcommand enable" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand disable" -l dry-run -d 'Show what would change without touching anything'
complete -c ngx -n "__fish_ngx_using_subcommand disable" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand suspend" -l page -d 'HTML page to show visitors (default: a short \\"temporarily unavailable\\" page)' -r -F
complete -c ngx -n "__fish_ngx_using_subcommand suspend" -l reason -d 'Note for yourself, shown by `ngx show` (never served to visitors)' -r
complete -c ngx -n "__fish_ngx_using_subcommand suspend" -l dry-run -d 'Show what would change without touching anything'
complete -c ngx -n "__fish_ngx_using_subcommand suspend" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand resume" -l dry-run -d 'Show what would change without touching anything'
complete -c ngx -n "__fish_ngx_using_subcommand resume" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and not __fish_seen_subcommand_from info issue renew auto help" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and not __fish_seen_subcommand_from info issue renew auto help" -f -a "info" -d 'Show certificate details: expiry, issuer, covered names, renewal'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and not __fish_seen_subcommand_from info issue renew auto help" -f -a "issue" -d 'Obtain a Let\'s Encrypt certificate for a site and switch it to HTTPS'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and not __fish_seen_subcommand_from info issue renew auto help" -f -a "renew" -d 'Renew a site\'s certificate now and reload nginx'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and not __fish_seen_subcommand_from info issue renew auto help" -f -a "auto" -d 'Turn automatic renewal on or off (sets `autorenew` in the certbot renewal config)'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and not __fish_seen_subcommand_from info issue renew auto help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from info" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from issue" -l email -d 'Email for the Let\'s Encrypt account (if none exists yet)' -r
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from issue" -l dns -d 'Use the DNS challenge with this certbot DNS plugin (e.g. cloudflare): wildcards, no port 80 needed' -r
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from issue" -l dns-credentials -d 'Credentials file for the DNS plugin (e.g. /root/.secrets/cloudflare.ini, mode 600)' -r
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from issue" -l staging -d 'Use the Let\'s Encrypt staging server'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from issue" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from renew" -l force -d 'Renew even if it is not close to expiry'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from renew" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from auto" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from help" -f -a "info" -d 'Show certificate details: expiry, issuer, covered names, renewal'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from help" -f -a "issue" -d 'Obtain a Let\'s Encrypt certificate for a site and switch it to HTTPS'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from help" -f -a "renew" -d 'Renew a site\'s certificate now and reload nginx'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from help" -f -a "auto" -d 'Turn automatic renewal on or off (sets `autorenew` in the certbot renewal config)'
complete -c ngx -n "__fish_ngx_using_subcommand cert; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ngx -n "__fish_ngx_using_subcommand logs" -s n -l lines -d 'Number of lines to show' -r
complete -c ngx -n "__fish_ngx_using_subcommand logs" -s e -l error -d 'Error log instead of access log'
complete -c ngx -n "__fish_ngx_using_subcommand logs" -s f -l follow -d 'Keep following new lines (Ctrl-C to stop)'
complete -c ngx -n "__fish_ngx_using_subcommand logs" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand test" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand reload" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand doctor" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand regen" -l all -d 'Every site created by ngx'
complete -c ngx -n "__fish_ngx_using_subcommand regen" -s y -l yes -d 'Don\'t ask for confirmation'
complete -c ngx -n "__fish_ngx_using_subcommand regen" -l dry-run -d 'Show the diffs without changing anything'
complete -c ngx -n "__fish_ngx_using_subcommand regen" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand split" -s y -l yes -d 'Don\'t ask for confirmation'
complete -c ngx -n "__fish_ngx_using_subcommand split" -l dry-run -d 'Show the plan without changing anything'
complete -c ngx -n "__fish_ngx_using_subcommand split" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and not __fish_seen_subcommand_from list add rm help" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and not __fish_seen_subcommand_from list add rm help" -f -a "list" -d 'List the users who can log in'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and not __fish_seen_subcommand_from list add rm help" -f -a "add" -d 'Add a user, or change their password (asked twice, or read from stdin with --password-stdin)'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and not __fish_seen_subcommand_from list add rm help" -f -a "rm" -d 'Remove a user'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and not __fish_seen_subcommand_from list add rm help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and __fish_seen_subcommand_from add" -l password-stdin -d 'Read the password from the first line of stdin'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and __fish_seen_subcommand_from add" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and __fish_seen_subcommand_from rm" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and __fish_seen_subcommand_from help" -f -a "list" -d 'List the users who can log in'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and __fish_seen_subcommand_from help" -f -a "add" -d 'Add a user, or change their password (asked twice, or read from stdin with --password-stdin)'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and __fish_seen_subcommand_from help" -f -a "rm" -d 'Remove a user'
complete -c ngx -n "__fish_ngx_using_subcommand auth; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ngx -n "__fish_ngx_using_subcommand check" -l max-5xx -d 'Warn when more than this percentage of a site\'s requests failed with 5xx in the last 24h' -r
complete -c ngx -n "__fish_ngx_using_subcommand check" -l notify -d 'POST a plain-text report to this URL (ntfy-style: Title/Priority headers). Repeatable' -r
complete -c ngx -n "__fish_ngx_using_subcommand check" -l notify-json -d 'POST a JSON report ({"text", "content", "status", "problems"}: Slack, Mattermost, Discord…). Repeatable' -r
complete -c ngx -n "__fish_ngx_using_subcommand check" -l mail -d 'Email the report through the local sendmail. Repeatable' -r
complete -c ngx -n "__fish_ngx_using_subcommand check" -l every -d 'How often the timer runs: 15m, 30m, 1h, 6h, 1d…' -r
complete -c ngx -n "__fish_ngx_using_subcommand check" -l json -d 'Print JSON'
complete -c ngx -n "__fish_ngx_using_subcommand check" -s q -l quiet -d 'No output, only the exit status'
complete -c ngx -n "__fish_ngx_using_subcommand check" -l always -d 'Notify on every run with problems (default: only when the problems change, and once when all is resolved)'
complete -c ngx -n "__fish_ngx_using_subcommand check" -l install-timer -d 'Install a systemd timer (or cron job) that runs this check with the same options'
complete -c ngx -n "__fish_ngx_using_subcommand check" -l remove-timer -d 'Remove the timer installed with --install-timer'
complete -c ngx -n "__fish_ngx_using_subcommand check" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand history" -s n -l limit -d 'Show at most this many entries' -r
complete -c ngx -n "__fish_ngx_using_subcommand history" -l json -d 'Print JSON'
complete -c ngx -n "__fish_ngx_using_subcommand history" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand undo" -s y -l yes -d 'Don\'t ask for confirmation'
complete -c ngx -n "__fish_ngx_using_subcommand undo" -l dry-run -d 'Show what would be restored without changing anything'
complete -c ngx -n "__fish_ngx_using_subcommand undo" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand init" -s y -l yes -d 'Don\'t ask for confirmation'
complete -c ngx -n "__fish_ngx_using_subcommand init" -l dry-run -d 'Show what would change without touching anything'
complete -c ngx -n "__fish_ngx_using_subcommand init" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand tui" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand completions" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand man" -s h -l help -d 'Print help'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "ls" -d 'List sites with status, SSL, renewal, ports and errors'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "show" -d 'Show everything about one site: SSL, routing, backend process, locations, logs, traffic'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "add" -d 'Create and enable a new site (asks for anything not given as a flag)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "edit" -d 'Change a site\'s settings: same questions as `add`, pre-filled with the current values'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "rm" -d 'Remove a site\'s configuration (a backup is kept)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "enable" -d 'Enable a disabled site'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "disable" -d 'Disable a site without deleting it (`ngx enable` brings it back with the same ID)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "suspend" -d 'Suspend a site: every request gets a 503 and a notice page (`ngx resume` brings it back as it was)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "resume" -d 'Bring a suspended site back'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "cert" -d 'Certificates: info, issue, renew, auto-renewal on/off (wraps certbot)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "logs" -d 'Show a site\'s access log (or error log with -e)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "test" -d 'Validate the nginx configuration (`nginx -t`)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "reload" -d 'Validate, then reload nginx (waits until the new config is live)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "doctor" -d 'Check the environment: nginx, certbot, layout, renewal timer, permissions'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "regen" -d 'Rewrite ngx-managed sites with the current template (after an upgrade): diffs, one confirmation, one nginx -t'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "split" -d 'Split a file that defines several sites into one file per site (so each can be edited on its own)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "auth" -d 'Users of password-protected sites (basic auth)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "check" -d 'Check every site for monitoring/cron: down sites, expired or expiring certificates, renewal problems'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "history" -d 'List recorded changes (newest first), optionally only those touching one site'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "undo" -d 'Undo a change: restore every file it touched (default: the latest change)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "init" -d 'Create sites-available/ + sites-enabled/ and include them from nginx.conf (Arch, RHEL, …)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "tui" -d 'Full-screen dashboard (also the default when no command is given)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "completions" -d 'Print a shell completion script (bash, zsh, fish, elvish, powershell)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "man" -d 'Generate man pages into a directory'
complete -c ngx -n "__fish_ngx_using_subcommand help; and not __fish_seen_subcommand_from ls show add edit rm enable disable suspend resume cert logs test reload doctor regen split auth check history undo init tui completions man help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and __fish_seen_subcommand_from cert" -f -a "info" -d 'Show certificate details: expiry, issuer, covered names, renewal'
complete -c ngx -n "__fish_ngx_using_subcommand help; and __fish_seen_subcommand_from cert" -f -a "issue" -d 'Obtain a Let\'s Encrypt certificate for a site and switch it to HTTPS'
complete -c ngx -n "__fish_ngx_using_subcommand help; and __fish_seen_subcommand_from cert" -f -a "renew" -d 'Renew a site\'s certificate now and reload nginx'
complete -c ngx -n "__fish_ngx_using_subcommand help; and __fish_seen_subcommand_from cert" -f -a "auto" -d 'Turn automatic renewal on or off (sets `autorenew` in the certbot renewal config)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and __fish_seen_subcommand_from auth" -f -a "list" -d 'List the users who can log in'
complete -c ngx -n "__fish_ngx_using_subcommand help; and __fish_seen_subcommand_from auth" -f -a "add" -d 'Add a user, or change their password (asked twice, or read from stdin with --password-stdin)'
complete -c ngx -n "__fish_ngx_using_subcommand help; and __fish_seen_subcommand_from auth" -f -a "rm" -d 'Remove a user'
