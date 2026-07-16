//! Immutable loaded font records and backend-neutral metric queries.

use crate::opentype::{
    FontContainer, FontFeaturePolicy, FontInstanceIdentity, FontObjectIdentity,
    FontProgramIdentity, OpenTypeFont, VariationSelection, WritingDirection,
};
use sha2::{Digest, Sha256};
use std::hash::Hash;
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
    source_parameters: Vec<Scaled>,
    metrics: FontMetricsSource,
    opentype: Option<OpenTypeFontSelection>,
    construction: FontConstruction,
}

/// Host-neutral provenance for an immutable font instance.
///
/// pdfTeX allocates copied and letterspaced fonts as distinct internal fonts,
/// even when their source bytes and nominal name are otherwise identical.
/// Keeping that distinction on the immutable record prevents state restore
/// and semantic hashing from accidentally folding generated instances back
/// into an ordinary file load.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontSourceIdentity([u8; 32]);

impl FontSourceIdentity {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FontConstruction {
    Loaded,
    Copied {
        source: FontSourceIdentity,
    },
    Letterspaced {
        source: FontSourceIdentity,
        amount: i16,
        no_ligatures: bool,
    },
    Expanded {
        source: FontSourceIdentity,
        ratio: i16,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontConstructionError {
    WidthOverflow { character: u8 },
}

impl std::fmt::Display for FontConstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WidthOverflow { character } => {
                write!(
                    f,
                    "letterspaced width for character {character} overflows scaled arithmetic"
                )
            }
        }
    }
}

impl std::error::Error for FontConstructionError {}

/// OpenType program selected alongside classic TeX metrics for artifact/output reuse.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenTypeProgramSelection {
    pub font: OpenTypeFont,
    pub variation: VariationSelection,
    pub features: FontFeaturePolicy,
    pub direction: WritingDirection,
}

/// Metrics selected for character existence and width queries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FontMetricsSource {
    Tfm(FontMetrics),
    OpenType(OpenTypeFontShaped),
}

impl std::hash::Hash for FontMetricsSource {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Tfm(metrics) => {
                0_u8.hash(state);
                metrics.hash(state);
            }
            Self::OpenType(font) => {
                1_u8.hash(state);
                font.font.identity.hash(state);
                font.classic_metrics.hash(state);
            }
        }
    }
}

/// Validated OpenType metrics prepared for layout queries.
///
/// Stage 1 retains the selected TFM tables for classic-only enquiries such as
/// lig/kern and math while character existence and advances dispatch through
/// the OpenType program. A later OpenType-only selection stage replaces that
/// compatibility input with synthesized TeX font parameters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenTypeFontShaped {
    font: Box<OpenTypeFont>,
    classic_metrics: FontMetrics,
}

impl OpenTypeFontShaped {
    #[must_use]
    pub const fn font(&self) -> &OpenTypeFont {
        &self.font
    }

    fn character_width(&self, ch: char, size: Scaled) -> Option<Scaled> {
        let glyph = usize::from(self.font.cmap.glyph(ch)?);
        let advance = *self.font.metrics.horizontal_advances.get(glyph)?;
        self.font
            .metrics
            .units_to_sp(i32::from(advance), size.raw())
            .ok()
            .map(Scaled::from_raw)
    }

    fn character_metrics(&self, ch: char, size: Scaled) -> Option<CharMetrics> {
        let glyph = usize::from(self.font.cmap.glyph(ch)?);
        let width = self.character_width(ch, size)?;
        let bounds = self.font.metrics.glyph_bounds.get(glyph).copied().flatten();
        let (height, depth, italic_correction) = if let Some((_, y_min, x_max, y_max)) = bounds {
            let project = |units| {
                self.font
                    .metrics
                    .units_to_sp(units, size.raw())
                    .ok()
                    .map(Scaled::from_raw)
            };
            (
                project(i32::from(y_max).max(0))?,
                project((-i32::from(y_min)).max(0))?,
                project(
                    (i32::from(x_max)
                        - i32::from(*self.font.metrics.horizontal_advances.get(glyph)?))
                    .max(0),
                )?,
            )
        } else {
            (
                Scaled::from_raw(0),
                Scaled::from_raw(0),
                Scaled::from_raw(0),
            )
        };
        Some(CharMetrics {
            width,
            height,
            depth,
            italic_correction,
            tag: CharTag::None,
        })
    }
}

impl FontMetricsSource {
    fn with_added_width(&self, delta: Scaled) -> Result<Self, FontConstructionError> {
        Ok(match self {
            Self::Tfm(metrics) => Self::Tfm(metrics.with_added_width(delta)?),
            Self::OpenType(font) => Self::OpenType(OpenTypeFontShaped {
                font: font.font.clone(),
                classic_metrics: font.classic_metrics.with_added_width(delta)?,
            }),
        })
    }

    fn with_expansion_ratio(&self, ratio: i16) -> Self {
        match self {
            Self::Tfm(metrics) => Self::Tfm(metrics.with_expansion_ratio(ratio)),
            Self::OpenType(font) => Self::OpenType(OpenTypeFontShaped {
                font: font.font.clone(),
                classic_metrics: font.classic_metrics.with_expansion_ratio(ratio),
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct OpenTypeFontSelection {
    pub program_identity: FontProgramIdentity,
    pub object_identity: FontObjectIdentity,
    pub instance_identity: FontInstanceIdentity,
    pub container: FontContainer,
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
        let source_parameters = parameters.clone();
        Self {
            name: name.into(),
            path: path.into(),
            content_hash,
            checksum,
            design_size,
            size,
            parameters,
            source_parameters,
            metrics: FontMetricsSource::Tfm(metrics),
            opentype: None,
            construction: FontConstruction::Loaded,
        }
    }

    #[must_use]
    pub fn with_opentype(mut self, selection: OpenTypeProgramSelection) -> Self {
        let OpenTypeProgramSelection {
            font,
            variation,
            features,
            direction,
        } = selection;
        let program_identity = font.identity;
        let object_identity = font.object_identity;
        let face_index = font.face_index;
        let container = font.container;
        let classic_metrics = match self.metrics {
            FontMetricsSource::Tfm(metrics) => metrics,
            FontMetricsSource::OpenType(font) => font.classic_metrics,
        };
        self.metrics = FontMetricsSource::OpenType(OpenTypeFontShaped {
            font: Box::new(font),
            classic_metrics,
        });
        self.opentype = Some(OpenTypeFontSelection {
            program_identity,
            object_identity,
            instance_identity: FontInstanceIdentity::new(
                program_identity,
                face_index,
                self.size.raw(),
                &variation,
                &features,
                direction,
            ),
            container,
        });
        self
    }

    #[must_use]
    pub const fn opentype(&self) -> Option<&OpenTypeFontSelection> {
        self.opentype.as_ref()
    }

    #[must_use]
    pub const fn construction(&self) -> &FontConstruction {
        &self.construction
    }

    /// Deterministic, host-neutral identity for generated-font ancestry.
    #[must_use]
    pub fn source_identity(&self) -> FontSourceIdentity {
        let mut hasher = Sha256::new();
        hasher.update(b"umber-font-source-v1");
        hasher.update((self.name.len() as u64).to_le_bytes());
        hasher.update(self.name.as_bytes());
        hasher.update(self.content_hash);
        hasher.update(self.checksum.to_le_bytes());
        hasher.update(self.design_size.raw().to_le_bytes());
        hasher.update(self.size.raw().to_le_bytes());
        hasher.update((self.parameters.len() as u64).to_le_bytes());
        for parameter in &self.parameters {
            hasher.update(parameter.raw().to_le_bytes());
        }
        match self.construction {
            FontConstruction::Loaded => hasher.update([0]),
            FontConstruction::Copied { source } => {
                hasher.update([1]);
                hasher.update(source.bytes());
            }
            FontConstruction::Letterspaced {
                source,
                amount,
                no_ligatures,
            } => {
                hasher.update([2]);
                hasher.update(source.bytes());
                hasher.update(amount.to_le_bytes());
                hasher.update([u8::from(no_ligatures)]);
            }
            FontConstruction::Expanded { source, ratio } => {
                hasher.update([3]);
                hasher.update(source.bytes());
                hasher.update(ratio.to_le_bytes());
            }
        }
        FontSourceIdentity(hasher.finalize().into())
    }

    /// Reattaches validated construction metadata at a detached restore
    /// boundary. Runtime callers should prefer [`Self::copied`] or
    /// [`Self::letterspaced`].
    #[must_use]
    pub fn with_construction(mut self, construction: FontConstruction) -> Self {
        self.construction = construction;
        self
    }

    /// Restores the original file-backed font parameters retained by a
    /// generated font. They are used when pdfTeX semantics reread the source
    /// metrics, as `\letterspacefont` does for a copied font.
    #[must_use]
    pub fn with_source_parameters(mut self, mut parameters: Vec<Scaled>) -> Self {
        parameters.resize(
            MIN_TEX_FONT_PARAMETERS.max(parameters.len()),
            Scaled::from_raw(0),
        );
        self.source_parameters = parameters;
        self
    }

    /// Creates pdfTeX's independent `\pdfcopyfont` metric record.
    ///
    /// The supplied parameters are the source font's current fontdimen bank,
    /// because pdfTeX copies mutable `font_info` rather than rereading the TFM.
    #[must_use]
    pub fn copied(&self, parameters: Vec<Scaled>) -> Self {
        let source = self.source_identity();
        let mut copied = self.clone();
        copied.parameters = parameters;
        copied.parameters.resize(
            MIN_TEX_FONT_PARAMETERS.max(copied.parameters.len()),
            Scaled::from_raw(0),
        );
        copied.construction = FontConstruction::Copied { source };
        copied
    }

    /// Creates pdfTeX's immutable letterspaced metric projection.
    ///
    /// pdfTeX rereads the source TFM at the existing size, so ordinary source
    /// fontdimen edits are not inherited. Its one exception is a zero TFM em:
    /// a positive current `fontdimen6` is used as the generated font's quad.
    pub fn letterspaced(
        &self,
        current_quad: Scaled,
        amount: i16,
        no_ligatures: bool,
    ) -> Result<Self, FontConstructionError> {
        debug_assert!((-1000..=1000).contains(&i32::from(amount)));
        let source = self.source_identity();
        let mut generated = self.clone();
        generated.parameters = self.source_parameters.clone();
        if generated.parameters[5].raw() == 0 && current_quad.raw() > 0 {
            generated.parameters[5] = current_quad;
        }
        let quad = generated.parameters[5];
        let delta = round_scaled_ratio(quad, i32::from(amount), 1000);
        generated.metrics = generated.metrics.with_added_width(delta)?;
        generated.name = if amount > 0 {
            format!("{}+{amount}ls", self.name)
        } else {
            format!("{}{amount}ls", self.name)
        };
        generated.construction = FontConstruction::Letterspaced {
            source,
            amount,
            no_ligatures,
        };
        Ok(generated)
    }

    /// Creates one of pdfTeX's lazily materialized expanded font instances.
    ///
    /// Expansion changes horizontal glyph metrics, italic corrections, and
    /// font kerns. Vertical metrics and font parameters remain unchanged.
    #[must_use]
    pub fn expanded(&self, ratio: i16) -> Self {
        debug_assert!((-500..=1000).contains(&i32::from(ratio)));
        let source = self.source_identity();
        let mut generated = self.clone();
        generated.metrics = generated.metrics.with_expansion_ratio(ratio);
        generated.construction = FontConstruction::Expanded { source, ratio };
        generated
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
    pub fn source_parameters(&self) -> &[Scaled] {
        &self.source_parameters
    }

    #[must_use]
    pub const fn metrics(&self) -> &FontMetrics {
        match &self.metrics {
            FontMetricsSource::Tfm(metrics) => metrics,
            FontMetricsSource::OpenType(font) => &font.classic_metrics,
        }
    }

    #[must_use]
    pub const fn metrics_source(&self) -> &FontMetricsSource {
        &self.metrics
    }

    #[must_use]
    pub fn character_exists(&self, ch: char) -> bool {
        match &self.metrics {
            FontMetricsSource::Tfm(metrics) => u8::try_from(ch as u32)
                .ok()
                .is_some_and(|code| metrics.char_exists(code)),
            FontMetricsSource::OpenType(font) => font.font.cmap.glyph(ch).is_some(),
        }
    }

    #[must_use]
    pub fn character_width(&self, ch: char) -> Option<Scaled> {
        match &self.metrics {
            FontMetricsSource::Tfm(metrics) => {
                let code = u8::try_from(ch as u32).ok()?;
                metrics.character(code).map(|metrics| metrics.width)
            }
            FontMetricsSource::OpenType(font) => font.character_width(ch, self.size),
        }
    }

    #[must_use]
    pub fn character_metrics(&self, ch: char) -> Option<CharMetrics> {
        match &self.metrics {
            FontMetricsSource::Tfm(metrics) => metrics.character(u8::try_from(ch as u32).ok()?),
            FontMetricsSource::OpenType(font) => font.character_metrics(ch, self.size),
        }
    }

    #[must_use]
    pub const fn uses_tfm_metrics(&self) -> bool {
        matches!(self.metrics, FontMetricsSource::Tfm(_))
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

    fn with_added_width(&self, delta: Scaled) -> Result<Self, FontConstructionError> {
        let mut characters = self.characters.clone();
        for (code, character) in characters.iter_mut().enumerate() {
            let Some(metrics) = character else {
                continue;
            };
            metrics.width =
                metrics
                    .width
                    .checked_add(delta)
                    .ok_or(FontConstructionError::WidthOverflow {
                        character: code as u8,
                    })?;
        }
        Ok(Self::new(
            characters,
            self.lig_kern_program.clone(),
            self.right_boundary_char,
            self.left_boundary_program,
            self.extensible_recipes.clone(),
        ))
    }

    fn with_expansion_ratio(&self, ratio: i16) -> Self {
        let mut characters = self.characters.clone();
        for metrics in characters.iter_mut().flatten() {
            metrics.width = scale_expanded_metric(metrics.width, ratio);
            metrics.italic_correction = scale_expanded_metric(metrics.italic_correction, ratio);
        }
        let mut lig_kern_program = self.lig_kern_program.clone();
        for instruction in &mut lig_kern_program {
            if let Some(LigKernCommand::Kern(kern)) = &mut instruction.command {
                *kern = scale_expanded_metric(*kern, ratio);
            }
        }
        Self::new(
            characters,
            lig_kern_program,
            self.right_boundary_char,
            self.left_boundary_program,
            self.extensible_recipes.clone(),
        )
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

fn round_scaled_ratio(value: Scaled, numerator: i32, denominator: i32) -> Scaled {
    debug_assert!(denominator > 0);
    let product = i64::from(value.raw()) * i64::from(numerator);
    let denominator = i64::from(denominator);
    let rounded = if product >= 0 {
        (product + denominator / 2) / denominator
    } else {
        -((-product + denominator / 2) / denominator)
    };
    Scaled::from_raw(i32::try_from(rounded).expect("bounded letterspace ratio fits i32"))
}

fn scale_expanded_metric(value: Scaled, ratio: i16) -> Scaled {
    round_scaled_ratio(value, 1000 + i32::from(ratio), 1000)
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
