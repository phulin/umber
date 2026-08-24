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
}

/// Pinned executable profile that produced or executes a format image.
///
/// TeX82 and e-TeX use the compact canonical-conformance process limits. The
/// pdfTeX profile uses the repository's pinned TeX Live 2026 configuration
/// and pdfTeX 1.40.29's fixed supplementary bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineCapacityProfile {
    Tex82Etex,
    Pdftex14029,
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
            Self::Pdftex14029 => EngineCapacityConfiguration {
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
        [Self::Tex82Etex, Self::Pdftex14029]
            .into_iter()
            .find(|profile| {
                let capacities = profile.configuration();
                (capacities.max_strings, capacities.pool_size) == (max_strings, pool_size)
            })
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

        let pdftex = EngineCapacityProfile::Pdftex14029.configuration();
        assert_eq!(
            (
                pdftex.main_memory_words,
                pdftex.hash_entries(),
                pdftex.max_strings,
                pdftex.pool_size,
                pdftex.font_info_words,
                pdftex.fonts,
                pdftex.trie_nodes,
            ),
            (
                5_000_000, 615_000, 500_000, 6_250_000, 8_000_000, 9_000, 1_100_000
            )
        );
        assert_eq!(
            (
                pdftex.input_stack,
                pdftex.nest_stack,
                pdftex.parameter_stack,
                pdftex.buffer_stack,
                pdftex.save_stack,
                pdftex.max_input_files,
            ),
            (10_000, 1_000, 20_000, 200_000, 200_000, 15)
        );
        assert_eq!(
            pdftex.pdf,
            Some(PdfEngineCapacities {
                pdf_memory_words: 10_000_000,
                object_table_entries: 8_388_607,
                destination_name_entries: 500_000,
            })
        );
    }
}
