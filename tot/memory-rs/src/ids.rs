//! Short identifier generators for memories and goals.
//!
//! Ports `generate_memory_id` / `generate_goal_id` from `helpers.py`:
//!
//! ```python
//! def generate_memory_id() -> str:
//!     raw = uuid.uuid4().bytes
//!     b32 = base64.b32encode(raw).decode("ascii").rstrip("=").lower()
//!     return f"m_{b32[:11]}"
//! ```
//!
//! 16 random bytes → RFC4648 base32 (no padding) → 26 chars (lowercase).
//! The Python takes only the first 11 chars (55 bits of entropy from 16-byte
//! UUID v4).  The alphabet after lowercasing is `[a-z2-7]`, fully covered by
//! the test regex `^[mg]_[0-9a-z]{11}$`.

use uuid::Uuid;

/// RFC4648 base32 alphabet (uppercase `A–Z` then `2–7`).
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Encode `bytes` as RFC4648 base32 without padding, returned as a lowercase
/// `String`.
///
/// Parity with Python:
/// ```python
/// base64.b32encode(raw).decode("ascii").rstrip("=").lower()
/// ```
fn b32_encode_lower(bytes: &[u8]) -> String {
    // Collect bits 5 at a time from the input.
    let n_bits = bytes.len() * 8;
    let n_chars = (n_bits + 4) / 5; // ceil(n_bits / 5)
    let mut out = Vec::with_capacity(n_chars);

    let mut bit_buf: u32 = 0;
    let mut bits_in_buf: u32 = 0;

    for &byte in bytes {
        bit_buf = (bit_buf << 8) | (byte as u32);
        bits_in_buf += 8;
        while bits_in_buf >= 5 {
            bits_in_buf -= 5;
            let idx = ((bit_buf >> bits_in_buf) & 0x1F) as usize;
            out.push(BASE32_ALPHABET[idx].to_ascii_lowercase());
        }
    }
    // Flush remaining bits (padded with zero on the right, matching RFC4648).
    if bits_in_buf > 0 {
        let idx = ((bit_buf << (5 - bits_in_buf)) & 0x1F) as usize;
        out.push(BASE32_ALPHABET[idx].to_ascii_lowercase());
    }

    // SAFETY: all bytes are ASCII lowercase letters or digits ('a'-'z', '2'-'7').
    unsafe { String::from_utf8_unchecked(out) }
}

/// Generate a unique memory identifier.
///
/// Format: `m_` + 11 base32-lowercase chars.
/// Pattern: `^m_[a-z2-7]{11}$`.
pub fn generate_memory_id() -> String {
    let raw = Uuid::new_v4().into_bytes();
    let b32 = b32_encode_lower(&raw);
    format!("m_{}", &b32[..11])
}

/// Generate a unique goal identifier.
///
/// Format: `g_` + 11 base32-lowercase chars.
/// Pattern: `^g_[a-z2-7]{11}$`.
pub fn generate_goal_id() -> String {
    let raw = Uuid::new_v4().into_bytes();
    let b32 = b32_encode_lower(&raw);
    format!("g_{}", &b32[..11])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn is_b32_lower_char(c: char) -> bool {
        c.is_ascii_lowercase() || ('2'..='7').contains(&c)
    }

    /// Memory IDs must match `^m_[a-z2-7]{11}$` — the RFC4648 base32 alphabet
    /// lowercased.  Run 100 iterations to exercise randomness.
    #[test]
    fn memory_id_format() {
        for _ in 0..100 {
            let id = generate_memory_id();
            assert_eq!(
                id.len(),
                13,
                "memory id must be exactly 13 chars ('m_' + 11): got {id:?}"
            );
            assert!(
                id.starts_with("m_"),
                "memory id must start with 'm_': {id:?}"
            );
            let suffix = &id[2..];
            assert!(
                suffix.chars().all(is_b32_lower_char),
                "memory id suffix must be base32-lowercase [a-z2-7]: {id:?}"
            );
        }
    }

    /// Goal IDs must match `^g_[a-z2-7]{11}$`.
    #[test]
    fn goal_id_format() {
        for _ in 0..100 {
            let id = generate_goal_id();
            assert_eq!(
                id.len(),
                13,
                "goal id must be exactly 13 chars ('g_' + 11): got {id:?}"
            );
            assert!(
                id.starts_with("g_"),
                "goal id must start with 'g_': {id:?}"
            );
            let suffix = &id[2..];
            assert!(
                suffix.chars().all(is_b32_lower_char),
                "goal id suffix must be base32-lowercase [a-z2-7]: {id:?}"
            );
        }
    }

    /// Two calls to `generate_memory_id` must produce different values.
    #[test]
    fn memory_ids_are_unique() {
        let a = generate_memory_id();
        let b = generate_memory_id();
        assert_ne!(a, b, "generate_memory_id must return unique values on each call");
    }

    /// Two calls to `generate_goal_id` must produce different values.
    #[test]
    fn goal_ids_are_unique() {
        let a = generate_goal_id();
        let b = generate_goal_id();
        assert_ne!(a, b, "generate_goal_id must return unique values on each call");
    }

    /// Memory IDs and goal IDs are distinguishable by their prefix.
    #[test]
    fn prefixes_are_distinct() {
        let m = generate_memory_id();
        let g = generate_goal_id();
        assert!(m.starts_with("m_"), "memory id must start with 'm_': {m}");
        assert!(g.starts_with("g_"), "goal id must start with 'g_': {g}");
    }

    /// Sanity-check the b32_encode_lower helper against a known Python output.
    ///
    /// Python: base64.b32encode(b'\x00' * 16).rstrip(b'=').lower() == b'aaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    /// (26 'a' chars, all zeroes → all 5-bit groups = 0 → 'A' → lowercase 'a')
    #[test]
    fn b32_encode_lower_all_zeroes() {
        let result = b32_encode_lower(&[0u8; 16]);
        assert_eq!(result.len(), 26, "16 bytes → 26 base32 chars");
        assert!(
            result.chars().all(|c| c == 'a'),
            "all-zero bytes encode to all 'a': got {result:?}"
        );
    }

    /// Python: base64.b32encode(b'\xff' * 16).rstrip(b'=').lower()
    /// 16 × 0xFF = 128 bits all set; 5-bit groups all = 0x1F → '7' (last alphabet char)
    /// First 25 groups = 0x1F = '7'; last group (3 bits leftover × 2 zero pads = 0x1F... wait:
    /// 128 bits / 5 = 25.6, so 25 full groups + 1 partial (3 bits 0b111 padded to 5 bits left
    /// = 0b11100 = 28 → '7' + 1 extra... let's compute properly)
    /// Actually: 128 bits, groups of 5: floor(128/5)=25 full groups (125 bits), then 3 bits remain.
    /// The last 3 bits of 0xFF...FF are 0b111; left-padded to 5 bits = 0b11100 = 28;
    /// BASE32_ALPHABET[28] = '6' (A=0..Z=25, 2=26, 3=27, 4=28); lowercase = '6'.
    /// So the 26-char result = "7777777777777777777777777" + "6"? let's check with alphabet:
    /// index 0x1F=31: alphabet[31] = '7' (A=0,B=1,...Z=25,2=26,3=27,4=28,5=29,6=30,7=31)
    /// Yes 7=index 31. Last group: 3 bits = 0b111, shifted left by (5-3)=2 = 0b11100 = 28,
    /// alphabet[28] = '4'. Lowercase = '4'. But '4' is not in base32 alphabet... wait:
    /// A-Z = indices 0-25, then 2,3,4,5,6,7 = indices 26-31.
    /// index 28 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'[28] = '4'. Lowercase = '4'.
    /// Wait, '4' IS a digit but NOT in base32 [a-z2-7]. That's fine — '4' is a digit.
    /// is_b32_lower_char includes lowercase ascii letters (a-z) and '2'-'7'.
    /// But wait: RFC4648 base32 alphabet = A-Z plus 2-7 (not 0-9). So '4' is in the alphabet.
    #[test]
    fn b32_encode_lower_all_ones() {
        let result = b32_encode_lower(&[0xFFu8; 16]);
        assert_eq!(result.len(), 26, "16 bytes → 26 base32 chars");
        // All chars must be valid RFC4648 base32 alphabet (lowercase: a-z, 2-7)
        let valid: fn(char) -> bool = |c| c.is_ascii_lowercase() || ('2'..='7').contains(&c);
        for (i, c) in result.chars().enumerate() {
            assert!(
                valid(c),
                "char at position {i} is not in base32 alphabet: {c:?} in {result:?}"
            );
        }
    }
}
