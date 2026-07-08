use core::fmt;

use tex_state::scaled::TfmConversionError;

use super::types::TfmTable;

/// TFM parse and validation errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    TooShort {
        actual_bytes: usize,
    },
    LengthNotMultipleOfFour {
        actual_bytes: usize,
    },
    DeclaredLengthMismatch {
        declared_words: u16,
        actual_words: usize,
    },
    SectionLengthMismatch {
        declared_words: u16,
        computed_words: usize,
    },
    SectionLengthOverflow,
    MissingRequiredHeader {
        lh: u16,
    },
    InvalidCharacterBounds {
        bc: u16,
        ec: u16,
    },
    EmptyMetricTable(TfmTable),
    NonZeroFirstMetric(TfmTable),
    InvalidHeaderString {
        field: &'static str,
        length: u8,
        capacity: usize,
    },
    InvalidDesignSize(TfmConversionError),
    InvalidFixWord {
        table: TfmTable,
        index: usize,
        source: TfmConversionError,
    },
    CharMetricIndexOutOfBounds {
        code: u8,
        table: TfmTable,
        index: u8,
        len: usize,
    },
    MissingCharacterHasTag {
        code: u8,
        tag: u8,
    },
    LigKernProgramIndexOutOfBounds {
        code: u8,
        index: u8,
        len: usize,
    },
    LigKernSkipOutOfBounds {
        index: usize,
        target: usize,
        len: usize,
    },
    LigKernRestartOutOfBounds {
        index: usize,
        target: u16,
        len: usize,
    },
    KernIndexOutOfBounds {
        instruction: usize,
        index: u16,
        len: usize,
    },
    NextLargerCharacterMissing {
        code: u8,
        next: u8,
    },
    NextLargerCycle {
        code: u8,
    },
    ExtensibleRecipeIndexOutOfBounds {
        code: u8,
        index: u8,
        len: usize,
    },
    ExtensibleRecipeCharacterMissing {
        recipe: usize,
        field: &'static str,
        code: u8,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { actual_bytes } => {
                write!(f, "TFM file is too short: {actual_bytes} bytes")
            }
            Self::LengthNotMultipleOfFour { actual_bytes } => {
                write!(
                    f,
                    "TFM length is not a multiple of four: {actual_bytes} bytes"
                )
            }
            Self::DeclaredLengthMismatch {
                declared_words,
                actual_words,
            } => write!(
                f,
                "TFM declared length {declared_words} words does not match actual length {actual_words}"
            ),
            Self::SectionLengthMismatch {
                declared_words,
                computed_words,
            } => write!(
                f,
                "TFM section lengths sum to {computed_words} words, not declared length {declared_words}"
            ),
            Self::SectionLengthOverflow => f.write_str("TFM section lengths overflow host size"),
            Self::MissingRequiredHeader { lh } => {
                write!(f, "TFM header has {lh} words; at least 2 are required")
            }
            Self::InvalidCharacterBounds { bc, ec } => {
                write!(f, "invalid TFM character bounds bc={bc}, ec={ec}")
            }
            Self::EmptyMetricTable(table) => write!(f, "TFM {table:?} table is empty"),
            Self::NonZeroFirstMetric(table) => {
                write!(f, "first TFM {table:?} table entry must be zero")
            }
            Self::InvalidHeaderString {
                field,
                length,
                capacity,
            } => write!(
                f,
                "TFM header {field} string length {length} exceeds capacity {capacity}"
            ),
            Self::InvalidDesignSize(source) => write!(f, "invalid TFM design size: {source}"),
            Self::InvalidFixWord {
                table,
                index,
                source,
            } => write!(
                f,
                "invalid TFM {table:?} fix_word at index {index}: {source}"
            ),
            Self::CharMetricIndexOutOfBounds {
                code,
                table,
                index,
                len,
            } => write!(
                f,
                "character {code} references {table:?} index {index}, but table has {len} entries"
            ),
            Self::MissingCharacterHasTag { code, tag } => {
                write!(f, "missing character {code} has nonzero tag {tag}")
            }
            Self::LigKernProgramIndexOutOfBounds { code, index, len } => write!(
                f,
                "character {code} references lig/kern program index {index}, but table has {len} entries"
            ),
            Self::LigKernSkipOutOfBounds { index, target, len } => write!(
                f,
                "lig/kern instruction {index} skips to {target}, but table has {len} entries"
            ),
            Self::LigKernRestartOutOfBounds { index, target, len } => write!(
                f,
                "lig/kern instruction {index} restarts at {target}, but table has {len} entries"
            ),
            Self::KernIndexOutOfBounds {
                instruction,
                index,
                len,
            } => write!(
                f,
                "lig/kern instruction {instruction} references kern index {index}, but table has {len} entries"
            ),
            Self::NextLargerCharacterMissing { code, next } => {
                write!(
                    f,
                    "character {code} points to missing next-larger character {next}"
                )
            }
            Self::NextLargerCycle { code } => {
                write!(f, "next-larger chain starting at character {code} cycles")
            }
            Self::ExtensibleRecipeIndexOutOfBounds { code, index, len } => write!(
                f,
                "character {code} references extensible recipe {index}, but table has {len} entries"
            ),
            Self::ExtensibleRecipeCharacterMissing {
                recipe,
                field,
                code,
            } => write!(
                f,
                "extensible recipe {recipe} {field} references missing character {code}"
            ),
        }
    }
}

impl std::error::Error for ParseError {}
