# Screen TUI implementation plan

Oct 20, 2026 · Plan only; no code yet. This plan implements [proposals/Screen-TUI-Revue.md](proposals/Screen-TUI-Revue.md). Revue was read at 3.10.1 (commit `011741e`, the clone at `~/work/moquiwork/3p_other/revue`). Revue paths below are relative to that clone. Every other path is relative to this repository.

The proposal sets the architecture: a `tui` render mode emits a JSON screen model, and one Revue client interprets it and reaches the server over several channels. This plan adds four things:

- the terminal UI design: a TUI variant of every widget the `qvt` mode uses, the shell frame, the width classes and the keyboard scheme;
- the server work;
- the client crate;
- the channels.

It ends with phases, tests and the open questions, updated from what reading Revue showed. Code appears only where it shows a nuance.

---

## 0. Decisions this plan takes

Most of these are beyond what the proposal settles.

| # | Decision | Why |
| --- | --- | --- |
| D1 | `tui` is a JS macro module, `ScreenTui.js`. It is registered like the other modes and emits JSON through `sri.write`. There is no Rust model builder. | Every mode is built this way (`crates/moqui-core/src/screen/generate.rs:46` `MODE_MODULES`). The JS macro walk already handles field chains, conditional fields, layouts and form assembly (`ScreenSupport.js`), and a module can be overridden from a resource (`generate.rs:445` `mode_source`). |
| D2 | `tui` is **always-standalone**: each screen level is its own request, as in `qvt`. | This is the qvt protocol (`runtime/base-component/webroot/screen/webroot/js/README.md:130`, "A level is its own screen"). The client reloads only from the first level that changed. Parent levels such as a record header above tabs stay as they are. Pre-actions start at index 2, as qvt's do (`crates/moqui-core/src/screen/render.rs:781-784`). |
| D3 | The client is a new crate, `crates/moqui-tui`. Its `model` module has no Revue import and is usable alone through a `model` feature. Revue is an optional dependency behind the `ui` feature (3.1). | This is the proposal's single adapter layer. It also lets `moqui-core` tests deserialize the model without pulling in Revue. A Ratatui client stays possible. |
| D4 | `moqui-tui` does no I/O of its own. It defines a `ScreenLink` trait, and `moqui-host` implements it (in-process and HTTP) behind a new `tui` feature. | `crates/moqui-host/Cargo.toml:9` says only `moqui-host` may depend on Tokio, axum, russh and the like. Revue is used with `default-features = false`, so its `async` feature does not pull in Tokio. |
| D5 | Revue is a **render pipeline and widget kit**, not the app framework. The interpreter owns focus, key routing, navigation and the form state. | Revue routes no keys to widgets. The app does that (`docs/refactor/findings-event-dispatch.md`, and `src/core/app/event_loop.rs:171`). Its three focus systems are not connected. `Form`/`FormFieldWidget` only render (`src/widget/form/form/mod.rs`). None of the examples uses its routers. |
| D6 | The SSH and browser channels run a **custom event loop on Revue's generic `Terminal<W>`**, in process, one per session. A child process on a pseudo-terminal is the fallback if the Phase 1 spike fails. | `App::run` is tied to the process: it hardcodes stdout and crossterm's `poll`/`read` (`event_loop.rs:76,94`), installs signal handlers (`src/core/app/signals.rs`) and installs a global panic hook (`src/runtime/render/terminal/panic_hook.rs:76`). None of that can run per session in a server. `Terminal<W: Write>` itself is generic (`src/runtime/render/terminal/core.rs:23`), and `PipelineHarness` already drives the real draw path into memory (`src/testing/pipeline.rs:42`). |
| D7 | Width classes are measured in **canvas columns**, which is the terminal width less a docked drawer. The second column starts at **120**. | Section 1.3. On the user's 180-column terminal the drawer docks and the canvas is 153 columns, which gives two columns of boxes holding forms. |
| D8 | The keyboard scheme follows **5250/3270 function keys plus Lynx link travel**, and every action has a key. | Section 1.5. This settles the proposal's "Keyboard conventions" open question. |
| D9 | Every string from the server is stripped of control characters before it is drawn. | A field value holding ESC or OSC bytes would otherwise drive the user's terminal. Revue writes cell characters as given. |
| D10 | **A screen that goes custom in one render mode needs its own implementation in each other mode it should work in.** A `.qvue` screen, or inline `render-mode` text of type `qvt`, gets a `tui` text beside it: a model fragment that may name a client component. Until it has one, it shows a placeholder with a browser link. | This is the user's rule. Custom presentation does not translate between modes, but the screen's transitions and data are shared, so only the presentation is written twice (section 2.9). |
| D11 | Key bindings are a **theme value**, with the scheme of section 1.5 as the default. Per-user bindings and a UI for them wait. | One scheme per deployment keeps the help, the key line and `docs/TUI.md` true for everyone. |
| D12 | Rich-text fields (`editor-type="html"`, `rich`, `rich-light`) are read-only in the TUI, with "open in browser". Markdown fields (`editor-type="md"`) are edited as source with a preview. A two-way Markdown editor that looks like `markdown-view` comes later (section 3.8). | No terminal editor today edits rich text, or Markdown in place, without losing content (section 3.8). |

---

## 1. UI design

### 1.1 Principles

- **Green screen first.** Fixed regions, dense forms with dot-leader labels, a message line, and a function-key line that names what each key does on this screen. Nothing needs a mouse. The mouse works where Revue gives it for free: click to focus, wheel to scroll.
- **Lynx for links.** Up and Down move through links and fields in reading order. Right or Enter follows a link, and Left goes back. `/` finds text on the screen. `#` numbers the links (`[12]Acme Supply Co`), so typing `12` then Enter follows that link. `=` shows where the current link goes and the browser URL of the screen. HTML content gets a numbered References list at its end.
- **Same screens, same terms.** The TUI renders the XML Screens as written. There are no TUI-only screens and no TUI attributes in the XSD for the first release. Where a screen has gone custom for the browser (a `.qvue` file, inline `qvt` text), it adds a `tui` text of its own (D10, section 2.9). It does not get a separate TUI screen. The BOSS style guide's widget rules (`runtime/component/ironstone-boss/plan/style-guide.md` §4) are the visual source: one primary action per form, status as a chip, numbers right-aligned, a list's first column links to the record.
- **Color is never the only signal.** Every color cue has a glyph or text cue. `NO_COLOR` and monochrome terminals stay fully usable, and an ASCII mode replaces box drawing and symbol glyphs.

### 1.2 The frame

The frame has the same regions as the qvt shell (`m-qapps-root`, `WebrootVue.qvt.js:6498`; style guide §2.1). Each is mapped to terminal rows and columns.

| Region | Terminal form | qvt counterpart |
| --- | --- | --- |
| Title line (row 1) | `≡` when the drawer is hidden, then app ▸ breadcrumbs ▸ screen title on the left. On the right: counts (Today, Chat, Notif), user, clock, and a busy spinner while a request runs. | `q-header`: launcher, app title, breadcrumbs, counters, account |
| Drawer | Left, 26 columns. The app menu tree (two levels under the app root), pinned saved finds, then Settings and Home. Docked from 150 terminal columns, an overlay (F9) below that. | `q-drawer` 260 px, `m-menu-nav-item` |
| Canvas | The screen levels, stacked from the app root down. It scrolls vertically. A form-list can scroll horizontally on its own. | `q-page` > `m-subscreens-active` |
| Message line (row −2) | The last message. An error stays until Esc or the next action. Alt+M opens the history (last 25). | `$q.notify` and `notifyHistoryList` |
| Key line (row −1) | The function keys that apply to the focused element, then the global ones, cut to the width. Revue's `StatusBar` with `KeyHint`s (`src/widget/feedback/statusbar/`). | none (green-screen convention) |
| Overlays | Palette, dialogs, drop-down lists, help, confirmations, toasts | `m-shell-palette`, `m-dialog`, `q-menu` |
| Log pane (local only) | A bottom split of 10 rows, toggled with Alt+L | none |
| Tray | Today / Chat / Notifications as a right panel 48 columns wide when the canvas left beside it is still ≥120 (1.3), otherwise a modal | `m-shell-tray` |

The frame at 180 columns, drawer docked, with a canvas of 153 columns (md, two columns). Links are numbered (`#` on).

```
 Moqui ▸ Sales ▸ Orders ▸ SO10023 · Order Detail                                                                        Today 3  Chat 1  Notif 2   john.doe   Tue 14:32
──────────────────────────┬─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
 ▾ Sales                  │ ┌─ SO10023  Acme Supply Co ───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
   Dashboard              │ ┃ Status: [Approved]  ● Placed ─ ● Approved ─ ○ Shipped ─ ○ Completed      < Ship >  [ Hold ]  [ Cancel ]  [ More ▾ ]                                 │
 ▸ Customers              │ ┃ 1:Detail  2:Items  3:Shipments  4:Payments  5:Notes                                                                                                 │
 ▾ Orders                 │ └─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
   ▸ Find Orders          │ ┌─ Order ─────────────────────────────────────────────────────────────────┐ ┌─ Customer ──────────────────────────────────────────────────────────────┐
   ▶ Order Detail         │ │ Order ID . . . . . : SO10023                                            │ │ Customer . . . . . : [12]Acme Supply Co                                 │
   ▸ Returns              │ │ Order date . . . . : [2026-10-20 ]  F4                                  │ │ Contact  . . . . . : Dana Ortiz                                         │
 ▸ Quotes                 │ │ Status . . . . . . : [Approved            ▾]                            │ │ Email  . . . . . . : [dana@acme.example         ]                       │
 ▸ Reports                │ │ Ship by  . . . . . : [2026-10-27 ]  F4                                  │ │ Bill to  . . . . . : 400 Main St, Springfield                           │
                          │ │ Priority . . . . . : (•) Normal  ( ) Rush                               │ │ Ship to  . . . . . : [Warehouse dock B     ▾]                           │
 Saved finds              │ │ * Sales rep  . . . : [Jo Lin              ▾]                            │ │ Terms  . . . . . . : Net 30                                             │
   Open > $10k            │ │                                                                         │ │                                                                         │
   My approvals           │ │ < Save >  [ Reset ]                                                     │ │ [x] Notify on ship                                                      │
                          │ └─────────────────────────────────────────────────────────────────────────┘ └─────────────────────────────────────────────────────────────────────────┘
 Settings                 │ ┌─ Items · 3 of 3 ────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
 Home                     │ │   Seq  Product                      Description                                     Qty  Unit price       Total   Status                            │
                          │ │   ───  ──────────────────────────── ──────────────────────────────────────────── ────── ─────────── ───────────   ────────                          │
                          │ │ ▌ 01   [13]WID-100 Widget, blue     Blue anodized widget, 10 mm                      40       12.50      500.00   Approved                          │
                          │ │   02   [14]WID-220 Bracket          Steel mounting bracket                           40        3.75      150.00   Approved                          │
                          │ │   03   [15]SVC-010 Install          Installation service, per hour                    6       85.00      510.00   Pending                           │
                          │ │                                                                                                     ───────────                                     │
                          │ │                                                                                               Total    1,160.00                                     │
                          │ │ Rows 1–3 of 3 · Page 1/1                                         a=Add item  e=Edit row  Space=Select  F7/F8=Page                                   │
                          │ └─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
──────────────────────────┴─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
 Order SO10023 updated.
 F1=Help  F3=Back  F4=Prompt  F5=Refresh  F6=Create  F9=Menu  F10=Actions  F12=Cancel   Ctrl+K=Go to   →/Enter=Follow  ←=Back   /=Find   #=Number links
```

The same screen at 100 columns (sm: drawer hidden, one column, short labels in the title line):

```
 ≡ Sales ▸ Orders ▸ SO10023                                     T3 C1 N2  john.doe
────────────────────────────────────────────────────────────────────────────────────────────────────
 ┌─ SO10023  Acme Supply Co ──────────────────────────────────────────────────────────────────────┐
 ┃ Status: [Approved]   < Ship >  [ Hold ]  [ More ▾ ]                                            │
 ┃ 1:Detail  2:Items  3:Shipments  4:Payments  5:Notes                                            │
 └────────────────────────────────────────────────────────────────────────────────────────────────┘
 ┌─ Order ────────────────────────────────────────────────────────────────────────────────────────┐
 │ Order ID . . . . : SO10023                                                                     │
 │ Order date . . . : [2026-10-20 ]  F4                                                           │
 │ Status . . . . . : [Approved            ▾]                                                     │
 │ …                                                                                              │
 └────────────────────────────────────────────────────────────────────────────────────────────────┘
 ┌─ Customer ─────────────────────────────────────────────────────────────────────────────────────┐
 │ Customer . . . . : [12]Acme Supply Co                                                          │
 │ …                                                                                              │
 └────────────────────────────────────────────────────────────────────────────────────────────────┘
────────────────────────────────────────────────────────────────────────────────────────────────────
 Order SO10023 updated.
 F1=Help F3=Back F4=Prompt F5=Refresh F9=Menu F10=Actions F12=Cancel
```

And at 72 columns (xs: labels above values, the mobile column set of the list, horizontal scroll marked `▸`):

```
 ≡ Orders ▸ SO10023                               N2  john.doe
────────────────────────────────────────────────────────────────────────
 ┌─ Order ────────────────────────────────────────────────────────────┐
 │ Order ID                                                           │
 │   SO10023                                                          │
 │ Order date                                                         │
 │   [2026-10-20 ]  F4                                                │
 │ Status                                                             │
 │   [Approved            ▾]                                          │
 │ …                                                                  │
 └────────────────────────────────────────────────────────────────────┘
 ┌─ Items · 3 ────────────────────────────────────────────────────────┐
 │ Seq Product                 Qty     Total ▸                        │
 │ 01  [13]WID-100 Widget       40    500.00                          │
 │ 02  [14]WID-220 Bracket      40    150.00                          │
 │ …                                                                  │
 └────────────────────────────────────────────────────────────────────┘
────────────────────────────────────────────────────────────────────────
 F1=Help F3=Back F4=Prompt F9=Menu F12=Cancel
```

### 1.3 Width classes and how layouts reduce

The client measures the canvas on every `Event::Resize`. Revue has no CSS media queries (`src/runtime/style/parser/parse.rs:72-80`). Its `Breakpoints` and `ResponsiveValue` (`src/runtime/layout/responsive/breakpoint.rs:52,155`) can be driven from that canvas width, or the interpreter keeps its own small table. The classes keep the qvt names, so `row-col`/`field-col` sizes map directly. At about 8.5 px per column they also line up with Quasar's pixel breakpoints (style guide §7).

| Class | Canvas columns | Quasar | What changes |
| --- | --- | --- | --- |
| xs | < 80 | < 600 px | Labels sit above values. The `columns type="mobile"` set is used (the client sends `_uiType=mobile`, as qvt does at `WebrootVue.qvt.js:277`; `ScreenSupport.js:290`). Dialogs are full screen. The title line shows the screen title and counts only. |
| sm | 80–119 | 600–1023 | One column of boxes. A `field-row` keeps up to 2 cells side by side. Dialogs are 90% wide. |
| md | 120–179 | 1024–1439 | **Two columns**: `row-col md`/`lg` sizes apply. A `field-row` keeps up to 4 cells. A `container-split` shows side and main together. |
| lg | 180–239 | 1440–1919 | As md, plus three columns where `row-col lg="4"` asks for them. |
| xl | ≥ 240 | ≥ 1920 | As lg |

Rules:

- **Docking the drawer.** The drawer docks only when the canvas left over is still at least 120 columns, which with the 26-column drawer means a terminal of 150 columns or more. So docking never pushes a screen from md down to sm. F9 toggles it at any width, and the choice is kept as a user preference (the `QUASAR_LEFT_OPEN` counterpart, `TUI_DRAWER_OPEN`).
- **Docking the tray.** The same rule: the 48-column tray docks only when the canvas left beside it is still at least 120 columns, so from a canvas of 168. The width class is always that of the canvas left over. So docking the tray on a canvas of 180–227 moves the screen from lg to md, and a three-column `row-col lg="4"` row shows two columns while the tray is open. Neither panel ever pushes a screen below md.
- **`row-col`.** The 12-column grid works as `colOpen` does (`runtime/base-component/framework/template/screen/ScreenQvt.js:1107-1122`). When a smaller class has no size, it is 12. A column's width is the canvas width × n/12, less a one-column gutter. A box narrower than 40 columns falls to the next row rather than squeezing.
- **`field-row`.** This is width-driven rather than the fixed qvt rule (`ScreenQvt.js:2033`). Cells stay side by side while each cell gets at least the form's label width plus 24 columns. Otherwise they stack.
- **Labels.** Each form column pads its labels to the longest title in that column, capped at 24 columns (18 in sm), with dot leaders as in the mockup. A longer title ends in `…`, and its full text is on the key line while the field has focus.
- **`form-list`.**
  - Column widths come first from `print-width`/`print-width-type` through `form_list_column_char_widths` (`crates/moqui-core/src/screen/sri/layout.rs:27`), which the text mode already uses. Otherwise from the widest value on the page, between the title width and 40 columns.
  - When the total is wider than the canvas, the first column stays fixed and the rest scroll sideways, with `◂`/`▸` marks.
  - The page size follows the visible rows of the table area, from 10 to 100. The client sends `pageSize` when the area changes, which the Lynx-style full-screen pages call for.
- **Breakpoints are settings.** The breakpoints and drawer width are theme values with these defaults. One deployment can move the second-column line without code.

### 1.4 Visual vocabulary

| Thing | Look | Notes |
| --- | --- | --- |
| Editable field | `[value______]` in the field color, underscores filling the width (`size`, `maxlength`, or a default of 30) | 5250 input field. ASCII and color modes look the same. |
| Focused field | Reverse video, terminal cursor at the insert point | The terminal cursor stays on the field, so screen readers and `tmux` copy mode follow it. |
| Changed field | Changed-field color, plus `•` after the bracket | The qvt blue tint (style guide §2.6) |
| Required | `*` before the label | |
| Field error | Error color, `!` after the bracket, the message on the message line while focused, and a count in the form footer | qvt marks the field and toasts |
| Read-only value | Plain text, no brackets | style guide §4.5: a display never looks like an input |
| Drop-down | `[Approved      ▾]`. F4 or Enter opens the list. | F4=Prompt is the 5250 key |
| Date / time | `[2026-10-20 ]` with F4 for the picker | |
| Check / radio | `[x] Label`, `(•) A  ( ) B` | |
| Primary button | `< Save >`. Dim while the form is unchanged, bold and primary color once a field changes. | The outline-until-changed rule |
| Secondary (tonal) button | `[ Hold ]`. Danger types use the danger color plus a confirmation. | |
| Text link | Underlined link color. `[n]` prefix when numbering is on. | Lynx |
| Disabled | Dim, and skipped by the focus order unless `=` asks why | |
| Box | Single-line border with the title in the top edge. `type` gives a heavy left edge (`┃`) in the type color. | `m-container-box` |
| Status chip | `[Approved]` in the status color (from `StatusItem.statusColor` / the status color map) | style guide §5.2 |
| Badge | `Notif 2`, or a `(3)` suffix on a button | |
| Selected row | `▌` in the left gutter plus the selection color. Checked rows show `[x]` in a selection column. | |
| Sort | `▲`/`▼` after the column title, the active column in the primary color | |

**Theme.** Revue's widgets use fixed constants from `src/widget/theme.rs` and do not read its theme signal (`src/runtime/style/theme_signal.rs:86`). So the client defines its own `Theme` of roles: primary, secondary, positive, negative, warning, info, muted, field, focus, changed and selection. Each role maps to a Revue `Style`, and the adapter applies those to every widget it draws. The roles take their values from the server theme where it has them (brand primary and status colors) and from a built-in dark/light pair otherwise. Color depth is detected (truecolor, 256, 16). `NO_COLOR` gives monochrome, with reverse and underline only. `--ascii`, or a `TERM` of `linux`/`vt100`, uses the `ascii` border preset (`src/widget/layout/border/mod.rs:158-200`) and `v`, `*`, `>` in place of the symbol glyphs.

### 1.5 Keyboard scheme

This is one scheme for every screen. It is documented in a new `docs/TUI.md` and shown by F1. It follows three rules:

1. Single-letter keys work only when focus is not in a text field, as in Lynx.
2. Function keys and Alt combinations work everywhere.
3. No default binding uses Ctrl+A, Ctrl+B, Ctrl+Z, Ctrl+\\ or Ctrl+], so tmux, screen, job control and ssh escapes are left alone. Ctrl+S is safe because raw mode turns IXON off, but every binding that uses it also has an F-key or Alt alternative.

| Key | Action | Source |
| --- | --- | --- |
| Tab / Shift+Tab | Next / previous field or button in the form | |
| Down / Up | Next / previous focusable in reading order (fields, links, rows) | Lynx |
| Enter | On a single-line field: submit its form (a find form runs the find). On a link: follow it. On a button: press it. On a row: open its first link. | 5250 Enter, Lynx |
| Right / Left | Not in a field: follow the link / go back. In a field: move the cursor. | Lynx |
| Alt+Enter, Ctrl+S | Submit the focused form from anywhere, a text area included | |
| Esc / F12 | Close the topmost overlay. Otherwise clear the message line. Otherwise leave the form's edits, after asking if anything changed. | 5250 F12=Cancel |
| F1, `?` | Help: keys for the focused element, the screen's documents (`screenDoc`) | |
| F3, Alt+Left, Backspace (not in a field) | Back, through history | 5250 F3=Exit |
| F4 | Prompt: the list of a drop-down, the picker of a date, the operator menu of a text-find | 5250 F4=Prompt |
| F5, Ctrl+R | Refresh the screen (re-request from the current level) | |
| F6, `+` | Quick create (the shell providers' `quick-create` items) | |
| F7 / F8, PgUp / PgDn on a list | Previous / next page of the focused list | 5250 roll keys |
| F9 | Show or hide the drawer, and focus it | |
| F10 | Actions: one menu of the screen's `button-menu`s, title-row links and the focused list's row actions | |
| Ctrl+K, `:` | Palette: screens, records, saved finds, provider actions, recent screens, apps | `m-shell-palette` |
| `g` | Go to a path (`/apps/...` or a menu title) | Lynx `g` |
| Alt+1…9 or `1`…`9` (not in a field, links unnumbered) | Subscreen tab N | |
| Alt+letter | The button or tab whose mnemonic letter is underlined | |
| `/`, `n`, `N` | Find text on the screen, next, previous | Lynx |
| `#` | Number the links on or off. With numbers on, digits then Enter follow link N. | Lynx |
| `=` | Info: screen path, browser URL, the current link's target and parameters | Lynx `=` |
| `o` | Open the screen (or the focused link) in the browser: an OSC 8 hyperlink plus a copy to the clipboard where available | |
| Space, `*` (list) | Toggle the row's selection, select all | `row-selection` |
| `f` (list) | Find options dialog of the focused list (its header fields) | |
| `s` (list column) | Cycle the sort of the focused column | |
| Alt+M | Message history | |
| Alt+L | Log pane (local mode) | |
| Ctrl+L | Redraw | |
| Ctrl+Q | Quit, after asking if any form has changed | |

Revue's `KeymapConfig` and its chord parser (`src/utils/keymap/mod.rs:97`, `parse.rs:12`) hold the table. The layered priority pattern of `src/state/patterns/keys.rs:130` (modal, then confirm, then popup, then view) is the dispatch order.

**Bindings are per theme (D11).** The table above is the built-in default. A theme may override entries by action name through the `tui` theme values (section 2.6), as chord strings in Revue's syntax, for example `back = "f3 | alt+left"`. The client merges the theme's entries over the default when the shell data loads. An entry that does not parse, or that names an unknown action, is logged and skipped, and the default for that action stays.

These bindings stay fixed whatever the theme says: Esc, Enter and Tab, and the typing keys inside a text field. A binding that would take them is refused, so no theme can leave a user stuck in a dialog. F1 help and the key line are built from the merged keymap, so they always show the theme's keys. Per-user bindings, and a screen to edit them, come later; they would be one more layer over the theme's.

### 1.6 Widget catalog

There is a TUI variant for each element the qvt mode writes. The qvt sources are the macro table (`ScreenQvt.js:2498-2575`) and the wrappers in `WebrootVue.qvt.js`, which are listed by line in `webroot/js/README.md`. "Revue base" is what the adapter builds on. "Own" means a widget written in `moqui-tui/src/ui/widgets/`.

#### Shell and navigation

| Element / qvt widget | TUI variant | Revue base | Notes |
| --- | --- | --- | --- |
| Apps menu, launcher | Palette group "Apps", and the AppList screen | `CommandPalette` (`src/widget/command_palette/`) | From `shellData` (`crates/moqui-core/src/screen/shell.rs:712`) |
| App drawer (`m-menu-nav-item`) | Tree: `▾`/`▸` levels, `▶` active screen, saved finds | `Sidebar` (`src/widget/layout/sidebar/`) | From `menuData` (`crates/moqui-core/src/screen/menu.rs:150`) |
| Breadcrumbs | Title line. A crumb with siblings opens a menu on Enter when the title line has focus (F9 then Up). | `Breadcrumb` | |
| `subscreens-panel type="tab"`, `m-subscreens-tabs` | Tab bar `1:Detail  2:Items …`, active tab reversed, then the active level | `Tabs` (bar only) | Shown only with more than one tab, as in qvt |
| `subscreens-panel type="popup"` | Nothing in the canvas; the siblings are in the drawer and the breadcrumb menu | | |
| `subscreens-panel type="wizard"` | Step header `Step 2 of 4 · Options  ● ● ○ ○`, Back and Next buttons, PgDn = Next after the step form submits | `Stepper` | Follows the `m-subscreens-wizard` submit protocol (`WebrootVue.qvt.js:3659`) |
| `subscreens-panel type="stack"` | Breadcrumb stack over the active level. F3 pops. | `Breadcrumb` | |
| `button-menu` | `[ More ▾ ]`; Enter or F10 opens a menu list with badges | `ContextMenu` (`src/widget/feedback/menu/`) | |
| `tree` | Indented tree. Right expands (children fetched), Left collapses, Enter follows the node link. | `Tree` (`src/widget/data/tree/`) | Revue's tree has no lazy-load hook. The own wrapper fills children when the fetch returns. |
| Palette (`m-shell-palette`) | Top overlay with an input and grouped results | `CommandPalette` | Results from `shellSearch` (`shell.rs:952`) |
| Quick create (`m-shell-quick-create`) | F6 menu, then the create form in a dialog | `ContextMenu`, own dialog | Providers from `shell_providers` (`shell.rs:767`) |
| Counters | Title line `Today 3  Chat 1  Notif 2` | | Read from `shellCounts` at each navigation and every 60 s (Phase 4: push) |
| Tray (`m-shell-tray`) | Right panel when the canvas left beside it is still md (1.3), a modal otherwise. Tabs Today, Chat, Notifications. | `TabView` | Its screens are theme values. A `.qvue` tray screen needs its `tui` text (D10). |
| Keys overlay (`m-shell-keys`) | F1 | `Modal` | |
| Account menu | Palette group "Account": status, preferences, log out | | |

#### Containers and layout

| Element / qvt widget | TUI variant | Revue base | Notes |
| --- | --- | --- | --- |
| `container` | No border; children in a vertical stack. `row`/`col` classes in `style` are ignored. | `vstack` | |
| `container-row` / `row-col` | 12-column grid, by width class (section 1.3) | `Grid` with `TrackSize::Fr` (`src/widget/layout/grid/types.rs:7`) | CSS `grid-*` properties are parsed but not applied (`docs/guides/styling.md:182-200`), so the layout is built in code |
| `container-box` (+ header, toolbar, body) | Bordered box, title in the edge, toolbar right-aligned on the first line, type as a heavy left edge. Collapsible with Enter on the title; `initial=closed` starts collapsed. | `Border` (`src/widget/layout/border/mod.rs`) | |
| `container-panel` | Fixed side columns (`size` px ÷ 8), the center between them. Stacked below md. | `hstack` with `child_sized` | qvt never stacks panels; the TUI does, since a terminal cannot scroll the page sideways |
| `container-split` | Side and main from md, side width px ÷ 8. Below md: the side list, then main with F3 back. The selected id stays in the parameters. | `SplitView` (`src/widget/layout/split_view/`) | `m-container-split` (`WebrootVue.qvt.js:3801`) |
| `container-dialog` | Button, then a centered modal with a title and a scrolling body. The body is in the model and hidden until opened. | `Modal` (`src/widget/feedback/modal/`), `show_with_focus_trap` | Full screen at xs, as qvt is below `sm` |
| `dynamic-dialog` | Button, then a modal whose body is a TUI render of the transition URL | `Modal` + a nested level | `submit-reload-id`/`submit-hide-id` reload or close by id, as in `m-form` |
| `dynamic-container` | A nested level loaded by URL, reloadable by id | | |
| `section`, `section-iterate`, `section-include` | Resolved on the server; the client sees only the output | | `section-iterate paginate` gets the list paging footer |
| `include-screen` | Inline, as the server renders it | | |
| `dashboard` | Grid of cards, columns = canvas ÷ (`min-width` px ÷ 8) | `Grid` | |
| `number-card` | Small box: title in the edge, value bold, delta `▲ 12%` in the good/bad color, Enter follows its link | `Border` + `Text` | `m-number-card` (`WebrootVue.qvt.js:5255`) |
| `status-bar` | `● Placed ─ ● Approved ─ ○ Shipped`, then the transition buttons, each its own form | own `StatusFlow` | `m-status-bar` (`WebrootVue.qvt.js:3988`) |
| `context` | Not drawn. Kept as the screen context for quick create and the palette, as `m-screen-context` does. | | |

#### `form-single`

| Element | TUI variant | Notes |
| --- | --- | --- |
| `form-single` | A focus group. Fields in field-layout order. Enter submits. The footer holds the submit row and an error count. | Submits as `m-form` does: all fields as form parameters, plus the model's hidden fields |
| `field-layout` > `field-row` | Side by side while each cell fits (section 1.3) | |
| `field-row-big` | One wrapping line (find rows, action rows) | |
| `field-col-row` / `field-col` | Grid by width class, `label-cols` ignored (labels are padded per column) | |
| `field-group` | Subtitle line. `box="true"` draws a box. | |
| `field-accordion` | Each group a collapsible line `▸ Shipping options`; Enter toggles | Revue `Accordion` (`src/widget/layout/accordion/`) |
| `fields-not-referenced` | The remaining fields, in order | |
| `visible-when` | Re-evaluated in the client when the named field changes | Same rule data as qvt |
| `focus-field` | Initial focus | |
| `submit` / `reset` | `< Save >` / `[ Reset ]`. `confirmation` asks in a yes/no modal first. | |

#### `form-list`

| Feature | TUI variant | Notes |
| --- | --- | --- |
| Table | Header row, rule, rows, totals. Numbers right-aligned. First column fixed when scrolling sideways. | Revue `DataGrid` (`src/widget/data/datagrid/`): in-memory rows, virtual scroll of visible rows (`core.rs:301`), frozen columns, column types. The client fills it one server page at a time and turns its local sort and filter off. |
| Paging (`paginate`, `show-page-size`, `show-all-button`) | Footer `Rows 21–40 of 312 · Page 2/16`; F7/F8; `g` on the footer for a page number | Paging and sort are re-requests of the level with `pageIndex`, `pageSize` and `orderByField`, so the URL holds the state, as in Lynx |
| `header-field` find | `f` opens the Find dialog: a form of the header fields. A filter summary line appears above the table (`Status = Approved ×`) with a Clear link. | An inline filter row is too tight in a terminal, so the dialog is always used, as `header-dialog="true"` does in qvt. The summary follows qvt's (`ScreenQvt.js:1229-1240`). |
| Sort (`show-order-by`) | `s` on the focused column; `▲`/`▼` on the title | |
| `row-selection`, `multi` | Selection column `[ ]`. Space toggles, Shift+Up/Down extend, `*` selects all. F10 lists the actions; each opens its dialog or form with the selected ids. | `m-checkbox-set` semantics (`WebrootVue.qvt.js:1607`) |
| Per-row links | Text links in the cell. Enter on the row follows the first. | |
| Row forms (each row its own `m-form`) | A row is a focus group. Editing a cell marks the row changed. Enter submits that row. | |
| `multi="true"` editable rows | Inline cell editing on the focused row. Ctrl+S submits all changed rows as the multi form (`_rowSubmit_N`, `field_N`). | `DataGrid` `editable` (`input/editing.rs`) has no validation callback, so the own wrapper owns the edit buffer and validation. Fallback is the proposal's row edit modal. |
| `editable` cell (`m-editable`) | Enter opens a one-field popup and saves to the editable URL | `WebrootVue.qvt.js:1376` |
| `first-row-field`, `second-row-field`, `last-row-field` | A create row above the rows (`a` focuses it), totals and last row below | |
| `columns` / `form-list-column` | Several fields stacked in one cell (multi-line rows) | `DataGrid` rows are one line, so stacked columns need the own row renderer. Phase 3. |
| `select-columns` | Columns dialog: a check list; Alt+Up/Down reorders | `SelectionList` (`src/widget/input/input_widgets/selection_list/`) |
| `saved-finds` | Saved Finds dialog, and pinned finds in the drawer | |
| Exports (`show-csv-button`, xlsx, text, pdf) | Local and remote: saved to the downloads directory, path on the message line. SSH and browser: "View as text" only, the `text` render mode in a pager. | No file leaves the server through an SSH TUI session |
| `views` (board, calendar) | Board: groups as columns at lg and up (cards with title, fields and badge), otherwise a grouped list. `m` moves a card to another group through the board transition. Calendar: an agenda list by day; month grid at lg. | Phase 4. `m-list-views` (`WebrootVue.qvt.js:4368`) |
| Empty list | Header kept, one row "No orders match · Clear filters" or "No orders yet · New" | style guide §6 |

#### Fields

| Element / qvt wrapper | TUI variant | Revue base | Gaps and nuances |
| --- | --- | --- | --- |
| `text-line` (`m-text-line`) | `[____]`, `maxlength` enforced, `input-type="number"` right-aligned | `Input` (`src/widget/input/input_widgets/input/`) | `Input` has no max length or mask, so the own wrapper filters keys. `mask` is enforced as a pattern on input (literal characters typed for the user). `default-transition` fills a value on blur. |
| `text-line ac-transition` | Input with a suggestion list below it, fetched after 250 ms idle | `Autocomplete` (`set_suggestions`, `min_chars`) | Request runs on the worker (section 3.6). The label/value pair is kept in the wrapper. |
| `password` | `[••••••]` | `MaskedInput` (`src/widget/form/masked_input/`) | The value is never in the model |
| `text-area` (`m-text-area`) | Bordered multi-line box, `rows` high. Enter is a newline; Alt+Enter submits. | `TextArea` | `editor-type="md"`: the source is edited, and F4 toggles a preview drawn by the same renderer as `markdown-view`. `html`, `rich` and `rich-light`: the content is shown read-only, converted as `html-view` is, with `o` "Edit in browser" opening the screen there (D12). The field is left out of the form submit, so the stored HTML is never overwritten from the terminal. The later two-way Markdown editor is section 3.8. |
| `drop-down` (`m-drop-down`) | `[Label ▾]`, F4/Enter opens a list that filters as you type | Own `DropDown` over `OptionList` + `Input` | Revue `Select` holds labels only, with no values (`select/mod.rs`). `Combobox` has label/value but is builder-only, with no `set_options`. Options of 300 or fewer come in the model; above that, or `server-search`, they load from the options URL. `allow-multiple`: `MultiSelect` tags. `combo-box`: free text allowed. `allow-empty`: a blank first entry. `submit-on-select`: submits. `show-not` (find forms): `!` toggles Not. |
| `drop-down` with `dynamic-options` / `depends-on` | As above. A change of the named field re-fetches, and a value no longer in the list is cleared. | | The URL and parameter names come from the model, as `options-url`/`options-parameters` do |
| `date-time` (`m-date-time`) | `[2026-10-20 14:30]`, F4 picker. Typed text is checked against the format. | `DateTimePicker` (`src/widget/datetime_picker/`) | Revue's `Date::today()` is fixed at 2025-01-01 (`src/widget/data/calendar/date.rs:21`) and `Time::now()` at 12:00, and it has no string parser. The wrapper parses with `jiff` in the user's time zone and seeds the picker with today. |
| `date-find`, `date-period` (`m-date-period`) | From / thru pair; period mode as two drop-downs (period, offset) with `p` toggling the mode | `RangePicker` presets are wrong until the today bug is fixed | |
| `range-find` | From / thru number pair | `NumberInput` | |
| `text-find` (`m-text-find`) | Input plus an operator chip `[contains]`; F4 opens operator, Not, ignore case | | |
| `check` | `[x] Label` per option, Space toggles; one option with `[x]` alone in a find form | `Checkbox`, `SelectionList` | |
| `radio` | `(•) A  ( ) B`, Left/Right moves | `RadioGroup` | |
| `file` (`m-file`) | `[ choose… ]`. Local and remote: a file picker on the user's machine. SSH and browser: disabled, with a note. | `FilePicker` | **In SSH and browser the client runs on the server, so a picker would browse the server's disk.** Upload is off there. |
| `display`, `display-entity` (`m-display`) | Plain text; `format`, currency and `also-hidden` as on the server; `value-url` re-fetched on `depends-on` changes | `Text` | `Text` does not wrap words on its own; the wrapper uses `src/text/wrap.rs` |
| `hidden`, `ignored` | Not drawn; hidden values are in the form's parameters | | |
| `label` | `h1`–`h3` bold with a rule, `h4`–`h6` bold, `p`/`div` wrapped text, `pre`/`code` unwrapped in the code color, `strong`/`i` as bold/italic where the terminal has them | `Text`, `RichText` | |
| `image` | `▣ alt-text`, a link to open in the browser | | Kitty/sixel images (Revue `image` feature) are later and only for local mode |
| `link` (screen, transition, external, anchor, button, hidden-form) | Text link or button by the style guide's hierarchy (§4.6). `confirmation` asks. `badge` as `(n)`. External URLs are OSC 8 hyperlinks opened by the user's terminal. | `Link` (`src/widget/link.rs`), `Button` | A transition link is a JSON transition call (section 2.5). An external link is never fetched by the client. |
| `markdown-view` (`m-markdown`) | Rendered Markdown with wiki links as links | `Markdown` (feature `markdown`, pulldown-cmark already in `Cargo.lock`) | |
| `html-view` (`m-html-view`) | HTML to text on the server (blocks, lists, quotes collapsed), links numbered with a References list at the end | | No HTML-to-text helper exists in the crates; one is added (section 2.3) |
| `render-mode` `text type="tui"` | The model fragment it holds, in place (section 2.9) | | The TUI counterpart of a `qvt` text |
| `render-mode` `text` for other types (`qvt`, `qvue`, `html`) | Skipped. An `any` text in the same element is used instead, converted to text. When the element has neither a `tui` nor an `any` text, a placeholder `▣ <screen> has a browser-only part · o=Open in browser` is drawn instead, so the gap is visible. | | The lint reports each one (section 2.9) |
| Charts (`m-chart`), diagrams (`m-mermaid`) in qvt text | The screen's `tui` text names the `chart` or `diagram` component with the same data expression the qvt text uses. Until then, the placeholder. | Own `chart` component: a table of the datasets plus bars for one series. `mermaid` (`src/widget/mermaid/`) for diagrams, to evaluate. | |
| `chatter`, `record-files` | A nested level of the theme screen. The BOSS ones are `.qvue`, so they work once their screens add `tui` texts (D10). | | |
| `.qvue` screens | With `tui` in `render-modes` and a `tui` text: the fragment, usually one component node. Without: a box "This screen has no terminal view yet", the browser URL, and `o` to open it. | | The TUI checks `renderModes` in the menu data (`menu.rs:259`) before requesting, so a screen without `tui` costs no request |

### 1.7 States and feedback

These follow the BOSS style guide §6.

- **Loading.** The spinner in the title line. The old content stays, dimmed, until the new level arrives. Input is not blocked except the submit that is running.
- **Transition errors.** The message line in the error color. The form keeps its values, fields named in `validationErrors` are marked, and focus moves to the first of them.
- **Success.** The `submit-message`, or "Saved", on the message line for 3 s.
- **Confirmations.** A yes/no modal whose text names the record, defaulting to No.
- **Unsaved changes.** Leaving a changed form (navigation, F3, Ctrl+Q) asks: Save, Discard or Stay.
- **Permissions.** Actions the user may not run are absent, as the server leaves them out of the model. A screen the user may not view gives the server's 403 as a box with the way back.
- **Session.** A 401 opens the login dialog (username, password, OTP if required). After login the same request is retried.

---

## 2. Server: the `tui` render mode

### 2.1 Registration

- `runtime/base-component/framework/conf/MoquiDefaultConf.xml:671-698` gets a `screen-text-output` entry: type `tui`, mime type `application/json`, `always-standalone="true"`, module `framework/template/screen/ScreenTui.js`.
- `crates/moqui-core/src/screen/cache.rs:143` `BUILT_IN_MODES` grows to 9 entries.
- `generate.rs:46` `MODE_MODULES` gets the `include_str!`, and `generate_xml`/`generate_unit_source` (`generate.rs:518,549`) get the mode name.
- `ScreenSupport.js:33-56` lists every element a module must name, so the module is complete by construction. The golden test fails on an unnamed element.
- The menu data reports `renderModes` per screen (`menu.rs:259,308`). A screen with `render-modes="all"` (the default) includes `tui`.

### 2.2 `ScreenTui.js`

The module follows `ScreenXml.js` and `ScreenText.js`: a `prelude` with the runtime helpers, `macros` per element, and `opaque` for the elements whose children it walks itself. It uses `ScreenSupport.js` (`fieldChain` :125, `displayValueExpr` :248, `formListColumns` :293) the way the other modes do. The widget values come from the same `sri` calls the qvt mode makes:

- `getFieldValueString` (`crates/moqui-core/src/screen/sri/fields.rs:94`);
- `getFieldOptions` (`sri/options.rs:33`);
- `openFormList` / `startFormListRow` (`sri/forms.rs:194,311`);
- the URL helpers (`crates/moqui-core/src/screen/urls.rs:256` `build_url`);
- `field_validation_js_rules` (`sri/fields.rs:412`) for the client checks.

**Nuance: JSON across nested renders.** A screen is several units: the root section, named sections, `section-iterate` entries, includes and dynamic forms. Each writes through `sri.write` into one output, and a nested render writes in the middle of its parent's output. So each unit writes **one complete JSON array**, with its comma state local to the unit. A parent writes a nested render only in a value position:

```js
// in the section macro: the nested unit fills the "c" value with its own [ ... ]
$node({t: "section", id: "s3"}, () => sri.renderSection("OrderHeader"));
// $node writes {"t":"section","id":"s3","c": then runs the body, then }
```

A unit that writes nothing must still write `[]`. The golden fixtures cover an empty section, a failing section, and a section inside a `section-iterate`.

**Nuance: errors inside a section.** `guarded` (`crates/moqui-core/src/screen/sri/sections.rs:44`) writes an error as HTML text where the section would have been. For qvt it first truncates what the failed section wrote (`web::truncate_output`). For `tui` it must also truncate and then write `[{"t":"error","text":…}]`, so the document stays valid JSON. `render_single` (`sections.rs:292`), which writes action errors in place of the widgets, needs the same treatment. Rather than another string test on the mode name, `TextOutput` (`cache.rs:95`) gets an output kind (text, html, json). `guarded` and `render_single` pick the error form from that kind.

**Ids.** Every node that holds state gets an `id` that is stable across renders, so the client keeps focus, scroll, collapsed state and unsent edits:

- the XML `id` where there is one;
- forms by form name, fields by form plus field name;
- rows by the list's `id-field` (or row-selection `id-field`), else by index on the page;
- anything else by its element path within the screen (`s0.3.1`).

### 2.3 The model

The response is one JSON object. `run_job` writes the envelope around the level's widgets when the mode's output kind is json (in `render_body`, `render.rs:842`).

| Envelope key | Content | Source |
| --- | --- | --- |
| `v` | Model version, starting at 1. The client refuses a major version it does not know. | |
| `screen` | `path`, `pathList`, `title`, `renderModes`, `standalone` level index | `ScreenUrlInfo` (`crates/moqui-core/src/screen/url.rs:38`) |
| `parameters` | Current parameters, credentials stripped | `strip_credential_parameters` (`crates/moqui-core/src/web/mod.rs:1516`) |
| `messages` | `{info[], errors[], validationErrors[]}` from the flash of the last transition | `user::session::flash_messages` (`crates/moqui-core/src/user/session.rs:764`) |
| `focus` | The `focus-field` of the first form, if any | |
| `c` | The level's nodes | |

Node types (`t`), each with the properties the client needs and nothing about how qvt draws them:

| Group | Types |
| --- | --- |
| Structure | `section`, `container`, `box` (title, toolbar, `type`, `initial`), `row` / `col` (`xs`/`sm`/`md`/`lg` sizes), `panel` (header/left/center/right/footer, sizes), `split`, `dialog` (button + body), `dynamicDialog` / `dynamicContainer` (button, URL, id), `dashboard`, `subscreens` (kind: tab/popup/wizard/stack; items with title, URL, active, disabled), `active` (the placeholder for the next level) |
| Content | `text` (style h1…dd, wrap), `image` (alt, URL), `markdown`, `html` (already converted: paragraphs, list items, quotes, link references), `numberCard`, `statusBar`, `tree` (root nodes, children URL), `placeholder` (no TUI view: browser URL), `error` |
| Custom (section 2.9) | `fragment` (the nodes of a `tui` text), `component` (name, `props`, the screen's base path for its transitions) |
| Actions | `link` (kind screen/transition/external/anchor/hiddenForm, URL, method, parameters, confirm, role primary/secondary/flat/text, `btnType`, icon as a word, badge, disabled with reason), `buttonMenu` (items) |
| Forms | `form` (name, kind single/link, transition URL, method, hidden parameters, `background-*` ids, `submit-message`, layout tree of field refs), `formList` (columns with title/align/width/sort/header field, rows of cells, first/second/last rows, totals, paging `{index,size,count,maxIndex,rangeLow,rangeHigh}`, filter summary, row selection with actions, tools: find/columns/savedFinds/exports, views) |
| Fields | `field` (name, title, tooltip, required, rules, `visibleWhen`, error) with one `w` widget of kind `textLine`, `textArea`, `password`, `dropDown` (options or `optionsUrl` + `optionsParameters`, `dependsOn`, `multiple`, `combo`, `serverSearch`, `allowEmpty`, `submitOnSelect`, `showNot`), `dateTime` (type, format, min, max), `datePeriod`, `dateFind`, `rangeFind`, `textFind`, `check`, `radio`, `file`, `display` (text, `alsoHidden`, `valueUrl`), `hidden`, `submit`, `reset`, `link`, `label`, `image`, `editable` (URL, widget) |

Icons are sent as their Material Symbols names. The client maps the common ones (add, edit, delete, search, save, close, more) to a glyph or a short word, and drops the rest. HTML content (`html-view`, and the untyped `render-mode` text a few screens use) is converted to the `html` node shape on the server by a new helper module in `moqui-core`, with an allow list of block elements, link collection and the control-character strip of D9. It uses `html5ever`, already in the lock (through `ammonia`), rather than adding a parser.

The schema is written once, as the serde types in `crates/moqui-tui/src/model/`. A section of `webroot/js/README.md` documents it as the contract, as that file documents the qvt JSON today.

### 2.4 Options, dependent options and autocomplete

- **Static options** are inline, from `getFieldOptions`.
- **`dynamic-options`, `ac-transition`, server-search and `depends-on`** are sent as a URL plus parameter names, never pre-fetched beyond the initial value's label. The server already labels the current value (`field_transition_value`, `sri/options.rs:227`).
- The client calls the transition with `Accept: application/json` and reads the same list or `{options:[…]}` shape that `dynamic_options` reads (`sri/options.rs:99`), using the model's `label-field`/`value-field`.
- `default-autocomplete-rows` (`MoquiDefaultConf.xml:671`) caps the suggestion lists.

### 2.5 Transitions and responses

- A form submit, a transition link or a row action is a request to the transition URL with `Accept: application/json`.
- `run_transition` (`render.rs:1106`) and `wants_json` (`render.rs:1432`) already answer with `send_json_redirect` (`render.rs:1640`): `screenUrl`, `screenPathList`, `messageInfos`, `validationErrors`, `currentParameters`.
- The client shows the messages, marks the fields, and then navigates to `screenUrl` in `tui`, or reloads or hides the container named by `background-reload-id`/`background-hide-id`.
- Transitions that answer through `ec.web.sendJsonResponse` (`web/mod.rs:611`) give `messages`/`errors`, used as-is. A `type="none"` response gives nothing, as in Java.
- Downloads (exports, `transition` responses with a file) come back as the body with its content type. The client saves them to disk in local and remote mode, and refuses them over SSH.
- **CSRF.** The HTTP link authenticates with the `login_key` header, which skips the session-token check (`render.rs:1937` `csrf_rejected`, `crates/moqui-core/src/screen/mod.rs:485` `authenticate`). The in-process link renders without `enforce_request_checks`, as the SSH shell does today.

### 2.6 Shell data

The client uses the endpoints the qvt shell uses, unchanged:

- `menuData` (`webroot.xml:39`) for the drawer, the tab bars' sibling lists and `renderModes`;
- `shellData` and `shellSearch` (`webroot.xml:50,59`) for the apps, providers and the palette;
- `shellCounts` for the badges (`crates/moqui-core/src/service/shell.rs:97`).

The one addition is a `tui` value in the theme of the shell data: breakpoints, drawer width, color roles and the key binding overrides (D11). So the settings of sections 1.3 to 1.5 live with the theme. They are `ScreenThemeResource` values of new resource types `STRT_TUI_BREAKPOINTS`, `STRT_TUI_COLORS` and `STRT_TUI_KEYS`. These are Enumeration rows beside `STRT_TRAY_TODAY` in `runtime/base-component/framework/entity/ScreenEntities.xml:157`.

They are read in `shell_data` (`crates/moqui-core/src/screen/shell.rs:712`) from the user's qapps theme, which is how the shell's other values are read: `sri::user_theme_id` with `QUASAR_THEME_TYPE`, falling back to `DEFAULT_QUASAR`. They are not read through `get_theme_values` (`sri/theme.rs:158`). That function takes the theme type from the request's render defs, and `/shellData` is a `webroot.xml` transition with none, so it would fall back to `STT_INTERNAL` and never find them.

`STRT_TUI_KEYS` holds one `action = chords` entry per value. `Shell::theme_value` (`shell.rs:404`) keeps only the first value, which is right for the one-value types but would drop every key override but one. So the keys are read with `sri::theme_resource_values` (`sri/theme.rs:135`), which returns all of them in sequence.

### 2.7 Security

- **Server authz, no client rights.** Each request is rendered as the logged-in user with artifact authorization on, in a fresh context on the request pool. This is the pattern of the SSH shell (`crates/moqui-host/src/ssh/shell.rs:80` `run`, `crates/moqui-host/src/ssh/mod.rs:35` `on_pool`, then `internal_login_user` at `crates/moqui-core/src/user/login.rs:231`). The client holds no permission logic. What the user may not do is not in the model.
- **No secrets in the model.** Password widgets carry no value, and credential parameters are stripped (2.3).
- **Terminal safety (D9).** The model writer (JSON escaping) and the client (before drawing) both drop C0/C1 controls except tab and newline, and the client also drops bidi override characters. OSC 8 links are written only for `http`/`https`/`mailto` URLs.
- **Remote.** The remote client refuses `http://` except to loopback. It stores a login key, never a password (section 4.3).

### 2.8 Server tests

- `crates/moqui-core/src/screen/tests/golden.rs`: a `tui` entry for each fixture in `FIXTURES`, with expected units in `crates/moqui-core/src/script/testdata/screen/tui/`. The existing checks (repeatable, parses, no globals) apply.
- A new `crates/moqui-core/src/screen/tests/tui.rs`: renders fixtures to JSON and compares with `testdata/screen/tui/render/*.json`, with `UPDATE_GOLDEN=1` like the others. Every output is also deserialized into the `moqui-tui` model types (dev-dependency with `default-features = false` and only the `model` feature, 3.1). The fixtures are the Render fixtures of `tests/widgets.rs` plus cases for an empty section, a failing section, iterate, a dialog, depends-on and a multi list.
- `runtime/base-component/example/test/Gallery.spec.js`: render every Gallery screen in `tui` as well, so every widget in the gallery (`Gallery/Widgets.xml` above all) produces a valid model. Run with `./moq test example`.
- Fragment fixtures: a valid `tui` text, one that is not JSON (an `error` node, the rest of the page intact), an `any` text alone, and a `qvt` text alone (the placeholder).
- A render test that a password value and a credential parameter never appear in the JSON, and one that a value holding `\u001b]52;…` comes out without the control bytes.

### 2.9 Custom screens: a `tui` implementation per mode

D10's rule: when a screen goes custom in one render mode, it is implemented again for each other mode it should work in. For the TUI that means a `render-mode` `text type="tui"` wherever the screen has a `qvt` text or is a `.qvue` screen. The screen's actions, transitions and data stay shared. Only the presentation is written twice.

**Two forms.**

| Form | For | How it looks in the screen |
| --- | --- | --- |
| Inline fragment | A small custom part inside an XML screen: a chart, a diagram, a hand-made card row, a banner | A `text type="tui"` in the same `render-mode` element as the `qvt` text. Its content is the JSON of one node or an array of nodes of 2.3. |
| Component screen | The counterpart of a `.qvue` screen: a board, a calendar, a chat, a mail reader | The screen's `render-modes` adds `tui` (`vue,qvue,tui`), and a `text type="tui" location="component://…/Name.tui.json"` sits beside the `qvue` text. The file is usually one `component` node with its props. |

**Which text is used.** The XSD's rule holds (`runtime/base-component/framework/xsd/xml-screen-3.xsd`, the `type` attribute of `text`, default `any`). A `tui` text is used in the `tui` mode. An `any` text is used when no text of the element matches. An `any` text is written for HTML in practice, so its output is converted to an `html` node by the helper of 2.3. A `qvt`, `qvue` or `html` text alone gets the placeholder.

**Template expansion.** The text is expanded like any other `render-mode` text (`render_text`, `crates/moqui-core/src/screen/sri/sections.rs:430`). A static file uses `template="false"`, and its component fetches its own data, as a `.qvue` does. An inline fragment usually expands values from the context.

Nuance: a bare expansion in JSON is an injection, just as a bare `${}` in qvt text is an HTML injection. Every value goes through `JSON.stringify`, which the template's JavaScript expressions already have:

```
{"t":"component","name":"chart","props":{"type":"bar","labels":${JSON.stringify(monthLabels)},"series":${JSON.stringify(totals)}}}
```

**How the macro checks the text.**

- `ScreenTui.js` writes the expanded text as a `fragment` node, so the comma rule of 2.2 holds.
- It parses the text first. A text that is not valid JSON, or not one node or a node array, becomes an `error` node naming the screen and the text's location. This follows the failure-tolerance rule: one bad fragment does not lose the page.
- The control-character strip of 2.7 applies to the parsed strings.

**Components.**

- **The registry.** A `component` node names a client widget registered in `moqui-tui`. The registry uses `inventory`, as native components do: `inventory::collect!` in `moqui-tui`, `inventory::submit!` in a component crate (`crates/moqui-core/src/native.rs:42,23`). A registration gives a name, a constructor from `props` plus a `ScreenLink` and the screen's base path, and the widget's key help for F1.
- **The framework set** lives in `moqui-tui/src/ui/components/`. It starts with what the Gallery and the qvt texts in the framework need: `board`, `calendar` (agenda list, and a month grid from lg), `chart`, `diagram` and `timeline`.
- **Add-on components.** An add-on crate (BOSS, for its chat, inbox and chatter) registers its own behind a `tui` feature of its crate. The generated app crate in `moq` forwards `tui` to every component crate that declares the feature, as it forwards the package features.
- **Data.** A component gets its data from the screen's own transitions through the link's JSON call, the same `sendJsonResponse` transitions its `.qvue` twin calls. That is what keeps the two variants to presentation only.
- **Two failure cases, both logged and drawn rather than fatal.**
  - An unknown component name draws a placeholder naming it.
  - A remote `tui-client` binary knows only the components built into it, so a server whose add-ons it was not built with shows those placeholders. The client's `=` info lists the components it knows.

**Finding the work.** `lint_screens` (`crates/moqui-core/src/script/screen_lint.rs:461`) gets two findings:

1. A screen whose `render-modes` include `tui` (explicitly or as `all`), with a `qvt`/`qvue` text that has no `tui` text in the same `render-mode` element. Also a `.qvue` screen without `tui` in its modes. This is reported as information, not an error, since a browser-only screen is allowed.
2. A `tui` text with a `${` that is not inside `JSON.stringify(`.

**The worklist today.**

- `runtime/component/ironstone-boss/screen`: 25 screens have inline `qvt` text and 18 screens use `qvue`.
- `runtime/base-component` and the `mr-*`/`Mr*` components: 14 more screens have inline `qvt` text.

`moqui lint` lists them by path. The framework's own screens are done with the framework components in Phase 3. The Gallery's `Board` and `Calendar` are the reference pair: `Board.tui.json` and `Calendar.tui.json` beside their `.qvue` files. BOSS's screens are BOSS's work, tracked in its plan.

---

## 3. Client: `crates/moqui-tui`

### 3.1 Crate and dependencies

| Module | Content |
| --- | --- |
| `model` | Serde types of 2.3, version check. Behind feature `model` (default on, and the only feature `moqui-core` enables). |
| `link` | `ScreenLink` trait: render a level, call a transition, fetch options or JSON, fetch the shell data. Plus a `FakeLink` over fixture files for tests. |
| `nav` | The level stack, history, the first-changed-level reload rule, URL ↔ path and parameters |
| `state` | Per-form edit buffers, dirty tracking, client validation, selection, scroll and focus by node id |
| `layout` | Width classes, the grid, the field-row and label rules of 1.3 |
| `ui` | Revue adapter: frame, regions, overlays, the widget adapters, own widgets in `ui/widgets/`, the component registry and the framework components in `ui/components/` (section 2.9) |
| `keys` | The keymap of 1.5 and the layered dispatch |
| `theme` | Roles, server theme mapping, color depth, NO_COLOR, ASCII |
| `run` | Two loops (section 3.5): `run_local` on `App::run`, and `run_session` on a writer and an event source |
| `logpane` | The log pane widget fed by a channel |

**Dependencies.**

- `revue = { version = "=3.10.1", default-features = false, features = ["markdown"], optional = true }`. The version is exact, as the proposal says to pin it, and is upgraded deliberately.
- **Features.** `model` has no dependencies beyond serde. `ui` (default on, and it implies `model`) enables `dep:revue` and gates the `ui`, `keys`, `theme`, `run` and `logpane` modules. `link`, `nav`, `state` and `layout` build with `model` alone. `moqui-core` takes `moqui-tui` with `default-features = false, features = ["model"]`, so its test build never compiles Revue or crossterm (D3).
- `serde`, `serde_json`, `jiff` and `unicode-width` are already in the workspace.
- `regex` (already in the lock) checks the client rules.
- No Tokio, no reqwest (D4).

**New crates in the lock.** crossterm 0.29, unic-emoji-char and textwrap are new. Each needs a line in `plans/archive/License-Review.md` and must pass `deny.toml`: Revue is MIT, crossterm MIT, textwrap MIT, unic-* MIT/Apache.

**Workspace.** `Cargo.toml` members get `crates/moqui-tui`, and `[workspace.dependencies]` gets `moqui-tui` and `revue`. The workspace comment's toolchain note becomes "Revue 3.x requires Rust 1.95". The local toolchain is 1.99, and the repository pins none.

### 3.2 Interpreting the model

- **Rebuilt every frame from our own data.** Revue calls `View::render(&self)` every frame and rebuilds the tree (`src/widget/traits/view.rs:121`). The interpreter keeps the model, plus the per-id state of 3.1, in one `TuiApp` struct, and its `render` builds the view tree from them each frame. Stateful Revue widgets (`Input`, `TextArea`, `DataGrid`, `Tree`) live in that struct, keyed by node id. They are drawn with `ctx.render_child(&widget, rect)` (`src/widget/traits/render_context/mod.rs:225`), because `Stack::child` wants `'static` children.
- **A new model for a level replaces that level's nodes only.** Widget state whose id survives is kept. When a transition reloads the screen, the submitted form's buffers are dropped and other forms' unsent edits are kept, with their dirty marks.
- **Focus order** is the model's reading order, computed once per model change. The focus list is ours. Revue's DOM `:focus` is set to match only so its CSS hover and focus styles apply. Tab navigation in the builder stays off (`src/core/app/builder.rs:177`).
- **Visible-when and depends-on** are evaluated in `state` on each change, and an options fetch is queued when needed.

### 3.3 Key dispatch

Each key event goes through the layers in order: modal, then confirm, then open popup (a drop-down list, the palette), then the focused widget's adapter, then the screen keymap, then the global keymap. The first layer that handles it stops it. An adapter's `handle_key` calls the Revue widget's own handler where the widget has one (`Interactive`, `src/widget/traits/interactive.rs:60`; `Modal` returns the button index at `src/widget/feedback/modal/key_handling.rs:30`). Otherwise it implements the keys itself, as `examples/form_validation.rs:135` does.

**Paste.** Revue's `Terminal::init` does not enable bracketed paste (`EnableBracketedPaste` is only in the unused `CrosstermBackend`, `src/runtime/render/backend/crossterm.rs:97`), so `Event::Paste` likely never fires. The local loop enables it itself after init and turns it off on exit. The session loop's input decoder reports pasted text as one event, so a paste with newlines does not submit a form.

### 3.4 Layout

`layout` turns the model plus the width class into a tree of boxes with fixed cell sizes, with no Revue import. `ui` builds the Revue `Stack`/`Grid`/`Border` nodes from that tree. Revue's CSS applies width/height/margin/gap but not padding, flex or grid properties (`docs/guides/styling.md:182-200`), so all layout is computed in code. CSS is used for colors and borders only, through `AppBuilder::css` (`builder.rs:226`), with the role styles generated from `theme`. A resize recomputes the class and the layout. The model is not re-requested, except when a list's page size or `_uiType` changes.

### 3.5 Event loops

**Local (`run_local`).** `App::builder()` with mouse capture on, then `App::run(view, handler)` (`src/core/app/event_loop.rs:62`). The handler is the dispatch of 3.3. On `Event::Tick` it drains finished link requests and returns true when one changed the state. That is the pattern of `examples/worker_basic.rs:282`. Revue polls every 16 ms (`src/runtime/event/reader.rs:42`), which is fine for one local user.

**Session (`run_session`, for SSH and the browser terminal).** This loop takes:

- a `Write` for output, wrapped in `Terminal::with_size` (`src/runtime/render/terminal/core.rs:34`);
- a byte source for input;
- the initial size;
- a resize signal.

It must not touch the process terminal. It skips `Terminal::init`'s raw mode and terminal size query, the signal handlers and the panic hook. It writes the alternate screen, cursor and mouse-mode sequences itself. Input bytes go through a small ANSI decoder into Revue `Event`s: keys with modifiers, SGR mouse, bracketed paste, focus. The decoder is either an own parser over the `vte` crate, or crossterm's parser if the spike finds a public entry. The loop blocks on "input bytes, resize, request done or the next deadline" rather than polling. The deadline is the earliest of the timed things the local loop gets from its 16 ms tick:

- the success message's 3 s expiry (1.7);
- the 60 s counter read (1.6);
- the title-line clock's next minute;
- the spinner's next frame while a request runs.

So an idle session wakes about once a minute, and SSH and browser sessions behave as local mode does.

**Drawing.** The loop needs the same draw path `App::run` uses: DOM, CSS, layout, diff and dirty-rect output. Today that path is reachable from outside the crate only through `PipelineHarness` (`src/testing/pipeline.rs:42`, `send`/`send_with` at :252), which draws into a `Vec<u8>`. The spike (Phase 1) settles which of three routes the loop uses:

1. An upstream change to Revue: a public `App::run_on(terminal, events)`, or making the harness's draw step public. **Preferred.**
2. Driving `PipelineHarness`, then copying its byte buffer to the writer after each frame. Workable, but it leans on a testing API.
3. The fallback of D6: spawn `moqui tui --attach <socket>` on a server-side pseudo-terminal (`portable-pty` or `rustix` openpty), with the parent bridging bytes and window size. The child reaches the parent through the HTTP link over a Unix socket with a one-time login key, since an in-process link is impossible across processes. This costs a process per session and a fork of a large binary, which is why it is the fallback.

### 3.6 Background requests

Every `ScreenLink` call runs off the UI thread. The local loop uses Revue's `WorkerHandle::spawn_blocking` (`src/state/worker/handle.rs:41`), which needs no `async` feature. The session loop uses the host's request pool through the in-process link. Results come back on a channel, and the loop marks the state dirty and redraws.

- **Superseding requests.** A request carries a sequence number per purpose (level, options for field X, suggestions), and a late answer to a superseded request is dropped. So fast typing in an autocomplete, or paging twice, shows only the last answer.
- **One in flight per purpose.** Dropping late answers is not enough on its own, since every request sent still holds a worker and a queue slot. The host's request pool is shared with HTTP: 8 workers and a queue of 64 by default (`crates/moqui-core/src/service/request_pool.rs:30-31`), and it refuses work with "request queue is full" once the queue is full. A held PgDn or F5 at key-repeat speed would fill it and refuse browser users. So each purpose has at most one request in flight and holds only the newest pending one, sent when the running one returns. The requests in between are never sent.
- **Request errors.** A network or 5xx error is a message-line error and leaves the screen as it was. A 401 opens the login dialog (1.7).

### 3.7 Own widgets

These are written because Revue lacks them or gets them wrong, as found in the survey.

| Widget | Why | Built on |
| --- | --- | --- |
| `DropDown` | `Select` has labels only; `Combobox` cannot change options after build | `OptionList`, `Input`, `VirtualList` |
| `FieldInput` | max length, mask, number alignment, changed and error marks, bracket look | `Input` |
| `DateField` | today and now are fixed in Revue; it has no string parsing; it uses its own `Date` type | `DateTimePicker`, `jiff` |
| `ListTable` | server paging, our sort/filter, row forms, selection column, stacked columns, sub-rows | `DataGrid` (single-line rows), own row renderer for stacked columns |
| `LazyTree` | expand fetches children | `Tree` |
| `StatusFlow` | the status-bar stages | `Text` |
| `FieldLabel` | dot leaders, `*`, truncation | `Text` |
| `LogPane` | levels, filter, follow | `LogViewer` (`src/widget/data/log_viewer/`) |

Each own widget that fixes a Revue bug gets an upstream issue, and the fix is dropped from the wrapper when a pinned upgrade carries it (section 7).

### 3.8 Markdown editing: now and later

**Now (D12).**

- `editor-type="md"` text areas edit the Markdown source in a `TextArea`, with F4 toggling a preview drawn by the `markdown-view` renderer.
- `html`, `rich` and `rich-light` fields are read-only, converted to text, with "Edit in browser".
- Rich text never round-trips through the terminal, because each conversion between HTML and Markdown loses something: attributes, spans, table layouts, mentions and record links.

**What exists to build on** (surveyed Oct 20, 2026).

- **Revue's `RichTextEditor`** (`src/widget/form/rich_text_editor/`) is the nearest fit.
  - It has block types (headings, quote, code block, bullet and numbered lists, rule), inline formats (bold, italic, underline, strike, code), a toolbar, Markdown shortcuts, a preview mode and undo.
  - It converts with `from_markdown` / `to_markdown` (`core.rs:26,53`).
  - Its import is one block per line (`markdown.rs`). Inline markup on import stays literal text, so `**bold**` is kept but not drawn bold, and the round trip is close to lossless in text but styled at block level only.
  - Tables, nested lists and reference links would need a fidelity check.
- **Elsewhere.** The Ratatui ecosystem has Markdown renderers ([ratatui-markdown](https://docs.rs/crate/ratatui-markdown/latest), [ratkit](https://docs.rs/crate/ratkit/latest)) and editors with a split preview or inline syntax highlighting ([mdt / mdtui](https://docs.rs/mdtui/0.3.0/mdt), [smarkdown](https://docs.rs/crate/smarkdown/0.2.0), [midetor](https://rust-digger.code-maven.com/crates/midetor)). None edits in place WYSIWYG, and none is a Revue widget.

**Later: a styled-source editor.** The likely good-enough two-way editor is not WYSIWYG but **styled source**:

- The Markdown text is the value and is edited directly, so nothing is ever converted and nothing can be lost.
- Each line is drawn with the same styles as `markdown-view`: headings bold and in the heading color; `**bold**`, `_italic_` and `` `code` `` styled, with their markers dimmed; links underlined; list bullets and quote bars in the muted color; fenced code in the code color.
- The styling comes from `pulldown-cmark`'s source offsets over the visible lines. pulldown-cmark is already a dependency through Revue's `markdown` feature, so no tree-sitter grammar is needed.
- Toolbar keys wrap the selection (Alt+B, Alt+I, Alt+K for a link), and F4 still toggles the full preview.

It would be an own widget over `TextArea`, or a contribution to Revue's `RichTextEditor` as a "source" view mode. Before choosing, Phase 4 checks `RichTextEditor`'s round trip on the Gallery's Markdown samples and the wiki pages. If its output equals its input for all of them, it may serve as is. If the HTML fields later get a lossless Markdown form on the server, they can use the same editor.

---

## 4. Channels

### 4.1 `moqui tui` (local)

- **CLI.**
  - A match arm in `dispatch` (`crates/moqui-host/src/lib.rs:281`).
  - The name in `usage_error` (`lib.rs:380`) and in `is_known_command` (`crates/moqui-host/src/usage.rs:307`). Without the last, `moqui tui --server=https://…` is taken for server options, because it holds `=` (`looks_like_server_option`, `usage.rs:326`).
  - Help text in `usage.rs` beside the other commands.
- **Start.** A new `start_local` in `lib.rs` boots the runtime with no listeners. It returns a handle whose `factory()` the in-process link uses, and dropping it shuts down on exit.
  - `start_server` (`lib.rs:196`) cannot do this. It always binds the plain HTTP listener and the cluster listener (`serve_thread`, `lib.rs:885`), and `boot_server_factory` (`lib.rs:985`) itself starts the SSH server, the cluster edge listener (8444 by default) and the SMTP listener. On a machine where a server already runs, the TUI would fail on a port before it reached the DB lock check below.
  - So `boot_server_factory` takes a choice of listeners. With none, it skips those three, and `start_local` binds no HTTP port.
  - On drop, the handle does what `serve_thread` does after its listener stops (`lib.rs:942-960`): flush the artifact hits and the sessions, and stop log shipping. Then it drops the process.
  - `--serve` uses `start_server_in` with the configured address instead.
- **Logging.** Before the boot, logging is installed with stderr off, since any stderr line would tear the screen.
  - `install_with_console(None)` (`crates/moqui-host/src/logging.rs:84`) does the opposite. `None` means stderr gets every line at `moqui_log_level` (`logging.rs:105`), and no `Level` turns stderr off, since `event` writes whenever `level <= console` (`logging.rs:546`).
  - So `logging.rs` gets `install_without_console()`. Its `LineSubscriber` holds `console: Option<Level>`, and `event` skips stderr when it is `None`.
  - It has to be the first install in the process, because the `OnceLock` makes any later install a no-op.
  - The log files are written as usual, and the log pane below shows the lines.
- **Log pane.** `LineSubscriber::event` (`logging.rs:506,528`) gets a third sink: a static list of bounded channels that `logging::tap()` registers. `moqui-tui` gets the receiver. The subscriber is hand-written and has no layers, and `tracing-subscriber` is not in the lock, so this is a tap rather than a layer, as the proposal assumed.
- **Login.** The TUI's login dialog, through `login_user`. On an instance whose purpose is dev, `--user <name>` logs in without a password, as the test fixtures do.
- **Flags.** `--path <screen path>` for the first screen, `--ascii`, `--no-color`, and the HTTP/SSH listeners off by default (`--serve` turns them on, so a developer can have the browser beside the TUI).
- **Already running.** If the runtime directory is already served by a running instance (the DB lock), the command says so and suggests `--server`.

### 4.2 SSH `apps`

- **Command and permission.** `apps` is a new command in `crates/moqui-host/src/ssh/shell.rs`, in the `match` and the `HELP` text. It is also the whole session for a user who has the new `SSH_APPS` permission but not `SSH_SHELL`. `SSH_APPS` is a `UserPermission` in `runtime/base-component/framework/data/SecurityTypeData.xml:58-61` beside `SSH_SFTP`/`SSH_SHELL`, granted to `ADMIN` by default. It gives no rights beyond opening the TUI: every screen is still checked as that user.
- **Where the permission is checked.** `ADMIN` has `SSH_SFTP` and `SSH_GIT`, but `SSH_SHELL` only through `ADMIN_ADV`. So the typical user of `apps` has no shell, and every place that gates on the shell today needs to know about `apps`:
  - `Permissions` and `ssh_permissions` (`crates/moqui-host/src/ssh/server.rs:360`) get `apps`, and `Connection` gets `can_apps`.
  - `Connection::accept` (`server.rs:371`) lets in a user who has only `apps`. It also leaves such a user out of the git-only rule at `server.rs:380`, which refuses password logins for anyone without SFTP or SHELL.
  - `shell_request` (`server.rs:613`) accepts `can_apps`. When `can_shell` is false it starts `apps` directly rather than the `moqui>` prompt.
  - `exec_request` (`server.rs:571`) accepts `can_apps` for the command `apps` alone, so `ssh -t host apps` works when a pty was requested. Other commands still need `can_shell`.
- **Pty.** `pty_request` (`server.rs:489`) keeps only that a pty was asked for and drops the size and `TERM`. It must store `term`, columns and rows on the `Connection`. A `window_change_request` handler is added (there is none today) and forwards the size to the session loop. Without a pty, `apps` refuses with a one-line message.
- **Stream.** The session takes over the channel stream that `interactive` (`server.rs:691`) reads line by line today. That loop drops escape bytes, so `apps` hands the stream to `run_session` and gets it back on exit, returning to the `moqui>` prompt when the user came from the shell.
- **Sessions.** One `run_session` per channel, on its own thread (the loop is synchronous). Requests go through `on_pool` with `internal_login_user` per request, as the shell's commands do.
- **Session limit.** `ssh_max_sessions` (`server.rs:142`) does not bound the threads. It counts logged-in connections (`log_in`, `server.rs:219`), and `channel_open_session` (`server.rs:473`) accepts any number of channels on one connection. So one login could open hundreds of `apps` channels, each a thread, or a process under route 3.
  - A new `tui_max_sessions` property bounds the live `run_session`s, in total and per user, with small defaults.
  - SSH channels and `/termws` sockets (4.4) take slots from the same count, and a slot is freed when its session ends.
  - Over the limit, `apps` refuses with a one-line message, and the socket closes with a reason.
- **Downloads and uploads** are off over SSH (1.6, `file` and exports). The log pane is off, unless the user has the permission to read logs, in which case it shows only lines at WARN and above. The proposal leaves this to permissions, and it is Phase 4.

### 4.3 Remote client (`moqui tui --server <url>`)

- **Link.** The HTTP link lives in `moqui-host`, which already depends on blocking `reqwest` (workspace `Cargo.toml:78`). It sends:
  - `GET <url><path>?renderMode=tui` for a level (`pick_render_mode`, `render.rs:863`, reads the `renderMode` parameter);
  - transitions with `Accept: application/json`;
  - the `login_key` header (`crates/moqui-core/src/user/login.rs:348` `apply_header_keys`).
- **No runtime.** The remote client boots no factory. It runs `run_local` with the HTTP link only. A client-only build (`--no-default-features --features tui-client`) needs no database or server pieces; the gating of `moqui-host`'s modules allows it (`lib.rs:23,28-66`).
- **Login.** The client prompts for username and password (and OTP through `sendOtp`/`verifyOtp`, `crates/moqui-core/src/screen/dispatch.rs:54-55`). It posts them once to the `login` REST transition, then asks for a login key (`get_login_key`, `user/login.rs:305`, with the `login-key` expiry of `login_key_expire_hours`, `user/policy.rs:67`). The key is stored in `~/.config/moqui/tui/<host>.toml`, mode 0600, with its expiry, and the password is forgotten. `--forget` removes it. A missing server-side transition to mint a key is the one server addition this needs, if `login` does not already return one.
- **Downloads** go to `~/Downloads` (or `XDG_DOWNLOAD_DIR`). The file picker browses the user's machine, and uploads are multipart posts.

### 4.4 Browser terminal (if wanted)

- **Server.** A `/termws` WebSocket route beside `/notws` (`crates/moqui-host/src/http.rs:211`), exempt from the request timeout as `/notws` is (`http.rs:429`). It is authenticated by the session cookie as `notws::upgrade` is (`crates/moqui-host/src/notws.rs:85`), and needs `SSH_APPS`-equivalent permission (the same permission, renamed `TERMINAL_APPS` if the SSH name reads wrong). Its bytes feed `run_session` exactly as SSH does. Because it is exempt from the request deadline, each socket takes a slot of `tui_max_sessions` (4.2) before its session starts.
- **Page.** A small page with xterm.js, vendored through `libs/refresh.mjs` with a manifest line and a license review entry (MIT).
- **Placement.** Phase 4, only on demand, as the proposal says.

### 4.5 Headless

The same interpreter draws through `PipelineHarness` at a given size with a `FakeLink` or the in-process link. This gives the client snapshot tests (section 6) and a `moqui tui --snapshot <path> --size 180x50` command that prints the screen text. That command is useful for CI and for checking a design without a terminal.

### 4.6 Packaging

- **The `tui` feature.** A new `tui` feature in `crates/moqui-host/Cargo.toml` enables `dep:moqui-tui`. `enterprise` and `maximal` include it. `lite` leaves it out, because `lite` is the small server build, and a `lite` user runs the remote client from any other build.
- **The `tui-client` feature.** A `tui-client` feature builds the remote client alone.
- **Forwarding.** `crates/moqui/Cargo.toml` forwards both features.
- **The generated app crate.** The template in `moq` (the `[features]` block near `moq:1067-1074`, `package_features` at `moq:941`) gets them too, so `./moq build` of an add-on set carries the feature.
- **Checks and docs.** `scripts/check-packages.sh` checks the new sets, and `docs/Packaging.md:18-42` gets the rows.

---

## 5. Phases

These are the proposal's four phases, each ending in something usable. No dates are set.

### Phase 1 — Spike (go or no-go on Revue)

1. **Crate and model.** Create `crates/moqui-tui` with the pinned Revue, and the `model` types for: box, text, link, form-single with text-line, drop-down and date-time, and form-list with paging.
2. **Hand-built screens.** Write two fixture models: the Gallery's List screen (find form plus list) and its Detail screen (two boxes plus a list). Write them by hand from the screens' qvt output. Draw them with `run_local` and a `FakeLink` at 72, 100 and 180 columns, and check the layouts of 1.2 and 1.3.
3. **The session loop.** Build a `run_session` prototype over russh in a throwaway host test, then settle the route (3.5, routes 1–3): open an issue or PR upstream for a public run-on-writer entry, and measure route 2 with a 180×50 screen. Check over SSH: resize, mouse, paste, F-keys in xterm, kitty, GNOME Terminal, tmux and the Linux console (Alt and F-key sequences differ).
4. **Unicode.** Confirm Revue's width handling with CJK and emoji in a list cell (`src/utils/unicode.rs:191`).

**Exit:** both fixture screens usable at all three widths by keyboard alone, the SSH route chosen, and a short findings note added to this plan. **No-go** if the session loop needs a fork of Revue we would carry. Then the Ratatui + rat-salsa fallback of the proposal starts from the same `model` and `link` modules.

### Phase 2 — Core: browse and edit records in one real app

1. **Server.** `ScreenTui.js` for the elements of 1.6 minus views, trees, charts and wizard/stack. The registration of 2.1, the error kinds of 2.2 and the envelope of 2.3. `text type="tui"` fragments with their JSON check, and the placeholder where a `qvt` text has no `tui` twin (2.9). The two lint findings of 2.9, and the worklist they print. The goldens and render tests of 2.8, and the Gallery spec in `tui`.
2. **Client.** `nav`, `state`, `layout`, `keys` (with the theme's `STRT_TUI_KEYS` overrides) and `theme`. The frame with title, drawer, message and key lines. Forms with validation. Lists with paging, sort, the find dialog and selection. Drop-down with inline and URL options. Date fields. Dialogs. Transitions with messages and errors.
3. **Local channel.** `moqui tui` with the in-process link, login, and the log pane with its logging tap.
4. **Docs.** `docs/TUI.md` first version: start, keys, widths.

**Exit:** in the example app and one Mantle app, a user finds, opens, edits and saves records, uses dialogs and dependent drop-downs, and pages lists, all by keyboard, at 100 and 180 columns. Every Gallery screen renders a valid model.

### Phase 3 — Shell and dynamics: a full admin task over SSH

1. **SSH.** `apps` over SSH with the chosen session route, `SSH_APPS` at every gate of 4.2, `tui_max_sessions`, pty size, window change, and the guard against downloads and uploads.
2. **Dynamics.** Autocomplete, `depends-on` everywhere, `dynamic-dialog`/`dynamic-container` with reload and hide ids, `editable`, multi lists with inline editing, stacked columns, row actions, saved finds and the columns dialog.
3. **Shell.** Palette, quick create, counters, tray (as a modal), help with screen documents, message history.
4. **Structure.** Trees, wizard and stack panels, `container-split`, `status-bar`, `number-card` and `dashboard`.
5. **Components.** The component registry of 2.9 and the framework set (`board`, `calendar`, `chart`, `timeline`). The Gallery's `Board.tui.json` and `Calendar.tui.json` as the reference pair. `tui` texts for the framework's own inline-`qvt` screens on the lint worklist, and the `tui` feature forwarding to component crates in `moq`.

**Exit:** an administrator connects by `ssh user@host`, lands in the apps, and completes a Tools task (for example: find a user, change their groups, run a service job, read its result) without leaving the TUI. The Gallery's Board and Calendar work in the TUI through their components. The lint worklist has no framework screen left on it.

### Phase 4 — Reach and polish

- **Remote.** Remote client mode with stored login keys, and the `tui-client` build.
- **Theming.** Themes from the server theme, dark/light detection, `NO_COLOR` and ASCII verified.
- **Views and diagrams.** List views (board, agenda) on the Phase 3 components, and the Mermaid evaluation for the `diagram` component.
- **Markdown editor.** The `RichTextEditor` round-trip check and the styled-source editor of 3.8.
- **Push.** Counter updates in place of polling: the in-process link subscribes to the notification topic directly; the remote client uses `/notws`.
- **CI.** Snapshot tests in CI (section 6), the `--snapshot` command.
- **On demand.** The browser terminal; the log pane over SSH for permitted users.

---

## 6. Client tests

| Test | How | Where |
| --- | --- | --- |
| Model round trip | Every server golden from 2.8 deserializes and re-serializes equal | `moqui-core` render tests |
| Layout | `layout` computed for fixture models at 72/100/120/153/180/240 columns, asserted as a text outline of boxes and widths | `crates/moqui-tui/tests/layout.rs` |
| Snapshots | `PipelineHarness` draws fixture models at 72×24, 100×40, 180×50. The screen text is compared with checked-in files, rewritten with `UPDATE_GOLDEN=1` as in the rest of the repository (not Revue's `REVUE_UPDATE_SNAPSHOTS`). One set uses ASCII and no color. | `crates/moqui-tui/tests/snapshots/` |
| Key flows | `FakeLink` records requests. Scripted keys check focus order, Enter submit, F4 prompt, F7/F8 paging, Esc and dirty-leave, palette, numbered links, that a superseded answer is dropped, and that a held PgDn sends at most one request in flight and one pending. | `crates/moqui-tui/tests/keys.rs` |
| Input decoder | Byte sequences of xterm, kitty, tmux and the Linux console to events, paste with newlines, split sequences across reads | unit tests in `run` |
| Sanitizing | Control and bidi characters in every string field of a model are not drawn | unit test |
| In-process link | A real factory: login, render, submit with a validation error, options, 401 | `crates/moqui-host/tests/it/tui.rs` |
| SSH | pty request with size, `apps` start, window change to a new size, exit back to the prompt, `SSH_APPS` refused without permission. A user with `SSH_APPS` but no `SSH_SHELL` logs in by password and lands in `apps` from `ssh user@host` and from `ssh -t host apps`. Channels past `tui_max_sessions` are refused. | extend `crates/moqui-host/tests/it/ssh.rs` |
| Gallery | Every Gallery screen at 180×50 as a snapshot in the example component's tests, with a fixed date, so a style change shows in the diff as the screenshots do for qvt. Board and Calendar through their components. | `runtime/base-component/example/test/` |
| Components | Each framework component drawn from fixture props at two widths, its keys, and the unknown-name placeholder | `crates/moqui-tui/tests/components.rs` |
| Theme keys | `STRT_TUI_KEYS` overrides merge, a bad entry is skipped and logged, a binding of Esc/Enter/Tab is refused, F1 and the key line show the merged keys | `crates/moqui-tui/tests/keys.rs` |

---

## 7. Risks and open questions

These update the proposal's list with what the survey found.

| Item | State after this plan |
| --- | --- |
| API churn | Pin `=3.10.1`. Upgrade only with the snapshot suite green. Revue's `STABILITY.md` marks `FormState` and `Store` as Evolving, and this plan uses neither. |
| One maintainer | Unchanged. Contained by D3 and D5: the model, link, nav, state and layout modules have no Revue import. |
| Toolchain | Revue needs 1.95; the workspace needs 1.94 and builds on 1.99. No conflict. |
| Terminal I/O (was the proposal's "pluggable backend?") | **Answered: no.** Revue has a `Backend` trait that `Terminal` does not use, and `App::run` is tied to the process. The Phase 1 spike picks route 1, 2 or 3 of 3.5. |
| Date picker | **Exists**, with the fixed today/now bug and no parser: the `DateField` wrapper. |
| Inline table editing | **Exists** in `DataGrid`, without a validation hook: the `ListTable` wrapper owns validation. |
| Large lists | **Answered:** `DataGrid` and `Table` draw only visible rows. Server paging stays, with the page sized to the screen. |
| Key routing | Revue does not route keys. The interpreter does (D5), which the JSON model needs anyway. |
| Paste | Not enabled by Revue. Enabled by our loops (3.3). |
| Idle cost | Revue ticks every 16 ms. The session loop blocks until input or its next deadline, about once a minute when idle (3.5). |
| `.qvue` screens and inline `qvt` | **Decided (D10):** each gets a `tui` implementation, and until then a placeholder with a browser link (2.9). For the BOSS Comms Inbox (a `.qvue`) to work as an email client in the terminal, BOSS writes its `tui` texts and its components (message list, reader, compose on the 3.8 editor) behind its crate's `tui` feature. |
| Custom components in a remote client | A `tui-client` binary has only the components it was built with. Shown as placeholders, and listed by `=` (2.9). |
| Security over SSH | Per-request login as the user, `SSH_APPS`, no file transfer, sanitized output (2.7, 4.2). |
| Rich text | **Decided (D12):** read-only with "Edit in browser". A two-way Markdown editor in the `markdown-view` look comes later (3.8). |
| Preferences | **Decided (D11):** breakpoints, colors and key bindings are per theme. Only the drawer's open state is per user. Per-user bindings and their UI wait. |

**Upstream asks for Revue** (issues first, PRs where small):

- a public run-on-writer entry with an external event source;
- bracketed paste in `Terminal::init`;
- real `Date::today`/`Time::now`;
- values on `Select` options, and mutable `Combobox` options;
- a lazy-load hook on `Tree`;
- word wrap in `Text`;
- widgets reading the theme signal;
- a source view mode for `RichTextEditor`, if 3.8 goes that way.

---

## 8. Bookkeeping

- `plans/README.md`: list this plan under the active plans, and keep the proposal in `proposals/`. Per the README conventions, the proposal moves to `archive/` when Phase 4 is done, and the parts still open go to `Remaining-Work.md`.
- `plans/Future-Plans.md`: a paragraph for the proposal pointing here.
- `plans/Parity-Mapping.md`: `tui` has no Java counterpart, so it goes in the "not in Java" list with `ScreenTui.js` and `crates/moqui-tui`.
- `docs/TUI.md` (new): the user guide (channels, keys, widths, themes), and for screen authors: writing a `tui` text, the component props of the framework set, and registering a component from a component crate. `docs/Packaging.md`: the features. `webroot/js/README.md`: the model contract section.
- `plans/archive/License-Review.md` and `deny.toml`: the new crates of 3.1, and xterm.js if 4.4 is built.
- `xml-screen-3.xsd`: the `text` `type` documentation names `tui` beside `qvt` and `qvue`, and states D10's rule.
- In the ironstone-boss repository (its own git repo): the style guide has the rule since Oct 20, 2026, in principle 3, §4.8 "The terminal twin", the Terminal row of §7, the §8.1 checklist and a §9 reference. Still to do there: BOSS's lint worklist goes into its plan once the lint exists, and design-doc-guide's **Impl** column may name the `tui` twin of a `qvue` screen.
