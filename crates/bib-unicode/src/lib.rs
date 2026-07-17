//! Versioned immutable Unicode compatibility resources.
//!
//! Semantic tables arrive in focused implementation issues. This foundation
//! fixes their explicit version and ownership boundary without consulting the
//! host locale or platform services.

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
mod tests {
    use super::*;

    #[test]
    fn pinned_identity_is_complete() {
        let version = UnicodeData::pinned().compatibility();
        assert_eq!(version.program_version, "2.22 beta");
        assert_eq!(version.control_schema, "3.11");
        assert_eq!(version.bbl_schema, "3.3");
    }
}
