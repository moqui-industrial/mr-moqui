# The Terminal UI

`moqui tui` shows the apps in a terminal. It draws the same XML Screens the browser does, through the `tui` render mode:
the server writes each screen as a JSON document of nodes, and the client in `crates/moqui-tui` draws it with
[Revue](https://github.com/hawk90/revue). There are no TUI-only screens. The design is in
[plans/Screen-TUI-Impl.md](../plans/Screen-TUI-Impl.md).

It is in every package, Default (`lite`) included (the `tui` feature, [Packaging](Packaging.md)). `apps` over SSH also
needs the `ssh` feature, which Default leaves out.

## Starting it

```
moqui tui                                   the apps on this runtime, after a login
moqui tui --user john.doe                   log in as john.doe (an instance_purpose of dev only)
moqui tui --path /apps/gallery/List         start on a screen
moqui tui --serve                           also open the HTTP, SSH and cluster ports, for the browser beside it
moqui tui --snapshot /apps/gallery/Detail --size 100x40     print one screen as text and exit
```

The client runs in this process on this runtime. It opens no port unless `--serve`, and it writes nothing to stderr: the
log goes to the log files and to the log pane (Alt+L). If a server already answers on the runtime's HTTP port, `moqui
tui` says so and stops; stop the server first.

Each screen is rendered as the logged-in user with artifact authorization on, as the browser's requests are. The client
holds no permission logic: what the user may not do is not in the document.

## Over SSH

When the built-in SSH server runs (`ssh_port`, see `crates/moqui-host/src/ssh/server.rs`), the same client runs on an
SSH channel, in the server's process:

```
ssh -p <ssh_port> <user>@<host>             with SSH_APPS and no SSH_SHELL, the apps are the whole session
ssh -t -p <ssh_port> <user>@<host> apps     the apps from any account with SSH_APPS
moqui> apps                                 from the admin shell; quitting the apps (Ctrl+Q) comes back to the prompt
```

- **Permission.** `SSH_APPS`, granted to `ADMIN` in the seed data. It opens the client and nothing more: every screen is
  still rendered as the user with artifact authorization on, as in the browser.
- **Terminal.** It needs a pty (`ssh -t`, which plain `ssh user@host` asks for). The client takes the pty's `TERM` and
  window size, and follows window changes. Colors follow `TERM` (`*-256color` is 256 colors). Send
  `COLORTERM=truecolor` for true color, or `NO_COLOR=1` for none, with `SendEnv COLORTERM NO_COLOR LANG` in
  `~/.ssh/config`. A locale that is set and is not UTF-8 draws in ASCII.
- **Limits.** At most `tui_max_sessions` (16) apps sessions run at once, and `tui_max_sessions_per_user` (4) of them for
  one user; `ssh_max_sessions` counts connections, not these. A session ends after 10 minutes without input.
- **Files.** Exports are not downloaded, because the client runs on the server; the text export still shows in a
  dialog. Uploads are off. The log pane is not offered.

## For agents

`agent-apps` is the same client for a program, an AI agent above all: commands in, the screen as text out. Each
command answers once the screen has settled (its requests done, a search's wait for typing fired at once), so there is
no timing to guess and no escape sequence to read. It needs `SSH_APPS` and no pty.

```
ssh -p 2222 john.doe@localhost agent-apps open [path] [--size 120x40] [--json]     a session; answers its id
ssh -p 2222 john.doe@localhost agent-apps <id> <command>                            one command in it
ssh -p 2222 john.doe@localhost agent-apps                                           line mode: a command per line on stdin
moqui tui --agent --user john.doe                                                   line mode on this runtime (dev)
```

The server keeps an opened session between commands, so each step is one `ssh`, and the agent holds no connection. A
session is found only by its own user. It ends on `close`, or after 10 minutes without a command, and counts in
`tui_max_sessions` and `tui_max_sessions_per_user` as `apps` does. The exit status is 0 for an answer that ends
`--- ok`, 1 for `--- error`, and 2 when there is no such session or none may start.

| Command | |
| --- | --- |
| `look` | the screen again |
| `key <chord>...` | keys, as `F10`, `Enter`, `Esc`, `Tab`, `Shift+Tab`, `Down`, `Ctrl+K`, `Alt+M`, `F4` |
| `type <text>` | characters as typed; `\r` is Enter, `\t` is Tab |
| `go <path[?query]>` | go to a screen |
| `focus <element>` | move the focus |
| `set <element> <value>` | replace a field's value; a drop-down takes a choice's value or label and chooses it as a person does, so its dependent fields follow |
| `do <element>` | focus and press Enter: a button, link, dialog button, menu; with a confirm open, `do Yes` answers it |
| `find <text>` | move to the text on the screen, a list row's included; then `key Right` follows the row's link |
| `elements` | the focusable elements: number, kind, id, label, value, and required, changed or error marks |
| `size <cols>x<rows>` | the drawing size; a tall one shows a long list whole (the page size follows the rows) |
| `format text\|json` | the answers' form; `--json` on one command for that one |
| `help`, `close` | the commands and the screen's keys; end the session |

An element is its id, its number from `elements` (`#3`), or its label: the same label first, then one that holds the
text, ignoring case. `set` takes the longest run of leading words that names an element, so `set Thru Date 2026-10-21
00:53` needs no quotes. An answer is:

```
path /apps/system/Security/UserAccount/UserAccountDetail?userId=EX_JOHN_DOE
title system ▸ Security ▸ UserAccount ▸ UserAccountDetail ▸ User Account Detail
focus dialog AddGroupMemberDialog "Add Group"
overlay none                                  (a dialog, a confirm with its choices, a drop-down, the palette, ...)
message Saved                                 (or: error ..., or no line)
<the screen as text; the focused element between [[ and ]]; long runs of empty rows as one counted line>
--- ok                                        (or: --- error <why>)
```

`--json` gives `{ok, reason, session, path, parameters, title, focus, overlay, message, messageError, screen, elements}`,
with every row of the screen.

A worked example, the Tools task, abridged (the whole of it took 20 commands on Oct 21, 2026):

```
agent-apps open                                      session hFY74uac ...
agent-apps hFY74uac go /apps/system/Security/UserAccount/UserAccountList
agent-apps hFY74uac key f                            the list's find dialog
agent-apps hFY74uac set Username john
agent-apps hFY74uac key Enter
agent-apps hFY74uac find EX_JOHN_DOE                 the row
agent-apps hFY74uac key Right                        its account
agent-apps hFY74uac do Add Group
agent-apps hFY74uac set Group My Account Users
agent-apps hFY74uac do Add                           message Saved
agent-apps hFY74uac go /apps/system/ServiceJob/Jobs/ServiceJobDetail?jobName=clean_ServiceJobRun_daily
agent-apps hFY74uac do Run Job                       overlay confirm "Run job now ...", choices No|Yes
agent-apps hFY74uac do Yes
agent-apps hFY74uac close
```

**Logging in.** Each step is its own `ssh`, so let it not ask for a password every time:

- Give the agent a key: a `UserSshKey` row with the key's fingerprint, for the agent's user.
- Or keep one connection open and run every step over it, with this in `~/.ssh/config` (the first `ssh` logs in, the
  rest reuse it):

  ```
  Host moqui-agent
      HostName localhost
      Port 2222
      User john.doe
      ControlMaster auto
      ControlPath /tmp/moqui-agent-%C
      ControlPersist 30m
  ```

  A control socket path must be short: a Unix socket path holds at most 108 bytes.
- Where a desktop is around, `ssh` may ask for a password in a window (`SSH_ASKPASS`). Set `SSH_ASKPASS_REQUIRE=never`
  so it asks on the terminal, or not at all.

## Keys

The scheme follows 5250 function keys and Lynx link travel. Single-letter keys act only when focus is not in a text
field; function keys and Alt combinations work everywhere. No default uses Ctrl+A, Ctrl+B, Ctrl+Z, Ctrl+\ or Ctrl+], so
tmux, screen, job control and ssh escapes are left alone. F1 lists the keys in use, a theme's included.

| Key | Action |
| --- | --- |
| Tab, Down / Shift+Tab, Up | Next / previous field, link, button or list |
| Enter | In a field: submit its form (a find form finds). On a link: follow it. On a button: press it. On a list: edit the row when it has fields, else open its first link. On a menu button: its items. On a box title: open or close it. In a text area: a new line |
| Right / Left | Not in a field: follow / back (on a list row, its first link). In a field: move the cursor. On a tree: open / close a node |
| Alt+Enter, Ctrl+S | Submit the focused form from anywhere, a text area included |
| Esc, F12 | Close the top dialog or list; else clear the message; else leave the edits, after asking |
| F1, `?` | The keys, the focused component's keys, and the screen's documents: a document's number opens it |
| F3, Alt+Left, Backspace (not in a field) | Back to the screen before |
| F4 | Prompt: a drop-down's list, a date's picker, a text find's operator |
| F5, Ctrl+R | Refresh the screen |
| F6, `+` | Quick create: the apps' create forms, each in a dialog |
| F7 / F8, PgUp / PgDn | Previous / next page of the focused list (else the first list); on a wizard with no list, PgDn is Next |
| Shift+Left / Shift+Right | Scroll a list wider than the screen; its first column stays |
| F9 | Show the drawer and focus it; again to hide it. The choice is kept for the user (`TUI_DRAWER_OPEN`) |
| F10 | Actions: the screen's button menus, then the focused list's row actions, Edit row, Find, Saved finds, Columns and exports |
| Ctrl+K, `:` | The palette: apps, recent screens, actions, and what the server's search finds as you type |
| `g` | Go to a path (`/apps/...`) or a menu title |
| `/`, `n`, `N` | Find text on the screen (links, fields, list rows), then the next and previous match |
| Space, `*` | On a list: select the row, select all. On a check box: toggle |
| `f`, `s`, `e` | On a list: the find dialog of its header fields, cycle the sort, edit the row |
| Alt+1…Alt+9, 1…9 | Subscreen tab N |
| `#` | Number the links; then digits and Enter follow link N |
| `=` | Info: the screen, its address, the focused link's target |
| `o` | Open the screen or the focused link in the browser (its address on the message line) |
| Alt+L | The log pane |
| Alt+M | The last 25 messages |
| Alt+T | The tray: Today, Chat and Notifications; Left and Right change the tab |
| Ctrl+L | Redraw |
| Ctrl+Q | Quit, after asking if a form changed |

In a drop-down list, typing narrows it, Up and Down move, Enter picks and Esc closes; a search drop-down (an
`ac-transition` text line, or `server-search`) asks the server for what is typed once the typing stops. In the date
picker the arrows move a day or a week, PgUp and PgDn a month, Home is today and Enter picks.

A list row with fields (a list with a `transition`) opens in the row editor with Enter or `e`: its fields under their
column titles; Enter or Ctrl+S saves the row, or every row of a `multi` list. A row action (`row-selection` `action`)
is on F10, and its form posts the rows selected with Space. In the Columns dialog Space shows or hides a field,
Alt+Up and Alt+Down move it, Enter saves and `r` puts back the list's own columns. An export saves a file to
`XDG_DOWNLOAD_DIR` (else `~/Downloads`), with its path on the message line; the text export shows in a dialog.

The title line shows the counters (`Today 3  Chat 1  Notif 2`, read again each minute) when the server has a count
service.

Leaving a changed form (a link, F3, Ctrl+Q) asks: Stay, Discard or Save. A button or link with a `confirmation` asks
No or Yes, starting at No.

## Widths

The client measures the canvas: the terminal less a docked drawer. The width classes keep the browser's names, so
`row-col` and `field-col` sizes apply as written.

| Class | Canvas columns | What changes |
| --- | --- | --- |
| xs | under 80 | Labels sit above values; dialogs are full screen; the mobile column set is asked for |
| sm | 80–119 | One column of boxes; a `field-row` keeps its cells side by side while each has its label and 24 columns |
| md | 120–179 | Two columns: `row-col md` and `lg` sizes apply; panels and splits show their sides |
| lg | 180–239 | As md, three columns where `row-col lg="4"` asks |
| xl | 240 and up | As lg |

The 26-column drawer docks while the canvas left beside it is still md, from a terminal of 146 columns; F9 shows or
hides it at any width. A box narrower than 40 columns goes on a line of its own rather than squeeze. A list's page size
follows the rows of the terminal (10 to 100).

## Themes

A screen theme (the user's `STT_INTERNAL_QUASAR` theme, else `DEFAULT_QUASAR`) can set the client's values through
`ScreenThemeResource` rows, each `name = value`:

| Resource type | Names | Example |
| --- | --- | --- |
| `STRT_TUI_BREAKPOINTS` | `sm`, `md`, `lg`, `xl`, `drawer` (columns) | `md = 110` |
| `STRT_TUI_COLORS` | `text`, `primary`, `secondary`, `positive`, `negative`, `warning`, `info`, `muted`, `field`, `focus`, `changed`, `selection`, `border` | `primary = #1976d2` |
| `STRT_TUI_KEYS` | an action name: `next`, `prev`, `activate`, `submit`, `back`, `cancel`, `help`, `prompt`, `refresh`, `page-prev`, `page-next`, `drawer`, `select`, `select-all`, `find`, `sort`, `number-links`, `info`, `open-browser`, `redraw`, `log`, `palette`, `go-to`, `quick-create`, `actions`, `find-text`, `find-next`, `find-prev`, `messages`, `tray`, `edit-row`, `quit`, `tab-1` to `tab-9` | `back = f3 \| alt+left` |

A value that does not read is logged and the built-in one stays. Esc, Enter and Tab keep what they do whatever a theme
binds. Color is never the only signal: every cue has a glyph or text too. `NO_COLOR` (or `--no-color`) draws in reverse
and underline only, and `--ascii` (also a `TERM` of `linux` or `vt100`) draws borders and symbols in plain ASCII.

## For screen authors

A screen needs nothing extra for the terminal: forms, lists, boxes, rows, dialogs, links and the find widgets draw as
they are. Where a screen has gone custom for the browser (a `.qvue` screen, or a `render-mode` text of type `qvt`), it
needs a `tui` text of its own beside it to show in the terminal; until it has one, the terminal shows a placeholder with
the browser address. `moqui lint --screens` lists those parts under "TUI worklist".

A `tui` text is one node or an array of nodes of the model, in JSON:

```xml
<render-mode>
    <text type="qvt"><![CDATA[<m-chart :data="..."/>]]></text>
    <text type="tui"><![CDATA[{"t":"text","style":"h3","text":${JSON.stringify("Total " + total)}}]]></text>
</render-mode>
```

Every value goes through `JSON.stringify`: a bare `${}` can write a quote that breaks the JSON, as a bare `${}` in qvt
text is an HTML injection. The lint reports a `${` outside `JSON.stringify(`. A text that is not JSON, or not nodes,
becomes an error node naming the screen, and the rest of the page is kept. A static file uses `template="false"`.

The node types and their properties are documented with the other client contracts in
`runtime/base-component/webroot/screen/webroot/js/README.md` ("The tui screen model"); the serde types in
`crates/moqui-tui/src/model.rs` are the schema.

### Components

A `.qvue` screen's twin is usually one `component` node: a client widget registered by name. The screen adds `tui` to its
`render-modes` and puts the text beside its `qvue` text; the server sets the component's `base` to the screen's path, so
the component's transitions are the screen's own, the ones the `.qvue` file calls:

```xml
<screen render-modes="vue,qvue,tui">
    <widgets><render-mode>
        <text type="qvue" location="component://example/screen/Gallery/Board.qvue" template="false"/>
        <text type="tui" location="component://example/screen/Gallery/Board.tui.json" template="false"/>
    </render-mode></widgets>
</screen>
```

```json
{"t": "component", "name": "board", "props": {"source": "boardRows", "groups": "stages", "groupOf": "stage",
 "fields": ["amount", "owner", "next"], "move": "moveCard", "moveGroup": "stage", "open": "../Detail?opportunityId={id}"}}
```

The framework's components, with their props documented at the top of each file in `crates/moqui-tui/src/components/`:

| Name | Shows | Keys |
| --- | --- | --- |
| `board` | Cards in columns by group from lg, else grouped under headings; `source` answers `{groups, rows}` | Up/Down, Left/Right (wide), `m` moves a card through `move`, Enter opens `open` |
| `calendar` | An agenda by day, with the month grid from lg; `source` answers `{month, events}` | Up/Down, PgUp/PgDn months, `t` today |
| `chart` | A table of the datasets (`labels`, `series`) with a bar for the first series | `r` reads again |
| `timeline` | Entries (`items`) in order, each its time, title and text | Up/Down, Enter |

A name the client does not know draws a placeholder naming it, and `=` lists the components the client has: a remote
client knows only those it was built with. An add-on crate registers its own behind a `tui` feature of its crate, which
`./moq build` turns on when the binary has the TUI:

```rust
#[cfg(feature = "tui")]
inventory::submit! { moqui_tui::Registered(&MyInbox) }
```

A component implements `moqui_tui::Component`: where its data comes from (a transition under the screen's path), its
lines at a width (spans in theme roles, so it needs no Revue), its keys (moves, and what it asks of the session: reload,
post, follow, a message), and its help.

## Not yet

Phase 4 brings the remote client (`moqui tui --server <url>`), list views, a Markdown editor, and the log pane over
SSH for users who may read the logs.
