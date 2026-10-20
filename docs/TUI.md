# The Terminal UI

`moqui tui` shows the apps in a terminal. It draws the same XML Screens the browser does, through the `tui` render mode:
the server writes each screen as a JSON document of nodes, and the client in `crates/moqui-tui` draws it with
[Revue](https://github.com/hawk90/revue). There are no TUI-only screens. The design is in
[plans/Screen-TUI-Impl.md](../plans/Screen-TUI-Impl.md).

It is in the Enterprise and Maximal packages (the `tui` feature, [Packaging](Packaging.md)).

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

## Keys

The scheme follows 5250 function keys and Lynx link travel. Single-letter keys act only when focus is not in a text
field; function keys and Alt combinations work everywhere. No default uses Ctrl+A, Ctrl+B, Ctrl+Z, Ctrl+\ or Ctrl+], so
tmux, screen, job control and ssh escapes are left alone. F1 lists the keys in use, a theme's included.

| Key | Action |
| --- | --- |
| Tab, Down / Shift+Tab, Up | Next / previous field, link, button or list |
| Enter | In a field: submit its form (a find form finds). On a link: follow it. On a button: press it. On a list: open the row's first link. In a text area: a new line |
| Right / Left | Not in a field: follow / back. In a field: move the cursor |
| Alt+Enter, Ctrl+S | Submit the focused form from anywhere, a text area included |
| Esc, F12 | Close the top dialog or list; else clear the message; else leave the edits, after asking |
| F1, `?` | The keys |
| F3, Alt+Left, Backspace (not in a field) | Back to the screen before |
| F4 | Prompt: a drop-down's list, a date's picker, a text find's operator |
| F5, Ctrl+R | Refresh the screen |
| F7 / F8, PgUp / PgDn | Previous / next page of the focused list (else the first list) |
| Shift+Left / Shift+Right | Scroll a list wider than the screen; its first column stays |
| F9 | Show the drawer and focus it; again to hide it |
| Space, `*` | On a list: select the row, select all. On a check box: toggle |
| `f`, `s` | On a list: the find dialog of its header fields, cycle the sort |
| Alt+1…Alt+9, 1…9 | Subscreen tab N |
| `#` | Number the links; then digits and Enter follow link N |
| `=` | Info: the screen, its address, the focused link's target |
| `o` | Open the screen or the focused link in the browser (its address on the message line) |
| Alt+L | The log pane |
| Ctrl+L | Redraw |
| Ctrl+Q | Quit, after asking if a form changed |

In a drop-down list, typing narrows it, Up and Down move, Enter picks and Esc closes. In the date picker the arrows move
a day or a week, PgUp and PgDn a month, Home is today and Enter picks.

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
| `STRT_TUI_KEYS` | an action name: `next`, `prev`, `activate`, `submit`, `back`, `cancel`, `help`, `prompt`, `refresh`, `page-prev`, `page-next`, `drawer`, `select`, `select-all`, `find`, `sort`, `number-links`, `info`, `open-browser`, `redraw`, `log`, `quit`, `tab-1` to `tab-9` | `back = f3 \| alt+left` |

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

## Not yet

Phase 3 brings the SSH `apps` command, the palette, quick create, counters and the tray, trees, wizards and stack panels,
row actions and saved finds, autocomplete, and the component registry for `.qvue` twins. Phase 4 brings the remote
client (`moqui tui --server <url>`), list views, and a Markdown editor.
