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

//! The `/elastic` and `/kibana` proxies, Java's `ElasticSearchProxy` and `KibanaProxy` servlets
//! behind `MoquiAuthFilter`. They exist only on the OpenSearch backend. A request needs a login
//! (session, Basic, or `api_key`) and the `ElasticRemote` or `KibanaRemote` permission, as Java's
//! filter has it: 401 with a Basic challenge for nobody, 403 without the permission.
//!
//! `/elastic/...` goes to `elasticsearch_url`, `/kibana/...` to `http://${kibana_host}:${kibana_port}`
//! (set `server.basePath: "/kibana"` in Dashboards, which then expects the prefix stripped). The prefix is
//! removed for both, as Jetty's transparent proxy does with the servlets' `prefix`. The client's
//! `Authorization` and `Cookie` headers are not forwarded; `elasticsearch_user` and
//! `elasticsearch_password`, when set, are.

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::{ConnectInfo, Request, State};
use axum::response::Response;
use moqui_core::outbound::redacted;
use moqui_core::screen::PermissionCheck;
use moqui_core::web::WebRequest;

use crate::http::{internal_error, plain, read_body, HttpState};

/// Request and response bodies through the proxy are capped at this size.
const MAX_BODY: usize = 100 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);

/// Headers that belong to one connection and are not passed on.
const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
];

#[derive(Clone, Copy)]
enum Target {
    Elastic,
    Kibana,
}

pub(crate) async fn elastic(state: State<Arc<HttpState>>, peer: ConnectInfo<SocketAddr>, request: Request) -> Response {
    proxy(state, peer, request, Target::Elastic).await
}

pub(crate) async fn kibana(state: State<Arc<HttpState>>, peer: ConnectInfo<SocketAddr>, request: Request) -> Response {
    proxy(state, peer, request, Target::Kibana).await
}

async fn proxy(
    State(state): State<Arc<HttpState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    target: Target,
) -> Response {
    let has_elastic =
        moqui_core::search::installed(&state.factory).is_some_and(|facade| facade.backend().has_elastic_api());
    if !has_elastic {
        return plain(
            404,
            "the search backend is not OpenSearch, so there is nothing to proxy",
        );
    }
    let (permission, base) = match target {
        Target::Elastic => {
            let url = moqui_lib::env::get_prop_or_env("elasticsearch_url");
            let url = if url.trim().is_empty() {
                "http://127.0.0.1:9200".to_string()
            } else {
                url
            };
            ("ElasticRemote", url.trim().trim_end_matches('/').to_string())
        }
        Target::Kibana => {
            let host = moqui_lib::env::get_prop_or_env("kibana_host");
            if host.trim().is_empty() {
                return plain(404, "kibana_host is not set, so there is no Dashboards to proxy");
            }
            let port = moqui_lib::env::get_prop_or_env("kibana_port");
            let port = if port.trim().is_empty() {
                "5601".to_string()
            } else {
                port
            };
            ("KibanaRemote", format!("http://{}:{}", host.trim(), port.trim()))
        }
    };

    let (parts, body) = request.into_parts();
    let mut web = WebRequest::new(parts.method.as_str(), parts.uri.path());
    web.peer_ip = peer.ip().to_string();
    web.headers = crate::http::header_map(&parts.headers);
    web.query = moqui_core::web::form_pairs(parts.uri.query().unwrap_or(""));
    web.raw_query = parts.uri.query().map(str::to_string);
    let factory = Arc::clone(&state.factory);
    let job_factory = Arc::clone(&factory);
    let check_factory = Arc::clone(&factory);
    let (done, result) = tokio::sync::oneshot::channel();
    let submitted = factory.request_pool().try_submit(job_factory, move || {
        let _ = done.send(moqui_core::screen::check_permission(&check_factory, &web, permission));
        Ok(())
    });
    if submitted.is_err() {
        return state.guard.unavailable("service unavailable");
    }
    match result.await {
        Ok(Ok(PermissionCheck::Allowed)) => {}
        Ok(Ok(PermissionCheck::Unauthenticated)) => {
            return Response::builder()
                .status(401)
                .header("www-authenticate", "Basic realm=\"Moqui\"")
                .header("content-type", "text/plain; charset=utf-8")
                .body(axum::body::Body::from("login required"))
                .unwrap_or_else(|_| plain(401, "login required"));
        }
        Ok(Ok(PermissionCheck::Forbidden)) => return plain(403, &format!("the {permission} permission is required")),
        Ok(Err(err)) => return internal_error(err.message()),
        Err(_) => return plain(500, "request worker stopped"),
    }

    let mut url = format!("{base}{}", upstream_path(target, parts.uri.path()));
    if let Some(query) = parts.uri.query() {
        url.push('?');
        url.push_str(query);
    }
    let bytes = match read_body(&state, body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(refused) => return refused.response(),
    };
    match forward(&parts.method, &parts.headers, &url, bytes.to_vec(), target).await {
        Ok(response) => response,
        Err(message) => plain(502, &message),
    }
}

/// The path the upstream gets: the request's, less the `/elastic` or `/kibana` prefix, as Jetty's
/// `ProxyServlet$Transparent` strips its `prefix`. Nothing left is `/`.
fn upstream_path(target: Target, path: &str) -> &str {
    let prefix = match target {
        Target::Elastic => "/elastic",
        Target::Kibana => "/kibana",
    };
    let rest = path.strip_prefix(prefix).unwrap_or(path);
    if rest.is_empty() {
        "/"
    } else {
        rest
    }
}

fn client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(TIMEOUT)
                .build()
                .map_err(|err| format!("could not build the proxy HTTP client: {err}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

async fn forward(
    method: &axum::http::Method,
    headers: &axum::http::HeaderMap,
    url: &str,
    body: Vec<u8>,
    target: Target,
) -> Result<Response, String> {
    let method = reqwest::Method::from_bytes(method.as_str().as_bytes()).map_err(|err| err.to_string())?;
    let mut builder = client()?.request(method, url);
    for (name, value) in headers {
        let name = name.as_str();
        if HOP_BY_HOP.contains(&name) || name == "authorization" || name == "cookie" || name == "content-length" {
            continue;
        }
        builder = builder.header(name, value.as_bytes());
    }
    if matches!(target, Target::Elastic) {
        let user = moqui_lib::env::get_prop_or_env("elasticsearch_user");
        if !user.trim().is_empty() {
            let password = moqui_lib::env::get_prop_or_env("elasticsearch_password");
            builder = builder.basic_auth(user.trim(), Some(password));
        }
    }
    let mut response = builder
        .body(body)
        .send()
        .await
        .map_err(|err| format!("could not reach {}: {}", redacted(url), err.without_url()))?;
    let mut out = Response::builder().status(response.status().as_u16());
    for (name, value) in response.headers() {
        let name = name.as_str();
        if HOP_BY_HOP.contains(&name) || name == "content-length" {
            continue;
        }
        out = out.header(name, value.as_bytes());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|err| err.without_url().to_string())? {
        if bytes.len() + chunk.len() > MAX_BODY {
            return Err(format!(
                "the response from {} is larger than {MAX_BODY} bytes",
                redacted(url)
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    out.body(axum::body::Body::from(bytes))
        .map_err(|err| format!("could not build the proxy response: {err}"))
}

#[cfg(test)]
mod tests {
    use super::{upstream_path, Target};

    #[test]
    fn both_prefixes_are_stripped_as_jetty_strips_them() {
        assert_eq!(upstream_path(Target::Elastic, "/elastic/_cat/indices"), "/_cat/indices");
        assert_eq!(
            upstream_path(Target::Kibana, "/kibana/app/dashboards"),
            "/app/dashboards"
        );
        assert_eq!(upstream_path(Target::Kibana, "/kibana"), "/");
        assert_eq!(upstream_path(Target::Elastic, "/elastic/"), "/");
    }
}
