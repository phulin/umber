//! Portable, versioned deterministic aHash64 identities.
//!
//! This deliberately does not wrap `ahash::AHasher`: upstream permits its
//! output to differ by version and target CPU features. Persisted identities
//! need one byte-for-byte algorithm on every supported host.

use std::fmt;

const PREFIX: &[u8] = b"umber-ahash64\0";
pub const ALGORITHM_VERSION: u8 = 1;
const SEED: u64 = 0x243f_6a88_85a3_08d3;
const PAD: u64 = 0x1319_8a2e_0370_7344;
const MULTIPLE: u64 = 6364136223846793005;

/// Stable namespace identifiers. Existing discriminants are persisted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum HashDomain {
    DistributionContent = 1,
    DistributionShardKey = 2,
    CacheEnvelope = 3,
    DistributionTree = 4,
    FontMetric = 16,
    FontEncodingMap = 17,
    RealizedFont = 18,
    OpenTypeObject = 19,
    OpenTypeProgram = 20,
    OpenTypeInstance = 21,
    Type1Program = 22,
    PkProgram = 23,
    HtmlResource = 24,
    HtmlRender = 25,
    PdfDocument = 26,
    TrueTypeProgram = 27,
    VirtualFontProgram = 28,
    PdfFontClosure = 29,
}

/// A stable 64-bit identity, displayed as 16 lowercase hexadecimal digits.
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AHash64(u64);

impl AHash64 {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn to_le_bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }

    #[must_use]
    pub fn hex(self) -> String {
        format!("{:016x}", self.0)
    }

    pub fn parse_hex(value: &str) -> Result<Self, ParseAHash64Error> {
        if value.len() != 16
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(ParseAHash64Error);
        }
        u64::from_str_radix(value, 16)
            .map(Self)
            .map_err(|_| ParseAHash64Error)
    }

    #[must_use]
    pub fn for_bytes(domain: HashDomain, bytes: &[u8]) -> Self {
        let mut hasher = AHash64Hasher::new(domain);
        hasher.write(bytes);
        hasher.finish()
    }
}

impl fmt::Debug for AHash64 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("AHash64").field(&self.hex()).finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseAHash64Error;

impl fmt::Display for ParseAHash64Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("aHash64 must contain 16 lowercase hexadecimal digits")
    }
}

impl std::error::Error for ParseAHash64Error {}

/// Incremental portable aHash64 state. Part boundaries do not affect output.
#[derive(Clone, Debug)]
pub struct AHash64Hasher {
    state: u64,
    length: u64,
    tail: [u8; 8],
    tail_len: usize,
}

impl AHash64Hasher {
    #[must_use]
    pub fn new(domain: HashDomain) -> Self {
        let mut hasher = Self {
            state: SEED,
            length: 0,
            tail: [0; 8],
            tail_len: 0,
        };
        hasher.write(PREFIX);
        hasher.write(&[ALGORITHM_VERSION]);
        hasher.write(&(domain as u64).to_le_bytes());
        hasher
    }

    pub fn write(&mut self, bytes: impl AsRef<[u8]>) {
        let mut bytes = bytes.as_ref();
        self.length = self
            .length
            .checked_add(bytes.len() as u64)
            .expect("aHash64 input length overflowed");
        if self.tail_len != 0 {
            let copied = (8 - self.tail_len).min(bytes.len());
            self.tail[self.tail_len..self.tail_len + copied].copy_from_slice(&bytes[..copied]);
            self.tail_len += copied;
            bytes = &bytes[copied..];
            if self.tail_len != 8 {
                return;
            }
            self.mix(u64::from_le_bytes(self.tail));
            self.tail_len = 0;
        }
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.mix(u64::from_le_bytes(
                chunk.try_into().expect("exact aHash64 word"),
            ));
        }
        let remainder = chunks.remainder();
        self.tail[..remainder.len()].copy_from_slice(remainder);
        self.tail_len = remainder.len();
    }

    #[must_use]
    pub fn finish(mut self) -> AHash64 {
        if self.tail_len != 0 {
            let mut tail = [0_u8; 8];
            tail[..self.tail_len].copy_from_slice(&self.tail[..self.tail_len]);
            self.mix(u64::from_le_bytes(tail) ^ (self.tail_len as u64).rotate_left(48));
        }
        let state = folded_multiply(self.state ^ self.length, PAD ^ self.length.rotate_left(17));
        AHash64(state.rotate_left((state & 63) as u32))
    }

    fn mix(&mut self, word: u64) {
        self.state = folded_multiply(self.state ^ word, MULTIPLE).wrapping_add(PAD);
    }
}

fn folded_multiply(left: u64, right: u64) -> u64 {
    let product = u128::from(left) * u128::from(right);
    product as u64 ^ (product >> 64) as u64
}

#[cfg(test)]
mod tests {
    use super::{AHash64, AHash64Hasher, HashDomain};

    #[test]
    fn stable_vectors_and_domains() {
        assert_eq!(
            AHash64::for_bytes(HashDomain::DistributionContent, b"abc").hex(),
            "fc0a6198a9870843"
        );
        assert_ne!(
            AHash64::for_bytes(HashDomain::DistributionContent, b"abc"),
            AHash64::for_bytes(HashDomain::FontMetric, b"abc")
        );
    }

    #[test]
    fn part_boundaries_do_not_change_identity() {
        let bytes = (0_u8..=255).collect::<Vec<_>>();
        let expected = AHash64::for_bytes(HashDomain::DistributionContent, &bytes);
        for split in 0..=bytes.len() {
            let mut hasher = AHash64Hasher::new(HashDomain::DistributionContent);
            hasher.write(&bytes[..split]);
            hasher.write(&bytes[split..]);
            assert_eq!(hasher.finish(), expected);
        }
    }

    #[test]
    fn hex_is_canonical() {
        let value = AHash64::new(0x0123_4567_89ab_cdef);
        assert_eq!(value.hex(), "0123456789abcdef");
        assert_eq!(AHash64::parse_hex(&value.hex()), Ok(value));
        assert!(AHash64::parse_hex("0123456789ABCDEf").is_err());
    }
}
