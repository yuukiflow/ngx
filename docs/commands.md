# Command reference

Commands that change anything (`add`, `edit`, `rm`, `enable`, `disable`, `suspend`, `resume`, `regen`, `split`, `cert issue|renew|auto`,
`auth add|rm`, `undo`, `reload`, `init`, `check --install-timer`) need root. Read-only commands (`ls`, `show`, `logs`, `check`, `history`, `test`, `doctor`,
the dashboard) work as a normal user, with some details hidden (see [Running without root](troubleshooting.md#running-without-root)).

Wherever a command takes `<site>`, you can pass the site's **ID** (the first column of `ngx ls`) or **any of its
domain names**, including aliases: `ngx show 4`, `ngx show staging.example.com`, `ngx show www.staging.example.com`.

Every command that changes files goes through the same [safe change procedure](how-it-works.md#safe-changes):
backup → write → `nginx -t` → roll back on failure → reload and wait until nginx is serving the new config.
Each command is recorded in the [change history](#ngx-history), so it can be [undone](#ngx-undo). Only one ngx
command changes things at a time: a second one waits for the first to finish. Commands with `--dry-run` print what
they would do and change nothing.

Dates are shown as `dd/mm/yy`.

- [ngx (dashboard)](#ngx)
- [ngx ls](#ngx-ls)
- [ngx show](#ngx-show)
- [ngx add](#ngx-add)
- [ngx edit](#ngx-edit)
- [ngx regen](#ngx-regen)
- [ngx split](#ngx-split)
- [ngx rm](#ngx-rm)
- [ngx enable / disable](#ngx-enable--ngx-disable)
- [ngx suspend / resume](#ngx-suspend--ngx-resume)
- [ngx cert](#ngx-cert)
- [ngx auth](#ngx-auth)
- [ngx check](#ngx-check)
- [ngx history](#ngx-history)
- [ngx undo](#ngx-undo)
- [ngx logs](#ngx-logs)
- [ngx test](#ngx-test)
- [ngx reload](#ngx-reload)
- [ngx doctor](#ngx-doctor)
- [ngx init](#ngx-init)
- [ngx completions](#ngx-completions)

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
| `SSL` | `valid until <date>`, `valid until <date> ⚠` (renewal is late: less than a third of the certificate's lifetime left, at most 30 days), `expired <date>`, `none`, `missing` (the configured certificate file doesn't exist) or `unreadable` (usually: not root). |
| `RENEW` | `auto`, `auto ⚠` (renews, but nginx isn't reloaded afterwards, so it would keep serving the old certificate), `manual` or `—` (no certificate). See [renewal](ssl.md#auto-or-manual). |
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
  `ssl.{certificate,expires,issuer,names,expired,expiring_soon,renewal,reload_after_renewal}`, `locations[].{match,action}`.

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
| `ROUTING` | Inbound ports (`:443 (HTTPS) · :80 → redirect to HTTPS`), type (reverse proxy / FastCGI / static files / redirect), and upstream, root or redirect target. For sites created by ngx: `Settings` (upload size, HSTS, password protection with the number of users, rate limit, alias redirect) and `Snippets` (your own `ngx.d/<domain>/*.conf` files). |
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
4. **SSL**:
   - *Let's Encrypt (HTTP challenge)*: the usual choice; the domain must point here and port 80 be reachable.
   - *Let's Encrypt with DNS challenge*: for wildcard names (`*.example.com`) or servers not reachable on port 80.
     Asks for the **certbot DNS plugin** (`cloudflare`, `route53`, `digitalocean`, `ovh`…, the
     `certbot-dns-<plugin>` package must be installed) and its **credentials file**.
   - *existing certificate*: asks for the certificate and key paths.
   - *none*.
5. **Email**: only for Let's Encrypt when certbot has no account on this machine yet. Optional. Let's Encrypt no
   longer emails about expiring certificates; use [`ngx check`](#ngx-check) for that.
6. **Inbound (public) port**: default 443 with SSL, 80 without.
7. **Options**: a checklist (space toggles, Enter confirms). Each one you tick asks for its value:
   - *Max upload size*: `client_max_body_size` (nginx's default is 1m, too small for most upload forms).
   - *HSTS*: `Strict-Transport-Security: max-age=31536000`. Browsers will then refuse plain HTTP for this domain for a
     year, so turn it on once HTTPS works.
   - *Password protection*: basic auth; ngx asks for the first user and password (see [`ngx auth`](#ngx-auth)).
   - *Rate limit per client*: e.g. `10r/s`; short bursts (twice the rate, at least 10) are allowed, beyond that
     clients get `429 Too Many Requests`.
   - *Redirect aliases to the main domain*: `www.example.com/x` → `example.com/x` (301).
   - *Extra paths proxied to other apps*: e.g. `/api=8080` sends `/api/…` to `127.0.0.1:8080`.
   - *Security headers*: `X-Content-Type-Options: nosniff`, `X-Frame-Options: SAMEORIGIN` and
     `Referrer-Policy: strict-origin-when-cross-origin`. (nginx drops server-level `add_header`s in a location that
     has its own; keep that in mind in snippets.)
   - *Health check path* (proxy sites): e.g. `/health`. The status and [`ngx check`](#ngx-check) call it on the app
     directly, and anything but 2xx/3xx marks the site down, so an app that's running but broken is caught too.

HTTPS sites are served over **HTTP/2** (`http2 on;`, or `listen … ssl http2` on nginx before 1.25.1).

Then ngx prints the generated config and asks **Create and enable this site?** Answering no exits with status 1 and
changes nothing.

With SSL on a port other than 80, ngx also adds a port 80 server that redirects to HTTPS (unless `--no-redirect`).
That server also answers Let's Encrypt challenges, so renewals keep working.

### What happens with Let's Encrypt

1. If no certificate exists yet, ngx first enables the site over **HTTP only**, so the domain can answer the ACME challenge.
2. It runs `certbot certonly --webroot -w /var/lib/ngx/acme --cert-name <domain> -d <domain> -d <alias>…`.
3. It switches the site to the final HTTPS config and reloads again.

certbot records a deploy hook (`systemctl reload nginx`) in the certificate's renewal config, so nginx serves each
renewed certificate.

If certbot fails (DNS not pointing here, port 80 blocked, rate limits…), the site stays on HTTP, and ngx tells you
to run `ngx cert issue <site>` once the problem is fixed. See [ssl.md](ssl.md).

With the **DNS challenge**, nothing needs to be served first: ngx runs `certbot certonly --authenticator dns-<plugin>`,
then writes the HTTPS config. If certbot fails, nothing is written.

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
| `--ssl <MODE>` | `letsencrypt` (aliases: `le`, `certbot`), `dns`, `existing` or `none`. |
| `--dns <PLUGIN>`, `--dns-credentials <FILE>` | Let's Encrypt with the DNS challenge through `certbot-dns-<PLUGIN>`. Implies `--ssl dns`. |
| `--cert <PATH>`, `--key <PATH>` | Certificate and key for `--ssl existing`. Passing `--cert` implies `--ssl existing`. |
| `--email <EMAIL>` | Let's Encrypt account email. |
| `--websocket` | Proxy WebSocket upgrades (also sets a 1-hour read timeout). |
| `--no-redirect` | Don't redirect port 80 to HTTPS; serve the site on both. |
| `--staging` | Use the Let's Encrypt staging server (certificates are not trusted by browsers; good for testing). |
| `--max-body <SIZE>` | Max upload size, e.g. `50m`; `off` for nginx's default. |
| `--hsts on\|off` | HSTS (HTTPS sites only). |
| `--auth on\|off` | Password protection (users: [`ngx auth`](#ngx-auth)). |
| `--rate-limit <RATE>` | Requests per client, e.g. `10r/s` or `300r/m`; `off` to remove. |
| `--canonical on\|off` | Redirect the aliases to the main domain. |
| `--route <PATH=PORT>` | Proxy a path to another app: `/api=8080`, `/ws=10.0.0.5:9000`. Repeat for more; `--route off` removes them. |
| `--headers on\|off` | Security headers. |
| `--health <PATH>` | Health check path for proxy sites, e.g. `/health`; `off` to remove. |
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
sudo ngx add -y -d '*.example.com' -l 3000 --dns cloudflare --dns-credentials /root/.secrets/cloudflare.ini
sudo ngx add -y -d shop.example.com -a www.shop.example.com -l 3000 --canonical on --max-body 50m --route /api=8080
ngx add -y -d app.example.com -l 3000 --dry-run                  # preview only
```

### Files written

- `/etc/nginx/sites-available/<domain>.conf` and a link in `sites-enabled/`.
- `00-ngx-logformat.conf` (once): defines the `ngx` log format with request timing.
- `/etc/nginx/ngx.d/<domain>/`: the site's own directory, for your [snippets](#--snippet) and the password file.
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
- **Options** (upload size, HSTS, password, rate limit, alias redirect, extra paths) come back pre-ticked in the checklist. Flags change them directly: `--max-body off`, `--route off`, `--auth on`…
- **Sites you wrote yourself** can be edited too, if they are a plain proxy, static or PHP site. ngx reads their current settings and warns that saving **replaces the file with a generated config**; the original is in the [backup](how-it-works.md#backups), and access/error log paths are kept. What ngx doesn't generate is **kept**:
  - `client_max_body_size` and an HSTS header become the matching options,
  - a `location` that only proxies to `http://host:port` becomes an [extra route](#the-questions),
  - every other server-level directive and location (`gzip`, `add_header`, `location /files/ { alias … }`…) is moved, as written, into the snippet `ngx.d/<domain>/converted.conf`, shown before you confirm,
  - directives *inside* `location /` (or the PHP location) can't be kept, since the generated one replaces it. ngx lists them as warnings; re-add them with a more specific snippet location, or keep the site hand-written.
- Files that define **several sites**: split them first with [`ngx split`](#ngx-split). Setups ngx can't generate (variable upstreams, redirect-only sites, upstreams that aren't TCP or a unix socket): use `--raw`.

### --snippet

```
ngx edit <site> --snippet <name>
```

Your own nginx directives for a site created by ngx, in `/etc/nginx/ngx.d/<domain>/<name>.conf`. Every `*.conf` file
there is included inside the site's server block and **kept when ngx regenerates the config**. That's the way to add
what the questions don't cover (extra `location` blocks, headers, timeouts) without leaving the managed setup.

The snippet opens in your editor like `--raw`: on save it's checked with `nginx -t` and rolled back (with an offer to
edit again) if nginx rejects it. Don't add `location /`, since the generated config already has it.

```
sudo ngx edit 3 --snippet headers      # e.g. add_header X-Frame-Options DENY;
sudo ngx edit 3 --snippet health       # e.g. location = /health { return 200 "ok\n"; }
```

Snippets move along when the domain is renamed and are removed (and restorable with `ngx undo`) with `ngx rm`.

### --raw

Opens the nginx file in `$VISUAL`, `$EDITOR`, `nano` or `vi` (first one found). You edit a temporary copy. When you
save and quit, ngx checks the syntax, writes it and runs `nginx -t`. If anything fails, the change is rolled back
and ngx offers to reopen the editor. A file created by ngx that you edit with `--raw` stops being "managed by ngx",
but you can still use the prompt-based `ngx edit` on it later.

---

## ngx regen

```
ngx regen <site>... | --all  [-y] [--dry-run]
```

Rewrites sites created by ngx with the current template, after an upgrade brought template improvements (HTTP/2,
hidden-file protection, the snippets include…). Your settings and snippets are kept; only the generated parts change.
Every diff is shown, then one confirmation, one `nginx -t` for all of them, and one history entry (so one
`ngx undo` reverts them all). Sites still waiting for their Let's Encrypt certificate keep their HTTP-only config.
Sites ngx didn't create are skipped; convert them with `ngx edit`.

```
sudo ngx regen --all --dry-run      # what would change
sudo ngx regen --all
```

---

## ngx split

```
ngx split <site> [-y] [--dry-run]
```

For a file that defines several sites: moves each site's server blocks, as they are (with the comments right above
them), into its own `<domain>.conf` in the same directory, enabled the same way (links in `sites-enabled/` too).
Upstreams, maps and other shared directives stay in the original file; if nothing else is left, it's removed. Each
site keeps its ID. After that, each one can be edited, disabled or removed on its own.

```
$ sudo ngx split shop.example.com
sites-available/multi.conf defines 2 sites:
  #8 shop.example.com → sites-available/shop.example.com.conf
  #9 docs.example.com → sites-available/docs.example.com.conf
  the other directives (upstreams, maps…) stay in sites-available/multi.conf
> Split it? Yes
```

---

## ngx rm

```
ngx rm <site> [--delete-cert] [-y] [--dry-run]
```

Asks for confirmation, then removes the site's `sites-enabled` link and its config file, and reloads nginx. The file
is kept in the [backups](how-it-works.md#backups) and `ngx undo` brings it back. For sites created by ngx, the
site's `ngx.d/<domain>/` files (snippets, passwords) are removed too. If the file defines other sites too, the
question says so; they are removed together.

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

## ngx suspend / ngx resume

```
ngx suspend <site> [--page FILE] [--reason TEXT] [--dry-run]
ngx resume <site> [--dry-run]
```

`suspend` keeps a site online but answers every request with `503 Service Unavailable` and a notice page, for
example while an invoice is unpaid. `resume` brings the site back exactly as it was.

- Only sites created by ngx can be suspended; convert a hand-written one first with `ngx edit`.
- The page is `--page FILE` (HTML, up to 1 MB), or a short "temporarily unavailable" page. It is stored as
  `ngx.d/<domain>/suspended.html` and can't be requested directly.
- `--reason` is a note for you, shown by `ngx show` and `--json`; visitors never see it.
- While suspended, the backend, extra routes, password protection and snippets are left out of the config. The
  ACME location stays, so the certificate keeps renewing. Responses carry `Retry-After` and `Cache-Control: no-store`.
- `ngx ls` shows the site as `◐ suspended`, `ngx check` doesn't report it as down and doesn't probe its backend.
- Running `suspend` again on a suspended site keeps it suspended and updates the page or the reason. `resume` on a
  site that isn't suspended does nothing. Both exit 0 in these cases, so scripts can call them freely.
- `ngx edit` keeps the suspension; only `ngx resume` lifts it.

---

## ngx cert

### ngx cert info

```
ngx cert info <site>
```

Prints the site's SSL section from `ngx show`, plus the certificate file path.

### ngx cert issue

```
ngx cert issue <site> [--email <EMAIL>] [--staging] [--dns <PLUGIN> [--dns-credentials <FILE>]]
```

Gets a Let's Encrypt certificate for the site's domain and aliases. The certificate's renewal config gets a deploy
hook that reloads nginx.

- **Sites created by ngx**: `certbot certonly --webroot` via the ACME location the site already has. Then the config is regenerated with HTTPS on 443 and a port 80 redirect (if it was HTTP-only).
- **With `--dns`** (sites created by ngx): the DNS challenge through `certbot-dns-<PLUGIN>`, needed for wildcard names.
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

Sets `autorenew = True|False` in the certificate's certbot renewal config. Turning it on also adds a deploy hook that
reloads nginx after each renewal, if the certificate has none (that's the fix for `auto ⚠`), and warns if no certbot
timer or cron job exists, since nothing would run the renewal. See [ssl.md](ssl.md#auto-or-manual).

---

## ngx auth

```
ngx auth list <site>
ngx auth add <site> <user> [--password-stdin]
ngx auth rm <site> <user>
```

Users who can log in to a password-protected site (turned on with `ngx edit <site> --auth on`, or *Password protection*
in the options). `add` asks for the password twice (at least 8 characters), or reads one line from stdin with
`--password-stdin`; adding an existing user changes their password.

Passwords are stored as SHA-512 crypt hashes in `/etc/nginx/ngx.d/<domain>/htpasswd`, readable only by root and
nginx's worker group (`0640`). Changes are recorded in the history like any other.

```
sudo ngx auth add 4 alice
echo "$PASSWORD" | sudo ngx auth add 4 deploy-bot --password-stdin
```

---

## ngx check

```
ngx check [--json] [-q] [--max-5xx <PERCENT>] [--notify <URL>]... [--notify-json <URL>]... [--mail <ADDRESS>]... [--always]
ngx check --install-timer [--every <INTERVAL>] <notification options>
ngx check --remove-timer
```

Checks every enabled site in one go, for cron jobs and monitoring. Let's Encrypt no longer sends expiry emails, so
this is what tells you a certificate isn't renewing.

```
$ ngx check
WARN #2 api.example.com: certificate expires in 9 days (10/11/26) · renewal is manual (autorenew disabled)
CRIT #4 staging.example.com: down: 502 Bad Gateway: upstream 127.0.0.1:4000 refused connection
CRIT #4 staging.example.com: certificate expired 03/10/26
```

| Level | When |
|---|---|
| `CRIT` | a site is down (including a failing [health check](#the-questions)), a certificate expired or its file is missing, `nginx -t` fails |
| `WARN` | a certificate is expiring (renewal late or manual), doesn't cover all the site's names, or renews without reloading nginx; more than `--max-5xx` percent (default 5) of a site's requests failed with 5xx in the last 24 hours (sites with their own access log and at least 20 requests) |

Exit status: `0` all good, `1` warnings, `2` critical, `3` ngx itself failed. `--json` prints
`{status, sites, problems: [{level, site, domain, message}]}`; `-q` prints nothing.

### Notifications

- `--notify <URL>`: POSTs the report as plain text, with `Title`, `Priority` (`high` when something is critical) and
  `Tags` headers: [ntfy](https://ntfy.sh) (`https://ntfy.sh/<your-secret-topic>`, or your own server) shows it as a
  push notification; other webhooks get the text as the body.
- `--notify-json <URL>`: POSTs `{"text", "content", "status", "host", "problems"}`. `text` is what Slack and
  Mattermost incoming webhooks show, `content` is Discord's.
- `--mail <ADDRESS>`: sends the report through the local `sendmail` (any MTA: postfix, msmtp-mta…).

A notification is sent when the set of problems **changes**, and once more when everything is resolved ("all clear"),
not on every run; `--always` sends one on every run with problems. If sending fails, the next run tries again.
Notifications are sent with `curl` (installed almost everywhere), and URLs are checked so they can't inject anything.

### Running it on a schedule

```
sudo ngx check --install-timer --every 1h --notify https://ntfy.sh/my-secret-topic --mail you@example.com
sudo ngx check --remove-timer
```

installs a systemd timer (`ngx-check.timer`, logs in `journalctl -u ngx-check`) running `ngx check -q` with the same
options, or `/etc/cron.d/ngx-check` without systemd. `--every` takes `15m`, `30m`, `1h`, `6h`, `1d`… (at least 5
minutes). The files are root-only (`0600`), since notification URLs are often secrets. ngx refuses to install the
timer if its own binary isn't owned by root and protected from other users (a root timer must not run a file
someone else can replace); install it system-wide first. The change is in `ngx history`.

It works the same with any monitoring that runs a command and reads the exit status (Nagios, Icinga, Zabbix…).

---

## ngx history

```
ngx history [<site>] [-n <N>] [--json]
```

The changes ngx made, newest first: ID, time, user (the one who ran `sudo`), the command, and the files it touched
(`+` created, `~` changed, `→` link). With a site, only changes touching it. The last 50 changes are kept.

```
ID                 TIME               USER   COMMAND                        FILES (+ created, ~ changed, → link)
20261101-093422-2  01/11/26 09:34:22  alice  ngx edit 3 -y --local 4000     ~sites-available/app.conf
20261101-093422    01/11/26 09:34:22  alice  ngx add -y -d app… -l 3000     +sites-available/app.conf  +sites-enabled/app.conf
```

All the steps of one command (for example the HTTP bootstrap and the final HTTPS config of `ngx add`) are one entry.

---

## ngx undo

```
ngx undo [<ID>] [-y] [--dry-run]
```

Puts every file a change touched back as it was before it: changed files are restored (with a diff shown first),
created files are removed, links are restored. Without an ID, it undoes the latest change. If later changes touched
the same files, ngx lists them and warns that they'll be lost.

The undo itself goes through `nginx -t` with rollback, and is recorded, so it can be undone too. Site IDs follow: undoing
a rename gives the site its ID back, undoing a removal brings back the removed site's ID (IDs are never reused, so
it's always free).

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
- every certbot certificate reloads nginx when renewed (otherwise: `sudo ngx cert auto <site> on`)
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

---

## ngx completions

```
ngx completions bash|zsh|fish|elvish|powershell
```

Prints a completion script for subcommands and flags. The release packages, the AUR package and `install.sh`
install the bash, zsh and fish ones; by hand:

```
ngx completions bash | sudo tee /usr/share/bash-completion/completions/ngx
ngx completions zsh  | sudo tee /usr/share/zsh/site-functions/_ngx
ngx completions fish | sudo tee /usr/share/fish/vendor_completions.d/ngx.fish
```
