//! Application entropy. The host installs coordinator-seeded bytes only in deterministic test mode.
//! Semantic domains keep internal draws from shifting entity ids. Never use test mode in production.
use std::sync::{Arc, OnceLock};

use crate::error::Error;

type Source = dyn Fn(&str, usize) -> Result<Vec<u8>, Error> + Send + Sync;
static SOURCE: OnceLock<Arc<Source>> = OnceLock::new();

pub fn install(source: Arc<Source>) -> Result<(), Error> {
    SOURCE
        .set(source)
        .map_err(|_| Error::new("deterministic random source already installed"))
}

pub fn deterministic() -> bool {
    crate::clock::deterministic() || std::env::var("MOQUI_DETERMINISTIC").as_deref() == Ok("true")
}

pub fn fill(domain: &str, bytes: &mut [u8]) -> Result<(), Error> {
    if deterministic() {
        if bytes.is_empty() {
            return Ok(());
        }
        let source = SOURCE
            .get()
            .ok_or_else(|| Error::new("deterministic random source not installed"))?;
        let result = source(domain, bytes.len())?;
        if result.len() != bytes.len() {
            return Err(Error::new("invalid deterministic random length"));
        }
        bytes.copy_from_slice(&result);
        return Ok(());
    }
    getrandom::fill(bytes).map_err(|err| Error::new(format!("random source failed: {err}")))
}

pub fn unit(domain: &str) -> Result<f64, Error> {
    let mut bytes = [0; 8];
    fill(domain, &mut bytes)?;
    Ok((u64::from_be_bytes(bytes) >> 11) as f64 * (1.0 / (1_u64 << 53) as f64))
}

pub fn below(domain: &str, bound: u32) -> Result<u32, Error> {
    if bound == 0 {
        return Err(Error::new("random bound must be positive"));
    }
    let limit = (1_u64 << 32) - (1_u64 << 32) % u64::from(bound);
    loop {
        let mut bytes = [0; 4];
        fill(domain, &mut bytes)?;
        let value = u32::from_be_bytes(bytes);
        if u64::from(value) < limit {
            return Ok(value % bound);
        }
    }
}
