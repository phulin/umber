//! Pinned executable-process capacity profiles.
//!
//! Format images retain the string-pool coordinate pair selected by their
//! producer. That pair identifies the complete process profile used to
//! validate every other format-retained capacity coordinate. Runtime-only
//! stack bounds are carried by the same typed configuration so execution and
//! terminal reporting cannot silently select a different profile.

/// Capacity bounds present only in the pinned pdfTeX process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfEngineCapacities {
    pub pdf_memory_words: usize,
    pub object_table_entries: usize,
    pub destination_name_entries: usize,
}

/// Complete process-capacity configuration selected by one engine binary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineCapacityConfiguration {
    pub main_memory_words: usize,
    pub hash_size: usize,
    pub hash_extra: usize,
    pub max_strings: usize,
    pub pool_size: usize,
    pub font_info_words: usize,
    pub fonts: usize,
    pub trie_nodes: usize,
    pub input_stack: usize,
    pub nest_stack: usize,
    pub parameter_stack: usize,
    pub buffer_stack: usize,
    pub save_stack: usize,
    pub max_input_files: usize,
    pub pdf: Option<PdfEngineCapacities>,
}

impl EngineCapacityConfiguration {
    #[must_use]
    pub const fn hash_entries(self) -> usize {
        self.hash_size + self.hash_extra
    }

    /// Number of compact Symbol rows needed by the fixed TeX control
    /// sequence namespaces which do not occupy `hash`.
    ///
    /// The null control sequence, all 8-bit single-character control
    /// sequences, active-character cells, and one inaccessible engine cell
    /// have stable slots outside the multiletter hash. Font identifiers are
    /// also retained as non-hash symbols by the runtime. Keeping this
    /// derivation beside the executable profile makes the allowance visible
    /// and prevents a workload-specific interner constant from becoming an
    /// accidental capacity policy.
    #[must_use]
    pub const fn reserved_non_hash_symbols(self) -> usize {
        1 + 256 + 256 + 1 + self.fonts
    }

    /// Total control-sequence Symbol capacity for this executable profile.
    #[must_use]
    pub const fn interner_control_sequence_capacity(self) -> usize {
        self.hash_entries() + self.reserved_non_hash_symbols()
    }

    /// Total Symbol-row capacity, including retained non-control spellings.
    ///
    /// `max_strings` is the process-owned upper bound for those additional
    /// pool spellings. It is deliberately accounted separately from hash
    /// occupancy, since retained spellings do not create a TeX `hash` row.
    #[must_use]
    pub const fn interner_slot_capacity(self) -> usize {
        self.interner_control_sequence_capacity() + self.max_strings
    }

    /// UTF-8 byte budget for the canonical interner byte pool.
    #[must_use]
    pub const fn interner_byte_capacity(self) -> usize {
        self.pool_size
    }
}

/// Pinned executable profile that produced or executes a format image.
///
/// The compact profile reproduces the canonical TeX/e-TeX conformance
/// executables. The TeX Live profile reproduces the repository's pinned 2026
/// `texmf.cnf` limits for both the e-TeX and pdfTeX binaries; its supplementary
/// PDF bounds are simply unused by an engine without the PDF command family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineCapacityProfile {
    Tex82Etex,
    Texlive2026,
}

impl EngineCapacityProfile {
    #[must_use]
    pub const fn configuration(self) -> EngineCapacityConfiguration {
        match self {
            Self::Tex82Etex => EngineCapacityConfiguration {
                main_memory_words: 250_000,
                hash_size: 15_000,
                hash_extra: 0,
                max_strings: 15_000,
                pool_size: 125_000,
                font_info_words: 20_000,
                fonts: 75,
                trie_nodes: 8_000,
                input_stack: 200,
                nest_stack: 40,
                parameter_stack: 60,
                buffer_stack: 500,
                save_stack: 600,
                max_input_files: 6,
                pdf: None,
            },
            Self::Texlive2026 => EngineCapacityConfiguration {
                main_memory_words: 5_000_000,
                hash_size: 15_000,
                hash_extra: 600_000,
                max_strings: 500_000,
                pool_size: 6_250_000,
                font_info_words: 8_000_000,
                fonts: 9_000,
                trie_nodes: 1_100_000,
                input_stack: 10_000,
                nest_stack: 1_000,
                parameter_stack: 20_000,
                buffer_stack: 200_000,
                save_stack: 200_000,
                max_input_files: 15,
                pdf: Some(PdfEngineCapacities {
                    pdf_memory_words: 10_000_000,
                    object_table_entries: 8_388_607,
                    destination_name_entries: 500_000,
                }),
            },
        }
    }

    pub(crate) fn from_string_pool_coordinates(
        max_strings: usize,
        pool_size: usize,
    ) -> Option<Self> {
        [Self::Tex82Etex, Self::Texlive2026]
            .into_iter()
            .find(|profile| {
                let capacities = profile.configuration();
                (capacities.max_strings, capacities.pool_size) == (max_strings, pool_size)
            })
    }

    /// Returns the interner limits derived from this executable profile.
    ///
    /// The profile is the sole production owner of Symbol-row and canonical
    /// UTF-8 capacity. The fallible conversion remains internal so a future
    /// profile cannot silently widen the packed token representation.
    #[must_use]
    pub fn interner_budget(self) -> crate::interner::InternerBudget {
        crate::interner::InternerBudget::from_profile(self)
            .expect("pinned engine profile fits the compact interner domain")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_profiles_retain_canonical_and_pdftex_process_coordinates() {
        let tex = EngineCapacityProfile::Tex82Etex.configuration();
        assert_eq!(
            (
                tex.main_memory_words,
                tex.hash_size,
                tex.hash_extra,
                tex.max_strings,
                tex.pool_size,
                tex.font_info_words,
                tex.fonts,
                tex.trie_nodes,
            ),
            (250_000, 15_000, 0, 15_000, 125_000, 20_000, 75, 8_000)
        );
        assert_eq!(
            (
                tex.input_stack,
                tex.nest_stack,
                tex.parameter_stack,
                tex.buffer_stack,
                tex.save_stack,
                tex.max_input_files,
            ),
            (200, 40, 60, 500, 600, 6)
        );
        assert_eq!(tex.pdf, None);

        let texlive = EngineCapacityProfile::Texlive2026.configuration();
        assert_eq!(
            (
                texlive.main_memory_words,
                texlive.hash_entries(),
                texlive.max_strings,
                texlive.pool_size,
                texlive.font_info_words,
                texlive.fonts,
                texlive.trie_nodes,
            ),
            (
                5_000_000, 615_000, 500_000, 6_250_000, 8_000_000, 9_000, 1_100_000
            )
        );
        assert_eq!(
            (
                texlive.input_stack,
                texlive.nest_stack,
                texlive.parameter_stack,
                texlive.buffer_stack,
                texlive.save_stack,
                texlive.max_input_files,
            ),
            (10_000, 1_000, 20_000, 200_000, 200_000, 15)
        );
        assert_eq!(
            texlive.pdf,
            Some(PdfEngineCapacities {
                pdf_memory_words: 10_000_000,
                object_table_entries: 8_388_607,
                destination_name_entries: 500_000,
            })
        );
    }
}
