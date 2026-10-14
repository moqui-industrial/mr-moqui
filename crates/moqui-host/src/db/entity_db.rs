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

//! The entity database a conf names. The `transactional` datasource is the embedded cluster unless
//! `entity_ds_db_conf` (or the datasource) names `postgres`, `mysql` or `mysql8`; then it is that
//! server, and no postmaster starts for it (`plans/EmbeddedPostgres.md`).

use std::path::Path;
use std::sync::Arc;

use moqui_core::context::ExecutionContextFactory;
use moqui_core::driver::DbDriver;
use moqui_core::entity::datasource::{datasource_conf, DatasourceConf, DatasourceKind};
use moqui_lib::error::Error;
use moqui_lib::node::MNode;

/// Postgres keeps three connection slots for superusers. The embedded role is a superuser, so they
/// are room for `pg_dump` beside a full pool rather than for the pool.
pub const SUPERUSER_SLOTS: u32 = 3;
const POSTGRES_PORT: u16 = 5432;
const MYSQL_PORT: u16 = 3306;

/// The open entity database. Drop the facade before this, and this before nothing else: the
/// driver goes first, then the embedded cluster when there is one.
pub struct EntityDb {
    pub conf: DatasourceConf,
    pub driver: Arc<dyn DbDriver>,
    #[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
    cluster: Option<super::embedded::EmbeddedCluster>,
}

impl EntityDb {
    /// True for embedded and external Postgres, whose `search` schema can hold documents.
    #[must_use]
    pub fn is_postgres(&self) -> bool {
        self.conf.kind != DatasourceKind::Mysql
    }

    /// True when this process serves the database from the embedded cluster.
    #[must_use]
    pub fn is_embedded(&self) -> bool {
        self.conf.kind == DatasourceKind::Embedded
    }
}

impl Drop for EntityDb {
    fn drop(&mut self) {
        // The driver's clients go before the postmaster stops.
        self.driver = Arc::new(Closed);
        #[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
        self.cluster.take();
    }
}

struct Closed;

impl DbDriver for Closed {
    fn connect(&self) -> Result<Box<dyn moqui_core::driver::DbConnection>, moqui_lib::error::Error> {
        Err(Error::new("the entity database is closed"))
    }
}

/// The `transactional` datasource of `factory`'s merged conf.
///
/// # Errors
///
/// Returns an error when the conf does not parse or the datasource cannot be read.
pub fn transactional_conf(factory: &ExecutionContextFactory) -> Result<DatasourceConf, Error> {
    let conf = MNode::parse_text("moqui-conf", factory.conf_text())?;
    datasource_conf(&conf, "transactional")
}

/// Open the database `conf` names. Embedded starts (or adopts) the cluster at `cluster_dir`, on the
/// theseus install at `install_dir`, with `max_connections` of `pool_max` plus [`SUPERUSER_SLOTS`].
///
/// # Errors
///
/// Returns an error when the cluster does not start, or the driver for the kind is not in this
/// build or cannot be made.
pub fn open(conf: DatasourceConf, cluster_dir: &Path, install_dir: &Path, pool_max: u32) -> Result<EntityDb, Error> {
    match conf.kind {
        DatasourceKind::Embedded => open_embedded(conf, cluster_dir, install_dir, pool_max),
        DatasourceKind::Postgres => {
            let driver = postgres_driver(&conf)?;
            Ok(external(conf, driver))
        }
        DatasourceKind::Mysql => {
            let driver = mysql_driver(&conf)?;
            Ok(external(conf, driver))
        }
    }
}

fn external(conf: DatasourceConf, driver: Arc<dyn DbDriver>) -> EntityDb {
    EntityDb {
        conf,
        driver,
        #[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
        cluster: None,
    }
}

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
fn open_embedded(
    conf: DatasourceConf,
    cluster_dir: &Path,
    install_dir: &Path,
    pool_max: u32,
) -> Result<EntityDb, Error> {
    use super::embedded::{ClusterRole, EmbeddedCluster, StartOptions};

    crate::xmlshim::provide_libxml2_soname(cluster_dir)?;
    let options = StartOptions {
        max_connections: pool_max + SUPERUSER_SLOTS,
        role: ClusterRole::Entity,
        install_dir: Some(install_dir.to_path_buf()),
    };
    let (cluster, driver) = EmbeddedCluster::start_with(cluster_dir, &options)?;
    Ok(EntityDb {
        conf,
        driver: Arc::new(driver),
        cluster: Some(cluster),
    })
}

#[cfg(not(all(feature = "embedded-postgres", feature = "postgres")))]
fn open_embedded(
    _conf: DatasourceConf,
    _cluster_dir: &Path,
    _install_dir: &Path,
    _pool_max: u32,
) -> Result<EntityDb, Error> {
    Err(Error::new(
        "embedded postgres is not in this build; set entity_ds_db_conf to postgres or mysql",
    ))
}

#[cfg(feature = "postgres")]
fn postgres_driver(conf: &DatasourceConf) -> Result<Arc<dyn DbDriver>, Error> {
    use super::postgres::PostgresDriver;

    let port = conf.port.unwrap_or(POSTGRES_PORT);
    // As in libpq, a host that is an absolute path is the server's Unix socket directory.
    #[cfg(unix)]
    let socket = conf.host.starts_with('/');
    #[cfg(not(unix))]
    let socket = false;
    let driver = if socket {
        #[cfg(unix)]
        {
            PostgresDriver::unix_socket(&conf.user, &conf.database, &conf.host).port(port)
        }
        #[cfg(not(unix))]
        {
            unreachable!("a socket host is unix only")
        }
    } else if conf.tls {
        PostgresDriver::external(&conf.user, &conf.database, &conf.host, port)?
    } else {
        PostgresDriver::tcp_no_tls(&conf.user, &conf.database, &conf.host, port)
    };
    let driver = if conf.password.is_empty() {
        driver
    } else {
        driver.password(&conf.password)
    };
    Ok(Arc::new(driver))
}

#[cfg(not(feature = "postgres"))]
fn postgres_driver(_conf: &DatasourceConf) -> Result<Arc<dyn DbDriver>, Error> {
    Err(Error::new("the postgres driver is not in this build"))
}

#[cfg(feature = "mysql")]
fn mysql_driver(conf: &DatasourceConf) -> Result<Arc<dyn DbDriver>, Error> {
    Ok(Arc::new(super::mysql::MysqlDriver::new(
        &conf.host,
        conf.port.unwrap_or(MYSQL_PORT),
        &conf.user,
        &conf.password,
        &conf.database,
        conf.tls,
    )))
}

#[cfg(not(feature = "mysql"))]
fn mysql_driver(_conf: &DatasourceConf) -> Result<Arc<dyn DbDriver>, Error> {
    Err(Error::new("the mysql driver is not in this build"))
}
