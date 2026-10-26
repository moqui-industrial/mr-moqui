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

//! The Workspace client's node test (screen/devstatic/DevAide.test.js). Once that file has moved to
//! `test/client/DevAide.spec.js` this returns. The runner itself goes away with the other node runners.

#[test]
fn the_workspace_client_passes_its_node_tests() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("screen/devstatic/DevAide.test.js");
    if !file.is_file() {
        return;
    }
    let version = std::process::Command::new("node").arg("--version").output();
    let Ok(version) = version else {
        eprintln!("skipping the Workspace client tests: no node");
        return;
    };
    let major: u32 = String::from_utf8_lossy(&version.stdout)
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|major| major.parse().ok())
        .unwrap_or(0);
    if major < 18 {
        eprintln!("skipping the Workspace client tests: node {major} has no test runner");
        return;
    }
    let output = std::process::Command::new("node")
        .arg("--test")
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
