# SSO relying party

The crate for an OpenID Connect and OAuth 2 client. The redirect, the callback, and the account sync are not built in this pass. The crate is linked from `moqui-host` so the resolve and the license graph are real.

This follows Sessions and login in `plans/Architecture-Proposal.md`. Password login, the login key, the API key, and artifact authorization stay as they are.

## Decision

`huskarl` 0.11 is the relying party. It is MIT OR Apache-2.0, and it declares Rust 1.92. It lives in `moqui-host`, with the other process-edge dependencies. Default features stay on. The only default feature is `default-jws-verifier-platform`, which on Linux links `huskarl-crypto-native`. `authorization-flow-loopback`, `metrics`, and `experimental-oidc-key-binding` stay off. `huskarl-reqwest`, `huskarl-redis`, and `huskarl-google-cloud` are not dependencies. The host already has blocking reqwest. The callback, when it is built, adapts that client.

The locked crate is `huskarl` 0.11.6, with `huskarl-crypto-native` 0.11.1. JOSE verification uses that RustCrypto stack: `ed25519-dalek` 3, `p256` 0.14, and `p384` 0.14, the same generations `russh` uses. `rsa` resolves to 0.10.0-rc.18, the same copy `russh` already links. TLS stays on `aws-lc-rs`. `huskarl` does not add `ring`. P-256 and P-384 are covered. P-521 (`ES512`, the Java `OjaES512` row) is not. Token expiry stays inside `huskarl`. Moqui time stays `jiff`. `huskarl` does not pull `chrono`.

`openidconnect` stays out. It pins older `rsa`, `p256`, `ed25519-dalek`, and reqwest. `oauth2` and `jsonwebtoken` stay out. They are the fallback if a later review rejects `huskarl`, and they are not a second stack.

Nothing in this pass redirects a browser or reads a token. The dependency is the choice, recorded before the flow exists.

## Later: login and logout

An `AuthFlow` row selects the provider. OpenID Connect is the primary protocol. OAuth 2 authorization code is the same client, with authorize, token, and userinfo URLs stored on the row. The Java generic OAuth rows never set those URLs. The named pac4j clients are that endpoint data. OAuth 1.0 does not carry forward.

PKCE stays on. The first client authentication method is `client_secret_basic`. Apple is the one provider quirk: the client secret is a JWT signed with Apple's private key.

On the callback, profile fields map onto the user, the account is created or updated, and IdP roles sync onto `UserGroupMember`. The same `loginUser` path as a password login runs, including the session-id replacement. `returnTo` stays same-origin. Logout can also end the session at the identity provider. Artifact authorization is unchanged. Roles become user groups.

## Later: SAML

SAML 2 is a service provider: a keystore, IdP metadata, force-auth, passive, and single logout. The crate, when it is reviewed, is `saml-rs` with `default-features = false` and `crypto-aws-lc`. Its defaults turn on RustCrypto, legacy algorithms, PKCS#11, and post-quantum, so it stays unlinked. `bergshamra` is the XML-DSig crate behind it. `samael` stays out. It needs libxml2, xmlsec, and the `openssl` crate.

## Later: tokens without a browser login

A client-credentials token can authenticate an outbound system-message remote. An inbound bearer JWT can be checked against a JWKS. The Java `inbound` flag and `systemMessageRemoteId` are stored and unused, and Moqui does not become an authorization server. Either use is a separate pass on this same client.

## Out of scope

- A redirect, a callback, an account sync, or an `AuthFlow` entity, in this pass.
- Linking `saml-rs`, `openidconnect`, `oauth2`, `jsonwebtoken`, or `huskarl-reqwest`.
- OAuth 1.0, and a crate per social provider.
- Moqui as an authorization server.
- LDAP, Kerberos, and passkeys. `webauthn-rs` is MPL-2.0 and stays out.
- Using `huskarl`'s clock as the Moqui clock.
