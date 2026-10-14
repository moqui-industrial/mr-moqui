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

//! `org.moqui.sso.AuthServices`: `login#User`, `handle#Callback`, and `logout#User`, from the `moqui-sso` component's
//! `AuthenticationFlow` and `MoquiSecurityGrantedAccessAdapter`, on the host's [`crate::sso::SsoClient`].
//!
//! The account sync is Java's: the `AuthFlowFieldMap` rules map the profile attributes (an expression, or a mapping
//! service), the `UserAccount` with the profile's username is updated through `update#UserAccount` or created, and
//! `UserGroupMember` follows `AuthFlowRoleMap`: a mapped role adds its group, a current group no mapped role names is
//! ended unless it is the flow's `defaultUserGroupId`, and a user left with no group gets the default group. The login
//! replaces the session id. A failure is logged and the browser goes back to `/Login`, as in Java.

use std::collections::{BTreeMap, BTreeSet};

use moqui_lib::error::Error as LibError;
use moqui_lib::object::{is_empty, to_plain_string, Value};

use crate::context::ExecutionContext;
use crate::entity::{EntityFacade, EntityValue};
use crate::service::call::{sync_with, CallOptions};
use crate::sso::{FlowConfig, FlowKind, OauthConfig, OidcConfig, SsoProfile};
use crate::user::session::{session_attribute, set_session_attribute};

const LOGIN: &str = "org.moqui.sso.AuthServices.login#User";
const CALLBACK: &str = "org.moqui.sso.AuthServices.handle#Callback";
const LOGOUT: &str = "org.moqui.sso.AuthServices.logout#User";

const PENDING: &str = "moquiAuthFlowPending";
const RETURN_TO: &str = "moquiAuthFlowReturnTo";
const EXTERNAL_LOGOUT: &str = "moquiAuthFlowExternalLogout";
const FLOW_ID: &str = "moquiAuthFlowId";
const ID_TOKEN: &str = "moquiAuthFlowIdToken";

pub(super) fn runs(name: &str) -> bool {
    matches!(name, LOGIN | CALLBACK | LOGOUT)
}

pub(super) fn run(
    name: &str,
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let outcome = match name {
        LOGIN => login(parameters, facade),
        CALLBACK => callback(facade),
        LOGOUT => logout(parameters, facade),
        other => return Err(LibError::new(format!("SSO service {other} is not implemented"))),
    };
    if let Err(err) = outcome {
        tracing::error!("SSO {name} failed: {}", err.message());
        crate::web::send_redirect(&format!("{}/Login", base()));
    }
    Ok(BTreeMap::new())
}

/// The absolute root of this server, for the callback URL the provider redirects to: the render's base URL when it is
/// absolute, else the scheme and host of the request, else the conf `webroot` URL.
fn base() -> String {
    let base = crate::web::base_url();
    if base.contains("://") {
        return base.trim_end_matches('/').to_string();
    }
    if let Some(url) = crate::web::request_url() {
        if let Some((scheme, rest)) = url.split_once("://") {
            let host = rest.split('/').next().unwrap_or("");
            return format!("{scheme}://{host}");
        }
    }
    ExecutionContext::with_current(|ctx| ctx.factory().webapp_root_url("webroot"))
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_string()
}

fn text(parameters: &BTreeMap<String, Value>, name: &str) -> String {
    parameters
        .get(name)
        .filter(|value| !is_empty(value))
        .map(to_plain_string)
        .unwrap_or_default()
}

fn field(row: &EntityValue, name: &str) -> String {
    row.get(name)
        .filter(|value| !is_empty(value))
        .map(to_plain_string)
        .unwrap_or_default()
}

/// The same-origin `returnTo`, or none.
fn safe_return_to(url: &str) -> Option<String> {
    (!url.is_empty() && crate::web::is_same_origin_redirect(url, None)).then(|| url.to_string())
}

fn find_one(facade: &EntityFacade, entity: &str, id: &str) -> Result<Option<EntityValue>, LibError> {
    if id.is_empty() {
        return Ok(None);
    }
    facade
        .find(entity)?
        .disable_authz()
        .eq("authFlowId", Value::string(id))
        .one()
}

/// The flow and its protocol row. A disabled or inbound flow is not a login.
fn load_flow(facade: &EntityFacade, auth_flow_id: &str) -> Result<(EntityValue, FlowConfig), LibError> {
    let flow = find_one(facade, "moqui.security.sso.AuthFlow", auth_flow_id)?
        .ok_or_else(|| LibError::new(format!("AuthFlow {auth_flow_id} not found")))?;
    if field(&flow, "disabled") == "Y" || field(&flow, "inbound") == "Y" {
        return Err(LibError::new(format!(
            "AuthFlow {auth_flow_id} is not available for login"
        )));
    }
    let kind = match field(&flow, "authFlowTypeEnumId").as_str() {
        "AftOidc" => {
            let row = find_one(facade, "moqui.security.sso.OidcFlow", auth_flow_id)?
                .ok_or_else(|| LibError::new(format!("AuthFlow {auth_flow_id} has no OidcFlow")))?;
            let algorithm_id = field(&row, "preferredJwsAlgorithmEnumId");
            let preferred_jws_algorithm = if algorithm_id.is_empty() {
                String::new()
            } else {
                facade
                    .find("moqui.basic.Enumeration")?
                    .disable_authz()
                    .eq("enumId", Value::string(&algorithm_id))
                    .one()?
                    .map(|row| field(&row, "optionValue"))
                    .unwrap_or_default()
            };
            FlowKind::Oidc(OidcConfig {
                client_type: field(&row, "clientTypeEnumId"),
                client_id: field(&row, "clientId"),
                secret: field(&row, "secret"),
                discovery_uri: field(&row, "discoveryUri"),
                realm: field(&row, "realm"),
                base_uri: field(&row, "baseUri"),
                preferred_jws_algorithm,
                use_nonce: field(&row, "useNonce") == "Y",
                apple_team_id: field(&row, "appleTeamId"),
                apple_key_id: field(&row, "appleKeyId"),
            })
        }
        "AftOauth" => {
            let row = find_one(facade, "moqui.security.sso.OauthFlow", auth_flow_id)?
                .ok_or_else(|| LibError::new(format!("AuthFlow {auth_flow_id} has no OauthFlow")))?;
            FlowKind::Oauth(OauthConfig {
                client_type: field(&row, "clientTypeEnumId"),
                client_id: field(&row, "clientId"),
                secret: field(&row, "secret"),
                authorize_url: field(&row, "authorizeUrl"),
                token_url: field(&row, "tokenUrl"),
                user_info_url: field(&row, "userInfoUrl"),
                scope: field(&row, "scope"),
            })
        }
        "AftSaml" => {
            return Err(LibError::new(format!(
                "AuthFlow {auth_flow_id} is SAML, which is not available in this port"
            )))
        }
        other => {
            return Err(LibError::new(format!(
                "AuthFlow {auth_flow_id} has unknown type {other}"
            )))
        }
    };
    Ok((
        flow,
        FlowConfig {
            auth_flow_id: auth_flow_id.to_string(),
            kind,
        },
    ))
}

/// `login#User`: keep a same-origin `returnTo`, start the flow, keep its state in the session, and redirect.
fn login(parameters: &BTreeMap<String, Value>, facade: &EntityFacade) -> Result<(), LibError> {
    let auth_flow_id = text(parameters, "authFlowId");
    let (_, flow) = load_flow(facade, &auth_flow_id)?;
    match safe_return_to(&text(parameters, "returnTo")) {
        Some(url) => set_session_attribute(RETURN_TO, Some(&url)),
        None => set_session_attribute(RETURN_TO, None),
    }
    let redirect_uri = format!("{}/sso/callback", base());
    let started = crate::sso::client()?.start(&flow, &redirect_uri)?;
    let pending = serde_json::json!({ "authFlowId": auth_flow_id, "pending": started.pending });
    set_session_attribute(PENDING, Some(&pending.to_string()));
    crate::web::send_redirect(&started.authorize_url);
    Ok(())
}

/// `handle#Callback`: complete the flow the session started, sync the account, log in.
fn callback(facade: &EntityFacade) -> Result<(), LibError> {
    let stored = session_attribute(PENDING).ok_or_else(|| LibError::new("no SSO login is in progress"))?;
    // One callback per login: the state cannot be replayed.
    set_session_attribute(PENDING, None);
    let stored: serde_json::Value =
        serde_json::from_str(&stored).map_err(|err| LibError::new(format!("SSO state is not readable: {err}")))?;
    let auth_flow_id = stored
        .get("authFlowId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let pending = stored
        .get("pending")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let (flow_row, flow) = load_flow(facade, &auth_flow_id)?;
    let callback: BTreeMap<String, String> = crate::web::parameters()
        .into_iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "code" | "state" | "iss" | "error" | "error_description" | "error_uri"
            )
        })
        .map(|(name, value)| (name, to_plain_string(&value)))
        .collect();
    let profile = crate::sso::client()?.complete(&flow, &pending, &callback)?;
    let username = profile
        .username
        .clone()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| LibError::new(format!("AuthFlow {auth_flow_id}: the provider gave no username")))?;
    sync_account(facade, &flow_row, &profile, &username)?;
    crate::user::login_external_user(&username).map_err(|err| LibError::new(err.message()))?;
    set_session_attribute(EXTERNAL_LOGOUT, Some("true"));
    set_session_attribute(FLOW_ID, Some(&auth_flow_id));
    set_session_attribute(ID_TOKEN, profile.id_token.as_deref());
    match session_attribute(RETURN_TO).and_then(|url| safe_return_to(&url)) {
        Some(url) => set_session_attribute(RETURN_TO, Some(&url)),
        None => set_session_attribute(RETURN_TO, None),
    }
    tracing::info!("SSO login of {username} through AuthFlow {auth_flow_id}");
    Ok(())
}

/// `logout#User`: end the local login, then send the browser to the provider's end-session URL when it has one,
/// else to a same-origin `returnTo` or `/Login`.
fn logout(parameters: &BTreeMap<String, Value>, facade: &EntityFacade) -> Result<(), LibError> {
    let base = base();
    let after = safe_return_to(&text(parameters, "returnTo")).unwrap_or_else(|| format!("{base}/Login"));
    let after = if after.starts_with('/') {
        format!("{base}{after}")
    } else {
        after
    };
    let auth_flow_id = session_attribute(FLOW_ID).unwrap_or_default();
    let id_token = session_attribute(ID_TOKEN);
    let provider_url = if auth_flow_id.is_empty() {
        None
    } else {
        match load_flow(facade, &auth_flow_id) {
            Ok((_, flow)) => crate::sso::client()?.logout_url(&flow, id_token.as_deref(), &after)?,
            Err(err) => {
                tracing::warn!("SSO logout: {}", err.message());
                None
            }
        }
    };
    for name in [EXTERNAL_LOGOUT, FLOW_ID, ID_TOKEN, RETURN_TO] {
        set_session_attribute(name, None);
    }
    crate::user::logout().map_err(|err| LibError::new(err.message()))?;
    crate::web::send_redirect(provider_url.as_deref().unwrap_or(&after));
    Ok(())
}

/// A service call with authorization off, as the anonymous-all callback runs it. A failure is an error here.
fn call(name: &str, parameters: BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>, LibError> {
    let options = CallOptions {
        disable_authz: true,
        ..CallOptions::default()
    };
    let result = sync_with(name, &parameters, &options);
    let errors = ExecutionContext::with_current(|ctx| {
        let mut message = ctx.message();
        let text = message.errors().join("\n");
        message.clear();
        text
    })
    .unwrap_or_default();
    match result {
        Ok(_) if !errors.is_empty() => Err(LibError::new(format!("{name}: {errors}"))),
        Ok(out) => Ok(out),
        Err(err) => Err(LibError::new(format!("{name}: {}", err.message()))),
    }
}

/// `mapProfileFields`: each rule's expression (or its source field name) against the profile attributes; a value goes
/// to `dstFieldName`, or through the rule's mapping service, whose `destinationMap` and `sourceMap` are merged.
fn map_fields(
    facade: &EntityFacade,
    auth_flow_id: &str,
    attributes: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, LibError> {
    let rules = facade
        .find("moqui.security.sso.AuthFlowFieldMap")?
        .disable_authz()
        .eq("authFlowId", Value::string(auth_flow_id))
        .order_by("ruleSeqId", false)
        .list()?;
    let mut out = BTreeMap::new();
    for rule in rules.values() {
        let expression = match field(rule, "dstFieldExpression") {
            text if text.is_empty() => field(rule, "srcFieldName"),
            text => text,
        };
        if expression.is_empty() {
            continue;
        }
        let value = {
            let _frame = crate::script::ContextFrame::push();
            for (name, value) in attributes {
                crate::script::set_context_name(name, value.clone()).map_err(|err| LibError::new(err.message()))?;
            }
            // A claim the provider did not send is no value, as a missing name in a Groovy binding.
            match crate::script::eval_value(&expression) {
                Ok(value) => value,
                Err(err) => {
                    tracing::debug!("AuthFlowFieldMap {expression}: {}", err.message());
                    Value::Null
                }
            }
        };
        if is_empty(&value) || matches!(value, Value::Bool(false)) {
            continue;
        }
        let destination = field(rule, "dstFieldName");
        let register_id = field(rule, "mappingServiceRegisterId");
        if register_id.is_empty() {
            out.insert(destination, value);
            continue;
        }
        let register = facade
            .find("moqui.service.ServiceRegister")?
            .disable_authz()
            .eq("serviceRegisterId", Value::string(&register_id))
            .one()?
            .ok_or_else(|| LibError::new(format!("ServiceRegister {register_id} not found")))?;
        let mut parameters = BTreeMap::new();
        parameters.insert("srcFieldValue".to_string(), value);
        parameters.insert("dstFieldName".to_string(), Value::string(destination));
        let mapped = call(&field(&register, "serviceName"), parameters)?;
        for key in ["destinationMap", "sourceMap"] {
            if let Some(Value::Map(map)) = mapped.get(key) {
                out.extend(map.iter().map(|(name, value)| (name.clone(), value.clone())));
            }
        }
    }
    Ok(out)
}

fn now_value() -> Value {
    crate::user::now_value()
}

/// `MoquiSecurityGrantedAccessAdapter.adapt` for one profile.
fn sync_account(
    facade: &EntityFacade,
    flow: &EntityValue,
    profile: &SsoProfile,
    username: &str,
) -> Result<(), LibError> {
    let auth_flow_id = field(flow, "authFlowId");
    let mapped = map_fields(facade, &auth_flow_id, &profile.attributes)?;
    let existing = facade
        .find("moqui.security.UserAccount")?
        .disable_authz()
        .eq("username", Value::string(username))
        .use_cache(false)
        .one()?;
    let mut parameters = mapped;
    parameters.insert("externalUserId".to_string(), Value::string(&profile.id));
    parameters.insert("username".to_string(), Value::string(username));
    let user_id = match existing {
        Some(account) => {
            let user_id = field(&account, "userId");
            parameters.insert("userId".to_string(), Value::string(&user_id));
            call("org.moqui.impl.UserServices.update#UserAccount", parameters)?;
            user_id
        }
        None => {
            let created = call("create#moqui.security.UserAccount", parameters)?;
            created.get("userId").map(to_plain_string).unwrap_or_default()
        }
    };
    if user_id.is_empty() {
        return Err(LibError::new(format!("no UserAccount for {username}")));
    }

    let now = now_value();
    let current = active_members(facade, &user_id)?;
    let mut obsolete: BTreeSet<String> = current.iter().map(|member| field(member, "userGroupId")).collect();
    let roles: BTreeSet<&str> = profile.roles.iter().map(String::as_str).collect();
    for role in roles {
        let mapped = facade
            .find("moqui.security.sso.AuthFlowRoleMap")?
            .disable_authz()
            .eq("authFlowId", Value::string(&auth_flow_id))
            .eq("roleName", Value::string(role))
            .one()?;
        let Some(group) = mapped.map(|row| field(&row, "userGroupId")).filter(|id| !id.is_empty()) else {
            continue;
        };
        if !obsolete.remove(&group) {
            add_member(&group, &user_id, &now)?;
        }
    }
    let default_group = field(flow, "defaultUserGroupId");
    for member in &current {
        let group = field(member, "userGroupId");
        if obsolete.contains(&group) && group != default_group {
            let mut parameters = BTreeMap::new();
            parameters.insert("userGroupId".to_string(), Value::string(&group));
            parameters.insert("userId".to_string(), Value::string(&user_id));
            parameters.insert(
                "fromDate".to_string(),
                member.get("fromDate").cloned().unwrap_or(Value::Null),
            );
            parameters.insert("thruDate".to_string(), now.clone());
            call("update#moqui.security.UserGroupMember", parameters)?;
        }
    }
    if !default_group.is_empty() && active_members(facade, &user_id)?.is_empty() {
        add_member(&default_group, &user_id, &now)?;
    }
    Ok(())
}

fn add_member(group: &str, user_id: &str, now: &Value) -> Result<(), LibError> {
    let mut parameters = BTreeMap::new();
    parameters.insert("userGroupId".to_string(), Value::string(group));
    parameters.insert("userId".to_string(), Value::string(user_id));
    parameters.insert("fromDate".to_string(), now.clone());
    call("create#moqui.security.UserGroupMember", parameters).map(|_| ())
}

/// The user's `UserGroupMember` rows in date now.
fn active_members(facade: &EntityFacade, user_id: &str) -> Result<Vec<EntityValue>, LibError> {
    let now = crate::user::now_millis();
    let rows = facade
        .find("moqui.security.UserGroupMember")?
        .disable_authz()
        .eq("userId", Value::string(user_id))
        .use_cache(false)
        .list()?;
    Ok(rows
        .values()
        .iter()
        .filter(|row| {
            let from = row.get("fromDate").and_then(crate::user::value_millis);
            let thru = row.get("thruDate").and_then(crate::user::value_millis);
            from.is_none_or(|from| from <= now) && thru.is_none_or(|thru| thru > now)
        })
        .cloned()
        .collect())
}
