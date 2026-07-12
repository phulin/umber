use std::fmt;

const DOMAIN_PREFIX: &[u8] = b"umber-content\0";
const DOMAIN_VERSION: u8 = 1;

/// The byte namespace whose content is being identified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentDomain {
    Input = 1,
    Artifact = 2,
    FontMetric = 3,
}

/// Stable fixed-size identity for immutable bytes.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentIdentity([u8; 32]);

/// Compatibility name used by the engine's existing public API.
pub type ContentHash = ContentIdentity;

impl ContentIdentity {
    /// Identifies input bytes in the current domain-separated scheme.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::for_domain(ContentDomain::Input, bytes)
    }

    /// Identifies bytes in a versioned content domain.
    #[must_use]
    pub fn for_domain(domain: ContentDomain, bytes: &[u8]) -> Self {
        hash_parts(&[DOMAIN_PREFIX, &[DOMAIN_VERSION], &[domain as u8], bytes])
    }

    /// Reproduces the undomained identity written before content identity v1.
    #[must_use]
    pub fn legacy(bytes: &[u8]) -> Self {
        hash_parts(&[bytes])
    }

    /// Accepts the current domain identity or the explicitly supported legacy identity.
    #[must_use]
    pub fn matches_current_or_legacy(self, domain: ContentDomain, bytes: &[u8]) -> bool {
        self == Self::for_domain(domain, bytes) || self == Self::legacy(bytes)
    }

    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub fn hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            use fmt::Write as _;
            write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
        }
        out
    }
}

fn hash_parts(parts: &[&[u8]]) -> ContentIdentity {
    const OFFSETS: [u64; 4] = [
        0xcbf2_9ce4_8422_2325,
        0x8422_2325_cbf2_9ce4,
        0x9e37_79b9_7f4a_7c15,
        0x94d0_49bb_1331_11eb,
    ];
    const PRIMES: [u64; 4] = [
        0x0000_0100_0000_01b3,
        0x0000_0100_0000_01d3,
        0x0000_0100_0000_01f3,
        0x0000_0100_0000_0213,
    ];

    let len = parts.iter().map(|part| part.len()).sum::<usize>();
    let mut words = OFFSETS;
    let mut index = 0usize;
    for part in parts {
        for &byte in *part {
            for lane in 0..4 {
                words[lane] ^=
                    u64::from(byte).wrapping_add(((index as u64) << (lane * 7)) | lane as u64);
                words[lane] = words[lane].wrapping_mul(PRIMES[lane]);
                words[lane] ^= words[lane].rotate_right(17 + lane as u32);
            }
            index += 1;
        }
    }
    for word in &mut words {
        *word ^= len as u64;
        *word = splitmix64(*word);
    }

    let mut out = [0; 32];
    for (chunk, word) in out.chunks_exact_mut(8).zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    ContentIdentity(out)
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::{ContentDomain, ContentIdentity};

    #[test]
    fn domains_and_versions_separate_equal_bytes() {
        let bytes = b"same bytes";
        assert_ne!(
            ContentIdentity::for_domain(ContentDomain::Input, bytes),
            ContentIdentity::for_domain(ContentDomain::Artifact, bytes)
        );
        assert_ne!(
            ContentIdentity::for_domain(ContentDomain::Artifact, bytes),
            ContentIdentity::legacy(bytes)
        );
    }

    #[test]
    fn legacy_policy_reproduces_pre_v1_identity() {
        assert_eq!(
            ContentIdentity::legacy(b"abc").hex(),
            "8071320093de53eae81f371ba4a7e011805d89c64fa3d0b06014f14a6c370ce1"
        );
    }
}
