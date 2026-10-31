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

## "nginx did not pick up the new configuration within 5s"

nginx accepted the config (`nginx -t` passed) but the reload didn't complete. Look at the end of
`/var/log/nginx/error.log` (often a port already used by another program, or a permission problem on a log file) and
at `journalctl -u nginx`. Then run `sudo ngx reload` again.

## "nginx is not running"

All sites show down. Start it: `sudo systemctl start nginx` (and `enable` it to start at boot). If it doesn't start,
`sudo ngx test` shows why.

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

- *"also defines …"*: the file contains several sites. Split them into separate files, or use `ngx edit <site> --raw`.
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

Every change made by ngx is backed up; see [Backups](how-it-works.md#backups).
