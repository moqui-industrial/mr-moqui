# Markdown to PDF review

> Archived 2026-10-18: evaluation only. PDF stays on Chromium (`html-print`, `crates/moqui-host/src/pdf.rs`). The `.md.fancy` table fix found here is in `crates/moqui-core/src/script/template.rs` (`lift_tag_rows`), and the print findings were fixed the same day (see [Repository findings](#repository-findings)). The ideas under [Tabled ideas](#tabled-ideas) may be picked up separately.

Reviewed 2026-10-18. The question was whether a native Rust engine that takes Markdown could replace the Chromium renderer for print documents, or sit beside it as a lighter default. The candidate was `pdfrs`. The requirements were the print templates in `runtime/component/MrSimpleScreens/template/`. The test renders were done in scratch crates outside this repository, with the samples described below.

## Verdict

`pdfrs` 0.1.13 cannot lay out these documents. It loses table rows that do not fit on a page, and it has no column widths, page header or footer, or positioning. Markdown as the source of print templates is workable without it: `.md.fancy` already converts Markdown to HTML before the template runs, so a Markdown template with a print stylesheet goes through the existing Chromium path with no new crate. If a Chromium-free engine is ever needed, Typst is the one that did the job, at a large size cost. For PDF manipulation (merge, split, metadata) `lopdf` is the small, established choice.

## Requirements: the print templates

There are 18 `.print.html.fancy` documents: FinancialAccountStatement, Invoice, PaymentCheck, PaymentDetail, ReceivableStatement, Order, PartyBadge, CheckReceiptRegister, ShipmentBillOfLading, ShipmentBoxTypes, ShipmentByPackage, ShipmentInsert, ShipmentLoadPickAndPack, ShipmentLoadPick, ShipmentPack, ShipmentPick, ProductionRunPick, TeamTimesheet. Between them they need:

- Fixed column widths on every table.
- Tables that break across pages with the header repeated (picks, packs, statements, timesheets, long invoices).
- Absolute positioning: the check fields for pre-printed stock (`PaymentCheck-checkBody.html.fancy`), the 3.37 x 2.125 in badge, the header blocks of Invoice, Pack and Insert.
- Page sizes other than letter (the badge) and landscape (CheckReceiptRegister).
- Code128 barcodes as inline SVG from `ec.resource.code128Svg`, in 12 places.
- Images: logo, signature, face photo.
- A running "Page N of M" footer. The XSL-FO originals had one in 12 templates; the HTML ports do not (see [Repository findings](#repository-findings)).

## pdfrs 0.1.13

Apache-2.0. First published 2026-02-09, 0.1.13 on 2026-10-05. One author, 7 GitHub stars, 4 reverse dependencies on crates.io, about 38,000 lines of Rust, edition 2024.

**Dependency.** The default features add `clap` and `rayon`, so it would be `default-features = false`. With that the tree is 76 crates, 19 of them not in `Cargo.lock` at a compatible version: `pdfrs`, `syntect`, `fancy-regex`, `plist`, `yaml-rust` (unmaintained, RUSTSEC-2024-0320), `linked-hash-map`, `subsetter`, `ttf-parser`, `write-fonts`, `bit-set`, `bit-vec` 0.8, second copies of `read-fonts`, `skrifa`, `font-types` and `quick-xml`, and the older RustCrypto line (`aes` 0.8, `cbc` 0.1, `cipher` 0.4, `inout` 0.1, `block-padding` 0.3) next to the workspace's `aes` 0.9 and `cbc` 0.2. A stripped release binary that renders one Markdown file was 4.9 MB; a cold release build took 20 s on 12 threads.

**Layout test.** An invoice and a 70-row pick list in GitHub-flavored Markdown, rendered with `elements::parse_markdown` and `pdf_generator::generate_pdf_bytes(…, "Helvetica", 10.0, PageLayout::portrait())`:

- Tables never split. `render_table` reserves the whole table height (`ensure_space`) and draws it as one block, so a table taller than a page starts on a new page and runs off its bottom. Rows 17 to 70 of the pick list were drawn at negative y and lost, with no error.
- Column widths come from character counts and cannot be set. In the invoice item table, product and lot codes overlapped, and the amount and total columns ran together.
- The table style is fixed: `render_table` uses `TableStyle::default()`, so every table has a grid, a shaded header, striped rows and bold cell text, and gets a `Table N.` caption under it.
- No header or footer content. The only page furniture is a centered page number (`add_page_number`), so there is no "Page N of M" and no running footer text.
- No absolute positioning, so the check and the badge cannot be done.
- Its own line-based Markdown parser, not CommonMark: `<br>` printed literally, raw HTML is not supported, and a wiki page would print differently from how `moqui-lib/src/markdown.rs` (pulldown-cmark) shows it. The HTML input (`html::parse_html`) maps to the same element list, so it has the same table limits.
- Fonts: a Unicode font is embedded only when `PDFRS_UNICODE_FONT_PATH` (process-wide) names one or a macOS font path exists. On Linux it uses the base-14 fonts, and the em dash was dropped.
- Spaces were lost inside bold text ("Thankyou", "Paymentis"), in the drawn text and in extracted text.

Fixing these is a layout engine, not an upstream patch.

## chromiumoxide

Not applicable. The host never used it: its untagged-enum message parsing fails under the workspace's `serde_json` `arbitrary_precision`. The `pdf` feature is the host's own DevTools client (`crates/moqui-host/src/pdf/cdp.rs`, about 250 lines) on `tokio-tungstenite` and `futures-util`, which axum's `ws` feature already links. It adds no crate, and Chromium is an external binary. It stays on.

## Markdown through the existing path

The same invoice and pick list Markdown, converted with pulldown-cmark 0.13 (the workspace's version and options) and wrapped with about 15 lines of print CSS (`@page` size, margins and an `@bottom-center` page counter, body font, table borders, `thead { display: table-header-group }`), printed with headless Chrome:

- The invoice fit on one page and `<br>` in table cells worked.
- The pick list broke across two pages with the header repeated, and the margin box printed "Page 2 of 2".
- The only weak point was automatic column widths, which wrapped short codes. Per-document CSS for the column widths fixes that.

That is the `.md.fancy` route: Markdown structure first, then the template, then `html-print`. Positioned parts stay HTML, either inline in the Markdown or as the existing `.print.html.fancy` (check, badge). It needed the table fix below.

## Typst 0.15.1 as a library

`typst`, `typst-pdf` and `typst-layout` 0.15.1 (Apache-2.0) with a minimal `World` of about 50 lines that loads the Liberation and DejaVu fonts. The invoice, the 70-row pick list and a check layout in Typst markup all rendered correctly, in 5 to 25 ms each: a footer with "Page 1 of 2", the table header repeated on each page, the check fields placed absolutely with `place`, and an SVG barcode from bytes.

The cost: 284 crates in the tree, 110 not in the workspace at a compatible version (the Typst crates, `krilla`, `hayro`, `resvg`/`usvg`, `wasmi`, `hayagriva`, `citationberg`, `syntect`, `two-face`, ICU locale and segmenter data, and others). A stripped release binary was 39.5 MB; for scale, the current unstripped release `moqui` is 71 MB. A cold release build took about 77 s. There are no features to trim; most of the size is embedded data.

It would also need a Markdown to Typst conversion (pulldown-cmark events to Typst markup, the tags set aside as `.md.fancy` does) and Typst escaping of `${}` output. Off by default behind a host feature, it is the option for a host without Chromium, such as the browser build.

## lopdf for manipulation

`lopdf` 0.45 (MIT, widely used) with `default-features = false` adds 5 crates not in the workspace (`lopdf`, `ecb`, `rangemap`, `weezl`, `alloc-stdlib`) and uses the `aes` 0.9 and `cbc` 0.2 already in the graph. Only the dependency resolve was checked, not a build or its API.

## Repository findings

- **`.md.fancy` loops in tables (fixed 2026-10-18).** `markdown_fancy_source` sets template tags aside as placeholders before the Markdown pass. A tag on its own line inside a table became a row of its own (`<tr><td><#list …></td><td></td></tr>`), so a `<#list>` around a row repeated an empty row with each item. Rows that hold only tag placeholders are now replaced by the tags (`lift_tag_rows`); test `markdown_fancy_tag_lines_in_a_table_do_not_become_rows`. Java's `FtlMarkdownTemplateRenderer` also ran flexmark before FreeMarker, so this goes beyond Java rather than restoring it.
- **Images in `html-print` (fixed 2026-10-18).** The Chromium page fails every request except `data:` URLs (`crates/moqui-host/src/pdf.rs`, `Fetch.enable` with no patterns). Nothing inlined `<img src>` resource locations, and `logoImageLocation` is a content location from `get#InvoicePrintInfo` and the shipment services, so the logo, signature and face images were missing from the PDF. `PdfRenderer::render` (`crates/moqui-core/src/screen/pdf.rs`) now reads each `<img>` source that is a resource location through the resource facade and gives it to the engine as a `data:` URL, as Java's FOP resolved image URIs through the resource facade; an `http` or `https` source is still not fetched.
- **Page N of M (fixed 2026-10-18).** The XSL-FO documents had a page number footer in 12 templates; the `.print.html.fancy` ports printed the footer text once, at the end. Each of the 12 now prints its XSL-FO footer on every page as an `@page` `@bottom-center` margin box, with `counter(page)` (and `counter(pages)` on the receivable statement), the text quoted by the template helper `cssString`. A document printed several to a render (payment detail, shipment pack) has a page name of its own; Chromium ignores `counter-reset: page`, so the pages number through the render where XSL-FO restarted each payment at 1. The running page header (XSL-FO region-before) still prints on the first page only.
- **PDF links (found and fixed 2026-10-18).** The 30 PDF links in tools and MrSimpleScreens (`${webappRootUrl}/fop/apps/...`) returned 404: `MoquiFopServlet` was not ported, and `/fop` was resolved as a screen. `/fop/<path>` now renders the screen at `<path>` as `html-print` and sends it as a PDF, a download when `filename` or `saveFilename` names one (`crates/moqui-core/src/screen/mod.rs`).

## Tabled ideas

From the notes that started this review; none depends on the PDF engine:

- A button or form on the wiki page view that downloads a Markdown page as PDF: `.md` to HTML to `html-print`, with images inlined as `data:` URLs.
- Markdown versions of the flowing print documents (`.md.fancy` plus a shared print stylesheet), with the check and badge kept as HTML.
- An online editor for the print and email templates (a new System app screen first): a Markdown editor aware of the template markup, the originals kept in the filesystem with the code, edited overrides stored in the database and selectable from EmailTemplate and similar configuration.
- Admin screens in the System app for EmailTemplate and the other template configuration entities.
