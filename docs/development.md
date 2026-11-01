# Development

```
cargo build
cargo test            # unit tests: parser, site grouping, templates, validation/injection, log/nginx-output parsing
cargo run -- ls
```

## Sandbox mode

`NGX_ROOT=<dir>` puts every path ngx uses under `<dir>` (`/etc/nginx` → `<dir>/etc/nginx`, …). In sandbox mode ngx
**does not** run `nginx -t`, reload nginx or call certbot (it prints what it would have run), and doesn't inspect
processes. Upstreams listed one per line in `<dir>/probe/down` are reported as refusing connections; all others
are treated as up.

`demo/make_demo.py` builds a sandbox resembling a small production server: seven sites (proxy, PHP, static, one
disabled), certificates that are valid, expiring in 9 days, or expired 29 days ago, a duplicate `server_name`, a
backend that's down, error and access logs, and stable IDs.

```
python3 demo/make_demo.py /tmp/ngx-demo
NGX_ROOT=/tmp/ngx-demo cargo run -- ls
NGX_ROOT=/tmp/ngx-demo cargo run -- show 4
NGX_ROOT=/tmp/ngx-demo cargo run            # dashboard
NGX_ROOT=/tmp/ngx-demo cargo run -- add     # writes into the sandbox
```

The certificates are generated relative to the current date, so rerun the script for fresh data.

## End-to-end test with real nginx

`tests/e2e/arch.sh` runs in a throwaway Arch Linux container with real nginx and Node.js. It covers `init` on a
layout without `sites-enabled`, `add` for proxy and static sites (checked with `curl`), hidden files returning 403, rejected injection attempts, `show`, prompt-free `edit`
(port change and rename), rollback of a broken `--raw` edit, a backend going down, error mapping from `nginx -t`,
and `disable`.

```
cargo build --release
docker run --rm -v $PWD/target/release/ngx:/usr/local/bin/ngx:ro \
  -v $PWD/tests/e2e/arch.sh:/e2e.sh:ro archlinux:latest bash /e2e.sh
```

(The binary is built on the host, so the container needs a glibc at least as new as the host's; Arch's latest image does.)

## Environment variables

| Variable | Default | Purpose |
|---|---|---|
| `NGX_ROOT` | — | Sandbox prefix (see above) |
| `NGX_NGINX_DIR` | auto-detected | nginx configuration directory |
| `NGX_NGINX_BIN` | `nginx` on PATH or in `/usr/sbin` | nginx binary |
| `NGX_LETSENCRYPT_DIR` | `/etc/letsencrypt` | certbot directory |
| `NGX_STATE_DIR` | `/var/lib/ngx` | IDs, backups, ACME webroot |
| `NO_COLOR` | — | Disable colors (also off when output isn't a terminal) |
| `VISUAL` / `EDITOR` | `nano`, then `vi` | Editor for `ngx edit --raw` |

## Code map

| File | Responsibility |
|---|---|
| `src/main.rs` | CLI definitions (clap), `ls`, `show`, man page generation |
| `src/cmd.rs` | `add`, `edit`, `rm`, `enable`/`disable`, `cert`, `logs`, `test`, `reload`, `doctor`, `init` |
| `src/tui.rs` | Dashboard (ratatui) |
| `src/env.rs` | Paths, sandbox mapping, root check, helpers to run commands |
| `src/nginx/parser.rs` | Config lexer and parser → directive tree with line numbers |
| `src/nginx/site.rs` | Grouping server blocks into sites, loading the config directories, lookup by ID or domain |
| `src/nginx/template.rs` | Generated configs and the embedded spec |
| `src/nginx/ctl.rs` | `nginx -t` output parsing, reload and the wait for new workers |
| `src/status.rs` | Status rules, run in parallel per site |
| `src/ssl.rs` | Certificate parsing (x509-parser), certbot renewal configs, timers, certbot commands |
| `src/probe.rs` | Upstream connection checks, process / runtime / manager detection |
| `src/logs.rs` | Error log and access log parsing |
| `src/ops.rs` | Change transaction: backup, write, test, roll back, reload |
| `src/state.rs` | Stable site IDs |
| `src/validate.rs` | Validation of every value that goes into a config or command line; terminal-safe output |
| `src/render.rs` | Table and detail view, shared by the CLI (ANSI colors) and the dashboard |

## Regenerating the man pages

The pages in `man/` come from the clap definitions; regenerate them after changing commands or help text:

```
cargo run -- man man
```
