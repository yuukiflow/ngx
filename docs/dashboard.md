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
 ↑↓ select  ⏎ zoom  / filter  a add  e edit  d on/off  x remove  i issue cert  r renew  …
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
| `a` | Add a site (`ngx add`) |
| `e` | Edit the selected site (`ngx edit`, pre-filled questions) |
| `d` | Disable / enable the selected site |
| `x` / `Delete` | Remove the selected site (asks first) |
| `i` | Issue a Let's Encrypt certificate (`ngx cert issue`) |
| `r` | Renew the certificate (`ngx cert renew`) |
| `l` / `L` | Follow the access / error log. `Ctrl-C` returns to the dashboard |
| `t` | `nginx -t` |
| `R` | Test and reload nginx |
| `q` / `Esc` / `Ctrl-C` | Quit |

## How actions work

Actions leave the dashboard and run the normal command in your terminal, so you get the same questions, previews
and diffs as on the command line. When the command finishes, press `Enter` to return. Cancelling a question
(`Esc` or `Ctrl-C` inside a prompt) returns straight away.

`Ctrl-C` while following logs, in an editor, or during certbot stops only that command, not the dashboard.

Actions that change things need root. Without it, they fail with a message, and you're back in the dashboard after `Enter`.
