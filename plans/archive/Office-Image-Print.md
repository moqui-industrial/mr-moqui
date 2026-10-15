# XLSX, images, and network printing

The `moqui-poi`, `moqui-image`, and `moqui-cups` components in the framework. Built in phase 12, after SFTP and SSO.

This follows Tool components and print in `plans/archive/Architecture-Proposal.md`. The three were outside the Java framework for the size of `moqui.war`. Here they are always linked. They are not Cargo features. Their pure-Rust crates are small next to embedded Postgres: together they added 4.3 MB to the x86_64 release binary (58.8 MB to 63.2 MB).

## Decision

`rust_xlsxwriter` 0.99 writes xlsx. `calamine` 0.36 reads xlsx, xlsm, xlsb, xls, and ods. `image` 0.25 decodes, resizes, and encodes images. `ipp` 7 encodes and parses IPP messages. All four are MIT, or MIT OR Apache-2.0. The first three are in `moqui-core`. They have no I/O and no Tokio. `ipp` is in `moqui-host`, beside the other process-edge clients.

- `rust_xlsxwriter` and `calamine` keep their default features off. Dates cross as Excel serial numbers, so `chrono` stays out. Both depend on `zip` 8 with `deflate`, which is `zopfli` and `flate2` on `zlib-rs`. Those are pure Rust (Apache-2.0 and Zlib). That feature turns `zlib-rs` on for every `flate2` user in the graph. `libz-sys` stays out.
- `image` keeps its default features off. The features on are `bmp`, `gif`, `jpeg` (`zune-jpeg`), `png`, `tiff`, and `webp`. `rayon`, `avif`, `exr`, and the other formats stay off.
- `ipp` keeps its default features off. Its blocking and async clients stay out. The host posts the message with the async `reqwest` already in the graph, on the edge runtime (`crates/moqui-host/src/edge.rs`). cups4j is LGPL and is not a model. libcups is C. Neither is used.

## XLSX: `moqui-poi` in the framework

`xlsx` is a render mode. Conf `screen-output` (the Java element) defines it, and the port reads it next to `screen-text-output`. `widget-render-class` is read and ignored. The type selects the macro module `framework/template/screen/ScreenXlsx.js`, which `macro-template-location` can replace. A binary mode is always standalone. Its text writes are dropped, and the render sends the document as bytes with the mode's mime type and the `saveFilename` download header.

The module walks the screen as `ScreenWidgetRenderExcel` did:

- sections, with their conditions and actions;
- container-box bodies and container-row columns;
- the containers the csv mode also walks.

Each form-list fills a sheet through `sri.xlsxSheet`, `xlsxHeader`, `xlsxCell`, and `xlsxEndRow`. The workbook is `crates/moqui-core/src/screen/xlsx.rs`, the port of `FormListExcelRender`.

- **Header.** Bold, centered, on grey 40%, and frozen.
- **Single-widget cell.** It keeps its type:
  - text is left-aligned and wraps;
  - a number is centered, or right-aligned with `align="right"`;
  - a `currency-unit-field` or a format with `.00` is `#,##0.00_);[Red](#,##0.00)`;
  - a date is `yyyy-MM-dd`, a time `HH:mm:ss`, and a time stamp `yyyy-MM-dd HH:mm` in the user's zone.
- **Multi-widget cell.** The values are formatted and joined with line breaks, as in Java.
- **Widget value.** A display, display-entity, link, or label shows its text, its entity description, or the field value, then its default text. Any other widget shows the field value.
- **Columns.** They are fitted to their text.
- **Errors.** A screen with no form-list is "No sheets rendered".

The `show-xlsx-button` link, `ScreenRender.renderBytes()`, and `ec.screen.isRenderModeBinary()` serve the callers. An email attachment whose `screenRenderMode` is `xlsx` (`send#ScreenRenderEmail`, `EmailTemplateAttachment`) attaches the workbook. `render#ScheduledScreen` attaches it and saves it. The webroot `BasicTransitions.xml`, which the Java tree had and the port lacked, is copied. Its Email Export dialog defaults to XLS.

`org.moqui.impl.SpreadsheetServices` is new. Java read no spreadsheet.

- `read#Spreadsheet` reads one sheet. The input is `contentBytes` or `location`, and the format is found from the content. With `headerRow`, each row is a map by header. Otherwise each row is a list. A whole number is a `Long` and a date is a `Timestamp`. An empty row is skipped.
- `load#SpreadsheetEntity` stores or creates a record for each row through the entity auto service. Authorization and ECAs apply. A column sets the field its header names, or the one `fieldMap` gives.

## Images: `moqui-image` in the framework

`org.moqui.ImageServices.convert#Image` (the interface) and `org.moqui.impl.ImageServices` keep their names and parameters, with Rust bodies (`crates/moqui-core/src/service/images.rs`).

`convert#Image` reads `inputImageStream` (the bytes) or the new `inputLocation`. It writes each output given: the original, then the large, medium, small, and thumbnail widths, which keep the aspect ratio and use the triangle filter, as `ResampleOp.FILTER_TRIANGLE` did. An output is a location or a reference from `ec.resource.getLocationReference`. jpg and bmp get the image on white, as the Java fallback did. A format the encoder still refuses is written in the input's format, with that extension, as Java's retry did. `write#Image` takes encoded bytes in `image` where Java took an `IIOImage`. The four `get#Image*` services list the enabled formats: jpg, png, gif, tiff, webp (written lossless), and bmp.

## Printing: `moqui-cups` in the framework

Core holds a `PrintClient` slot (`crates/moqui-core/src/print.rs`). The host fills it with the IPP client (`crates/moqui-host/src/ipp.rs`). The print services in `crates/moqui-core/src/service/tools.rs` replace the placeholders and follow the `moqui-cups` `PrintServices.xml`:

- `get#ServerPrinters` makes a `NetworkPrinter` for each printer the server lists that no record names yet. It returns the ids of all of that server's records.
- `send#PrintJobInternal` sends the document to the printer named on the `NetworkPrinter`, or to the server's default printer when the record names none.
  - The request carries the username, job name, copies, duplex (`sides`), and page ranges.
  - When the server takes the job, it is `PtjPending` with the server's job id.
  - When the server refuses the job, or does not answer, the job is `PtjSendFailed` with the message, written in its own transaction, and the call returns the error.
  - The LLM sim guard stays.
- `get#PrintJobDetailsFromServer` maps `job-state` to the `Ptj*` status as cups4j did and saves a changed status. It returns the times, the job and printer URLs, the sheets printed, and the size.
- `hold#`, `release#`, and `cancel#PrintJob` set the status when the server does the action, and show the Java message when it does not.

The requests are the ones cups4j made:

- CUPS-Get-Printers and CUPS-Get-Default to `http://host:port/`;
- Print-Job, Hold-Job, Release-Job, and Cancel-Job to the printer's URI;
- Get-Job-Attributes by job URI under `/jobs/`.

The server is reached at `http://host:port`. Port 443, or a host that names `https://`, is TLS. The requesting user is the job's username, else the process user, as cups4j used `user.name`. `make#CupsPrinter` and `make#CupsClient` returned Java objects and are internal steps here. They are not services. The System app Print screens are unchanged.

## Out of scope

- Reading or writing `.xls`, `.docx`, or `.pptx`. calamine reads `.xls`, but nothing writes it.
- Formulas, charts, or styles beyond the Java ones in the xlsx render.
- wbmp, and SVG input through batik. `resvg` is reviewed if SVG is asked for.
- IPP Everywhere discovery (mDNS), printer setup, and CUPS administration.
- The saved-find Schedule form of the Java form-list, which offers xlsx. Saved-find scheduling is not part of the port.
