#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres", feature = "axum"))]
mod common;
use common::{basic, exchange, http_request, start};
#[test]
fn zdebug() {
    let gate = start("zdebug");
    let john = basic("john.doe", "moqui");
    let path = std::env::var("ZPATH").unwrap_or_else(|_| "/Tools/ZDebug.html".to_string());
    let (_, _, page) = exchange(gate.server.address(), &http_request("GET", &path, &[("Authorization", &john)], ""));
    let at = page.find("DEBUG").or_else(|| page.find("Error rendering")).unwrap_or(0);
    println!("PAGE>>{}", &page[at..(at + 6000).min(page.len())]);
}
