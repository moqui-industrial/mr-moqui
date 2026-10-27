//! Shared host identity and credential-name classification.
use std::sync::OnceLock;
const SECRET_PARAMETER_NAMES: &[&str] = &["code", "moquisessiontoken", "sessiontoken", "sql", "sharedsecret"];

pub(crate) fn secret_parameter(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    crate::web::CREDENTIAL_PARAMETER_NAMES.contains(&lower.as_str())
        || SECRET_PARAMETER_NAMES.contains(&lower.as_str())
        || ["password", "secret", "token"].iter().any(|part| lower.contains(part))
        || ["_key", "apikey", "loginkey"].iter().any(|end| lower.ends_with(end))
}

pub(crate) fn server_address() -> &'static (String, String) {
    static ADDRESS: OnceLock<(String, String)> = OnceLock::new();
    ADDRESS.get_or_init(|| {
        let name = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .or_else(|| std::env::var("HOSTNAME").ok())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        let Some(name) = name else {
            return ("127.0.0.1".to_string(), "localhost".to_string());
        };
        // Java orders the resolved addresses IPv4 first (java.net.preferIPv6Addresses defaults to false), keeping
        // the resolver's order within each family, and takes the first.
        let addresses: Vec<std::net::IpAddr> = std::net::ToSocketAddrs::to_socket_addrs(&(name.as_str(), 0))
            .map(|found| found.map(|address| address.ip()).collect())
            .unwrap_or_default();
        let ip = addresses
            .iter()
            .find(|address| address.is_ipv4())
            .or_else(|| addresses.first())
            .map_or_else(|| "127.0.0.1".to_string(), ToString::to_string);
        (ip, name)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn credential_names_remain_redacted_after_hit_storage_retirement() {
        for name in [
            "PASSWORD",
            "authToken",
            "sharedSecret",
            "sql",
            "code",
            "apiKey",
            "private_key",
        ] {
            assert!(super::secret_parameter(name), "{name}");
        }
        for name in ["orderId", "primaryKeyId", "monkeyId"] {
            assert!(!super::secret_parameter(name), "{name}");
        }
    }
}
