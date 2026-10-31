# Command reference

Commands that change anything (`add`, `edit`, `rm`, `enable`, `disable`, `cert issue|renew|auto`, `reload`, `init`) need
root. Read-only commands (`ls`, `show`, `logs`, `test`, `doctor`, the dashboard) work as a normal user, with some
details hidden (see [Running without root](troubleshooting.md#running-without-root)).

Wherever a command takes `<site>`, you can pass the site's **ID** (the first column of `ngx ls`) or **any of its
domain names**, including aliases: `ngx show 4`, `ngx show staging.example.com`, `ngx show www.staging.example.com`.

Every command that changes files goes through the same [safe change procedure](how-it-works.md#safe-changes):
backup → write → `nginx -t` → roll back on failure → reload and wait until nginx is serving the new config.
Commands with `--dry-run` print what they would do and change nothing.

Dates are shown as `dd/mm/yy`.

- [ngx (dashboard)](#ngx)
- [ngx ls](#ngx-ls)
- [ngx show](#ngx-show)
- [ngx add](#ngx-add)
- [ngx edit](#ngx-edit)
- [ngx rm](#ngx-rm)
- [ngx enable / disable](#ngx-enable--ngx-disable)
- [ngx cert](#ngx-cert)
- [ngx logs](#ngx-logs)
- [ngx test](#ngx-test)
- [ngx reload](#ngx-reload)
- [ngx doctor](#ngx-doctor)
- [ngx init](#ngx-init)

---

## ngx

With no command (or `ngx tui`), opens the full-screen dashboard. See [dashboard.md](dashboard.md).

---

## ngx ls

```
ngx ls [-a|--all] [--json]
```

Lists sites, sorted by ID.

```
ID  STATUS  DOMAIN               SSL                     RENEW   IN   LOCAL       CONFIG                      ERROR
1   ● up    example.com          valid until 14/01/27    auto    443  3000        sites-enabled/example.conf  —
2   ● up    api.example.com      valid until 09/11/26 ⚠  manual  443  8080        sites-enabled/api.conf      —
4   ○ down  staging.example.com  expired 02/10/26        manual  443  4000        sites-enabled/staging.conf  502 Bad Gateway: upstream 127.0.0.1:4000 refused connection

6 domains · 4 up · 2 down · 1 cert expired · 1 expiring soon (manual renewal)
```

| Column | Meaning |
|---|---|
| `ID` | Stable site number. It never changes and is never reused (see [IDs](how-it-works.md#site-ids)). |
| `STATUS` | `● up`, `○ down` or `◌ off` (disabled). See [how status is decided](how-it-works.md#how-status-is-decided). |
| `DOMAIN` | The first `server_name`. Other names are aliases (visible in `ngx show`). |
| `SSL` | `valid until <date>`, `valid until <date> ⚠` (less than 30 days left), `expired <date>`, `none`, `missing` (the configured certificate file doesn't exist) or `unreadable` (usually: not root). |
| `RENEW` | `auto`, `manual` or `—` (no certificate). See [renewal](ssl.md#auto-or-manual). |
| `IN` | Public port: the SSL port if there is one, otherwise the first `listen` port. |
| `LOCAL` | Where requests go: upstream port, unix socket name (`php8.3-fpm`), `files` for static sites, `→` for pure redirects. |
| `CONFIG` | Config file, relative to the nginx directory. |
| `ERROR` | Why the site is down, or `—`. |

The summary line counts the listed sites. Expiring certificates are flagged `(manual renewal)` when none of them
will renew by themselves.

Options:

- `-a, --all`: include disabled sites (hidden by default, with a note saying how many).
- `--json`: print a JSON array instead of the table. Fields: `id`, `domain`, `aliases`, `status` (`up`/`down`/`disabled`), `error`,
  `enabled`, `managed`, `config`, `file`, `kind`, `inbound`, `upstream`, `root`,
  `ssl.{certificate,expires,issuer,names,expired,expiring_soon,renewal}`, `locations[].{match,action}`.

```
ngx ls --json | jq -r '.[] | select(.status == "down") | "\(.id) \(.domain): \(.error)"'
```

---

## ngx show

```
ngx show <site> [--json]
```

Everything about one site. Sections only appear when they apply.

| Section | Contents |
|---|---|
| header | Domain and `● UP` / `○ DOWN` / `◌ DISABLED`. |
| `ID`, `Aliases`, `Config` | Config path with modification time, plus `disabled` / `managed by ngx` notes. |
| `Problem` | Only when the site is down: the same message as the `ERROR` column. |
| `SSL` | Status and days left, issuer (`Let's Encrypt (R11)`), renewal (`auto · certbot timer, next check …` or `manual · <reason>`), `Covers` (names on the certificate, when different from the domain), and `Not covered` (site names the certificate does **not** cover; browsers will show an error for these). |
| `ROUTING` | Inbound ports (`:443 (HTTPS) · :80 → redirect to HTTPS`), type (reverse proxy / FastCGI / static files / redirect), and upstream, root or redirect target. |
| `BACKEND` | For proxy and PHP sites: the process behind the upstream. Runtime (Node.js, PHP-FPM, Python + gunicorn/uvicorn/…, Bun, Deno, Java, otherwise the executable name), who manages it (systemd unit, pm2 app, Docker container, supervisord), process state with pid and uptime (and pool and worker count for php-fpm), and working directory or document root. When nothing is listening, ngx looks for the pm2 app (matched by `PORT` or working directory) or the systemd unit (guessed from the socket name) and shows its state and last exit. |
| `LOCATIONS` | Each `location` and what it does (`proxy …`, `fastcgi …`, `files …`, `try_files …`, `return …`). WebSocket upgrades are marked. |
| `LOGS` | The site's own access and error log, when it has them. |
| `RECENT ERRORS` | Last 3 errors from the site's error log. Without its own error log, ngx reads the shared `/var/log/nginx/error.log` and keeps only lines tagged with this domain. Each line shows time, likely HTTP status (502/504/404/403) and a shortened message. |
| `TRAFFIC (24h)` | Requests, share of 5xx responses and average latency over the last 24 hours, read from the site's access log (and its `.1` rotation). Latency needs `$request_time` in the log format, which sites created by ngx have; otherwise it shows `—`. |

`--json` prints the same fields as `ngx ls --json` for this site.

---

## ngx add

```
ngx add [flags]
```

Creates a site, enables it and reloads nginx. Asks for anything you don't pass as a flag.

### The questions

1. **Domain**: must be a valid name not already used by another site.
2. **Also serve www.&lt;domain&gt;?**: only asked for apex domains (`example.com`, not `app.example.com`).
3. **Site type**:
   - *reverse proxy*: forwards to an app on a local port. Then asks **Local port** and **Proxy WebSocket connections?**
   - *static files*: serves a directory. Then asks **Document root** (default `/var/www/<domain>`).
   - *PHP (php-fpm)*: asks **Document root** and **php-fpm socket**. If exactly one socket exists in
     `/run/php`, `/run/php-fpm` or `/var/run/php`, it's used automatically. With several, you pick one.
4. **SSL**: *Let's Encrypt (certbot)*, *existing certificate* (asks for the certificate and key paths) or *none*.
5. **Email**: only for Let's Encrypt when certbot has no account on this machine yet. Optional.
6. **Inbound (public) port**: default 443 with SSL, 80 without.

Then ngx prints the generated config and asks **Create and enable this site?**

With SSL on a port other than 80, ngx also adds a port 80 server that redirects to HTTPS (unless `--no-redirect`).
That server also answers Let's Encrypt challenges, so renewals keep working.

### What happens with Let's Encrypt

1. If no certificate exists yet, ngx first enables the site over **HTTP only**, so the domain can answer the ACME challenge.
2. It runs `certbot certonly --webroot -w /var/lib/ngx/acme --cert-name <domain> -d <domain> -d <alias>…`.
3. It switches the site to the final HTTPS config and reloads again.

If certbot fails (DNS not pointing here, port 80 blocked, rate limits…), the site stays on HTTP, and ngx tells you
to run `ngx cert issue <site>` once the problem is fixed. See [ssl.md](ssl.md).

### Flags

| Flag | Meaning |
|---|---|
| `-d, --domain <DOMAIN>` | Primary domain. |
| `-a, --alias <NAME>` | Extra server name. Repeat it for more names. |
| `-t, --type <TYPE>` | `proxy`, `static` or `php`. If left out, guessed from the other flags: `--socket` → php, `--root` → static, `--local` → proxy. |
| `-l, --local <PORT\|HOST:PORT>` | Upstream for proxy sites. A bare port means `127.0.0.1:<PORT>`. |
| `--root <DIR>` | Document root (static and php). Created if missing; static sites get a placeholder `index.html`. |
| `--socket <PATH>` | php-fpm socket. |
| `-i, --in <PORT>` | Public port. |
| `--ssl <MODE>` | `letsencrypt` (aliases: `le`, `certbot`), `existing` or `none`. |
| `--cert <PATH>`, `--key <PATH>` | Certificate and key for `--ssl existing`. Passing `--cert` implies `--ssl existing`. |
| `--email <EMAIL>` | Let's Encrypt account email. |
| `--websocket` | Proxy WebSocket upgrades (also sets a 1-hour read timeout). |
| `--no-redirect` | Don't redirect port 80 to HTTPS; serve the site on both. |
| `--staging` | Use the Let's Encrypt staging server (certificates are not trusted by browsers; good for testing). |
| `-y, --yes` | No questions. Missing values use defaults: proxy, Let's Encrypt, port 443. `--domain` is required, plus `--local` for proxy sites. |
| `--dry-run` | Print the config and the files that would be written. Doesn't need root. |

### Examples

```
sudo ngx add                                                     # interactive
sudo ngx add -y -d app.example.com -l 3000                       # proxy + Let's Encrypt
sudo ngx add -y -d app.example.com -l 3000 --websocket --email me@example.com
sudo ngx add -y -d docs.example.com --root /var/www/docs --ssl none
sudo ngx add -y -d blog.example.com -t php --root /var/www/blog
sudo ngx add -y -d api.internal -l 10.0.0.5:8080 --ssl existing --cert /etc/ssl/api.pem --key /etc/ssl/api.key
ngx add -y -d app.example.com -l 3000 --dry-run                  # preview only
```

### Files written

- `/etc/nginx/sites-available/<domain>.conf` and a link in `sites-enabled/`.
- `00-ngx-logformat.conf` (once): defines the `ngx` log format with request timing.
- `/var/log/nginx/<domain>.access.log` and `.error.log` (created by nginx).

If nginx doesn't use `sites-enabled/` yet, ngx offers to set it up first (see [`ngx init`](#ngx-init)). If you decline,
the site goes to `conf.d/<domain>.conf` instead.

---

## ngx edit

```
ngx edit <site> [flags]
ngx edit <site> --raw
```

Changes a site's settings **without you touching the nginx file**. ngx asks the same questions as `ngx add`, with
every answer pre-filled from the current settings. Press Enter to keep a value, or type a new one. Aliases are
asked as one space-separated list; clear it to remove all aliases.

ngx then shows a colored **diff** of the config and asks **Apply these changes?** before writing anything.

```
$ sudo ngx edit shop.example.com
Editing site #8 shop.example.com  (Enter keeps the current value)
> Domain: shop.example.com
> Aliases (space-separated, empty for none):
> Site type: reverse proxy (app on a local port)
> Local port (app listening on 127.0.0.1): 3200
> Proxy WebSocket connections? No
> SSL: none (HTTP only)
> Inbound (public) port: 80
--- sites-available/shop.example.com.conf (current)
+++ sites-available/shop.example.com.conf (new)
-        proxy_pass http://127.0.0.1:3100;
+        proxy_pass http://127.0.0.1:3200;
> Apply these changes? Yes
✓ site #8 shop.example.com updated
```

It takes the same flags as `ngx add`. With `-y`, nothing is asked: only the flags you pass change, everything else
stays as it is.

```
sudo ngx edit 3 -y --local 4000              # move the upstream port
sudo ngx edit 3 -y -d new.example.com        # rename: file and link are renamed, ID stays 3
sudo ngx edit 3 -y -a www.example.com        # set aliases (replaces the list)
sudo ngx edit 3 -y --ssl letsencrypt         # switch to HTTPS
sudo ngx edit 3 -y --local 4000 --dry-run    # show the diff only
```

Details:

- **Renaming** the domain renames `<domain>.conf` and its `sites-enabled` link. The site keeps its ID. A disabled site stays disabled.
- **Switching to Let's Encrypt** without an existing certificate follows the same steps as `add`: HTTP first, then certbot, then HTTPS.
- **Adding names to a Let's Encrypt site**: the new config is applied first (it answers challenges for the new names), then certbot extends the certificate (`--expand`) and nginx is reloaded.
- **Sites you wrote yourself** can be edited too, if they are a plain proxy, static or PHP site. ngx reads their current settings and warns that saving **replaces the file with a generated config**. The original is in the [backup](how-it-works.md#backups), and access/error log paths are kept. Custom directives (extra headers, rate limits, extra locations…) are not carried over, so check the diff.
- ngx refuses files that define **several sites**, and setups it can't generate (variable upstreams, redirect-only sites, upstreams that aren't TCP or a unix socket). Use `--raw` for those.

### --raw

Opens the nginx file in `$VISUAL`, `$EDITOR`, `nano` or `vi` (first one found). You edit a temporary copy. When you
save and quit, ngx checks the syntax, writes it and runs `nginx -t`. If anything fails, the change is rolled back
and ngx offers to reopen the editor. A file created by ngx that you edit with `--raw` stops being "managed by ngx",
but you can still use the prompt-based `ngx edit` on it later.

---

## ngx rm

```
ngx rm <site> [--delete-cert] [-y] [--dry-run]
```

Asks for confirmation, then removes the site's `sites-enabled` link and its config file, and reloads nginx. The file
is kept in the [backups](how-it-works.md#backups). If the file defines other sites too, the question says so; they
are removed together.

- `--delete-cert`: also delete the site's certbot certificate (`certbot delete --cert-name …`).
- `-y`: don't ask.

The ID is retired and never given to another site.

---

## ngx enable / ngx disable

```
ngx enable <site> [--dry-run]
ngx disable <site> [--dry-run]
```

`disable` takes a site offline without deleting it, and `enable` brings it back with the same ID.

- Sites in `sites-available/`: the link in `sites-enabled/` is removed or recreated.
- A real file sitting directly in `sites-enabled/` (not a link) is moved to `sites-available/` when disabled.
- Sites in `conf.d/` are renamed to `<name>.conf.disabled` and back.

If the file defines several sites, they are toggled together (ngx prints a note).

---

## ngx cert

### ngx cert info

```
ngx cert info <site>
```

Prints the site's SSL section from `ngx show`, plus the certificate file path.

### ngx cert issue

```
ngx cert issue <site> [--email <EMAIL>] [--staging]
```

Gets a Let's Encrypt certificate for the site's domain and aliases:

- **Sites created by ngx**: `certbot certonly --webroot` via the ACME location the site already has. Then the config is regenerated with HTTPS on 443 and a port 80 redirect (if it was HTTP-only).
- **Other sites**: certbot's nginx plugin (`certbot --nginx --redirect`), which edits the config itself. Needs the plugin package (`python3-certbot-nginx` or `certbot-nginx`).

### ngx cert renew

```
ngx cert renew <site> [--force]
```

Runs `certbot renew --cert-name <name>`, then reloads nginx. certbot only renews certificates that are close to
expiry; `--force` renews anyway.

### ngx cert auto

```
ngx cert auto <site> on|off
```

Sets `autorenew = True|False` in the certificate's certbot renewal config. When turning it on, ngx warns if no certbot
timer or cron job exists, since nothing would run the renewal. See [ssl.md](ssl.md#auto-or-manual).

---

## ngx logs

```
ngx logs <site> [-e|--error] [-f|--follow] [-n <N>]
```

Shows the last `N` lines (default 50) of the site's access log, or its error log with `-e`. `-f` keeps following new
lines (`tail -F`) until Ctrl-C. If the site has no log of its own, ngx shows the shared `/var/log/nginx/access.log` or
`error.log` and says so.

---

## ngx test

Runs `nginx -t` and prints warnings and errors with their file and line. It exits with an error code when the
configuration is invalid. Needs nginx installed; as a normal user, nginx itself may fail to open certificates or the
pid file.

---

## ngx reload

Runs `nginx -t`, then reloads: `systemctl reload nginx` if the systemd service is active, otherwise `nginx -s reload`.
ngx then **waits until nginx is actually serving the new configuration**, meaning new worker processes have started
and the old ones have stopped accepting requests (up to 5 seconds). If that doesn't happen, ngx reports it and points
to the error log. If nginx isn't running, ngx says so and doesn't start it.

---

## ngx doctor

Checks the environment and prints ✓ / ✗ for each item:

- nginx installed (path and version)
- nginx running
- `nginx.conf` found
- `sites-enabled` layout set up and included
- `nginx -t` passes (root only)
- certbot installed
- a renewal scheduler exists (certbot timer or cron)
- running as root
- state directory

---

## ngx init

```
ngx init [-y] [--dry-run]
```

For distributions without the Debian `sites-available/` + `sites-enabled/` layout (Arch, Fedora/RHEL, the nginx.org
packages…): creates both directories and adds `include /etc/nginx/sites-enabled/*;` at the end of the `http { }`
block in `nginx.conf`. The change is backed up, tested and reloaded like any other. `ngx add` runs this
automatically (with a question) when needed. Does nothing if the layout already exists and is included.
