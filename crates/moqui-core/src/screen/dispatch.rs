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

//! Rust bodies for the `rest.xml` transitions.

use std::collections::BTreeMap;
use std::time::Instant;

use moqui_lib::object::Value;

use super::definition::TransitionDef;
use crate::context::{Error, ExecutionContext};
use crate::entity::{query_rest, EntityFacade};
use crate::user::{self, login_user, logout};
use crate::web::{self, WebRequest};

const OTP_LATER: &str = "one-time passwords are not available until phase 11";
const SM_LATER: &str = "system messages not available until phase 11";
const ENTITY_AUTH: &str = "Authentication required for entity REST operations";

pub(crate) struct Call<'a> {
    pub request: &'a WebRequest,
    pub transition: &'a TransitionDef,
    pub extra: &'a [String],
    pub parameters: &'a BTreeMap<String, Value>,
    pub json_list: Option<&'a [Value]>,
    pub login_error: Option<&'a str>,
    pub started: Instant,
}

/// Run the wired body for a `rest.xml` transition. `None` means the transition is not wired, so
/// the caller runs its XML actions and response.
pub(crate) fn run(call: &Call<'_>) -> Result<Option<bool>, Error> {
    // Only the REST screen has Rust bodies. Another screen's transition of the same name is its own.
    if !call.transition.screen_location.ends_with("/webroot/rest.xml") {
        return Ok(None);
    }
    let name = call.transition.name.as_str();
    let sent = match name {
        "e1" | "v1" => entity_rest(call, false)?,
        "m1" => entity_rest(call, true)?,
        "s1" => service_rest(call)?,
        "login" => login(call)?,
        "logout" => logout_transition()?,
        "sendOtp" | "verifyOtp" => {
            web::send_json_error(501, OTP_LATER);
            true
        }
        "sm" => {
            web::send_json_error(501, SM_LATER);
            true
        }
        "entity.raml" | "master.raml" | "service.raml" => {
            web::send_json_error(404, "RAML is not built");
            true
        }
        "entity.swagger" | "master.swagger" => {
            web::send_json_error(404, "Swagger is not built");
            true
        }
        "entity.json" => super::schema::entity_json(call, false)?,
        "master.json" => super::schema::entity_json(call, true)?,
        "service.swagger" => super::schema::service_swagger(call)?,
        _ => return Ok(None),
    };
    Ok(Some(sent))
}

fn entity_rest(call: &Call<'_>, master: bool) -> Result<bool, Error> {
    if let Some(error) = call.login_error {
        web::send_json_error(401, error);
        return Ok(true);
    }
    if user::current_username()?.is_none() {
        web::send_json_error(401, ENTITY_AUTH);
        return Ok(true);
    }
    if call.extra.is_empty() {
        web::send_json_error(400, "No entity name or alias specified in path");
        return Ok(true);
    }
    let facade = facade()?;
    let verb = entity_verb(call.request);
    let entity_name = &call.extra[0];
    query_rest::begin_page_info();
    let result = match call.json_list {
        Some(items) => entity_list(&facade, &verb, entity_name, call, master, items),
        None => entity_one(&facade, &verb, entity_name, call.parameters, call.extra, master),
    };
    finish_entity(call.started, result);
    Ok(true)
}

fn entity_one(
    facade: &EntityFacade,
    verb: &str,
    entity_name: &str,
    parameters: &BTreeMap<String, Value>,
    extra: &[String],
    master: bool,
) -> Result<Value, Error> {
    let mut parameters = parameters.clone();
    put_rest_path(&mut parameters, extra, master);
    facade.rest(verb, entity_name, &parameters).map_err(Error::from)
}

fn entity_list(
    facade: &EntityFacade,
    verb: &str,
    entity_name: &str,
    call: &Call<'_>,
    master: bool,
    items: &[Value],
) -> Result<Value, Error> {
    let mut out = Vec::new();
    for item in items {
        let Value::Map(map) = item else {
            return Err(Error::new("JSON list elements must be objects"));
        };
        let mut parameters = call.parameters.clone();
        for (key, value) in map {
            parameters.insert(key.clone(), value.clone());
        }
        out.push(entity_one(facade, verb, entity_name, &parameters, call.extra, master)?);
    }
    Ok(Value::List(out))
}

fn finish_entity(started: Instant, result: Result<Value, Error>) {
    match result {
        Ok(value) => {
            page_headers();
            note_runtime(started);
            web::send_json_response(value);
        }
        Err(err) => {
            let _ = query_rest::take_page_info();
            web::send_json_error(web::status_for(&err), err.message());
        }
    }
}

fn service_rest(call: &Call<'_>) -> Result<bool, Error> {
    if let Some(error) = call.login_error {
        web::send_json_error(401, error);
        return Ok(true);
    }
    let before = message_error_count();
    let verb = call.request.method.to_ascii_lowercase();
    query_rest::begin_page_info();
    let result = match call.json_list {
        Some(items) => service_list(call, &verb, items),
        None => service_one(call.extra, call.parameters, &verb),
    };
    match result {
        Ok(value) => {
            let errors = message_errors();
            if !errors.is_empty() {
                let _ = query_rest::take_page_info();
                // Errors already recorded before the call predate this service, so keep them 500.
                // Errors the service added are the caller's input, 400.
                let status = if before > 0 { 500 } else { 400 };
                web::send_json_error(status, &errors.join("\n"));
            } else {
                page_headers();
                note_runtime(call.started);
                web::send_json_response(value);
            }
        }
        Err(err) => {
            let _ = query_rest::take_page_info();
            web::send_json_error(web::status_for(&err), err.message());
        }
    }
    Ok(true)
}

fn service_one(extra: &[String], parameters: &BTreeMap<String, Value>, verb: &str) -> Result<Value, Error> {
    let path: Vec<&str> = extra.iter().map(String::as_str).collect();
    crate::service::run(&path, verb, parameters)
}

fn service_list(call: &Call<'_>, verb: &str, items: &[Value]) -> Result<Value, Error> {
    let mut out = Vec::new();
    for item in items {
        let Value::Map(map) = item else {
            return Err(Error::new("JSON list elements must be objects"));
        };
        let mut parameters = call.parameters.clone();
        for (key, value) in map {
            parameters.insert(key.clone(), value.clone());
        }
        out.push(service_one(call.extra, &parameters, verb)?);
    }
    Ok(Value::List(out))
}

fn login(call: &Call<'_>) -> Result<bool, Error> {
    let username = text_param(call.parameters, "username");
    let password = text_param(call.parameters, "password");
    match login_user(&username, &password) {
        Ok(()) if user::session::session_attribute_flag("moquiAuthcFactorRequired") => {
            web::send_json_error(501, OTP_LATER);
        }
        Ok(()) => {
            let mut map = indexmap::IndexMap::new();
            map.insert("loggedIn".to_string(), Value::Bool(true));
            web::send_json_response(Value::Map(map));
        }
        Err(err) => {
            web::record_login_error(err.message());
            web::send_json_error(401, err.message());
        }
    }
    Ok(true)
}

fn logout_transition() -> Result<bool, Error> {
    if user::current_username()?.is_some() {
        logout()?;
        web::send_text_response("successful");
    } else {
        web::send_text_response("no_user");
    }
    Ok(true)
}

fn entity_verb(request: &WebRequest) -> String {
    let method = request.method.to_ascii_lowercase();
    if method == "post" {
        if let Some(over) = request.header("x-http-method-override") {
            let over = over.trim();
            if !over.is_empty() {
                return over.to_ascii_lowercase();
            }
        }
    }
    method
}

fn put_rest_path(parameters: &mut BTreeMap<String, Value>, extra: &[String], master: bool) {
    let path: Vec<Value> = extra.iter().skip(1).map(Value::string).collect();
    parameters.insert("path".to_string(), Value::List(path));
    if master {
        parameters.insert("masterNameInPath".to_string(), Value::Bool(true));
    }
}

fn facade() -> Result<EntityFacade, Error> {
    ExecutionContext::with_current(|ec| ec.factory().entity())
        .flatten()
        .ok_or_else(|| Error::new("no entity facade is installed"))
}

fn text_param(parameters: &BTreeMap<String, Value>, name: &str) -> String {
    parameters
        .get(name)
        .map(moqui_lib::object::to_plain_string)
        .unwrap_or_default()
}

fn message_error_count() -> usize {
    ExecutionContext::with_current(|ec| ec.message().errors().len()).unwrap_or(0)
}

fn message_errors() -> Vec<String> {
    ExecutionContext::with_current(|ec| ec.message().errors().to_vec()).unwrap_or_default()
}

/// The `X-Page-*` headers for the page info the last REST find recorded, if any.
fn page_headers() {
    if let Some(page) = query_rest::take_page_info() {
        header_i64("X-Total-Count", page.total);
        header_i64("X-Page-Index", page.index);
        header_i64("X-Page-Size", page.size);
        header_i64("X-Page-Max-Index", page.max_index);
        header_i64("X-Page-Range-Low", page.range_low);
        header_i64("X-Page-Range-High", page.range_high);
    }
}

fn header_i64(name: &str, value: i64) {
    web::add_header(name, value.to_string());
}

fn note_runtime(started: Instant) {
    let millis = u128::min(started.elapsed().as_millis(), u128::from(u64::MAX));
    web::add_header("X-Run-Time-ms", millis.to_string());
}
