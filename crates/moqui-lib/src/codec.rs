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

//! Small byte codecs: lowercase hex, a constant-time compare, and FNV-1a.

/// The FNV-1a 64-bit offset basis, the starting hash for [`fnv1a_extend`].
pub const FNV1A_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

const FNV1A_PRIME: u64 = 0x0100_0000_01b3;

/// Lowercase hex of `bytes`, two characters per byte.
#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// True when `left` and `right` are equal, comparing every byte rather than stopping at the first
/// difference. The length is not hidden.
#[must_use]
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && left.iter().zip(right).fold(0_u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// The 64-bit FNV-1a hash of `bytes`. Stable across runs and platforms, unlike `DefaultHasher`;
/// not for security.
#[must_use]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    fnv1a_extend(FNV1A_OFFSET, bytes)
}

/// Continue the FNV-1a `hash` over `bytes`, to hash several parts as one.
#[must_use]
pub fn fnv1a_extend(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_lowercase_two_per_byte() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn constant_time_eq_compares_length_and_bytes() {
        assert!(constant_time_eq(b"token", b"token"));
        assert!(!constant_time_eq(b"token", b"tokem"));
        assert!(!constant_time_eq(b"token", b"tokens"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn fnv1a_matches_reference_vectors() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(fnv1a_extend(fnv1a(b"foo"), b"bar"), fnv1a(b"foobar"));
    }
}
