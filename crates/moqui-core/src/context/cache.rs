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

//! One `MCache` per `<cache name>`. No second cache implementation.

use moqui_lib::cache::{CacheConfig, MCache, MCacheManager};
use moqui_lib::node::MNode;

use super::error::Error;

pub struct CacheFacade {
    manager: MCacheManager,
}

impl CacheFacade {
    pub(super) fn from_conf(conf: &MNode) -> Result<Self, Error> {
        let manager = MCacheManager::new();
        let Some(list) = conf.first_named("cache-list") else {
            return Ok(Self { manager });
        };
        let mut count = 0_usize;
        for node in list.children_named("cache") {
            let Some(name) = node.attribute("name").filter(|name| !name.is_empty()) else {
                continue;
            };
            let mut config = CacheConfig::new();
            if let Some(millis) = expire_millis(&node, &name, "expire-time-idle")? {
                config = config.expire_access_ms(millis);
            }
            if let Some(millis) = expire_millis(&node, &name, "expire-time-live")? {
                config = config.expire_creation_ms(millis);
            }
            manager.create::<String, String>(name, config)?;
            count += 1;
        }
        tracing::info!("Initialized {count} caches");
        Ok(Self { manager })
    }

    pub fn names(&self) -> Result<Vec<String>, Error> {
        self.manager.cache_names().map_err(Error::from)
    }

    pub fn get(&self, name: &str) -> Result<Option<MCache<String, String>>, Error> {
        self.manager.get(name).map_err(Error::from)
    }
}

fn expire_millis(node: &MNode, cache_name: &str, attr: &str) -> Result<Option<i64>, Error> {
    let Some(text) = node.attribute(attr).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if text == "0" {
        return Ok(None);
    }
    let seconds: i64 = text.parse().map_err(|_| {
        Error::new(format!(
            "cache {cache_name} has invalid {attr} `{text}`, expected seconds"
        ))
    })?;
    if seconds <= 0 {
        return Ok(None);
    }
    Ok(Some(seconds.saturating_mul(1_000)))
}
