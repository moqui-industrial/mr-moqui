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

//! Where the search store lives, and the backend the factory gets.
//!
//! The Postgres backend uses, in order: the `postgres` url from `search-facade`; the entity
//! Postgres (its `search` schema), when the entity datasource is Postgres; or a dedicated embedded
//! cluster under `runtime/db/search` when the entity datasource is not Postgres. The OpenSearch
//! backend talks to the conf `cluster` over HTTP; starting that process is not done here.

use std::path::Path;
use std::sync::Arc;

use moqui_core::context::ExecutionContextFactory;
use moqui_core::entity::EntityFacade;
use moqui_core::search::conf::{Backend, SearchConf};
use moqui_core::search::postgres::PostgresBackend;
use moqui_core::search::SearchFacade;
use moqui_core::transaction::{PoolConfig, TransactionManager};
use moqui_lib::error::Error;

/// Keeps a dedicated search postmaster alive. Drop it after the factory.
#[derive(Default)]
pub struct SearchStore {
    #[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
    cluster: Option<crate::db::embedded::EmbeddedCluster>,
}

impl SearchStore {
    /// True when this store started its own embedded postmaster.
    pub fn has_own_cluster(&self) -> bool {
        #[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
        {
            self.cluster.is_some()
        }
        #[cfg(not(all(feature = "embedded-postgres", feature = "postgres")))]
        {
            false
        }
    }
}

fn search_pool() -> PoolConfig {
    PoolConfig {
        min: 0,
        max: 8,
        reserve: 1,
        acquire_timeout: std::time::Duration::from_secs(30),
        ..Default::default()
    }
}

/// Build the configured backend and install the facade on `factory`.
///
/// `entity_is_postgres` says whether the entity datasource is Postgres, so its `search` schema can
/// hold the documents.
///
/// # Errors
///
/// Returns an error when the conf is bad, the store cannot be reached, or the backend is not in
/// this build.
pub fn attach_search(
    factory: &ExecutionContextFactory,
    entity: &EntityFacade,
    entity_is_postgres: bool,
) -> Result<SearchStore, Error> {
    let conf = SearchConf::from_conf_text(factory.conf_text())?;
    match conf.backend {
        Backend::OpenSearch => attach_opensearch(factory, entity, &conf),
        Backend::Postgres => {
            let mut store = SearchStore::default();
            let tx = if let Some(url) = &conf.postgres.url {
                external_tx(url, conf.postgres.user.as_deref(), conf.postgres.password.as_deref())?
            } else if entity_is_postgres {
                entity.transactions_for("moqui.entity.document.DataDocument")?
            } else {
                embedded_tx(factory.runtime_dir(), &mut store)?
            };
            let backend = PostgresBackend::open(tx, conf.postgres.schema.as_deref())?;
            moqui_core::search::install(factory, SearchFacade::new(Arc::new(backend), entity.clone()));
            Ok(store)
        }
    }
}

#[cfg(feature = "postgres")]
fn external_tx(url: &str, user: Option<&str>, password: Option<&str>) -> Result<TransactionManager, Error> {
    let url = moqui_core::search::conf::parse_postgres_url(url)?;
    let driver = crate::db::entity_db::postgres_server_driver(
        user.unwrap_or("postgres"),
        password,
        &url.database,
        &url.host,
        url.port,
        url.tls,
    )?;
    TransactionManager::new(Arc::new(driver), search_pool())
}

#[cfg(not(feature = "postgres"))]
fn external_tx(_url: &str, _user: Option<&str>, _password: Option<&str>) -> Result<TransactionManager, Error> {
    Err(Error::new("the postgres driver is not in this build"))
}

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
fn embedded_tx(runtime_dir: &Path, store: &mut SearchStore) -> Result<TransactionManager, Error> {
    use crate::db::embedded::{shared_install_dir, ClusterRole, EmbeddedCluster, StartOptions};

    let db_dir = runtime_dir.join("db");
    let dir = db_dir.join("search");
    crate::xmlshim::provide_libxml2_soname(&dir)?;
    let options = StartOptions {
        max_connections: search_pool().max + crate::db::entity_db::SUPERUSER_SLOTS,
        role: ClusterRole::Entity,
        install_dir: Some(shared_install_dir(&db_dir, &dir)),
    };
    let (cluster, driver) = EmbeddedCluster::start_with(&dir, &options)?;
    store.cluster = Some(cluster);
    TransactionManager::new(Arc::new(driver), search_pool())
}

#[cfg(not(all(feature = "embedded-postgres", feature = "postgres")))]
fn embedded_tx(_runtime_dir: &Path, _store: &mut SearchStore) -> Result<TransactionManager, Error> {
    Err(Error::new(
        "embedded postgres is not in this build; configure a search-facade postgres url or the opensearch backend",
    ))
}

fn attach_opensearch(
    factory: &ExecutionContextFactory,
    entity: &EntityFacade,
    conf: &SearchConf,
) -> Result<SearchStore, Error> {
    let cluster = conf
        .cluster
        .as_ref()
        .ok_or_else(|| Error::new("search-facade backend opensearch needs a cluster url"))?;
    let backend = moqui_core::search::opensearch::OpenSearchBackend::new(cluster);
    moqui_core::search::install(factory, SearchFacade::new(Arc::new(backend), entity.clone()));
    Ok(SearchStore::default())
}
