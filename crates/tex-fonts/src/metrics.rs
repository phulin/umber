//! Immutable loaded font records and backend-neutral metric queries.

use std::path::PathBuf;
use tex_arith::Scaled;

/// Stable content identity for loaded font bytes.
pub type FontContentHash = [u8; 32];

/// Immutable data captured when a TFM font is loaded.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LoadedFont {
    name: String,
    path: PathBuf,
    content_hash: FontContentHash,
    checksum: u32,
    design_size: Scaled,
    size: Scaled,
    parameters: Vec<Scaled>,
    metrics: FontMetrics,
}

impl LoadedFont {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        path: impl Into<PathBuf>,
        content_hash: FontContentHash,
        checksum: u32,
        design_size: Scaled,
        size: Scaled,
        parameters: Vec<Scaled>,
        metrics: FontMetrics,
    ) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            content_hash,
            checksum,
            design_size,
            size,
            parameters,
            metrics,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    #[must_use]
    pub const fn content_hash(&self) -> FontContentHash {
        self.content_hash
    }

    #[must_use]
    pub const fn checksum(&self) -> u32 {
        self.checksum
    }

    #[must_use]
    pub const fn design_size(&self) -> Scaled {
        self.design_size
    }

    #[must_use]
    pub const fn size(&self) -> Scaled {
        self.size
    }

    #[must_use]
    pub fn parameters(&self) -> &[Scaled] {
        &self.parameters
    }

    #[must_use]
    pub const fn metrics(&self) -> &FontMetrics {
        &self.metrics
    }

    #[must_use]
    pub fn fontname_text(&self) -> String {
        if self.size == self.design_size {
            self.name.clone()
        } else {
            format!("{} at {}", self.name, format_scaled(self.size))
        }
    }
}

/// Backend-neutral metric tables consumed by typesetting kernels.
///
/// The current producer is TFM parsing, but the query surface is deliberately
/// phrased in TeX engine terms so an OpenType backend can populate the same
/// immutable record or answer behind the same `Universe` facade later.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct FontMetrics {
    characters: Vec<Option<CharMetrics>>,
    lig_kern_program: Vec<LigKernInstruction>,
    right_boundary_char: Option<u8>,
    left_boundary_program: Option<u16>,
    extensible_recipes: Vec<ExtensibleRecipe>,
}

impl FontMetrics {
    #[must_use]
    pub fn new(
        characters: Vec<Option<CharMetrics>>,
        lig_kern_program: Vec<LigKernInstruction>,
        right_boundary_char: Option<u8>,
        left_boundary_program: Option<u16>,
        extensible_recipes: Vec<ExtensibleRecipe>,
    ) -> Self {
        Self {
            characters,
            lig_kern_program,
            right_boundary_char,
            left_boundary_program,
            extensible_recipes,
        }
    }

    #[must_use]
    pub fn character(&self, code: u8) -> Option<CharMetrics> {
        self.characters
            .get(usize::from(code))
            .and_then(|entry| *entry)
    }

    #[must_use]
    pub fn char_exists(&self, code: u8) -> bool {
        self.character(code).is_some()
    }

    #[must_use]
    pub fn next_larger(&self, code: u8) -> Option<u8> {
        match self.character(code)?.tag {
            CharTag::NextLarger(next) => Some(next),
            _ => None,
        }
    }

    #[must_use]
    pub fn lig_kern_iter(&self, left: LigKernChar, right: LigKernChar) -> LigKernIter<'_> {
        let next_index = self.lig_kern_start(left);
        let right_char = match right {
            LigKernChar::Char(code) => Some(code),
            LigKernChar::Boundary => self.right_boundary_char,
        };
        LigKernIter {
            metrics: self,
            next_index,
            right_char,
        }
    }

    #[must_use]
    pub fn lig_kern_command(
        &self,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand> {
        self.lig_kern_iter(left, right)
            .find_map(|step| step.matches_right.then_some(step.command).flatten())
    }

    #[must_use]
    pub fn extensible_recipe(&self, code: u8) -> Option<ExtensibleRecipe> {
        let character = self.character(code)?;
        let index = match character.tag {
            CharTag::Extensible(index) => index,
            _ => return None,
        };
        self.extensible_recipes.get(usize::from(index)).copied()
    }

    fn lig_kern_start(&self, left: LigKernChar) -> Option<u16> {
        match left {
            LigKernChar::Boundary => self.left_boundary_program,
            LigKernChar::Char(code) => match self.character(code)?.tag {
                CharTag::LigKern { start_index, .. } => Some(start_index),
                _ => None,
            },
        }
    }
}

/// Dimensions and tag data for a present character.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CharMetrics {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub italic_correction: Scaled,
    pub tag: CharTag,
}

/// Non-dimensional character table tag.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CharTag {
    None,
    LigKern { program_index: u8, start_index: u16 },
    NextLarger(u8),
    Extensible(u8),
}

/// A character code or TeX lig/kern boundary marker.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LigKernChar {
    Char(u8),
    Boundary,
}

/// One executable lig/kern program instruction.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LigKernInstruction {
    pub skip_byte: u8,
    pub next_char: u8,
    pub command: Option<LigKernCommand>,
}

/// Result of a matching lig/kern instruction.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LigKernCommand {
    Ligature(LigatureCommand),
    Kern(Scaled),
}

/// Ligature operation including TeX's retention and pass-over bits.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LigatureCommand {
    pub replacement: u8,
    pub delete_current: bool,
    pub delete_next: bool,
    pub pass_over: u8,
}

/// A visited instruction in the lig/kern scan for a concrete pair.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LigKernStep {
    pub instruction_index: u16,
    pub next_char: u8,
    pub command: Option<LigKernCommand>,
    pub matches_right: bool,
}

/// Iterator over the lig/kern instructions TeX examines for one pair.
#[derive(Clone, Debug)]
pub struct LigKernIter<'a> {
    metrics: &'a FontMetrics,
    next_index: Option<u16>,
    right_char: Option<u8>,
}

impl Iterator for LigKernIter<'_> {
    type Item = LigKernStep;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.next_index?;
        let instruction = self.metrics.lig_kern_program.get(usize::from(index))?;
        self.next_index = if instruction.skip_byte >= 128 {
            None
        } else {
            Some(index + u16::from(instruction.skip_byte) + 1)
        };
        Some(LigKernStep {
            instruction_index: index,
            next_char: instruction.next_char,
            command: instruction.command,
            matches_right: self.right_char == Some(instruction.next_char),
        })
    }
}

/// A TeX extensible delimiter recipe.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ExtensibleRecipe {
    pub top: Option<u8>,
    pub middle: Option<u8>,
    pub bottom: Option<u8>,
    pub repeated: u8,
}

fn format_scaled(value: Scaled) -> String {
    let raw = value.raw();
    let negative = raw < 0;
    let magnitude = if negative {
        i64::from(raw).wrapping_neg()
    } else {
        i64::from(raw)
    };
    let unity = i64::from(Scaled::UNITY);
    let mut integer = magnitude / unity;
    let fraction = magnitude % unity;
    let mut decimal = ((fraction * 100_000) + (unity / 2)) / unity;
    if decimal == 100_000 {
        integer += 1;
        decimal = 0;
    }
    let mut fraction_text = format!("{decimal:05}");
    while fraction_text.len() > 1 && fraction_text.ends_with('0') {
        fraction_text.pop();
    }
    let sign = if negative { "-" } else { "" };
    format!("{sign}{integer}.{fraction_text}pt")
}
