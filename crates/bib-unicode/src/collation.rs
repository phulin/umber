use unicode_normalization::UnicodeNormalization;

pub const COLLATION_TABLE_ID: &str = "biber-2.22/ducet-14.0.0";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CollationData;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CollationKey(Vec<u32>);

impl CollationKey {
    /// Pinned primary weights, exposed for deterministic locale tailoring by
    /// semantic stages. They are data, not host collation handles.
    #[must_use]
    pub fn weights(&self) -> &[u32] {
        &self.0
    }
}

impl CollationData {
    pub const fn table_id(self) -> &'static str {
        COLLATION_TABLE_ID
    }

    pub fn root_key(self, value: &str) -> CollationKey {
        let weights = value
            .nfkd()
            .flat_map(char::to_lowercase)
            .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
            .map(u32::from)
            .collect();
        CollationKey(weights)
    }
}
