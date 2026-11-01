# Dashboard

Run `ngx` with no command (or `ngx tui`). Use `sudo ngx` if you want to change things from it.

```
 ngx  7 domains · 4 up · 2 down · 1 disabled · 1 cert expired · 1 expiring soon (manual renewal)
┌ sites ───────────────────────────────────────────────────────────────────────────────────────┐
│  ID  STATUS  DOMAIN               SSL                     RENEW   IN   LOCAL       CONFIG    │
│  1   ● up    example.com          valid until 14/01/27    auto    443  3000        sites-ena…│
│▶ 4   ○ down  staging.example.com  expired 02/10/26        manual  443  4000        sites-ena…│
└──────────────────────────────────────────────────────────────────────────────────────────────┘
┌ details ─────────────────────────────────────────────────────────────────────────────────────┐
│staging.example.com                                                 ○ DOWN                    │
│ID              4                                                                             │
│…                                                                                             │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
 ↑↓ select  ⏎ zoom  / filter  a add  e edit  d on/off  x remove  i issue cert  r renew  l/L logs  u undo  …
```

- **Top bar**: the same summary as `ngx ls`, plus the first nginx error that couldn't be linked to a site, and
  `[sandbox]` when `NGX_ROOT` is set.
- **Sites**: the `ngx ls` table, disabled sites included (dimmed).
- **Details**: the `ngx show` view of the selected site. It loads in the background (`probing…`), because it inspects
  processes and reads logs.
- **Footer**: keys, and the result of the last action (`done`, `cancelled`, or the error).

The list refreshes every 5 seconds, and right after each action.

## Keys

| Key | Action |
|---|---|
| `↑` `↓` / `k` `j` | Select site |
| `Home` / `End` | First / last site |
| `PgUp` `PgDn` / `K` `J` | Scroll the detail pane |
| `Enter` / `Space` | Zoom: details full screen (again, or `Esc`, to go back) |
| `/` | Filter by domain (or type an ID). `Enter` keeps the filter, `Esc` clears it |
| `g` | Refresh now |
| `a` | Add a site: [the site form](#the-site-form) |
| `e` | Edit the selected site: the form, pre-filled with its settings |
| `d` | Disable / enable the selected site |
| `x` / `Delete` | Remove the selected site (asks first) |
| `i` | Issue a Let's Encrypt certificate (`ngx cert issue`) |
| `r` | Renew the certificate (`ngx cert renew`) |
| `l` / `L` | Open the [log viewer](#log-viewer) on the access / error log |
| `u` | Undo the latest change (`ngx undo`, shows what will be restored first) |
| `t` | `nginx -t` |
| `R` | Test and reload nginx |
| `q` / `Esc` / `Ctrl-C` | Quit |

## The site form

`a` and `e` open a form with every setting of a site on one screen: domain, aliases, type, local port or document
root, SSL mode (including the DNS challenge), public port, HTTP redirect, and the options (upload size, HSTS,
password, rate limit, alias redirect, security headers, health check, extra routes). Fields that don't apply are hidden (no php-fpm socket for a proxy,
no HSTS without SSL).

| Key | |
|---|---|
| `↑` `↓` / `Tab` | Move between fields (`Enter` too) |
| typing, `Backspace`, `Ctrl-U` | Edit a text field (`Ctrl-U` clears it) |
| `←` `→` / `Space` | Change a choice or toggle |
| `Ctrl-S` / `F2` | Review and save (also `Enter` on the last field) |
| `Esc` | Cancel |

Values are checked as you type, with the same rules as the command line; an invalid field is marked `✗` with the
reason, and saving jumps to it. Saving leaves the dashboard to show the generated config (or the diff for an edit)
and asks for confirmation, then applies it like `ngx add` / `ngx edit`, Let's Encrypt included. If you answer no,
press `Esc` at the confirmation, or it fails, you're back in the form with everything you typed.

## Log viewer

`l` (access log) or `L` (error log) shows the selected site's log inside the dashboard and follows new lines as they
arrive. Status codes are colored in access logs (5xx red, 4xx yellow), levels in error logs. When the site has no log
of its own, the shared one is shown (and, for the error log, only lines mentioning the site's names).

| Key | |
|---|---|
| `↑` `↓` / `PgUp` `PgDn` | Scroll (the view stays still while new lines arrive) |
| `End` / `f` | Back to following |
| `Home` | Oldest loaded line (the last ~2000 are kept) |
| `/` | Search: type a text (case-insensitive), `Enter` keeps it, `Esc` clears it |
| `p` | Problems only: 4xx/5xx in access logs, warnings and errors in error logs |
| `e` / `Tab` | Switch between access and error log |
| `Esc` / `q` | Close (`Esc` first clears a search or filter) |

## How actions work

Other actions leave the dashboard and run the normal command in your terminal, so you get the same questions,
previews and diffs as on the command line. When the command finishes, press `Enter` to return. Cancelling a question
(`Esc` or `Ctrl-C` inside a prompt) returns straight away.

`Ctrl-C` in an editor or during certbot stops only that command, not the dashboard. Each action is recorded in the
[change history](commands.md#ngx-history) as `dashboard: <action>`.

Actions that change things need root. Without it, they fail with a message, and you're back in the dashboard after `Enter`.
