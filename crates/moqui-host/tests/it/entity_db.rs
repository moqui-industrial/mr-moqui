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

//! The entity database a conf names (A3 of the plan): `entity_ds_db_conf=postgres` loads into that
//! server, here a second embedded cluster reached through its socket directory, and starts no
//! postmaster in the cluster directory. It sets process properties, so it holds the serial lock.

#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres"))]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use moqui_core::driver::DbDriver;
use moqui_host::db::embedded::EmbeddedCluster;
use moqui_host::{load_data, LoadFlags};
use moqui_lib::object::Value;

#[test]
fn an_external_postgres_from_the_conf_is_loaded_and_no_postmaster_starts() {
    let _serial = crate::common::serial();
    // The properties name this test's server, and no other test may start on it.
    let _properties = crate::common::PropertyReset::of(&[
        "entity_ds_db_conf",
        "entity_ds_host",
        "entity_ds_port",
        "entity_ds_user",
        "entity_ds_password",
        "entity_ds_database",
    ]);
    let server_dir = temp_dir("moqui-ext");
    let runtime = temp_dir("moqui-ext-rt");
    let cluster_dir = temp_dir("moqui-ext-unused");
    let _guards = (
        DirGuard(server_dir.clone()),
        DirGuard(runtime.clone()),
        DirGuard(cluster_dir.clone()),
    );
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&server_dir).unwrap();
    moqui_host::xmlshim::provide_libxml2_soname(&server_dir).unwrap();
    let (server, driver) = EmbeddedCluster::start(&server_dir, 20).unwrap();
    let password = std::fs::read_to_string(server_dir.join("pgpass")).unwrap();

    for (name, value) in [
        ("entity_ds_db_conf", "postgres"),
        ("entity_ds_host", server.data_dir().to_str().unwrap()),
        ("entity_ds_port", "5432"),
        ("entity_ds_user", "postgres"),
        ("entity_ds_password", password.trim()),
        ("entity_ds_database", "moqui"),
    ] {
        moqui_lib::env::set_property(name, value.to_string());
    }
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/framework");
    let report = load_data(&runtime, &framework, &cluster_dir, &LoadFlags::default()).unwrap();
    assert!(report.enumeration > 0, "enumeration {}", report.enumeration);
    assert!(
        !cluster_dir.join("data").exists(),
        "an external datasource started a postmaster in {}",
        cluster_dir.display()
    );

    let mut conn = driver.connect().unwrap();
    let rows = conn.query("SELECT COUNT(*) AS n FROM \"Enumeration\"", &[]).unwrap();
    let count = match rows[0].get("n") {
        Some(Value::I64(n)) => *n,
        other => panic!("count {other:?}"),
    };
    assert_eq!(count, report.enumeration);
    drop(conn);
    drop(server);
}

fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!("{prefix}-{}-{nanos}", std::process::id());
    if Path::new("/tmp").is_dir() {
        PathBuf::from("/tmp").join(name)
    } else {
        std::env::temp_dir().join(name)
    }
}

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
