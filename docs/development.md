# Development

```
cargo build
cargo test            # unit tests + CLI snapshot tests (tests/cli.rs)
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

## Tests

| What | Where | Run |
|---|---|---|
| Unit tests: parser, site grouping, templates, validation and injection attempts, history ordering, renewal configs, log and `nginx -t` parsing, password hashing | `src/**` | `cargo test` |
| Snapshot tests of `ls`, `show`, `check` and a generated config, against a fixed sandbox; `add` → `history` → `undo`; rejected injections | `tests/cli.rs`, `tests/snapshots/` | `cargo test --test cli` |
| Dashboard: the form (typing, validation, visible fields, what a submit produces) and the log viewer (following, search, problems filter, rotation, escape sequences), rendered with ratatui's `TestBackend` | `src/tui/form.rs`, `src/tui/logview.rs` | `cargo test tui` |
| End-to-end with a real nginx on Arch, Debian and Ubuntu | `tests/e2e/` | see below |
| Fuzzing | `fuzz/` | see below |

After an intended output change, accept the new snapshots with `UPDATE_SNAPSHOTS=1 cargo test --test cli` and check
the diff in `tests/snapshots/`.

### End-to-end test with real nginx

`tests/e2e/run.sh` runs in a throwaway container: it installs nginx and Node.js, then checks with `curl` (every
check prints `PASS`/`FAIL`, and the script fails if any did): `init`, `add` for proxy, static and PHP sites, hidden
files returning 403, rejected injection attempts, upload limits (413), rate limiting (429), extra routes, the alias
redirect, password protection (401/200 with a SHA-512 crypt hash, file mode), snippets (live, kept across
regeneration, broken ones rejected), HTTPS with an existing certificate and HSTS, renaming, rollback of a broken
`--raw` edit, `history`/`undo`, `check` exit codes, `nginx -t` error mapping, the change lock, `disable`, HTTP/2 and
security headers, rollback of a reload that fails (busy port), IDs after undoing a rename, health checks,
converting a hand-written site (options, routes, snippet), `split` and `regen`, notifications (plain and JSON, no
repeats), the check timer, and completions.

```
cargo build --release --target x86_64-unknown-linux-musl     # static, runs in any image
tests/e2e/docker.sh archlinux:latest
tests/e2e/docker.sh debian:stable
tests/e2e/docker.sh ubuntu:24.04
```

### Fuzzing

Three [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets (nightly Rust):

- `spec`: any spec that passes validation renders to a config that parses, has only the expected top-level
  directives and server blocks (nothing injected), and embeds the same spec back.
- `parser`: the nginx parser, site extraction and `nginx -t` output parsing never panic.
- `logs`: log line parsing never panics, and terminal sanitizing never lets a control character through.

```
cargo install cargo-fuzz
cd fuzz && mkdir -p corpus/spec && cp seeds/spec/* corpus/spec/
cargo +nightly fuzz run spec corpus/spec -- -max_total_time=300
```

### CI and releases

`.github/workflows/ci.yml` runs clippy (warnings are errors), the tests, a check that `man/` is up to date, the
end-to-end test on the three distributions, and each fuzzer for two minutes, on every push and pull request and
weekly. Pushing a tag `v*` runs `.github/workflows/release.yml`: static `x86_64` and `aarch64` musl binaries, packaged
with the man pages and completions, plus `.deb` packages (cargo-deb, metadata in `Cargo.toml`), published as a GitHub
release with `SHA256SUMS` (used by `scripts/install.sh`). The AUR `PKGBUILD` is in `packaging/aur/` (see its README).

## Regenerating the man pages and completions

`cargo run -- man man` writes `man/*.1` and `completions/` (bash, zsh, fish) from the clap definitions; CI fails if
they're out of date.

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
| `src/main.rs` | CLI definitions (clap), man page generation; everything else is the `ngx` library (`src/lib.rs`) |
| `src/cmd/` | One file per command group: `add` (and the questions shared with edit), `edit` (+ `--raw`, `--snippet`), `convert` (hand-written → managed), `regen`, `split`, `manage` (enable/disable/rm), `cert`, `auth`, `check` (+ notifications, timer), `history` (+ undo), `view` (ls/show), `misc` (logs/test/reload/doctor), `layout` (init) |
| `src/tui/` | Dashboard (ratatui): `mod.rs` (list, details, actions), `form.rs` (add/edit form), `logview.rs` (log viewer) |
| `src/env.rs` | Paths, sandbox mapping, root check and change lock, helpers to run commands |
| `src/nginx/parser.rs` | Config lexer and parser → directive tree with line numbers |
| `src/nginx/site.rs` | Grouping server blocks into sites, loading the config directories, lookup by ID or domain |
| `src/nginx/template.rs` | Generated configs and the embedded spec |
| `src/nginx/ctl.rs` | `nginx -t` output parsing, reload and the wait for new workers |
| `src/status.rs` | Status rules, run in parallel per site |
| `src/ssl.rs` | Certificate parsing (x509-parser), certbot renewal configs, timers, certbot commands |
| `src/probe.rs` | Upstream connection checks, process / runtime / manager detection |
| `src/logs.rs` | Error log and access log parsing |
| `src/ops.rs` | Change transaction (backup, write, test, roll back, reload), change history and undo |
| `src/state.rs` | Stable site IDs |
| `src/validate.rs` | Validation of every value that goes into a config or command line; terminal-safe output |
| `src/render.rs` | Table and detail view, shared by the CLI (ANSI colors) and the dashboard |

## Regenerating the man pages

The pages in `man/` come from the clap definitions; regenerate them after changing commands or help text:

```
cargo run -- man man
```
