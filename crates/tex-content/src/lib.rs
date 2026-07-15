use std::fmt;

const DOMAIN_PREFIX: &[u8] = b"umber-content\0";
const DOMAIN_VERSION: u8 = 2;
const DOMAIN_VERSION_V1: u8 = 1;

/// The byte namespace whose content is being identified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentDomain {
    Input = 1,
    Artifact = 2,
    FontMetric = 3,
    /// Exact bytes stored as one immutable virtual file.
    VirtualFile = 4,
    /// A canonical virtual path bound to immutable file content.
    VirtualPathBinding = 5,
    /// The deterministic contents and provenance of layered VFS storage.
    VirtualFileStorage = 6,
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
        hash_v2_parts(&[DOMAIN_PREFIX, &[DOMAIN_VERSION], &[domain as u8], bytes])
    }

    /// Reproduces the undomained identity written before content identity v1.
    #[must_use]
    pub fn legacy(bytes: &[u8]) -> Self {
        hash_v1_parts(&[bytes])
    }

    /// Accepts the current identity or either explicitly supported historical scheme.
    #[must_use]
    pub fn matches_current_or_legacy(self, domain: ContentDomain, bytes: &[u8]) -> bool {
        self == Self::for_domain(domain, bytes)
            || self == hash_v1_parts(&[DOMAIN_PREFIX, &[DOMAIN_VERSION_V1], &[domain as u8], bytes])
            || self == Self::legacy(bytes)
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

/// Portable block-wise identity used by domain-separated version 2.
///
/// Four position-keyed lanes consume successive little-endian words. Final
/// cross-lane diffusion makes every output word depend on the complete 256-bit
/// accumulator and total preimage length. This retains the existing fixed-size
/// non-cryptographic identity contract while avoiding four lane updates for
/// every individual byte.
fn hash_v2_parts(parts: &[&[u8]]) -> ContentIdentity {
    let mut hasher = V2Hasher::new();
    for part in parts {
        hasher.write(part);
    }
    hasher.finish()
}

struct V2Hasher {
    words: [u64; 4],
    tail: [u8; 8],
    tail_len: usize,
    len: u64,
    word_index: u64,
}

impl V2Hasher {
    const OFFSETS: [u64; 4] = [
        0x243f_6a88_85a3_08d3,
        0x1319_8a2e_0370_7344,
        0xa409_3822_299f_31d0,
        0x082e_fa98_ec4e_6c89,
    ];
    const PRIMES: [u64; 4] = [
        0x9e37_79b1_85eb_ca87,
        0xc2b2_ae3d_27d4_eb4f,
        0x1656_67b1_9e37_79f9,
        0x85eb_ca77_c2b2_ae63,
    ];
    const FINAL_KEYS: [u64; 4] = [
        0xd6e8_feb8_6659_fd93,
        0xa5a3_564e_27f8_864f,
        0x9e37_79b9_7f4a_7c15,
        0x94d0_49bb_1331_11eb,
    ];
    const POSITION_KEY: u64 = 0x517c_c1b7_2722_0a95;

    fn new() -> Self {
        Self {
            words: Self::OFFSETS,
            tail: [0; 8],
            tail_len: 0,
            len: 0,
            word_index: 0,
        }
    }

    fn write(&mut self, mut bytes: &[u8]) {
        self.len = self
            .len
            .checked_add(bytes.len() as u64)
            .expect("content identity input length overflowed");

        if self.tail_len != 0 {
            let copied = (8 - self.tail_len).min(bytes.len());
            self.tail[self.tail_len..self.tail_len + copied].copy_from_slice(&bytes[..copied]);
            self.tail_len += copied;
            bytes = &bytes[copied..];
            if self.tail_len != 8 {
                return;
            }
            self.mix_word(u64::from_le_bytes(self.tail));
            self.tail_len = 0;
        }

        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.mix_word(u64::from_le_bytes(
                chunk.try_into().expect("exact content identity word"),
            ));
        }
        let remainder = chunks.remainder();
        self.tail[..remainder.len()].copy_from_slice(remainder);
        self.tail_len = remainder.len();
    }

    fn mix_word(&mut self, word: u64) {
        let lane = self.word_index as usize & 3;
        let positioned = word
            .wrapping_add(self.word_index.wrapping_mul(Self::POSITION_KEY))
            .wrapping_add(Self::FINAL_KEYS[lane]);
        let mixed = self.words[lane] ^ positioned;
        self.words[lane] = mixed
            .rotate_left(23 + lane as u32 * 7)
            .wrapping_mul(Self::PRIMES[lane]);
        self.words[lane] ^= self.words[lane] >> (29 - lane as u32 * 3);
        self.word_index += 1;
    }

    fn finish(mut self) -> ContentIdentity {
        if self.tail_len != 0 {
            let mut tail = [0_u8; 8];
            tail[..self.tail_len].copy_from_slice(&self.tail[..self.tail_len]);
            self.mix_word(u64::from_le_bytes(tail));
        }

        let words = self.words;
        let mut out = [0; 32];
        for lane in 0..4 {
            let value = words[lane]
                ^ words[(lane + 1) & 3].rotate_left(17 + lane as u32 * 5)
                ^ words[(lane + 2) & 3].rotate_right(11 + lane as u32 * 3)
                ^ self.len.wrapping_mul(Self::FINAL_KEYS[lane])
                ^ self.word_index.rotate_left(lane as u32 * 13);
            out[lane * 8..lane * 8 + 8].copy_from_slice(&splitmix64(value).to_le_bytes());
        }
        ContentIdentity(out)
    }
}

fn hash_v1_parts(parts: &[&[u8]]) -> ContentIdentity {
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
    use super::{
        ContentDomain, ContentIdentity, DOMAIN_PREFIX, DOMAIN_VERSION_V1, hash_v1_parts,
        hash_v2_parts,
    };

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
        assert_ne!(
            ContentIdentity::for_domain(ContentDomain::VirtualFile, bytes),
            ContentIdentity::for_domain(ContentDomain::VirtualPathBinding, bytes)
        );
        assert_ne!(
            ContentIdentity::for_domain(ContentDomain::VirtualPathBinding, bytes),
            ContentIdentity::for_domain(ContentDomain::VirtualFileStorage, bytes)
        );
    }

    #[test]
    fn version_2_domain_identity_is_stable() {
        assert_eq!(
            ContentIdentity::for_domain(ContentDomain::Input, b"abc").hex(),
            "2d11f65de7d40180397441b17afec1c76e1205c3e56446ee91cca5716eccab10"
        );
    }

    #[test]
    fn legacy_policy_reproduces_pre_v1_identity() {
        assert_eq!(
            ContentIdentity::legacy(b"abc").hex(),
            "8071320093de53eae81f371ba4a7e011805d89c64fa3d0b06014f14a6c370ce1"
        );
    }

    #[test]
    fn version_1_domain_identity_remains_accepted() {
        let v1 = hash_v1_parts(&[
            DOMAIN_PREFIX,
            &[DOMAIN_VERSION_V1],
            &[ContentDomain::Input as u8],
            b"abc",
        ]);
        assert_eq!(
            v1.hex(),
            "e84c6002e661e17c7599c0d748998ae20257b997aa5b96b922c1c35401c8bf85"
        );
        assert!(v1.matches_current_or_legacy(ContentDomain::Input, b"abc"));
    }

    #[test]
    fn version_2_streaming_is_independent_of_part_boundaries() {
        let bytes = (0_u8..=255).collect::<Vec<_>>();
        let one_part = hash_v2_parts(&[&bytes]);
        for split in 0..=bytes.len() {
            assert_eq!(one_part, hash_v2_parts(&[&bytes[..split], &bytes[split..]]));
        }
    }
}
