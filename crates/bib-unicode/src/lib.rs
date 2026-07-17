//! Versioned immutable Unicode compatibility resources.
//!
//! The data and algorithms in this crate are deterministic and usable on both
//! native and wasm targets. They never consult a host locale or filesystem.

mod annotations;
mod collation;
mod date;
mod encoding;
mod langtag;
mod recode;
mod transliteration;
mod utils;

pub use annotations::{Annotation, AnnotationKind, AnnotationMap};
pub use collation::{CollationData, CollationKey};
pub use date::{DateError, DatePart, DateTime, ExtendedDate, Uncertainty, YearDivision};
pub use encoding::{EncodingError, LegacyEncoding, decode_legacy, encode_legacy};
pub use langtag::{LanguageTag, LanguageTagError};
pub use recode::{RecodeSet, TexRecoder};
pub use transliteration::{Transliteration, transliterate};
pub use utils::{
    RangeEnd, compatibility_hash, normalise_nfc, normalise_string, normalise_string_hash,
    normalise_string_underscore, parse_range, range_len, reduce_array, remove_outer, split_xsv,
    strip_noinit,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CompatibilityVersion {
    pub upstream_commit: &'static str,
    pub program_version: &'static str,
    pub control_schema: &'static str,
    pub bbl_schema: &'static str,
}

impl CompatibilityVersion {
    pub const BIBER_2_22_BETA: Self = Self {
        upstream_commit: "74252e608e5f8115375c532eb25416430a9f52eb",
        program_version: "2.22 beta",
        control_schema: "3.11",
        bbl_schema: "3.3",
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnicodeData {
    compatibility: CompatibilityVersion,
}

impl UnicodeData {
    #[must_use]
    pub const fn pinned() -> Self {
        Self {
            compatibility: CompatibilityVersion::BIBER_2_22_BETA,
        }
    }

    #[must_use]
    pub const fn compatibility(self) -> CompatibilityVersion {
        self.compatibility
    }
}

impl Default for UnicodeData {
    fn default() -> Self {
        Self::pinned()
    }
}

#[cfg(test)]
mod tests;
