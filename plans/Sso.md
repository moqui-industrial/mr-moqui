# SSO relying party

OpenID Connect and OAuth 2 login, the `moqui-sso` component in the framework. Built in phase 12.

This follows Sessions and login in `plans/Architecture-Proposal.md`. Password login, the login key, the API key, and artifact authorization stay as they are.

## Decision

`huskarl` 0.11 is the relying party. It is MIT OR Apache-2.0, and it declares Rust 1.92. It lives in `moqui-host`, with the other process-edge dependencies. Default features stay on. The only default feature is `default-jws-verifier-platform`, which on Linux links `huskarl-crypto-native`. `authorization-flow-loopback`, `metrics`, and `experimental-oidc-key-binding` stay off. `huskarl-reqwest`, `huskarl-redis`, and `huskarl-google-cloud` are not dependencies. The host adapts the async `reqwest` already in the graph (rustls on `aws-lc-rs`, 1 MiB bodies, no redirects) on its edge runtime.

The locked crate is `huskarl` 0.11.6, with `huskarl-crypto-native` 0.11.1. JOSE verification uses that RustCrypto stack: `ed25519-dalek` 3, `p256` 0.14, and `p384` 0.14, the same generations `russh` uses. `rsa` resolves to 0.10.0-rc.18, the same copy `russh` already links. TLS stays on `aws-lc-rs`. `huskarl` does not add `ring`. P-256 and P-384 are covered. P-521 (`ES512`, the Java `OjaES512` row) is not, and a flow that prefers it is refused. Token expiry stays inside `huskarl`. Moqui time stays `jiff`. `huskarl` does not pull `chrono`. Its claims parse under the workspace's `serde_json` `arbitrary_precision`, which was checked, since that feature broke `chromiumoxide`.

`openidconnect` stays out. It pins older `rsa`, `p256`, `ed25519-dalek`, and reqwest. `oauth2` and `jsonwebtoken` stay out. They are the fallback if a later review rejects `huskarl`, and they are not a second stack.

## Login and logout

The entities are the Java ones in `moqui.security.sso`: `AuthFlow`, `AuthFlowFieldMap`, `AuthFlowRoleMap`, `OidcFlow`, `OauthFlow`, and `SamlFlow`, with their enumerations. `OauthFlow` adds `authorizeUrl`, `tokenUrl`, `userInfoUrl`, and `scope`; `OidcFlow` adds `appleTeamId` and `appleKeyId`.

`/sso/login`, `/sso/callback`, and `/sso/logout` are the `sso` webroot screen, and their services are `org.moqui.sso.AuthServices` with Rust bodies (`crates/moqui-core/src/service/sso.rs`) on the host client (`crates/moqui-host/src/sso.rs`). The Login screen's SSO tab lists the flows that are not inbound and not disabled.

- OpenID Connect is the primary protocol. Discovery is `discoveryUri`, else the provider's document: Google and Apple are fixed, Keycloak is `baseUri/realms/realm`, Azure AD is the `common` tenant (name the tenant in `discoveryUri` so the issuer validates).
- OAuth 2 authorization code is the same client with authorize, token, and user info URLs from the row, else from the named client (GitHub, Google, Facebook, LinkedIn, Bitbucket, Dropbox, PayPal, WordPress, Yahoo, Foursquare), which are the endpoints of the Java pac4j clients. OAuth 1.0 (`OctOauth10`, Twitter) does not carry forward and reports so.
- PKCE stays on, with state and, for OIDC, nonce. The client secret is sent by `client_secret_basic`. Apple is the one provider quirk: the client secret is an ES256 JWT signed with the private key in `secret`, built on each grant.
- The ID token is validated against the provider's JWKS. User info is read with the access token and merged. The username is `preferred_username`, else `email` (for OAuth, the named client's field first).

On the callback, as `MoquiSecurityGrantedAccessAdapter`: the `AuthFlowFieldMap` rules map profile attributes onto the account (an expression, or a mapping service whose `destinationMap` and `sourceMap` are merged); the account with that username is updated through `update#UserAccount` or created, with `externalUserId` the subject; IdP roles (`roles`, `groups`, or Keycloak's realm roles) map through `AuthFlowRoleMap` to `UserGroupMember`, a current group no mapped role names is ended unless it is the flow's `defaultUserGroupId`, and the default group is added when the user has none. Then the user logs in and the session id is replaced, as after a password login. `returnTo` stays same-origin. A failure is logged and the browser goes back to `/Login`, as in Java.

Logout after an SSO login (the Login screen's `logout`, or `/sso/logout`) ends the local session and sends the browser to the provider's `end_session_endpoint` with the ID token hint and a same-origin return. Artifact authorization is unchanged. Roles become user groups.

## Later: SAML

SAML 2 is a service provider: a keystore, IdP metadata, force-auth, passive, and single logout. The crate, when it is reviewed, is `saml-rs` with `default-features = false` and `crypto-aws-lc`. Its defaults turn on RustCrypto, legacy algorithms, PKCS#11, and post-quantum, so it stays unlinked. `bergshamra` is the XML-DSig crate behind it. `samael` stays out. It needs libxml2, xmlsec, and the `openssl` crate. A `SamlFlow` row reports that SAML is not available.

## Later: tokens without a browser login

A client-credentials token can authenticate an outbound system-message remote. An inbound bearer JWT can be checked against a JWKS. The Java `inbound` flag and `systemMessageRemoteId` are stored and unused, and Moqui does not become an authorization server. Either use is a separate pass on this same client.

## Out of scope

- Linking `saml-rs`, `openidconnect`, `oauth2`, `jsonwebtoken`, or `huskarl-reqwest`.
- OAuth 1.0, and a crate per social provider.
- Moqui as an authorization server.
- LDAP, Kerberos, and passkeys. `webauthn-rs` is MPL-2.0 and stays out.
- Using `huskarl`'s clock as the Moqui clock.
- Admin screens for `AuthFlow`; Java has none. The rows are data, edited with the entity tools.
