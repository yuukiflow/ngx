# Troubleshooting

Start with `ngx doctor`. It shows what's missing in one go.

## `command not found: ngx`

The binary isn't on your PATH. After `cargo install --path .` it's in `~/.cargo/bin`. With `sudo`, see
[Using it with sudo](install.md#using-it-with-sudo).

## "this command modifies nginx configuration and must run as root"

Use `sudo ngx …`. Only `ls`, `show`, `logs`, `test`, `doctor`, `add --dry-run` and the dashboard work without root.

## Running without root

Read-only commands work, but:

- certificates in `/etc/letsencrypt/live` are root-only, so `SSL` shows `unreadable`,
- processes of other users can't be inspected, so `BACKEND` shows `running · details need root`,
- `nginx -t` isn't run (it needs root to open certificates and the pid file), so only syntax errors and duplicate
  names are detected,
- new site IDs aren't saved (they'll be the same on the next root run, since they're assigned in the same order).

## A site is `down` with "502 Bad Gateway: upstream … refused connection"

Nothing is listening on the upstream address. `ngx show <site>` → `BACKEND` tells you what should be running and its
last exit, when it can find out (pm2 app, systemd unit). Typical fixes: `pm2 restart <app>`,
`systemctl restart <unit>`, `journalctl -u <unit> -n 50`.

If the app listens on a different port, move the site there: `sudo ngx edit <site> -y --local <port>`.

`timed out` instead of `refused` usually means a firewall, or a host that isn't reachable from this server.

## "config error: duplicate server_name …"

Two enabled sites claim the same name on the same port. nginx uses the first one it loads (alphabetically by file)
and ignores the other, which is why the later one shows as down. Remove the name from one of them
(`ngx edit <site>` → Aliases) or disable the old site (`ngx disable <site>`).

## "nginx -t failed, changes rolled back"

Your change was rejected and **nothing changed**: every file was restored. The message shows the nginx error with
file and line. Common causes:

- `cannot load certificate`: the certificate path doesn't exist (yet). For Let's Encrypt, let ngx issue it (`ngx cert issue`).
- `unknown log format "ngx"`: `sites-enabled/00-ngx-logformat.conf` was deleted. Restore it from the
  [backups](how-it-works.md#backups), or recreate it (and link it from `sites-enabled/` if it lives in `sites-available/`):

  ```nginx
  log_format ngx '$remote_addr - $remote_user [$time_local] "$request" $status $body_bytes_sent "$http_referer" "$http_user_agent" rt=$request_time';
  ```
- `unknown directive`: a typo in a `--raw` edit.

## "cannot run nginx -t … nothing was changed"

ngx couldn't find the nginx binary, so it refused to leave an untested config in place. Install nginx, or point ngx
at it: `sudo NGX_NGINX_BIN=/opt/nginx/sbin/nginx ngx …`.

## "invalid domain name", "invalid document root", "refusing to publish … as a document root"

Values that go into the config are restricted so they can't change its meaning (see [Security](security.md)). Paths
must be absolute and can't contain spaces, quotes or `; { } $ #`; move the files to a plain path or use a symlink.
System directories and whole home directories can't be document roots; use a dedicated directory such as
`/var/www/<site>`.

## "reload failed, changes rolled back"

`nginx -t` accepted the change, but nginx couldn't switch to it, most often because the new public port is already
used by another program (`ss -ltnp | grep :<port>`), or a log file can't be opened. Everything was put back and nginx
reloaded with the previous configuration. Fix the cause and run the command again.

## A site is down with "health check /health returned …"

The app answers, but its health endpoint says it's not well (`503`), or doesn't answer in 3 seconds. Look at the app's
own logs. To stop checking: `sudo ngx edit <site> -y --health off`.

## "nginx did not pick up the new configuration within 5s"

nginx accepted the config (`nginx -t` passed) but the reload didn't complete. Look at the end of
`/var/log/nginx/error.log` (often a port already used by another program, or a permission problem on a log file) and
at `journalctl -u nginx`. Then run `sudo ngx reload` again.

## "nginx is not running"

All sites show down. Start it: `sudo systemctl start nginx` (and `enable` it to start at boot). If it doesn't start,
`sudo ngx test` shows why.

## "waiting for another ngx command to finish…"

Another ngx process is making a change (often the dashboard, in another terminal, waiting at a question). ngx
continues when it's done. To see which one: `sudo fuser -v /var/lib/ngx/lock`.

## `RENEW` says `auto ⚠`

The certificate renews, but nothing reloads nginx afterwards, so nginx would keep serving the old certificate. Fix:
`sudo ngx cert auto <site> on` (adds a deploy hook to the renewal config). `ngx doctor` lists every certificate in
that state.

## Uploads fail with 413, or clients get 429

- `413 Request Entity Too Large`: the body is larger than the site's upload limit (nginx's default is 1m). Raise it:
  `sudo ngx edit <site> -y --max-body 100m`.
- `429 Too Many Requests`: the site has a rate limit and the client went over it plus the burst allowance. Raise or
  remove it: `--rate-limit 50r/s` or `--rate-limit off`.

## A snippet is rejected

`ngx edit --snippet` runs `nginx -t` and rolls back on errors. The usual ones:

- `duplicate location "/"`: the generated config already has `location /`; use a more specific path, or change the
  site's settings with `ngx edit`.
- `"…" directive is not allowed here`: snippets are inside the `server { }` block; `http`-level directives (like
  `limit_req_zone` or `upstream`) belong in a file in `conf.d/`.

## DNS challenge: "The requested dns-… plugin does not appear to be installed"

Install the plugin for your provider (`apt install python3-certbot-dns-cloudflare`, `pacman -S certbot-dns-cloudflare`,
or `pip install certbot-dns-cloudflare` in certbot's environment) and check `certbot plugins` lists it. Then retry;
with the DNS challenge, nothing was changed by the failed attempt.

## Let's Encrypt fails

ngx keeps the site on HTTP and prints certbot's error. Check, in this order:

1. **DNS**: `dig +short <domain>` (and `AAAA`) must return this server's public IP. A wrong `AAAA` record is a
   frequent cause: Let's Encrypt prefers IPv6.
2. **Port 80**: from another machine, `curl -I http://<domain>/.well-known/acme-challenge/test` should reach nginx
   (a 404 is fine; a timeout isn't). Open port 80 in the firewall and any cloud security group.
3. **Proxies/CDNs**: with Cloudflare's proxy on, port 80 must still be forwarded; or turn the proxy off while issuing.
4. **Rate limits**: after several failures, wait an hour, or test with `--staging`.

Then retry: `sudo ngx cert issue <site>`. Details: `/var/log/letsencrypt/letsencrypt.log`.

## `RENEW` says manual

`ngx show <site>` gives the reason:

| Reason | Fix |
|---|---|
| `autorenew disabled` | `sudo ngx cert auto <site> on` |
| `no certbot timer or cron job` | see below |
| `no certbot renewal config` | the certificate was copied in by hand; reissue with `sudo ngx cert issue <site>` |
| `manual authenticator (DNS challenge)` | it was issued with `certbot --manual`; reissue with `ngx cert issue`, or set up a DNS plugin |
| `not issued by certbot` | your own certificate; replace it before expiry |

### No renewal scheduler

On most distributions: `sudo systemctl enable --now certbot.timer` (snap installs use `snap.certbot.renew.timer`).

Arch ships no timer. Create one:

```
sudo tee /etc/systemd/system/certbot-renew.service <<'EOF'
[Unit]
Description=Renew Let's Encrypt certificates
[Service]
Type=oneshot
ExecStart=/usr/bin/certbot renew --quiet --deploy-hook "systemctl reload nginx"
EOF
sudo tee /etc/systemd/system/certbot-renew.timer <<'EOF'
[Unit]
Description=Renew Let's Encrypt certificates twice a day
[Timer]
OnCalendar=*-*-* 00,12:00:00
RandomizedDelaySec=1h
Persistent=true
[Install]
WantedBy=timers.target
EOF
sudo systemctl enable --now certbot-renew.timer
```

## A certificate shows ⚠ although renewal is auto

certbot renews 30 days before expiry, so a certificate with fewer than 30 days left means renewals are failing.
Run `sudo certbot renew --dry-run` to see the error (usually DNS or port 80; see above).

## `ngx edit` refuses a site

- *"also defines …"*: the file contains several sites. Split it with `sudo ngx split <site>`, or use `ngx edit <site> --raw`.
- *"not a setup ngx can generate"*: a variable upstream, redirect-only site, or something other than a plain
  proxy/static/PHP site. Use `--raw`.

## TRAFFIC shows no latency, or no TRAFFIC section

Latency needs `$request_time` in the access log format, which sites created by ngx have. Hand-written sites can use
it with `access_log /var/log/nginx/<site>.access.log ngx;`. The section only appears for sites with their own
`access_log`, because the shared log doesn't record which site a request was for.

## IDs changed

IDs are tied to `<file name>#<domain>`. Renaming a file or changing the first `server_name` by hand (outside
`ngx edit`) looks like a new site and gets a new ID. The old ID is never reused.

## Undo a change

`ngx history` lists the changes ngx made, and `sudo ngx undo` reverts the latest one (or `sudo ngx undo <ID>` an
earlier one). In the dashboard, press `u`. See [Backups](how-it-works.md#backups).
