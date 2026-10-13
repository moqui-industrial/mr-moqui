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

//! The binary's screen lint and the server-static compatibility report of Gate 3: both run over the tree and exit 0.

use std::path::PathBuf;
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn moqui(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_moqui"))
        .args(args)
        .current_dir(repo())
        .env("MOQUI_RUNTIME", repo().join("runtime"))
        .env("MOQUI_FRAMEWORK", repo().join("framework"))
        .output()
        .unwrap()
}

#[test]
fn lint_screens_qvt_static_writes_the_report_and_exits_zero() {
    let out = std::env::temp_dir().join(format!("moqui-qvt-static-report-{}.txt", std::process::id()));
    let output = moqui(&["lint", "--screens", "--qvt-static", &out.display().to_string()]);
    let report = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    assert!(
        output.status.success(),
        "{:?}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let fixture = report
        .split("\nscreen ")
        .find(|block| block.contains("tools/screen/System/Security/ArtifactGroup/ArtifactGroupDetail.xml\n"))
        .unwrap_or_else(|| panic!("no block for the Gate 3 fixture in {report}"));
    assert!(fixture.contains("verdict clean"), "{fixture}");
    assert!(report.lines().last().unwrap_or("").contains("screens:"), "{report}");

    // without a report file the report is the standard output
    let output = moqui(&["lint", "--screens", "--qvt-static"]);
    assert!(output.status.success(), "{:?}", output.status);
    assert!(String::from_utf8_lossy(&output.stdout).contains("ArtifactGroupDetail.xml"));
}

#[test]
fn lint_screens_exits_zero() {
    let output = moqui(&["lint", "--screens"]);
    assert!(
        output.status.success(),
        "{:?}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
