use umber_hash::{AHash64, HashDomain};

pub(crate) fn digest(input: &[u8]) -> [u8; 8] {
    AHash64::for_bytes(HashDomain::DistributionContent, input)
        .value()
        .to_be_bytes()
}
