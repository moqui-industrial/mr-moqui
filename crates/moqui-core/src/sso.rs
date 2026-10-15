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

//! The SSO relying party of the `moqui-sso` component (`plans/archive/Sso.md`): OpenID Connect and OAuth 2 authorization code
//! logins configured by `moqui.security.sso.AuthFlow` rows. Core has no HTTP or JOSE stack; the host installs an
//! [`SsoClient`] (`huskarl`) that builds the authorization URL, completes the code exchange and validates the ID token,
//! and reads the user info. The account sync, the group sync, and the login are core
//! (`crate::service::sso`). SAML flows are defined and report that SAML is not available.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use moqui_lib::error::Error;
use moqui_lib::object::Value;

/// What a caller sees when SSO is used and the host installed no client.
pub const SSO_UNAVAILABLE: &str = "SSO is not available in this build";

/// One `OidcFlow` row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OidcConfig {
    /// `OctApple`, `OctAzureAd`, `OctGoogle`, `OctKeycloak`, or `OctOther`.
    pub client_type: String,
    pub client_id: String,
    pub secret: String,
    pub discovery_uri: String,
    /// Keycloak: with `base_uri`, the discovery URI when `discovery_uri` is empty.
    pub realm: String,
    pub base_uri: String,
    /// The `optionValue` of `preferredJwsAlgorithmEnumId`, such as `RS256`.
    pub preferred_jws_algorithm: String,
    pub use_nonce: bool,
    /// Apple: the client secret is a JWT signed with the private key in `secret`.
    pub apple_team_id: String,
    pub apple_key_id: String,
}

/// One `OauthFlow` row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OauthConfig {
    /// `OctGitHub`, `OctGoogle`, ..., or `OctOauth20` with the URLs set.
    pub client_type: String,
    pub client_id: String,
    pub secret: String,
    pub authorize_url: String,
    pub token_url: String,
    pub user_info_url: String,
    pub scope: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowKind {
    Oidc(OidcConfig),
    Oauth(OauthConfig),
}

/// An `AuthFlow` with its protocol row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowConfig {
    pub auth_flow_id: String,
    pub kind: FlowKind,
}

/// The redirect to the identity provider, and the state to keep in the session until the callback.
#[derive(Clone, Debug)]
pub struct SsoStart {
    pub authorize_url: String,
    /// Opaque to core: the host's pending state (PKCE verifier, state, nonce), serialized.
    pub pending: String,
}

/// Who the identity provider says logged in.
#[derive(Clone, Debug, Default)]
pub struct SsoProfile {
    /// The provider's subject (`sub`, or the user info `id`): `UserAccount.externalUserId`.
    pub id: String,
    /// The Moqui username, when the provider gave one.
    pub username: Option<String>,
    /// Every claim of the ID token and the user info, for `AuthFlowFieldMap`.
    pub attributes: BTreeMap<String, Value>,
    /// The provider's roles or groups, for `AuthFlowRoleMap`.
    pub roles: Vec<String>,
    /// The raw ID token, for the `id_token_hint` of a logout at the provider.
    pub id_token: Option<String>,
}

/// The protocol work, installed by the host.
pub trait SsoClient: Send + Sync {
    /// Build the authorization request.
    ///
    /// # Errors
    ///
    /// Returns an error when discovery or the request fails.
    fn start(&self, flow: &FlowConfig, redirect_uri: &str) -> Result<SsoStart, Error>;

    /// Complete the callback: check the state, exchange the code, validate the ID token, read the user info.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider refused, the state does not match, or a token does not validate.
    fn complete(
        &self,
        flow: &FlowConfig,
        pending: &str,
        callback: &BTreeMap<String, String>,
    ) -> Result<SsoProfile, Error>;

    /// The provider's end-session URL, when it has one.
    ///
    /// # Errors
    ///
    /// Returns an error when discovery fails.
    fn logout_url(
        &self,
        flow: &FlowConfig,
        id_token_hint: Option<&str>,
        post_logout_redirect_uri: &str,
    ) -> Result<Option<String>, Error>;
}

fn slot() -> &'static RwLock<Option<Arc<dyn SsoClient>>> {
    static SLOT: RwLock<Option<Arc<dyn SsoClient>>> = RwLock::new(None);
    &SLOT
}

/// Use `client` for every SSO flow in this process.
pub fn install_sso_client(client: Arc<dyn SsoClient>) {
    *slot().write().unwrap_or_else(PoisonError::into_inner) = Some(client);
}

/// Remove the installed client.
pub fn clear_sso_client() {
    *slot().write().unwrap_or_else(PoisonError::into_inner) = None;
}

pub(crate) fn client() -> Result<Arc<dyn SsoClient>, Error> {
    slot()
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or_else(|| Error::new(SSO_UNAVAILABLE))
}
