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

//! Second factor: when a user needs one, TOTP, and the random codes of the single-use factor.
//!
//! The authenticator app codes are the samstevens defaults the Java services use: HMAC-SHA1, six
//! digits, a 30 second period, and one period of drift either way. The secret is 32 base32
//! characters, 160 bits.

use moqui_db::sql::Condition;
use moqui_lib::object::Value;
use totp_rs::{Algorithm, Secret, Totp};

use super::login::{entity_facade, now_value};
use super::password::simple_hash;
use super::preference::user_group_ids;
use crate::context::{Error, ExecutionContext};

const AUTHC_FACTOR: &str = "moqui.security.UserAuthcFactor";
const USER_GROUP: &str = "moqui.security.UserGroup";
/// The salt Java hashes single-use codes with. The hash type is the conf password default.
const SINGLE_USE_SALT: &str = "SaltySalt";
const TOTP_DIGITS: usize = 6;
const TOTP_STEP: u64 = 30;
const TOTP_SKEW: u16 = 1;

/// `get#UserAuthcFactorRequired`: the user has a factor in date, or is in a group with
/// `requireAuthcFactor` set. Login asks this before it sets the user.
///
/// # Errors
///
/// Returns an error when the factor or group rows cannot be read.
pub fn second_factor_required(user_id: &str) -> Result<bool, Error> {
    let _authz = ExecutionContext::with_current(|ctx| ctx.disable_authz());
    let facade = entity_facade()?;
    let now = now_value();
    let factors = facade
        .find(AUTHC_FACTOR)?
        .eq("userId", Value::string(user_id))
        .condition(date_filter(&now))
        .count()?;
    if factors > 0 {
        return Ok(true);
    }
    let groups: Vec<Value> = user_group_ids(Some(user_id))?.into_iter().map(Value::String).collect();
    let required = facade
        .find(USER_GROUP)?
        .condition(Condition::is_in("userGroupId", groups))
        .eq("requireAuthcFactor", Value::string("Y"))
        .count()?;
    Ok(required > 0)
}

/// The `date-filter` of the factor finds: from is null or not after now, thru is null or after now.
fn date_filter(now: &Value) -> Condition {
    Condition::and(vec![
        Condition::or(vec![
            Condition::is_null("fromDate"),
            Condition::less_equal("fromDate", now.clone()),
        ]),
        Condition::or(vec![
            Condition::is_null("thruDate"),
            Condition::greater("thruDate", now.clone()),
        ]),
    ])
}

/// A new TOTP shared secret, as `DefaultSecretGenerator` makes it: base32 of 20 random bytes.
pub fn totp_secret() -> String {
    Secret::generate().to_base32()
}

/// True when `code` is the authenticator code for `secret` now, or one period either side.
///
/// A secret that is not base32, or is too short to be a TOTP key, never matches.
pub fn totp_code_valid(secret: &str, code: &str) -> bool {
    let Some(totp) = totp(secret, "", None) else {
        return false;
    };
    totp.check_current(code.trim()).is_some()
}

/// The `otpauth://totp/` URI an authenticator app scans: the issuer and label, the secret, SHA1,
/// six digits, and 30 seconds, as `QrData` holds them. `None` when the secret or label is not valid.
pub fn totp_uri(label: &str, issuer: &str, secret: &str) -> Option<String> {
    let issuer = Some(issuer).filter(|text| !text.is_empty());
    totp(secret, label, issuer)?.to_url().ok()
}

fn totp(secret: &str, label: &str, issuer: Option<&str>) -> Option<Totp> {
    let bytes = Secret::try_from_base32(secret.trim().to_ascii_uppercase()).ok()?;
    totp_rs::Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(u8::try_from(TOTP_DIGITS).ok()?)
        .with_step_duration(TOTP_STEP)
        .with_skew(TOTP_SKEW)
        .with_secret(bytes)
        .with_account_name(label)
        .with_issuer(issuer)
        .build()
        .ok()
}

/// The stored form of a single-use code: `getSimpleHash(code, 'SaltySalt')`, the conf password
/// hash type in hex. It is a lookup key, so it is never argon2.
///
/// # Errors
///
/// Returns an error when the conf names a hash type this port does not have.
pub fn single_use_hash(code: &str) -> Result<String, Error> {
    simple_hash(code, SINGLE_USE_SALT, "", false)
}

/// `new SecureRandom().nextInt(bound).toString().padLeft(width, '0')`, with `bound` the largest
/// number of `width` digits as in the Java services: 6 digits for a sent code, 8 for a printed one.
///
/// # Errors
///
/// Returns an error when the system random source fails.
pub fn random_code(width: usize) -> Result<String, Error> {
    let width = width.clamp(1, 9);
    let bound = 10_u32.pow(u32::try_from(width).unwrap_or(9)) - 1;
    let number = random_below(bound)?;
    Ok(format!("{number:0width$}"))
}

/// Uniform in `0..bound`, rejecting the top of the `u32` range that would skew the remainder.
fn random_below(bound: u32) -> Result<u32, Error> {
    let zone = u32::MAX - (u32::MAX % bound);
    loop {
        let mut bytes = [0_u8; 4];
        getrandom::fill(&mut bytes).map_err(|err| Error::new(format!("random source failed: {err}")))?;
        let value = u32::from_le_bytes(bytes);
        if value < zone {
            return Ok(value % bound);
        }
    }
}

/// `StringUtilities.getRandomString(length)`: URL-safe base64 of `length` random bytes, cut to
/// `length` characters.
///
/// # Errors
///
/// Returns an error when the system random source fails.
pub fn random_string(length: usize) -> Result<String, Error> {
    use base64::Engine;
    let mut bytes = vec![0_u8; length];
    getrandom::fill(&mut bytes).map_err(|err| Error::new(format!("random source failed: {err}")))?;
    let mut text = base64::engine::general_purpose::URL_SAFE.encode(bytes);
    text.truncate(length);
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 appendix B, SHA1, as six digits: the key is the ASCII `12345678901234567890`.
    #[test]
    fn totp_matches_the_rfc_vector_and_rejects_other_codes() {
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let totp = totp(secret, "", None).unwrap();
        assert_eq!(totp.generate(59).to_string(), "287082");
        assert_eq!(totp.generate(1_111_111_109).to_string(), "081804");
        assert!(totp.check("287082", 59).is_some());
        assert!(
            totp.check("287082", 59 + 30).is_some(),
            "one period of drift is allowed"
        );
        assert!(totp.check("287082", 59 + 90).is_none());
        let now = totp.generate_current().to_string();
        assert!(totp_code_valid(&secret.to_ascii_lowercase(), &now));
        assert!(!totp_code_valid(secret, "12345"));
        assert!(!totp_code_valid("not base32!", &now));
    }

    #[test]
    fn a_new_secret_is_32_base32_characters_and_the_uri_names_issuer_and_label() {
        let secret = totp_secret();
        assert_eq!(secret.len(), 32, "{secret}");
        assert!(secret
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ('2'..='7').contains(&ch)));
        let uri = totp_uri("ada", "example.com", &secret).unwrap();
        assert!(uri.starts_with("otpauth://totp/example.com:ada?"), "{uri}");
        assert!(uri.contains(&format!("secret={secret}")), "{uri}");
        assert!(uri.contains("issuer=example.com"), "{uri}");
    }

    #[test]
    fn random_codes_have_the_width_and_strings_are_url_safe() {
        for _ in 0..50 {
            let code = random_code(6).unwrap();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|ch| ch.is_ascii_digit()));
        }
        assert_eq!(random_code(8).unwrap().len(), 8);
        let text = random_string(12).unwrap();
        assert_eq!(text.len(), 12);
        assert!(text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
    }
}
