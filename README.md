# ngx

Manage nginx sites and Let's Encrypt certificates from one binary, with a full-screen dashboard and commands you can script.
You answer questions and ngx writes the nginx config: you never have to edit the files yourself.

```
$ ngx ls

ID  STATUS  DOMAIN               SSL                     RENEW   IN   LOCAL       CONFIG                      ERROR
1   ● up    example.com          valid until 14/01/27    auto    443  3000        sites-enabled/example.conf  —
2   ● up    api.example.com      valid until 09/11/26 ⚠  manual  443  8080        sites-enabled/api.conf      —
3   ● up    blog.example.com     valid until 22/03/27    auto    443  php8.3-fpm  sites-enabled/blog.conf     —
4   ○ down  staging.example.com  expired 02/10/26        manual  443  4000        sites-enabled/staging.conf  502 Bad Gateway: upstream 127.0.0.1:4000 refused connection
5   ● up    status.example.com   none                    —       80   3001        sites-enabled/status.conf   —
6   ○ down  old.example.com      valid until 28/06/27    auto    443  5000        sites-enabled/old.conf      config error: duplicate server_name www.example.com (line 11)

6 domains · 4 up · 2 down · 1 cert expired · 1 expiring soon (manual renewal)
```

## Features

- **See everything at a glance**: status, certificate expiry, auto/manual renewal, ports, and *why* a site is down
  (upstream refusing connections, config errors, duplicate names, nginx not running).
- **Drill into one site** (`ngx show 4`): certificate details, routing, the process behind the upstream (runtime,
  pm2/systemd/Docker, uptime), locations, recent errors, 24h traffic and 5xx rate.
- **Create sites by answering questions** (`ngx add`, or a form in the dashboard): reverse proxy, static files or
  PHP-FPM, with Let's Encrypt issued automatically (HTTP or DNS challenge, wildcards included).
- **Common needs built in**: upload size, HSTS, password protection, rate limiting, `www` → apex redirect, extra
  paths proxied to other apps. Your own directives go in per-site [snippets](docs/commands.md#--snippet) that survive
  regeneration.
- **Change them the same way** (`ngx edit`): the questions come back pre-filled; you see a diff before anything is written.
- **Changes can't break nginx**: every change is backed up, checked with `nginx -t`, rolled back if it fails, and
  reported as done only once nginx is serving the new config. `ngx history` shows who changed what; `ngx undo`
  reverts it.
- **Renewals you can trust**: certificates reload nginx when renewed, and `ngx check` (for cron or monitoring) tells
  you about down sites and certificates that aren't renewing, now that Let's Encrypt no longer emails about it.
- **No new attack surface**: every value is validated before it goes into a config, generated configs hide dotfiles
  and only run existing PHP scripts, and nothing a backend user controls ever runs as root
  ([details](docs/security.md)).
- **Stable IDs**: `ngx show 4` always means the same site.
- Works with **existing configs** as they are, on Debian/Ubuntu (`sites-enabled`) and Arch/Fedora (`ngx init` sets up the layout).

## Install

Prebuilt static binary for any Linux (x86_64, aarch64), checksum-verified:

```
curl -fsSL https://raw.githubusercontent.com/yuukiflow/ngx/main/scripts/install.sh | sudo sh
ngx doctor
```

From source:

```
cargo install --path .
sudo install -m755 ~/.cargo/bin/ngx /usr/local/bin/ngx    # so `sudo ngx` works
```

More in [docs/install.md](docs/install.md) (manual download, man pages, upgrade).

## Quick start

```
ngx                                    # dashboard
ngx ls                                 # list sites
ngx show blog.example.com              # one site, in detail
sudo ngx add                           # new site: domain, type, local port, SSL, inbound port
sudo ngx add -y -d app.example.com -l 3000                 # same, no questions (proxy + Let's Encrypt)
sudo ngx edit 4                        # change settings (pre-filled questions, diff, confirm)
sudo ngx edit 4 -y --local 4001        # change one setting directly
sudo ngx edit 4 -y --max-body 50m --route /api=8080
sudo ngx edit 4 --snippet headers      # your own directives, validated
sudo ngx auth add 4 alice              # password-protected sites
ngx check                              # for cron / monitoring (exit 0/1/2)
ngx history && sudo ngx undo           # what changed, and revert it
sudo ngx disable 4 / sudo ngx enable 4
sudo ngx cert renew 2
sudo ngx cert auto 2 on
ngx logs 4 -e -f
```

## Commands

| Command | |
|---|---|
| `ngx` | Full-screen dashboard |
| `ngx ls [-a] [--json]` | List sites |
| `ngx show <site> [--json]` | Everything about one site |
| `ngx add` | Create and enable a site |
| `ngx edit <site> [--snippet <name>\|--raw]` | Change a site's settings / your own directives |
| `ngx rm <site> [--delete-cert]` | Remove a site (backup kept) |
| `ngx enable` / `disable <site>` | Toggle a site |
| `ngx cert info\|issue\|renew\|auto <site>` | Certificates (wraps certbot) |
| `ngx auth list\|add\|rm <site>` | Users of password-protected sites |
| `ngx check [--json]` | Problems across all sites, with an exit status for monitoring |
| `ngx history [<site>]` / `ngx undo [<id>]` | Change history and undo |
| `ngx logs <site> [-e] [-f]` | Access / error log |
| `ngx test` / `reload` | `nginx -t` / validated reload |
| `ngx doctor` | Check the environment |
| `ngx init` | Set up `sites-available`/`sites-enabled` |

`<site>` is an ID or any of the site's domain names. Changes need root.

## Documentation

- [Command reference](docs/commands.md)
- [Dashboard](docs/dashboard.md)
- [Certificates and renewal](docs/ssl.md)
- [How it works](docs/how-it-works.md): status rules, generated configs, safe changes, backups, files touched
- [Security](docs/security.md): why using ngx is as safe as writing the configs yourself
- [Troubleshooting](docs/troubleshooting.md)
- [Development](docs/development.md): sandbox/demo mode, tests, code map

`ngx <command> --help` and `man ngx-<command>` are available too.

## Try it without nginx

```
python3 demo/make_demo.py /tmp/ngx-demo
NGX_ROOT=/tmp/ngx-demo ngx
```

## Requirements

Linux, nginx, and certbot for Let's Encrypt. Building needs Rust 1.85+.
