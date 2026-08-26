use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct VersionedRows<T> {
    pub version: u32,
    pub rows: T,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct FormatName {
    pub kind: u8,
    pub hash_entry: bool,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct FormatDefinition {
    pub parameter_text: Vec<u32>,
    pub replacement_text: Vec<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct FormatGlue {
    pub width: i32,
    pub stretch: i32,
    pub stretch_order: u8,
    pub shrink: i32,
    pub shrink_order: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FormatFont {
    pub name: String,
    pub content_hash: [u8; 8],
    pub checksum: u32,
    pub design_size: i32,
    pub size: i32,
    pub parameters: Vec<i32>,
    pub source_parameters: Vec<i32>,
    pub font_info_words: u32,
    pub characters: Vec<Option<tex_fonts::CharMetrics>>,
    pub lig_kern_program: Vec<tex_fonts::LigKernInstruction>,
    pub right_boundary_char: Option<u8>,
    pub left_boundary_program: Option<u16>,
    pub extensible_recipes: Vec<tex_fonts::metrics::ExtensibleRecipe>,
    pub identifier: Option<u32>,
    pub expansion: Option<crate::font::FontExpansion>,
    pub construction: FormatFontConstruction,
    pub runtime: FormatFontRuntime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FormatFontRuntime {
    pub parameters: Vec<i32>,
    pub hyphen_char: i32,
    pub skew_char: i32,
    pub pdf_codes: Vec<Option<Vec<i32>>>,
    pub ligatures_disabled: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum FormatFontConstruction {
    Loaded,
    Copied {
        source: [u8; 8],
    },
    Letterspaced {
        source: [u8; 8],
        amount: i16,
        no_ligatures: bool,
    },
    Expanded {
        source: [u8; 8],
        ratio: i16,
    },
}

/// One durable-list row. Child-list references are one-based row indices;
/// zero names the canonical empty list. Each node is an independently framed
/// semantic record so validation can reject a malformed graph before staging.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct FormatNodeList {
    pub nodes: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct FormatCode {
    pub kind: u8,
    pub scalar: u32,
    pub value: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) enum FormatMeaning {
    Static(u64),
    Font(u32),
    Macro { flags: u8, definition: u32 },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) enum FormatCell {
    Meaning(u32, FormatMeaning),
    Count(u16, i32),
    Dimension(u16, i32),
    TokenRegister(u16, u32),
    GlueRegister(u16, u32),
    MuGlueRegister(u16, u32),
    BoxRegister(u16, u32),
    IntegerParameter(u16, i32),
    DimensionParameter(u16, i32),
    TokenParameter(u16, u32),
    GlueParameter(u16, u32),
    CurrentFont(u32),
    MathFamilyFont(u8, u32),
    Code { kind: u8, scalar: u32, value: i64 },
}
