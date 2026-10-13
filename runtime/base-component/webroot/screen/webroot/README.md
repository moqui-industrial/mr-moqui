# Static files of the webroot component

Everything in this directory is served by the path walk as a file under the `webroot` screen (`/css/moqui-html.css`,
`/js/moqui-html.js`, `/images/...`, `/favicon.ico`, `/libs/...`). The rules are in `screen/static_files.rs`: no hidden
files, no `..`, no screen or template sources (`.xml`, `.ftl`, `.fancy`, `.groovy`, `.properties`, `.sql`), nothing that a link
leads out of this directory, a media type by extension, a weak `ETag` and 304, `Cache-Control` of a year for `*.min.*` and for
a request with a `v` parameter and an hour with revalidation for the rest. The host compresses text with gzip.

## Moqui files

| Path | What |
|---|---|
| `css/moqui-html.css` | style sheet of the `html` mode (light and dark, responsive, print) |
| `js/moqui-html.js` | client of the `html` mode. `js/README.md` lists the data attributes. `js/moqui-html.test.js` runs under Node |
| `images/`, `MoquiLogo100.png`, `favicon.ico` | logo and icon |

## Library paths the html mode references (vendored by the browser libs slice)

| Library | Paths |
|---|---|
| Font Awesome 4.7 | `/libs/fontawesome/css/font-awesome.min.css` and its `fonts/` directory beside `css/` (`/libs/fontawesome/fonts/fontawesome-webfont.woff2` and the other formats) |
| Tom Select (complete build) | `/libs/tom-select/tom-select.complete.min.js` and `/libs/tom-select/tom-select.css` |
| SortableJS | `/libs/sortablejs/Sortable.min.js` |

The theme (`WebrootThemeData.xml`) links Font Awesome, the style sheet and `moqui-html.js`. A widget that needs Tom Select or
SortableJS adds its paths to `footer_scripts` and `footer_stylesheets`, which `Footer.html.fancy` writes. A fragment that
a page loads later has no footer, so the client loads the library itself the first time it needs it.
