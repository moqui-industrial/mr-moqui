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

//! The `search-facade` conf element.
//!
//! ```xml
//! <search-facade backend="postgres"/>
//! <search-facade backend="postgres"><postgres url="postgresql://host:5432/db" user=".." password=".."/></search-facade>
//! <search-facade backend="opensearch"><cluster url=".." user=".." password=".." index-prefix=".."/></search-facade>
//! ```

use moqui_lib::error::Error;
use moqui_lib::node::MNode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Postgres,
    OpenSearch,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PostgresConf {
    /// `postgresql://host:port/database`. `None` is the entity Postgres, or a dedicated embedded
    /// cluster when the entity database is not Postgres.
    pub url: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub schema: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClusterConf {
    pub url: String,
    pub user: Option<String>,
    pub password: Option<String>,
    pub index_prefix: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchConf {
    pub backend: Backend,
    pub postgres: PostgresConf,
    pub cluster: Option<ClusterConf>,
}

impl SearchConf {
    /// Read the merged conf. No `search-facade` element is the Postgres default.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown backend, or an OpenSearch backend with no cluster url.
    pub fn from_conf(conf: &MNode) -> Result<Self, Error> {
        let node = if conf.name() == "search-facade" {
            Some(conf.clone())
        } else {
            conf.first_named("search-facade")
        };
        let Some(node) = node else {
            return Ok(Self {
                backend: Backend::Postgres,
                postgres: PostgresConf::default(),
                cluster: None,
            });
        };
        let backend = match attr(&node, "backend").as_deref() {
            None | Some("postgres") => Backend::Postgres,
            Some("opensearch") | Some("elasticsearch") => Backend::OpenSearch,
            Some(other) => {
                return Err(Error::new(format!(
                    "search-facade backend {other} is not postgres or opensearch"
                )))
            }
        };
        let postgres = node
            .first_named("postgres")
            .map(|child| PostgresConf {
                url: attr(&child, "url"),
                user: attr(&child, "user"),
                password: attr(&child, "password"),
                schema: attr(&child, "schema"),
            })
            .unwrap_or_default();
        let cluster = match node.first_named("cluster") {
            Some(child) => attr(&child, "url").map(|url| ClusterConf {
                url,
                user: attr(&child, "user"),
                password: attr(&child, "password"),
                index_prefix: attr(&child, "index-prefix"),
            }),
            None => None,
        };
        if backend == Backend::OpenSearch && cluster.is_none() {
            return Err(Error::new("search-facade backend opensearch needs a cluster url"));
        }
        Ok(Self {
            backend,
            postgres,
            cluster,
        })
    }

    /// Read from the merged conf text a factory keeps.
    ///
    /// # Errors
    ///
    /// Returns an error when the text does not parse or [`Self::from_conf`] fails.
    pub fn from_conf_text(text: &str) -> Result<Self, Error> {
        Self::from_conf(&MNode::parse_text("conf", text)?)
    }
}

/// An attribute with `${name}` expanded. Empty is `None`.
fn attr(node: &MNode, name: &str) -> Option<String> {
    let raw = node.attribute(name)?;
    let expanded = moqui_lib::env::expand(&raw).unwrap_or(raw);
    let trimmed = expanded.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// `postgresql://host:port/database` as host, port, and database. The port defaults to 5432.
///
/// # Errors
///
/// Returns an error when the url is not that form.
pub fn parse_postgres_url(url: &str) -> Result<(String, u16, String), Error> {
    let rest = url
        .strip_prefix("postgresql://")
        .or_else(|| url.strip_prefix("postgres://"))
        .or_else(|| url.strip_prefix("jdbc:postgresql://"))
        .ok_or_else(|| {
            Error::new(format!(
                "search postgres url {url} is not postgresql://host:port/database"
            ))
        })?;
    let (authority, database) = rest
        .split_once('/')
        .ok_or_else(|| Error::new(format!("search postgres url {url} has no database")))?;
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let database = database.split(['?', '#']).next().unwrap_or(database);
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse::<u16>()
                .map_err(|_| Error::new(format!("search postgres url {url} has a bad port")))?,
        ),
        None => (authority, 5432),
    };
    if host.is_empty() || database.is_empty() {
        return Err(Error::new(format!(
            "search postgres url {url} needs a host and a database"
        )));
    }
    Ok((host.to_string(), port, database.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conf_forms() {
        let default = SearchConf::from_conf_text("<moqui-conf/>").unwrap();
        assert_eq!(default.backend, Backend::Postgres);
        let external = SearchConf::from_conf_text(
            r#"<moqui-conf><search-facade backend="postgres"><postgres url="postgresql://db.example:6432/search" user="s"/></search-facade></moqui-conf>"#,
        )
        .unwrap();
        assert_eq!(
            external.postgres.url.as_deref(),
            Some("postgresql://db.example:6432/search")
        );
        assert!(
            SearchConf::from_conf_text(r#"<moqui-conf><search-facade backend="opensearch"/></moqui-conf>"#).is_err()
        );
        assert!(SearchConf::from_conf_text(r#"<moqui-conf><search-facade backend="solr"/></moqui-conf>"#).is_err());
    }

    #[test]
    fn postgres_urls() {
        assert_eq!(
            parse_postgres_url("postgresql://db.example:6432/search").unwrap(),
            ("db.example".to_string(), 6432, "search".to_string())
        );
        assert_eq!(
            parse_postgres_url("jdbc:postgresql://u@db/moqui?ssl=true").unwrap(),
            ("db".to_string(), 5432, "moqui".to_string())
        );
        assert!(parse_postgres_url("mysql://db/x").is_err());
    }
}
