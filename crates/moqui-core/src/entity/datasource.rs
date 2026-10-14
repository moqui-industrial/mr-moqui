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

//! A `<datasource>` of the merged conf, read the way Java's `EntityDatasourceFactoryImpl` and
//! `TransactionInternalBitronix` read it: the datasource's own `inline-jdbc` wins, else the
//! `<database>`'s. The host turns the result into a `DbDriver`, and the pool settings into the core
//! pool. `h2`, the Java default `entity_ds_db_conf`, and an empty name are the embedded Postgres
//! cluster (`plans/archive/EmbeddedPostgres.md`); H2 has no driver here.

use std::time::Duration;

use moqui_lib::error::Error;
use moqui_lib::node::MNode;

use crate::transaction::PoolConfig;

/// Which driver serves a datasource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatasourceKind {
    /// The embedded Postgres cluster under the runtime directory.
    Embedded,
    /// A Postgres server the conf names.
    Postgres,
    /// A MySQL or MariaDB server the conf names.
    Mysql,
}

/// One datasource, with `${}` expanded.
#[derive(Clone, Debug)]
pub struct DatasourceConf {
    pub group: String,
    pub kind: DatasourceKind,
    /// The `database-list` entry whose dialect the entity layer uses: `postgres` for embedded.
    pub dialect: String,
    pub schema: String,
    pub host: String,
    /// `None` takes the driver's default port.
    pub port: Option<u16>,
    pub database: String,
    pub user: String,
    pub password: String,
    /// False only when the conf says `sslmode`/`sslMode` disable(d). Embedded never uses TLS.
    pub tls: bool,
    pub pool: PoolConfig,
}

/// The datasource for `group` in the merged conf `conf` (the `moqui-conf` root).
///
/// # Errors
///
/// Returns an error when the group has no datasource, its database is not in `database-list`, the
/// database has no driver in this port, or a number does not parse.
pub fn datasource_conf(conf: &MNode, group: &str) -> Result<DatasourceConf, Error> {
    let entity = conf
        .first_named("entity-facade")
        .ok_or_else(|| Error::new("the conf has no entity-facade"))?;
    let datasource = entity
        .children_named("datasource")
        .into_iter()
        .find(|node| node.attribute("group-name").as_deref() == Some(group))
        .ok_or_else(|| Error::new(format!("the conf has no datasource for group {group}")))?;
    let conf_name = expanded(&datasource, "database-conf-name");
    let kind = match conf_name.as_str() {
        "" | "h2" => DatasourceKind::Embedded,
        "postgres" => DatasourceKind::Postgres,
        "mysql" | "mysql8" => DatasourceKind::Mysql,
        other => {
            return Err(Error::new(format!(
                "database {other} for datasource {group} has no driver in this port (postgres, mysql, mysql8, or the embedded default)"
            )))
        }
    };
    let dialect = if kind == DatasourceKind::Embedded {
        "postgres".to_string()
    } else {
        conf_name
    };
    let database_node = conf
        .first_named("database-list")
        .and_then(|list| {
            list.children_named("database")
                .into_iter()
                .find(|node| node.attribute("name").as_deref() == Some(dialect.as_str()))
        })
        .ok_or_else(|| Error::new(format!("database-list has no database {dialect}")))?;
    let inline = datasource
        .first_named("inline-jdbc")
        .or_else(|| database_node.first_named("inline-jdbc"));
    let pool = pool_config(inline.as_ref(), &database_node)?;
    let mut conf = DatasourceConf {
        group: group.to_string(),
        kind,
        dialect,
        schema: expanded(&datasource, "schema-name"),
        host: String::new(),
        port: None,
        database: String::new(),
        user: String::new(),
        password: String::new(),
        tls: true,
        pool,
    };
    if kind != DatasourceKind::Embedded {
        if let Some(inline) = &inline {
            connection(&mut conf, inline)?;
        }
        if conf.host.is_empty() {
            return Err(Error::new(format!(
                "datasource {group} names database {} and no server host",
                conf.dialect
            )));
        }
    }
    Ok(conf)
}

/// `inline-jdbc` pool attributes, with Java's fallbacks: min 5, max 50, wait 30 seconds, idle 60
/// seconds (Bitronix's default). `pool-reserve` is this port's, default 8.
fn pool_config(inline: Option<&MNode>, database: &MNode) -> Result<PoolConfig, Error> {
    let number = |name: &str, fallback: u32| -> Result<u32, Error> {
        let raw = inline.map(|node| expanded(node, name)).unwrap_or_default();
        if raw.is_empty() {
            return Ok(fallback);
        }
        raw.parse::<u32>()
            .map_err(|_| Error::new(format!("inline-jdbc {name}=\"{raw}\" is not a whole number")))
    };
    let test_query = inline
        .map(|node| expanded(node, "pool-test-query"))
        .filter(|sql| !sql.is_empty())
        .or_else(|| Some(expanded(database, "default-test-query")).filter(|sql| !sql.is_empty()));
    let max = number("pool-maxsize", 50)?;
    Ok(PoolConfig {
        min: number("pool-minsize", 5)?.min(max),
        max,
        reserve: number("pool-reserve", 8)?,
        acquire_timeout: Duration::from_secs(u64::from(number("pool-time-wait", 30)?)),
        idle_timeout: Some(Duration::from_secs(u64::from(number("pool-time-idle", 60)?))),
        test_query,
    })
}

/// Server, database and credentials from `xa-properties`, else from `jdbc-uri` and the
/// `jdbc-username`/`jdbc-password` attributes.
fn connection(conf: &mut DatasourceConf, inline: &MNode) -> Result<(), Error> {
    if let Some(xa) = inline.first_named("xa-properties") {
        let get = |names: &[&str]| -> String {
            names
                .iter()
                .map(|name| expanded(&xa, name))
                .find(|value| !value.is_empty())
                .unwrap_or_default()
        };
        conf.host = get(&["serverName"]);
        conf.port = port(&get(&["portNumber", "port"]))?;
        conf.database = get(&["databaseName"]);
        conf.user = get(&["user"]);
        conf.password = get(&["password"]);
        conf.tls = tls_wanted(&get(&["sslmode", "sslMode"]));
        let url = get(&["url", "URL"]);
        if conf.host.is_empty() && !url.is_empty() {
            jdbc_uri(conf, &url)?;
        }
        return Ok(());
    }
    let uri = expanded(inline, "jdbc-uri");
    if !uri.is_empty() {
        jdbc_uri(conf, &uri)?;
    }
    conf.user = expanded(inline, "jdbc-username");
    conf.password = expanded(inline, "jdbc-password");
    Ok(())
}

/// `jdbc:postgresql://host[:port]/db[?k=v&...]` or the `jdbc:mysql:` form.
fn jdbc_uri(conf: &mut DatasourceConf, uri: &str) -> Result<(), Error> {
    let rest = uri
        .strip_prefix("jdbc:postgresql://")
        .or_else(|| uri.strip_prefix("jdbc:mysql://"))
        .or_else(|| uri.strip_prefix("jdbc:mariadb://"))
        .ok_or_else(|| Error::new(format!("jdbc uri {uri} is not a postgresql or mysql uri")))?;
    let (address, rest) = rest.split_once('/').unwrap_or((rest, ""));
    let (database, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, port_text) = match address.rsplit_once(':') {
        // A bare IPv6 address has colons of its own; `[::1]:5432` has its port after the bracket.
        Some((host, port)) if !host.contains(':') || host.ends_with(']') => (host, port),
        _ => (address, ""),
    };
    conf.host = host.trim_start_matches('[').trim_end_matches(']').to_string();
    conf.port = port(port_text)?;
    conf.database = database.to_string();
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            if key.eq_ignore_ascii_case("sslmode") {
                conf.tls = tls_wanted(value);
            }
        }
    }
    Ok(())
}

fn port(text: &str) -> Result<Option<u16>, Error> {
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<u16>()
        .map(Some)
        .map_err(|_| Error::new(format!("datasource port {text:?} is not a port")))
}

/// An `sslmode` value asks for TLS unless it is `disable` or `disabled`; empty asks for it.
pub fn tls_wanted(mode: &str) -> bool {
    !matches!(mode.to_ascii_lowercase().as_str(), "disable" | "disabled")
}

fn expanded(node: &MNode, name: &str) -> String {
    let raw = node.attribute(name).unwrap_or_default();
    moqui_lib::env::expand(&raw).unwrap_or(raw).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conf(datasource: &str) -> MNode {
        let text = format!(
            r#"<moqui-conf>
                <entity-facade>{datasource}</entity-facade>
                <database-list>
                    <database name="postgres" default-test-query="SELECT 1">
                        <inline-jdbc pool-maxsize="90"><xa-properties serverName="db.example" portNumber="6543"
                            databaseName="moqui" user="moqui" password="pw"/></inline-jdbc>
                    </database>
                    <database name="mysql8" default-test-query="SELECT 2">
                        <inline-jdbc pool-maxsize="140"><xa-properties serverName="my.example" port="3307"
                            databaseName="moqui" user="u" password="p"/></inline-jdbc>
                    </database>
                    <database name="h2"/>
                    <database name="oracle"/>
                </database-list>
            </moqui-conf>"#
        );
        MNode::parse_text("test", &text).unwrap()
    }

    #[test]
    fn h2_and_empty_are_embedded_with_the_postgres_pool() {
        for name in ["h2", ""] {
            let node = conf(&format!(
                r#"<datasource group-name="transactional" database-conf-name="{name}"/>"#
            ));
            let ds = datasource_conf(&node, "transactional").unwrap();
            assert_eq!(ds.kind, DatasourceKind::Embedded);
            assert_eq!(ds.dialect, "postgres");
            assert_eq!(ds.pool.max, 90);
            assert_eq!(ds.pool.min, 5);
            assert_eq!(ds.pool.reserve, 8);
            assert_eq!(ds.pool.acquire_timeout, Duration::from_secs(30));
            assert_eq!(ds.pool.idle_timeout, Some(Duration::from_secs(60)));
            assert_eq!(ds.pool.test_query.as_deref(), Some("SELECT 1"));
            assert!(ds.host.is_empty());
        }
    }

    #[test]
    fn postgres_reads_the_database_xa_properties() {
        let node =
            conf(r#"<datasource group-name="transactional" database-conf-name="postgres" schema-name="public"/>"#);
        let ds = datasource_conf(&node, "transactional").unwrap();
        assert_eq!(ds.kind, DatasourceKind::Postgres);
        assert_eq!(
            (
                ds.host.as_str(),
                ds.port,
                ds.database.as_str(),
                ds.user.as_str(),
                ds.password.as_str()
            ),
            ("db.example", Some(6543), "moqui", "moqui", "pw")
        );
        assert_eq!(ds.schema, "public");
        assert!(ds.tls);
    }

    #[test]
    fn the_datasource_inline_jdbc_wins_with_its_pool_and_uri() {
        let node = conf(
            r#"<datasource group-name="transactional" database-conf-name="postgres">
                <inline-jdbc jdbc-uri="jdbc:postgresql://127.0.0.1:5433/other?sslmode=disable" jdbc-username="a"
                    jdbc-password="b" pool-maxsize="12" pool-minsize="2" pool-reserve="3" pool-time-wait="0"
                    pool-time-idle="5" pool-test-query="SELECT 42"/>
            </datasource>"#,
        );
        let ds = datasource_conf(&node, "transactional").unwrap();
        assert_eq!(
            (ds.host.as_str(), ds.port, ds.database.as_str()),
            ("127.0.0.1", Some(5433), "other")
        );
        assert_eq!((ds.user.as_str(), ds.password.as_str()), ("a", "b"));
        assert!(!ds.tls);
        assert_eq!((ds.pool.min, ds.pool.max, ds.pool.reserve), (2, 12, 3));
        assert_eq!(ds.pool.acquire_timeout, Duration::ZERO);
        assert_eq!(ds.pool.idle_timeout, Some(Duration::from_secs(5)));
        assert_eq!(ds.pool.test_query.as_deref(), Some("SELECT 42"));
    }

    #[test]
    fn mysql8_uses_port_and_its_own_test_query() {
        let node = conf(r#"<datasource group-name="transactional" database-conf-name="mysql8"/>"#);
        let ds = datasource_conf(&node, "transactional").unwrap();
        assert_eq!(ds.kind, DatasourceKind::Mysql);
        assert_eq!(ds.dialect, "mysql8");
        assert_eq!((ds.host.as_str(), ds.port), ("my.example", Some(3307)));
        assert_eq!(ds.pool.max, 140);
        assert_eq!(ds.pool.test_query.as_deref(), Some("SELECT 2"));
    }

    #[test]
    fn a_database_without_a_driver_or_a_group_without_a_datasource_is_an_error() {
        let node = conf(r#"<datasource group-name="transactional" database-conf-name="oracle"/>"#);
        let err = datasource_conf(&node, "transactional").unwrap_err();
        assert!(err.to_string().contains("no driver"), "{err}");
        let err = datasource_conf(&node, "nontransactional").unwrap_err();
        assert!(err.to_string().contains("no datasource"), "{err}");
    }
}
