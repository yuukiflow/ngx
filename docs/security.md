# Security

ngx runs as root and writes files nginx loads, so it is designed to be **no less safe than writing the configs
yourself**. That means it never accepts input that could add directives or reach outside the config directory, it
never runs code that isn't yours, and it never leaves a configuration in place that nginx hasn't accepted.

## Input never becomes config syntax

Every value that ends up in a generated config, a file name, or a certbot command line is validated first
(`src/validate.rs`), whether it came from a prompt, a flag, or the spec embedded in a managed file:

| Value | Allowed |
|---|---|
| Domain, aliases | DNS names: `a-z 0-9 -` labels, no leading/trailing `-`; `*` only as the whole first label |
| Upstream host | an IP address or a plain host name |
| Document root, php-fpm socket, certificate, key, log paths | absolute paths of `A-Z a-z 0-9 / . _ - + @ = , ~ :`, with no `.` or `..` components |
| Ports | 1–65535 |
| Email | `user@domain`, not starting with `-` |
| Upload size, rate limit | `50m`, `512k` / `10r/s`, `300r/m` |
| Extra route paths | `/api/`: letters, digits and `/ . _ -`, never `/.well-known/` |
| DNS plugin | lowercase name (`cloudflare`), not starting with `-` |
| Basic-auth user names | letters, digits and `. _ - @` (never `:`) |

So a value can't contain whitespace, quotes, `;`, `{`, `}`, `#`, `$` or newlines, and can't inject directives
(`example.com; include /etc/shadow`). A domain can't escape `sites-available/` (`../../etc/cron.d/x`). Nothing can
be passed to certbot as an option (`-x@example.com`). All external commands are run with an argument list, never
through a shell.

Some document roots are refused because publishing them would expose the system: `/`, `/etc`, `/root`, `/usr`,
`/var/lib`, `/var/log`, `/proc`, `/sys`, `/boot`, `/run`, the nginx, Let's Encrypt and ngx directories, whole home
directories (`/home/alice`), and top-level directories like `/home`, `/var` and `/srv`.

Two smaller checks. Let's Encrypt is refused for wildcard names, which need a DNS challenge. A domain that another
enabled site already serves is refused too, so a new site can't take over an existing one's traffic.

## Generated configs follow common hardening practice

- **Hidden files are never served** on static and PHP sites (`.git/`, `.env`, `.htpasswd`…) and return 403;
  `/.well-known/` stays reachable.
- **PHP only runs scripts that exist** (`try_files $fastcgi_script_name =404`), so a request like
  `/uploads/avatar.jpg/x.php` can't make php-fpm execute an uploaded file. The hidden-file rule comes before the PHP
  location, so `/.secret.php` isn't executed either.
- **TLS 1.2 and 1.3 only.** HTTP redirects to HTTPS by default.
- **The ACME challenge directory** (`/var/lib/ngx/acme`) holds nothing but challenge tokens, is served only on port 80
  and only under `/.well-known/acme-challenge/`.
- Proxied apps get the standard `Host`, `X-Real-IP`, `X-Forwarded-For` and `X-Forwarded-Proto` headers.

- **Password protection** stores SHA-512 crypt hashes (never the password), in a file readable only by root and
  nginx's worker group (`0640`); passwords need at least 8 characters. The ACME location is exempt, so renewals
  still work.
- **Extra routes** use `^~` locations, so a regex location (such as PHP's) can't take over their paths.

With `--ssl existing` or a DNS-challenge credentials file, ngx warns when the secret file is readable by other users
or not owned by root.

### Snippets

Files in `ngx.d/<domain>/*.conf` are your own nginx directives, so ngx doesn't restrict what they contain. It's the
same trust as writing nginx config by hand, and they're only writable by root. `ngx edit --snippet` still runs them
through `nginx -t` with rollback, and records them in the history.

## Changes are validated or undone

Every change is a transaction (see [How it works](how-it-works.md#safe-changes)):

- **One at a time.** An exclusive lock (`/var/lib/ngx/lock`, opened without following symlinks) means two ngx
  processes never interleave changes, backups and rollbacks.
- **Backups first.** Files are copied to `/var/lib/ngx/backups/<time>/` before anything is touched.
- **Atomic writes.** Each file is written to an exclusively created temp file, then renamed into place. nginx never
  sees a half-written file, and a pre-planted file or symlink is never followed. Existing files keep their mode; new
  configs are `0644`.
- **Tested before it's live.** `nginx -t -c <the nginx.conf ngx manages>` runs before the reload. If it fails, or if
  nginx can't be found to run it, every file is restored and nothing changes.
- **Reported only once it's live.** ngx waits until nginx's new workers have taken over before saying a change is done.
- **Recorded and reversible.** Every command's changes are in `ngx history` with who ran them, and `ngx undo` puts the
  files back (through the same validated path).
- **Renewals reach nginx.** Certificates get a deploy hook that reloads nginx, so a renewed certificate is actually
  served; `ngx check` and `ngx doctor` flag any that don't.

## No foreign code runs as root

`ngx show` and the dashboard ask the program behind a site for its version (`node --version`, `php-fpm -v`…).
That binary belongs to whoever runs the backend, often an unprivileged user, so as root ngx:

- runs it **as that process's own user and group** (supplementary groups dropped), so it gains nothing it didn't
  already have,
- runs it as root only when the backend itself runs as root *and* its binary is owned by root and not writable by
  anyone else,
- checks and executes the same open file, so a process that exits and has its PID reused in between can't swap in
  another binary,
- gives it a new session with no terminal, no stdin, a clean environment, `/` as working directory, and a
  2-second limit.

`pm2`, `systemctl`, `crontab` and `certbot` come from root's own `PATH`, as they would if you ran them yourself.

## Output can't hijack your terminal

Log lines, request URLs, user agents, process output and config values are shown in `ngx show`, the dashboard and
error messages. Control characters and Unicode direction overrides in them are displayed escaped (`\x{1b}`), so
a crafted request can't send escape sequences to your terminal when you look at the logs. (`ngx logs` is a plain
`tail`, the same as running it yourself.)

## Files ngx creates

| Path | Mode | Contents |
|---|---|---|
| `sites-available/*.conf`, `00-ngx-logformat.conf` | `0644` (or the file's existing mode) | site configs |
| `/var/lib/ngx/backups/` | `0700`, files `0600` | copies of replaced files, `manifest.json` (history) |
| `/var/lib/ngx/lock` | `0600` | change lock |
| `ngx.d/<domain>/htpasswd` | `0640`, group = nginx workers | password hashes |
| `/var/lib/ngx/state.json` | `0644` | site IDs |
| `/var/lib/ngx/acme/` | `0755` | ACME challenge tokens (must be readable by nginx) |
| document roots it creates | `0755` | an `index.html` placeholder |

ngx sets its umask to `022`, so none of these are ever group- or world-writable, whatever root's shell umask is.
`edit` and `edit --raw` work on copies in a private, randomly named temp directory (`0700`), never at a predictable
`/tmp` path.

## Environment variables and sudo

`NGX_ROOT`, `NGX_NGINX_DIR`, `NGX_NGINX_BIN`, `NGX_STATE_DIR`, `NGX_LETSENCRYPT_DIR` and `EDITOR` change what ngx
reads, writes and runs. `sudo` drops them by default (`env_reset`), so `sudo ngx …` uses the real system paths. Only
pass them through deliberately (`sudo NGX_NGINX_DIR=… ngx …`). `NGX_ROOT=/` is ignored, so sandbox mode (which skips
`nginx -t`) can't be pointed at the live system.

## Reporting a problem

Please open an issue at <https://github.com/yuukiflow/ngx/issues>. For anything exploitable, ask for a private
contact first rather than posting the details.
