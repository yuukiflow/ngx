# How ngx works

## Where sites come from

ngx reads, in this order:

1. every entry in `sites-enabled/` (enabled; links are followed to their target in `sites-available/`),
2. files in `sites-available/` that nothing in `sites-enabled/` links to (disabled),
3. `conf.d/*.conf` (enabled) and `conf.d/*.conf.disabled` (disabled).

Hidden files and editor/package leftovers (`~`, `.bak`, `.orig`, `.swp`, `.save`, `.dpkg-*`) are ignored. The nginx
directory is the first of `/etc/nginx`, `/usr/local/etc/nginx` and `/usr/local/nginx/conf` that contains
`nginx.conf` (override with `NGX_NGINX_DIR`).

ngx has its own parser for nginx config files. In each file, it groups the `server { }` blocks that share a
`server_name` into one **site**. The usual pair (a port 80 block that redirects, plus a 443 block that serves) is one
site. The **main block** is the one that serves content, preferring the SSL one. Its first name is the domain;
every other name is an alias. Named `upstream { }` blocks in the same file are resolved to their first `server`.

A file that can't be parsed still shows up, named after the file, with the parse error as its status.

## Site IDs

Each site gets a number the first time ngx sees it, stored in `/var/lib/ngx/state.json` under the key
`<file name>#<domain>`. Numbers are never reused, so `ngx show 4` always means the same site. Renaming with
`ngx edit` moves the number to the new key. Disabling and re-enabling keeps it. `ngx rm` retires it.

If the state file can't be written (not root), IDs are still assigned in a consistent order, but only for that run.

## How status is decided

Checked in this order; the first match wins:

| Condition | STATUS | ERROR |
|---|---|---|
| Not enabled | `◌ off` | — |
| The file can't be parsed | `○ down` | `config error: <parse error> (line N)` |
| A name+port already used by an earlier site (nginx ignores the duplicate) | `○ down` | `config error: duplicate server_name <name> (line N)` |
| `nginx -t` reports an error in this file (root only) | `○ down` | `config error: <message> (line N)` |
| No nginx master process is running | `○ down` | `nginx is not running` |
| Proxy / PHP site whose upstream can't be reached (TCP connect with a 1.5 s timeout, or unix socket connect) | `○ down` | `502 Bad Gateway: upstream <addr> refused connection` (or `timed out`, `socket does not exist`…) |
| Static site whose root directory doesn't exist | `○ down` | `404 Not Found: root <dir> does not exist` |
| Otherwise | `● up` | — |

Upstreams built from variables (`proxy_pass http://$backend`) aren't checked. `nginx -t` errors that point to no
particular site (e.g. in `nginx.conf`) are shown below the `ngx ls` table and in the dashboard's top bar.

All sites are checked in parallel, so `ngx ls` takes about as long as the slowest upstream.

## Backend detection (ngx show)

ngx finds the process listening on the upstream port or socket (via `/proc/net/tcp*`, `/proc/net/unix` and each
process's open files), then works out:

- **runtime**: from the executable, asking it for its version (`node --version`, `php-fpm -v`, …),
- **manager**: a pm2 daemon among its parent processes, supervisord, a Docker/containerd/Podman control group, or
  the systemd unit from its control group,
- **uptime**, **working directory**, and for php-fpm the pools and number of workers.

Seeing other users' processes requires root. Without it, ngx shows `running · details need root`.

## Generated configs

Sites created by `ngx add` (or converted by `ngx edit`) start with:

```
# managed by ngx — generated 31/10/26 20:48. Edit with `ngx edit`.
# ngx-spec: {"domain":"app.example.com","aliases":[],"inbound":443,"backend":{...},"ssl":{"mode":"letsencrypt"},"redirect_http":true}
```

The `ngx-spec` line records the answers the config was built from, and `ngx edit` uses it to pre-fill its questions.
Don't edit it by hand; use `ngx edit`.

A generated config contains:

- `listen` on IPv4 and IPv6 for the public port, and, with SSL, a port 80 server with the ACME location and a redirect,
- `ssl_certificate` / `ssl_certificate_key`, TLS 1.2 and 1.3, a shared session cache,
- `access_log /var/log/nginx/<domain>.access.log ngx` and `error_log /var/log/nginx/<domain>.error.log`,
- for **proxy** sites: `proxy_pass`, HTTP/1.1, the `Host`, `X-Real-IP`, `X-Forwarded-For` and `X-Forwarded-Proto` headers, and the WebSocket upgrade headers when enabled,
- for **static** sites: `root`, `index`, `try_files $uri $uri/ =404`,
- for **PHP** sites: `root`, `index index.php`, `try_files … /index.php?$query_string`, a `\.php$` location with
  `fastcgi_params` and `SCRIPT_FILENAME`, and a rule denying dotfiles (except `.well-known`).

The `ngx` log format (in `sites-enabled/00-ngx-logformat.conf`) is the standard "combined" format plus
`rt=$request_time`, so `ngx show` can report latency.

## Safe changes

Every change is one transaction:

1. **Back up** each file about to change to `/var/lib/ngx/backups/<YYYYmmdd-HHMMSS>/`.
2. **Write** the new files. Each file is written to a temporary name and then renamed over the old one, so nginx
   never sees a half-written file.
3. **Test** with `nginx -t`.
4. If the test fails, **roll back**: every touched file and link is restored exactly as it was, and the nginx errors
   are printed. nginx keeps running the old config the whole time.
5. If it passes, **reload** and wait until new nginx workers are running and the old ones have stopped accepting
   requests. Only then does ngx report success.

`--dry-run` stops before step 1 and lists what would be written.

### Backups

Backup files are named after their full path, with `/` replaced by `%`:

```
/var/lib/ngx/backups/20261031-204512/etc%nginx%sites-available%api.conf
```

To restore one by hand:

```
sudo cp '/var/lib/ngx/backups/20261031-204512/etc%nginx%sites-available%api.conf' /etc/nginx/sites-available/api.conf
sudo ngx reload
```

Backups are never deleted automatically.

## Files ngx touches

| Path | What |
|---|---|
| `/etc/nginx/sites-available/<domain>.conf` | Site configs created by ngx |
| `/etc/nginx/sites-enabled/<domain>.conf` | Links that enable them |
| `/etc/nginx/sites-*/00-ngx-logformat.conf` | The `ngx` log format |
| `/etc/nginx/conf.d/<domain>.conf` | Site configs, if you declined the `sites-enabled` layout |
| `/etc/nginx/nginx.conf` | Only `ngx init`: one `include` line at the end of `http { }` |
| `/var/lib/ngx/state.json` | Site IDs |
| `/var/lib/ngx/backups/` | Backups |
| `/var/lib/ngx/acme/` | ACME challenge webroot |
| `/etc/letsencrypt/renewal/<name>.conf` | Only `ngx cert auto` (the `autorenew` line) |
| `/var/www/<domain>/` | Document root for new static/PHP sites, if it doesn't exist (static sites get a placeholder `index.html`) |

Everything else in `/etc/letsencrypt` is handled by certbot.
