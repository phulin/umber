use std::fmt;

/// Stable 32-byte content identity used by page artifacts and font resources.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Hashes bytes with the same deterministic content-addressing family used
    /// by `World` inputs and artifact storage.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
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

        let mut words = OFFSETS;
        for (index, &byte) in bytes.iter().enumerate() {
            for lane in 0..4 {
                words[lane] ^=
                    u64::from(byte).wrapping_add(((index as u64) << (lane * 7)) | lane as u64);
                words[lane] = words[lane].wrapping_mul(PRIMES[lane]);
                words[lane] ^= words[lane].rotate_right(17 + lane as u32);
            }
        }
        for word in &mut words {
            *word ^= bytes.len() as u64;
            *word = splitmix64(*word);
        }

        let mut out = [0; 32];
        for (chunk, word) in out.chunks_exact_mut(8).zip(words) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        Self(out)
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

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
