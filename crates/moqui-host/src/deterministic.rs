//! Synchronous protocol-v1 HTTP provider. No retries: a lost response is an ambiguous operation.
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use moqui_core::deterministic::{self, Provider};
use moqui_lib::error::Error;
use serde_json::{json, Value};

struct Hook {
    client: reqwest::blocking::Client,
    url: reqwest::Url,
    run: String,
    log: Mutex<File>,
}

fn required(name: &str) -> Result<String, Error> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| Error::new(format!("deterministic mode requires {name}")))
}

/// Initialize once per process; factory restarts preserve provider, sequence and append log.
pub fn offline() -> bool {
    deterministic::enabled() && std::env::var("MOQUI_DETERMINISTIC_OFFLINE").as_deref() == Ok("true")
}

pub fn init() -> Result<(), Error> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    if !deterministic::requested() {
        return Ok(());
    }
    INIT.get_or_init(|| install().map_err(|err| err.to_string()))
        .clone()
        .map_err(Error::new)
}

fn install() -> Result<(), Error> {
    let url = reqwest::Url::parse(&required("MOQUI_DETERMINISTIC_HOOK")?)
        .map_err(|_| Error::new("invalid deterministic hook URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/transaction"
    {
        return Err(Error::new(
            "deterministic hook must be an HTTP(S) /transaction URL without credentials, query or fragment",
        ));
    }
    if required("MOQUI_DETERMINISTIC_ENGINE")? != "rust" {
        return Err(Error::new("MOQUI_DETERMINISTIC_ENGINE must be rust"));
    }
    let run = required("MOQUI_DETERMINISTIC_RUN")?;
    let path = required("MOQUI_DETERMINISTIC_LOG")?;
    if !Path::new(&path).is_absolute() {
        return Err(Error::new("deterministic log path must be absolute"));
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let log = options
        .open(path)
        .map_err(|_| Error::new("could not open deterministic append log"))?;
    let timeout = std::env::var("MOQUI_DETERMINISTIC_TIMEOUT_SECONDS")
        .unwrap_or_else(|_| "120".to_string())
        .parse::<u64>()
        .ok()
        .filter(|seconds| (1..=3600).contains(seconds))
        .ok_or_else(|| Error::new("MOQUI_DETERMINISTIC_TIMEOUT_SECONDS must be 1..3600"))?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(timeout))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| Error::new("could not initialize deterministic HTTP client"))?;
    tracing::warn!(
        "Deterministic mode: application work is serial, async services drain inline; autonomous scheduled jobs are unsupported"
    );
    deterministic::install(Arc::new(Hook {
        client,
        url,
        run,
        log: Mutex::new(log),
    }))?;
    if offline() {
        tracing::warn!("Explicit deterministic offline mode: search, cluster and SSH disabled");
    }
    Ok(())
}

pub fn bootstrap() -> Result<Option<deterministic::Operation>, Error> {
    init()?;
    deterministic::bootstrap()
}

impl Hook {
    fn envelope(&self, mut value: Value) -> Value {
        value["version"] = json!(1);
        value["run_id"] = json!(self.run);
        value["engine"] = json!("rust");
        value
    }

    fn post(&self, request: &Value) -> Result<Value, Error> {
        let response = self
            .client
            .post(self.url.clone())
            .json(request)
            .send()
            .map_err(|_| Error::new("deterministic hook transport failed"))?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(Error::new("deterministic hook returned non-200 status"));
        }
        let mut bytes = Vec::new();
        response
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::new("deterministic hook response read failed"))?;
        if bytes.len() > 65536 {
            return Err(Error::new("deterministic hook response too large"));
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::new("deterministic hook returned invalid JSON"))
    }

    fn append(&self, value: &Value) -> Result<(), Error> {
        let mut log = self.log.lock().unwrap();
        let mut bytes = serde_json::to_vec(value).map_err(|_| Error::new("deterministic event encoding failed"))?;
        bytes.push(b'\n');
        log.write_all(&bytes)
            .and_then(|()| log.flush())
            .map_err(|_| Error::new("deterministic log write/flush failed"))
    }
}

impl Provider for Hook {
    fn begin(&self, request: Value) -> Result<Value, Error> {
        let mut request = self.envelope(request);
        let response = self.post(&request)?;
        let id = response
            .get("operation_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        let millis = response.get("time_epoch_ms").and_then(Value::as_i64);
        let (Some(id), Some(millis)) = (id, millis) else {
            return Err(Error::new("invalid deterministic begin response"));
        };
        if id.len() > 256 || id.chars().any(char::is_control) || jiff::Timestamp::from_millisecond(millis).is_err() {
            return Err(Error::new("invalid deterministic assignment"));
        }
        request["operation_id"] = json!(id);
        request["time_epoch_ms"] = json!(millis);
        self.append(&request)?;
        Ok(response)
    }

    fn complete(&self, request: Value) -> Result<(), Error> {
        let request = self.envelope(request);
        let response = self.post(&request)?;
        if response.get("ok") != Some(&Value::Bool(true)) {
            return Err(Error::new("invalid deterministic completion response"));
        }
        self.append(&request)
    }
}
