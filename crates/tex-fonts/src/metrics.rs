//! Immutable loaded font records and backend-neutral metric queries.

use std::path::PathBuf;
use tex_arith::Scaled;

/// TeX82 guarantees `fontdimen1` through `fontdimen7` for every loaded font.
pub const MIN_TEX_FONT_PARAMETERS: usize = 7;

/// Maximum lig/kern program length addressable by the runtime `u16` cursor.
///
/// Length 65,536 is valid: its final instruction has index `u16::MAX` and
/// must terminate rather than advance. Any longer table has unaddressable
/// instructions and is rejected before becoming live metric state.
pub const MAX_LIG_KERN_PROGRAM_LEN: usize = u16::MAX as usize + 1;

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
        mut parameters: Vec<Scaled>,
        metrics: FontMetrics,
    ) -> Self {
        parameters.resize(
            MIN_TEX_FONT_PARAMETERS.max(parameters.len()),
            Scaled::from_raw(0),
        );
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
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FontMetrics {
    characters: Vec<Option<CharMetrics>>,
    /// Dense, immutable hot-path projection of `characters`.
    ///
    /// Missing byte characters have zero width. This is derived once when the
    /// font is loaded and therefore carries no independent semantic state.
    widths: [Scaled; 256],
    lig_kern_program: Vec<LigKernInstruction>,
    right_boundary_char: Option<u8>,
    left_boundary_program: Option<u16>,
    extensible_recipes: Vec<ExtensibleRecipe>,
}

/// Structural validation failure for a detached immutable metric record.
///
/// TFM parsing performs these checks while decoding the source tables. This
/// error type lets other untrusted-data boundaries, such as format restore,
/// enforce the same query-safety invariants before constructing live state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FontMetricsValidationError {
    TooManyCharacters {
        len: usize,
    },
    LigKernProgramIndexOutOfBounds {
        character: u8,
        field: &'static str,
        index: u16,
        len: usize,
    },
    ExtensibleRecipeIndexOutOfBounds {
        character: u8,
        index: u8,
        len: usize,
    },
    LeftBoundaryProgramOutOfBounds {
        index: u16,
        len: usize,
    },
    LigKernProgramTooLong {
        len: usize,
        max: usize,
    },
    LigKernSkipOutOfBounds {
        instruction: usize,
        target: usize,
        len: usize,
    },
    LigKernCharacterMissing {
        instruction: usize,
        field: &'static str,
        character: u8,
    },
    ExtensibleRecipeCharacterMissing {
        recipe: usize,
        field: &'static str,
        character: u8,
    },
    NextLargerCycle {
        character: u8,
    },
}

impl std::fmt::Display for FontMetricsValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyCharacters { len } => {
                write!(
                    f,
                    "character table has {len} entries; at most 256 are addressable"
                )
            }
            Self::LigKernProgramIndexOutOfBounds {
                character,
                field,
                index,
                len,
            } => write!(
                f,
                "character {character} lig/kern {field} index {index} is outside program length {len}"
            ),
            Self::ExtensibleRecipeIndexOutOfBounds {
                character,
                index,
                len,
            } => write!(
                f,
                "character {character} extensible recipe index {index} is outside recipe count {len}"
            ),
            Self::LeftBoundaryProgramOutOfBounds { index, len } => write!(
                f,
                "left-boundary lig/kern index {index} is outside program length {len}"
            ),
            Self::LigKernProgramTooLong { len, max } => write!(
                f,
                "lig/kern program has {len} entries; runtime cursor capacity is {max}"
            ),
            Self::LigKernSkipOutOfBounds {
                instruction,
                target,
                len,
            } => write!(
                f,
                "lig/kern instruction {instruction} skips to {target} outside program length {len}"
            ),
            Self::LigKernCharacterMissing {
                instruction,
                field,
                character,
            } => write!(
                f,
                "lig/kern instruction {instruction} {field} character {character} is absent"
            ),
            Self::ExtensibleRecipeCharacterMissing {
                recipe,
                field,
                character,
            } => write!(
                f,
                "extensible recipe {recipe} {field} character {character} is absent"
            ),
            Self::NextLargerCycle { character } => {
                write!(f, "next-larger chain from character {character} is cyclic")
            }
        }
    }
}

impl std::error::Error for FontMetricsValidationError {}

impl FontMetrics {
    #[must_use]
    pub fn new(
        characters: Vec<Option<CharMetrics>>,
        lig_kern_program: Vec<LigKernInstruction>,
        right_boundary_char: Option<u8>,
        left_boundary_program: Option<u16>,
        extensible_recipes: Vec<ExtensibleRecipe>,
    ) -> Self {
        let mut widths = [Scaled::from_raw(0); 256];
        for (code, character) in characters.iter().take(256).enumerate() {
            if let Some(metrics) = character {
                widths[code] = metrics.width;
            }
        }
        Self {
            characters,
            widths,
            lig_kern_program,
            right_boundary_char,
            left_boundary_program,
            extensible_recipes,
        }
    }

    /// Validates all shape and reference invariants needed by metric queries.
    ///
    /// This intentionally mirrors the structural checks made by the TFM
    /// parser after raw table indices have been projected into this detached
    /// representation. A next-larger target may be absent, as TeX82 permits;
    /// ligature and extensible-recipe character references must exist.
    pub fn validate(&self) -> Result<(), FontMetricsValidationError> {
        if self.characters.len() > 256 {
            return Err(FontMetricsValidationError::TooManyCharacters {
                len: self.characters.len(),
            });
        }
        if self.lig_kern_program.len() > MAX_LIG_KERN_PROGRAM_LEN {
            return Err(FontMetricsValidationError::LigKernProgramTooLong {
                len: self.lig_kern_program.len(),
                max: MAX_LIG_KERN_PROGRAM_LEN,
            });
        }

        for (code, character) in self.characters.iter().enumerate() {
            let Some(character) = character else {
                continue;
            };
            let code = code as u8;
            match character.tag {
                CharTag::None | CharTag::NextLarger(_) => {}
                CharTag::LigKern {
                    program_index,
                    start_index,
                } => {
                    for (field, index) in
                        [("source", u16::from(program_index)), ("start", start_index)]
                    {
                        if usize::from(index) >= self.lig_kern_program.len() {
                            return Err(
                                FontMetricsValidationError::LigKernProgramIndexOutOfBounds {
                                    character: code,
                                    field,
                                    index,
                                    len: self.lig_kern_program.len(),
                                },
                            );
                        }
                    }
                }
                CharTag::Extensible(index) => {
                    if usize::from(index) >= self.extensible_recipes.len() {
                        return Err(
                            FontMetricsValidationError::ExtensibleRecipeIndexOutOfBounds {
                                character: code,
                                index,
                                len: self.extensible_recipes.len(),
                            },
                        );
                    }
                }
            }
        }

        if let Some(index) = self.left_boundary_program
            && usize::from(index) >= self.lig_kern_program.len()
        {
            return Err(FontMetricsValidationError::LeftBoundaryProgramOutOfBounds {
                index,
                len: self.lig_kern_program.len(),
            });
        }

        for (index, instruction) in self.lig_kern_program.iter().enumerate() {
            if instruction.skip_byte < 128 {
                let target = index + usize::from(instruction.skip_byte) + 1;
                if target >= self.lig_kern_program.len() {
                    return Err(FontMetricsValidationError::LigKernSkipOutOfBounds {
                        instruction: index,
                        target,
                        len: self.lig_kern_program.len(),
                    });
                }
            }
            if instruction.skip_byte <= 128 {
                if Some(instruction.next_char) != self.right_boundary_char
                    && !self.char_exists(instruction.next_char)
                {
                    return Err(FontMetricsValidationError::LigKernCharacterMissing {
                        instruction: index,
                        field: "match",
                        character: instruction.next_char,
                    });
                }
                if let Some(LigKernCommand::Ligature(command)) = instruction.command
                    && !self.char_exists(command.replacement)
                {
                    return Err(FontMetricsValidationError::LigKernCharacterMissing {
                        instruction: index,
                        field: "replacement",
                        character: command.replacement,
                    });
                }
            }
        }

        for (index, recipe) in self.extensible_recipes.iter().enumerate() {
            for (field, character) in [
                ("top", recipe.top),
                ("middle", recipe.middle),
                ("bottom", recipe.bottom),
                ("repeated", Some(recipe.repeated)),
            ] {
                if let Some(character) = character
                    && !self.char_exists(character)
                {
                    return Err(
                        FontMetricsValidationError::ExtensibleRecipeCharacterMissing {
                            recipe: index,
                            field,
                            character,
                        },
                    );
                }
            }
        }

        for start in 0..self.characters.len() {
            if self.characters[start].is_none() {
                continue;
            }
            let mut seen = [false; 256];
            let mut code = start as u8;
            loop {
                if seen[usize::from(code)] {
                    return Err(FontMetricsValidationError::NextLargerCycle {
                        character: start as u8,
                    });
                }
                seen[usize::from(code)] = true;
                let Some(next) = self.next_larger(code) else {
                    break;
                };
                code = next;
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn character(&self, code: u8) -> Option<CharMetrics> {
        self.characters
            .get(usize::from(code))
            .and_then(|entry| *entry)
    }

    /// Dense TFM-byte width table used by compact node-run scans.
    #[must_use]
    pub const fn widths(&self) -> &[Scaled; 256] {
        &self.widths
    }

    /// Immutable character records parallel to the dense width projection.
    #[must_use]
    pub fn characters(&self) -> &[Option<CharMetrics>] {
        &self.characters
    }

    #[must_use]
    pub fn lig_kern_program(&self) -> &[LigKernInstruction] {
        &self.lig_kern_program
    }

    #[must_use]
    pub const fn right_boundary_char(&self) -> Option<u8> {
        self.right_boundary_char
    }

    #[must_use]
    pub const fn left_boundary_program(&self) -> Option<u16> {
        self.left_boundary_program
    }

    #[must_use]
    pub fn extensible_recipes(&self) -> &[ExtensibleRecipe] {
        &self.extensible_recipes
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
        let mut index = self.lig_kern_start(left)?;
        let right_char = match right {
            LigKernChar::Char(code) => code,
            LigKernChar::Boundary => self.right_boundary_char?,
        };
        loop {
            let instruction = self.lig_kern_program.get(usize::from(index))?;
            if instruction.next_char == right_char
                && let Some(command) = instruction.command
            {
                return Some(command);
            }
            if instruction.skip_byte >= 128 {
                return None;
            }
            let target = usize::from(index) + usize::from(instruction.skip_byte) + 1;
            index = u16::try_from(target).ok()?;
        }
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

impl Default for FontMetrics {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new(), None, None, Vec::new())
    }
}

/// Dimensions and tag data for a present character.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CharMetrics {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub italic_correction: Scaled,
    pub tag: CharTag,
}

/// Non-dimensional character table tag.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum CharTag {
    None,
    LigKern { program_index: u8, start_index: u16 },
    NextLarger(u8),
    Extensible(u8),
}

/// A character code or TeX lig/kern boundary marker.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum LigKernChar {
    Char(u8),
    Boundary,
}

/// One executable lig/kern program instruction.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct LigKernInstruction {
    pub skip_byte: u8,
    pub next_char: u8,
    pub command: Option<LigKernCommand>,
}

/// Result of a matching lig/kern instruction.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum LigKernCommand {
    Ligature(LigatureCommand),
    Kern(Scaled),
}

/// Ligature operation including TeX's retention and pass-over bits.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
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
            let target = usize::from(index) + usize::from(instruction.skip_byte) + 1;
            u16::try_from(target).ok()
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
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
