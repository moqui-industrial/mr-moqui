/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! The Chromium PDF engine on its own: no database and no server. The browser is `pdf_chromium_path`, else `chromium`,
//! `chromium-browser` or `google-chrome` on the `PATH`; with none the test logs that and passes.
#![cfg(feature = "pdf")]

use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use moqui_core::screen::PdfEngine;
use moqui_host::pdf::ChromiumPdf;

fn browser() -> Option<PathBuf> {
    if let Ok(Some(found)) = moqui_host::pdf::find_chromium() {
        return Some(found);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join("google-chrome"))
        .find(|candidate| candidate.is_file())
}

#[test]
fn an_html_print_document_becomes_pdf_and_reaches_no_network() {
    let Some(binary) = browser() else {
        eprintln!("no Chromium binary; skipped the PDF engine test");
        return;
    };
    // A listener the document points at. It must see no connection.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&hits);
    let watcher = std::thread::spawn(move || {
        for _ in 0..200 {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 512];
                let _ = stream.read(&mut buffer);
                counted.fetch_add(1, Ordering::SeqCst);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    });

    let temp = std::env::temp_dir().join(format!("moqui-pdf-test-{}", std::process::id()));
    let engine = ChromiumPdf::new(binary, temp.clone());
    let html = format!(
        "<!DOCTYPE html><html><head><style>@page {{ size: 8.5in 11in; margin: 0.5in }}</style>\
         <link rel=\"stylesheet\" href=\"http://127.0.0.1:{port}/style.css\"></head>\
         <body><h1>Invoice 100</h1><img src=\"http://127.0.0.1:{port}/logo.png\">\
         <script>fetch('http://127.0.0.1:{port}/fetch').catch(() => {{}})</script>\
         <table><thead><tr><th>Item</th></tr></thead><tbody><tr><td>Widget</td></tr></tbody></table></body></html>"
    );
    let first = engine.render(&html).expect("PDF bytes");
    assert!(first.starts_with(b"%PDF-"), "{:?}", &first[..first.len().min(16)]);
    // The same browser serves the next document.
    let second = engine.render("<p>second</p>").expect("second PDF");
    assert!(second.starts_with(b"%PDF-"));
    // Concurrent renders share the browser.
    let engine = Arc::new(engine);
    let threads: Vec<_> = (0..3)
        .map(|n| {
            let engine = Arc::clone(&engine);
            std::thread::spawn(move || engine.render(&format!("<p>page {n}</p>")))
        })
        .collect();
    for thread in threads {
        assert!(thread.join().unwrap().unwrap().starts_with(b"%PDF-"));
    }
    engine.shutdown();
    // After a shutdown (as after the browser exits) the next render starts it again.
    assert!(engine.render("<p>again</p>").unwrap().starts_with(b"%PDF-"));
    engine.shutdown();

    watcher.join().unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 0, "the document reached the network");
    let _ = std::fs::remove_dir_all(&temp);
}
