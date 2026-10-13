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

//! `entity.json`, `master.json`, and `service.swagger`. YAML and Swagger 2 stay unbuilt.

use super::dispatch::Call;
use crate::context::{Error, ExecutionContext};
use crate::entity::EntityFacade;
use crate::user;
use crate::web;

const SCHEMA_AUTH: &str = "Authentication required for REST schema";

pub(crate) fn entity_json(call: &Call<'_>, master: bool) -> Result<bool, Error> {
    if !logged_in(call.login_error)? {
        return Ok(true);
    }
    if yaml(call.extra) {
        web::send_json_error(404, "YAML is not built");
        return Ok(true);
    }
    let facade = facade()?;
    match crate::entity::schema_document(&facade, &plain_names(call.extra), master) {
        Ok(value) => web::send_json_response(value),
        Err(err) if client_error(err.message()) => web::send_json_error(400, err.message()),
        Err(err) => return Err(Error::from(err)),
    }
    Ok(true)
}

pub(crate) fn service_swagger(call: &Call<'_>) -> Result<bool, Error> {
    if !logged_in(call.login_error)? {
        return Ok(true);
    }
    if yaml(call.extra) {
        web::send_json_error(404, "YAML is not built");
        return Ok(true);
    }
    match crate::service::openapi_document(&plain_names(call.extra)) {
        Ok(value) => web::send_json_response(value),
        Err(err) if err.message() == "No root resource name specified in path" => {
            web::send_json_error(400, err.message());
        }
        Err(err) if err.message().contains("root resource not found") => {
            web::send_json_error(404, err.message());
        }
        Err(err) => return Err(err),
    }
    Ok(true)
}

fn logged_in(login_error: Option<&str>) -> Result<bool, Error> {
    if let Some(error) = login_error {
        web::send_json_error(401, error);
        return Ok(false);
    }
    if user::current_username()?.is_none() {
        web::send_json_error(401, SCHEMA_AUTH);
        return Ok(false);
    }
    Ok(true)
}

fn yaml(extra: &[String]) -> bool {
    extra.iter().any(|part| {
        let lower = part.to_ascii_lowercase();
        lower.ends_with(".yaml") || lower.ends_with(".yml")
    })
}

fn plain_names(extra: &[String]) -> Vec<String> {
    extra
        .iter()
        .map(|part| part.strip_suffix(".json").unwrap_or(part))
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn client_error(message: &str) -> bool {
    message.starts_with("No entity found") || message.starts_with("Master name") || message.contains("not unique")
}

fn facade() -> Result<EntityFacade, Error> {
    ExecutionContext::with_current(|ec| ec.factory().entity())
        .flatten()
        .ok_or_else(|| Error::new("no entity facade is installed"))
}
