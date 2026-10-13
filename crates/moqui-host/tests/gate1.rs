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

//! Framework data-file load against a fresh embedded cluster, plus a MySQL metadata smoke.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use moqui_core::driver::DbDriver;
use moqui_core::transaction::PoolConfig;
use moqui_host::db::entity_facade::open_entity_facade;
use moqui_host::{load_data, LoadFlags};
use moqui_lib::object::Value;

#[cfg(all(unix, feature = "embedded-postgres", feature = "postgres"))]
#[test]
fn framework_seed_and_fixture_load() {
    let runtime = temp_dir("moqui-rt");
    let cluster_dir = temp_dir("moqui-pg");
    let _runtime_guard = DirGuard(runtime.clone());
    let _cluster_guard = DirGuard(cluster_dir.clone());
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&cluster_dir).unwrap();
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");

    let report = load_data(&runtime, &framework, &cluster_dir, &LoadFlags::default()).unwrap();
    assert!(
        report.test_fts_before_load,
        "TestFts table was missing before the seed load"
    );
    assert!(report.enumeration > 0, "enumeration {}", report.enumeration);
    assert!(
        report.user_group_permission > 0,
        "user group permission {}",
        report.user_group_permission
    );
    println!(
        "seed enumeration {} user group permission {}",
        report.enumeration, report.user_group_permission
    );

    let service = runtime.join("copy-row.xml");
    let data = runtime.join("fixture-data.xml");
    std::fs::write(
        &service,
        r#"<services>
            <service verb="copy" noun="Row">
                <in-parameters><parameter name="seedId"/></in-parameters>
                <actions>
                    <entity-find entity-name="moqui.test.TestEntity" list="found">
                        <econdition field-name="testId" from="seedId"/>
                    </entity-find>
                    <entity-make-value entity-name="moqui.test.TestEntity" value-field="made"/>
                    <script>made.set("testId", "B"); made.set("testMedium", "made");</script>
                    <entity-create value-field="made"/>
                </actions>
            </service>
        </services>"#,
    )
    .unwrap();
    std::fs::write(
        &data,
        r#"<entity-facade-xml>
            <moqui.test.TestEntity testId="E1" testMedium="entity"/>
            <create-moqui.test.TestEntity testId="A1" testMedium="auto"/>
            <copy-Row seedId="E1"/>
        </entity-facade-xml>"#,
    )
    .unwrap();
    assert!(
        !service.display().to_string().contains("/service/"),
        "fixture service path must not gain a service prefix"
    );
    let flags = LoadFlags {
        location: Some(data.display().to_string()),
        service_files: vec![service],
        ..LoadFlags::default()
    };
    let fixture = load_data(&runtime, &framework, &cluster_dir, &flags).unwrap();
    assert!(fixture.test_fts_before_load);
    // The plain entity element is the counted record. The entity-auto create and the inline
    // service run, and `load_entity_data` does not add them to the record count.
    assert_eq!(fixture.records, 1, "fixture records {}", fixture.records);

    let (enumeration, permissions, medium_a, medium_b, medium_e) = read_back(&framework, &cluster_dir);
    assert!(enumeration > 0, "enumeration {enumeration}");
    assert!(permissions > 0, "user group permission {permissions}");
    assert_eq!(medium_e.as_deref(), Some("entity"));
    assert_eq!(medium_a.as_deref(), Some("auto"));
    assert_eq!(medium_b.as_deref(), Some("made"));
    println!("read back enumeration {enumeration} user group permission {permissions} fixture rows E1 A1 B");
}

#[cfg(all(unix, feature = "embedded-postgres", feature = "postgres"))]
fn read_back(framework: &Path, cluster_dir: &Path) -> (i64, i64, Option<String>, Option<String>, Option<String>) {
    use moqui_host::db::embedded::EmbeddedCluster;

    let (cluster, driver) = EmbeddedCluster::start(cluster_dir, 20).unwrap();
    let driver: Arc<dyn DbDriver> = Arc::new(driver);
    let facade = open_entity_facade(framework, driver, pool()).unwrap();
    let enumeration = facade.find("moqui.basic.Enumeration").unwrap().count().unwrap();
    let permissions = facade
        .find("moqui.security.UserGroupPermission")
        .unwrap()
        .count()
        .unwrap();
    let medium_e = medium(&facade, "E1");
    let medium_a = medium(&facade, "A1");
    let medium_b = medium(&facade, "B");
    drop(facade);
    drop(cluster);
    (enumeration, permissions, medium_a, medium_b, medium_e)
}

#[cfg(all(unix, feature = "embedded-postgres", feature = "postgres"))]
fn medium(facade: &moqui_core::entity::EntityFacade, test_id: &str) -> Option<String> {
    facade
        .find("moqui.test.TestEntity")
        .unwrap()
        .eq("testId", Value::string(test_id))
        .one()
        .unwrap()
        .and_then(|row| row.get("testMedium").and_then(Value::as_str).map(str::to_string))
}

#[cfg(feature = "mysql")]
#[test]
fn mysql_metadata_insert_and_select() {
    let Some((host, port, user, password, database)) = mysql_target() else {
        eprintln!("MySQL was not reachable; smoke skipped");
        return;
    };
    let driver = moqui_host::db::mysql::MysqlDriver::new(host, port, user, password, database, false);
    if driver.connect_mysql().is_err() {
        eprintln!("MySQL was not reachable; smoke skipped");
        return;
    }
    let driver: Arc<dyn DbDriver> = Arc::new(driver);
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
    let facade =
        moqui_host::db::entity_facade::open_entity_facade_dialect(&framework, Arc::clone(&driver), pool(), "mysql")
            .unwrap();
    facade.check_tables(&["moqui.test.TestEntity"]).unwrap();
    let mut row = facade.make_value("moqui.test.TestEntity").unwrap();
    row.set("testId", Value::string("MYSQL1"));
    row.set("testMedium", Value::string("mysql"));
    row.create(&facade).unwrap();
    let found = facade
        .find("moqui.test.TestEntity")
        .unwrap()
        .eq("testId", Value::string("MYSQL1"))
        .one()
        .unwrap()
        .unwrap();
    assert_eq!(found.get("testMedium").and_then(Value::as_str), Some("mysql"));
    found.delete(&facade).unwrap();
    drop(facade);
    let mut conn = driver.connect().unwrap();
    conn.execute("DROP TABLE IF EXISTS `TestEntity`", &[]).unwrap();
    eprintln!("MySQL smoke inserted and selected moqui.test.TestEntity");
}

#[cfg(feature = "mysql")]
fn mysql_target() -> Option<(String, u16, String, String, String)> {
    if let Ok(spec) = std::env::var("MOQUI_TEST_MYSQL") {
        let (user, password, host, port, database) =
            parse_mysql_spec(&spec).expect("MOQUI_TEST_MYSQL must be user:pass@host:port/database");
        return Some((
            host.to_string(),
            port,
            user.to_string(),
            password.to_string(),
            database.to_string(),
        ));
    }
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], 3306));
    if std::net::TcpStream::connect_timeout(&address, Duration::from_secs(1)).is_err() {
        return None;
    }
    Some((
        "127.0.0.1".to_string(),
        3306,
        "root".to_string(),
        String::new(),
        "mysql".to_string(),
    ))
}

#[cfg(feature = "mysql")]
fn parse_mysql_spec(spec: &str) -> Option<(&str, &str, &str, u16, &str)> {
    let spec = spec.trim();
    let (userinfo, rest) = spec.rsplit_once('@')?;
    let (username, password) = userinfo.split_once(':')?;
    let (hostport, database) = rest.split_once('/')?;
    let (host, port) = hostport.rsplit_once(':')?;
    let port = port.parse().ok()?;
    if host.is_empty() || database.is_empty() {
        return None;
    }
    Some((username, password, host, port, database))
}

fn pool() -> PoolConfig {
    PoolConfig {
        min: 0,
        max: 4,
        reserve: 1,
        acquire_timeout: Duration::from_secs(30),
    }
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
