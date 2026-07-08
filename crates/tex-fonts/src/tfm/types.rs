use tex_arith::{FontSizeSpec, Scaled};

use super::ParseError;
use crate::metrics::{
    CharMetrics, CharTag as MetricCharTag, ExtensibleRecipe as MetricExtensibleRecipe, FontMetrics,
    LigKernCommand, LigKernInstruction, LigatureCommand,
};

/// A fully parsed, immutable TeX Font Metric file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TfmFont {
    pub header: Header,
    pub bounds: CharacterBounds,
    pub font_size: Scaled,
    pub characters: Vec<Option<Character>>,
    pub widths: Vec<Scaled>,
    pub heights: Vec<Scaled>,
    pub depths: Vec<Scaled>,
    pub italic_corrections: Vec<Scaled>,
    pub lig_kern_program: Vec<LigKernStep>,
    pub right_boundary_char: Option<u8>,
    pub left_boundary_program: Option<u16>,
    pub kerns: Vec<Scaled>,
    pub extensible_recipes: Vec<ExtensibleRecipe>,
    pub parameters: FontParameters,
}

impl TfmFont {
    /// Parses a TFM byte slice using the design size stored in the file.
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        Self::parse_with_size(bytes, FontSizeSpec::Design)
    }

    /// Parses a TFM byte slice and scales metric dimensions for a TeX font size specification.
    pub fn parse_with_size(bytes: &[u8], size_spec: FontSizeSpec) -> Result<Self, ParseError> {
        super::parse::parse_tfm(bytes, size_spec)
    }

    /// Returns the parsed data for a character code, if the TFM marks it present.
    #[must_use]
    pub fn character(&self, code: u8) -> Option<&Character> {
        self.characters
            .get(usize::from(code))
            .and_then(Option::as_ref)
    }

    /// Converts parsed TFM data into the backend-neutral immutable metric record.
    #[must_use]
    pub fn font_metrics(&self) -> FontMetrics {
        FontMetrics::new(
            self.characters
                .iter()
                .map(|character| {
                    character.as_ref().map(|character| CharMetrics {
                        width: character.width,
                        height: character.height,
                        depth: character.depth,
                        italic_correction: character.italic_correction,
                        tag: character.tag.into(),
                    })
                })
                .collect(),
            self.lig_kern_program
                .iter()
                .map(|step| LigKernInstruction {
                    skip_byte: step.skip_byte,
                    next_char: step.next_char,
                    command: step.action.map(Into::into),
                })
                .collect(),
            self.right_boundary_char,
            self.left_boundary_program,
            self.extensible_recipes
                .iter()
                .copied()
                .map(Into::into)
                .collect(),
        )
    }
}

impl From<CharacterTag> for MetricCharTag {
    fn from(value: CharacterTag) -> Self {
        match value {
            CharacterTag::None => Self::None,
            CharacterTag::LigKern {
                program_index,
                start_index,
            } => Self::LigKern {
                program_index,
                start_index,
            },
            CharacterTag::NextLarger(code) => Self::NextLarger(code),
            CharacterTag::Extensible(index) => Self::Extensible(index),
        }
    }
}

impl From<LigKernAction> for LigKernCommand {
    fn from(value: LigKernAction) -> Self {
        match value {
            LigKernAction::Ligature(ligature) => Self::Ligature(LigatureCommand {
                replacement: ligature.replacement,
                delete_current: ligature.deletes.current,
                delete_next: ligature.deletes.next,
                pass_over: ligature.pass_over,
            }),
            LigKernAction::Kern(kern) => Self::Kern(kern.amount),
        }
    }
}

impl From<ExtensibleRecipe> for MetricExtensibleRecipe {
    fn from(value: ExtensibleRecipe) -> Self {
        Self {
            top: value.top,
            middle: value.middle,
            bottom: value.bottom,
            repeated: value.repeated,
        }
    }
}

/// Header metadata stored before the metric tables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Header {
    pub checksum: u32,
    pub design_size: Scaled,
    pub coding_scheme: Option<String>,
    pub family: Option<String>,
    pub seven_bit_safe: Option<bool>,
    pub face: Option<u8>,
    pub additional_words: Vec<[u8; 4]>,
}

/// The inclusive character range declared by the TFM preamble.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CharacterBounds {
    pub bc: u8,
    pub ec: u8,
}

/// A present character and its unpacked `char_info` fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Character {
    pub code: u8,
    pub width_index: u8,
    pub height_index: u8,
    pub depth_index: u8,
    pub italic_index: u8,
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub italic_correction: Scaled,
    pub tag: CharacterTag,
}

/// Meaning of a character's `tag` and `remainder` fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharacterTag {
    None,
    LigKern { program_index: u8, start_index: u16 },
    NextLarger(u8),
    Extensible(u8),
}

/// One raw lig/kern program word, with validated decoded operation data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LigKernStep {
    pub skip_byte: u8,
    pub next_char: u8,
    pub op_byte: u8,
    pub remainder: u8,
    pub restart_index: Option<u16>,
    pub action: Option<LigKernAction>,
}

/// Executable operation for a lig/kern program step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LigKernAction {
    Ligature(Ligature),
    Kern(Kern),
}

/// A ligature operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ligature {
    pub replacement: u8,
    pub deletes: LigatureDeletes,
    pub pass_over: u8,
}

/// Which input characters are deleted by a ligature operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LigatureDeletes {
    pub current: bool,
    pub next: bool,
}

/// A kern operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Kern {
    pub kern_index: u16,
    pub amount: Scaled,
}

/// A TFM extensible recipe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExtensibleRecipe {
    pub top: Option<u8>,
    pub middle: Option<u8>,
    pub bottom: Option<u8>,
    pub repeated: u8,
}

/// Parsed `fontdimen` parameters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontParameters {
    pub values: Vec<FontParameter>,
}

impl FontParameters {
    #[must_use]
    pub fn get(&self, number: u16) -> Option<&FontParameter> {
        if number == 0 {
            return None;
        }
        self.values.get(usize::from(number - 1))
    }

    #[must_use]
    pub fn slant(&self) -> Option<Scaled> {
        self.get(1).map(|param| param.value)
    }

    #[must_use]
    pub fn math_parameters(&self) -> &[FontParameter] {
        if self.values.len() <= 7 {
            &[]
        } else {
            &self.values[7..]
        }
    }
}

/// One `fontdimen` value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FontParameter {
    pub number: u16,
    pub value: Scaled,
    pub kind: FontParameterKind,
}

/// Scaling rule used for a font parameter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontParameterKind {
    /// `fontdimen1` is a dimensionless fix_word ratio; `Scaled::UNITY` represents 1.0.
    SlantRatio,
    /// All other parameters are font-size-scaled dimensions.
    Dimension,
}

/// Metric table names used in parse errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TfmTable {
    Width,
    Height,
    Depth,
    Italic,
    LigKern,
    Kern,
    Extensible,
    Param,
}
