//! Stateful font handles and rollback storage.

use crate::identity::{IdentityAllocator, IdentityMark};
use crate::ids::FontId;
use crate::interner::{ControlSequenceKind, SymbolId};
use crate::scaled::Scaled;
use crate::state_hash::StateHashFragment;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
pub use tex_fonts::metrics::{
    CharMetrics, CharTag, ExtensibleRecipe, FontConstruction, FontContentHash, FontMetrics,
    FontMetricsSource, FontMetricsValidationError, FontSourceIdentity, LigKernChar, LigKernCommand,
    LigKernInstruction, LigKernIter, LigKernStep, LigatureCommand, LoadedFont,
};

/// TeX's predefined null font.
pub const NULL_FONT: FontId = FontId::builtin(0);

/// One of pdfTeX's mutable per-font, per-byte character-code tables.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfFontCode {
    Lp,
    Rp,
    Ef,
    Tag,
    Knbs,
    Stbs,
    Shbs,
    Knbc,
    Knac,
}

/// Validated global `\pdffontexpand` settings attached to a base font.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct FontExpansion {
    pub stretch: u16,
    pub shrink: u16,
    pub step: u8,
    pub auto_expand: bool,
}

/// Handle-free description of how one immutable font was constructed.
///
/// Generated fonts name their source by semantic content identity.  The live
/// source slot is deliberately not part of this cold recipe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FontArtifactConstructionRecipe {
    Loaded,
    Copied {
        source_identity: FontSourceIdentity,
    },
    Letterspaced {
        source_identity: FontSourceIdentity,
        amount: i16,
        no_ligatures: bool,
    },
    Expanded {
        source_identity: FontSourceIdentity,
        ratio: i16,
    },
}

/// Owned OpenType metadata needed to lower a font into an artifact resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenTypeArtifactRecipe {
    pub program_identity: tex_fonts::FontProgramIdentity,
    pub object_identity: tex_fonts::FontObjectIdentity,
    pub instance_identity: tex_fonts::FontInstanceIdentity,
    pub container: tex_fonts::FontContainer,
    pub face_index: u32,
    pub variation: tex_fonts::VariationSelection,
    pub features: tex_fonts::FontFeaturePolicy,
    pub direction: tex_fonts::WritingDirection,
    pub script: Option<tex_fonts::OpenTypeTag>,
    pub language: Option<tex_fonts::FontLanguage>,
    pub encoding_map_version: Option<u8>,
    pub encoding_map_identity: Option<[u8; 8]>,
    pub fontdimen_synthesis_version: Option<u8>,
}

/// Owned, handle-free font metadata at the artifact publication boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontArtifactRecipe {
    pub name: String,
    pub tfm_content_hash: FontContentHash,
    pub tfm_checksum: u32,
    pub design_size: Scaled,
    pub at_size: Scaled,
    pub layout_policy: tex_fonts::FontLayoutPolicy,
    pub mapping_fallback: Option<tex_fonts::FontMappingFallbackPolicy>,
    pub opentype: Option<OpenTypeArtifactRecipe>,
    pub semantic_identity: FontSourceIdentity,
    pub construction: FontArtifactConstructionRecipe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontExpansionConfigError {
    ExpandedBase,
    DifferentStep,
    DifferentStretch,
    DifferentShrink,
    DifferentAutoExpand,
}

impl std::fmt::Display for FontExpansionConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ExpandedBase => "cannot expand an expanded font",
            Self::DifferentStep => "font has been expanded with different expansion step",
            Self::DifferentStretch => "font has been expanded with different stretch limit",
            Self::DifferentShrink => "font has been expanded with different shrink limit",
            Self::DifferentAutoExpand => {
                "font has been expanded with different auto expansion value"
            }
        })
    }
}

impl std::error::Error for FontExpansionConfigError {}

/// Largest TeX font-parameter number representable in a fontdimen cell key.
pub const MAX_FONT_DIMEN: u32 = 1 << 17;

/// TeX82's shared `font_info` word capacity (tex.web §11).
pub const FONT_INFO_CAPACITY: usize = 20_000;

/// TeX Live's Web2C runtime `font_mem_size` bound used by pdfTeX.
///
/// The pinned 2026 `texmf.cnf` selects this value; unlike TeX82's compiled
/// default, it is process configuration and is not part of a format image.
pub const WEB2C_FONT_INFO_CAPACITY: usize = 8_000_000;

/// Largest dense font id representable in a fontdimen cell key.
///
/// A font owns `2^17` possible parameter slots inside `CellId`'s 32-bit
/// index, leaving 15 bits for the dense font number.
pub const MAX_FONT_DIMEN_FONT_ID: u32 = (1 << 15) - 1;

/// Maximum number of loaded fonts, including `nullfont`.
pub(crate) const MAX_FONT_COUNT: usize = 1 << 15;
const IMMUTABLE_FONT_HASH_DOMAIN: u64 = 0x666f_6e74_5f69_6d6d;
const COMPLETE_FONT_HASH_DOMAIN: u64 = 0x666f_6e74_5f63_6d70;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FontStoreCapacityError;

/// A missing-character event for consumers to report according to policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MissingCharacter {
    pub font: FontId,
    pub code: u8,
}

/// Rollback watermark for loaded fonts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FontStoreMark {
    pub(crate) len: u32,
    pub(crate) identifier_writes_len: u32,
    pub(crate) expansion_writes_len: u32,
    non_parameter_font_info_words: usize,
    identities: IdentityMark,
}

impl FontStoreMark {
    pub(crate) fn checkpoint_retained_bytes(self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add((self.len as usize).saturating_mul(std::mem::size_of::<LoadedFont>()))
            .saturating_add(
                self.non_parameter_font_info_words
                    .saturating_mul(std::mem::size_of::<Scaled>()),
            )
            .saturating_add(
                (self.identifier_writes_len as usize)
                    .saturating_mul(std::mem::size_of::<(FontId, Option<SymbolId>)>()),
            )
            .saturating_add(
                (self.expansion_writes_len as usize)
                    .saturating_mul(std::mem::size_of::<(FontId, Option<FontExpansion>)>()),
            )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FontKey {
    name: String,
    size: Scaled,
    content_hash: FontContentHash,
}

/// Semantic font fields that remain immutable across the font's lifetime.
///
/// This is intentionally independent of the dense `FontId` and of the
/// rollback-coupled control-sequence identifier.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FontHashFragmentKey {
    name: String,
    content_hash: FontContentHash,
    checksum: u32,
    design_size: Scaled,
    size: Scaled,
    construction: FontConstruction,
}

#[derive(Debug)]
struct AcceptedFontBlock {
    parent: Option<Arc<Self>>,
    fonts: Arc<Vec<LoadedFont>>,
    identifiers: Arc<Vec<Option<SymbolId>>>,
    identifier_writes: Arc<Vec<(FontId, Option<SymbolId>, StateHashFragment)>>,
    identifier_writes_len: usize,
    expansion_specs: Arc<Vec<Option<FontExpansion>>>,
    expansion_writes: Arc<Vec<(FontId, Option<FontExpansion>)>>,
    expansion_writes_len: usize,
    by_key: Arc<BTreeMap<FontKey, FontId>>,
    hash_fragments: Arc<Vec<StateHashFragment>>,
    hash_fragments_by_key: Arc<BTreeMap<FontHashFragmentKey, usize>>,
    font_hash_fragments: Arc<Vec<StateHashFragment>>,
    complete_hash_fragments: Arc<Vec<StateHashFragment>>,
    len: usize,
    total_len: usize,
}

impl AcceptedFontBlock {
    fn base(&self) -> usize {
        self.total_len - self.len
    }

    fn font(&self, index: usize) -> Option<&LoadedFont> {
        if index < self.base() {
            return self.parent.as_ref()?.font(index);
        }
        self.fonts
            .get(index - self.base())
            .filter(|_| index < self.total_len)
    }

    fn identifier(&self, index: usize) -> Option<Option<SymbolId>> {
        if index < self.base() {
            return self.parent.as_ref()?.identifier(index);
        }
        let mut value = *self.identifiers.get(index - self.base())?;
        for (id, previous, _) in self.identifier_writes[self.identifier_writes_len..]
            .iter()
            .rev()
        {
            if id.raw() as usize == index {
                value = *previous;
            }
        }
        Some(value)
    }

    fn expansion(&self, index: usize) -> Option<Option<FontExpansion>> {
        if index < self.base() {
            return self.parent.as_ref()?.expansion(index);
        }
        let mut value = *self.expansion_specs.get(index - self.base())?;
        for (id, previous) in self.expansion_writes[self.expansion_writes_len..]
            .iter()
            .rev()
        {
            if id.raw() as usize == index {
                value = *previous;
            }
        }
        Some(value)
    }

    fn immutable_fragment(&self, index: usize) -> Option<&StateHashFragment> {
        if index < self.base() {
            return self.parent.as_ref()?.immutable_fragment(index);
        }
        self.font_hash_fragments
            .get(index - self.base())
            .filter(|_| index < self.total_len)
    }

    fn complete_fragment(&self, index: usize) -> Option<StateHashFragment> {
        if index < self.base() {
            return self.parent.as_ref()?.complete_fragment(index);
        }
        let mut value = *self.complete_hash_fragments.get(index - self.base())?;
        for (id, _, previous) in self.identifier_writes[self.identifier_writes_len..]
            .iter()
            .rev()
        {
            if id.raw() as usize == index {
                value = *previous;
            }
        }
        Some(value)
    }

    fn by_key(&self, key: &FontKey) -> Option<FontId> {
        self.by_key
            .get(key)
            .copied()
            .filter(|id| (id.raw() as usize) < self.total_len && (id.raw() as usize) >= self.base())
            .or_else(|| self.parent.as_ref()?.by_key(key))
    }

    fn cached_fragment(&self, key: &FontHashFragmentKey) -> Option<StateHashFragment> {
        self.hash_fragments_by_key
            .get(key)
            .and_then(|index| self.hash_fragments.get(*index))
            .copied()
            .or_else(|| self.parent.as_ref()?.cached_fragment(key))
    }
}

/// Immutable font payload split into accepted coarse blocks and one mutable
/// current-lineage suffix.
#[derive(Debug)]
pub(crate) struct FontStore {
    accepted: Option<Arc<AcceptedFontBlock>>,
    fonts: Arc<Vec<LoadedFont>>,
    /// TeX82's immutable `font_info` words other than mutable parameters.
    ///
    /// The parameter bank owns the current parameter extent, including
    /// §580 growth. Combining the two scalars recovers §549's `fmem_ptr`
    /// without retaining WEB addresses or rescanning every font at each
    /// capacity check.
    non_parameter_font_info_words: usize,
    identifiers: Arc<Vec<Option<SymbolId>>>,
    identifier_overrides: BTreeMap<FontId, (Option<SymbolId>, StateHashFragment)>,
    identifier_writes: Arc<Vec<(FontId, Option<SymbolId>, StateHashFragment)>>,
    identifier_writes_base: usize,
    expansion_specs: Arc<Vec<Option<FontExpansion>>>,
    expansion_overrides: BTreeMap<FontId, Option<FontExpansion>>,
    expansion_writes: Arc<Vec<(FontId, Option<FontExpansion>)>>,
    expansion_writes_base: usize,
    by_key: Arc<BTreeMap<FontKey, FontId>>,
    /// Append-only derived fragments keyed by semantic content. Rollback only
    /// truncates the live slot-to-fragment mapping, so a later equivalent load
    /// can reuse its domain-separated fingerprint.
    hash_fragments: Arc<Vec<StateHashFragment>>,
    hash_fragments_by_key: Arc<BTreeMap<FontHashFragmentKey, usize>>,
    font_hash_fragments: Arc<Vec<StateHashFragment>>,
    complete_hash_fragments: Arc<Vec<StateHashFragment>>,
    identities: IdentityAllocator,
}

impl Clone for FontStore {
    fn clone(&self) -> Self {
        Self {
            accepted: self.accepted.clone(),
            fonts: Arc::clone(&self.fonts),
            non_parameter_font_info_words: self.non_parameter_font_info_words,
            identifiers: Arc::clone(&self.identifiers),
            identifier_overrides: self.identifier_overrides.clone(),
            identifier_writes: Arc::clone(&self.identifier_writes),
            identifier_writes_base: self.identifier_writes_base,
            expansion_specs: Arc::clone(&self.expansion_specs),
            expansion_overrides: self.expansion_overrides.clone(),
            expansion_writes: Arc::clone(&self.expansion_writes),
            expansion_writes_base: self.expansion_writes_base,
            by_key: Arc::clone(&self.by_key),
            hash_fragments: Arc::clone(&self.hash_fragments),
            hash_fragments_by_key: Arc::clone(&self.hash_fragments_by_key),
            font_hash_fragments: Arc::clone(&self.font_hash_fragments),
            complete_hash_fragments: Arc::clone(&self.complete_hash_fragments),
            identities: self.identities.fork(),
        }
    }
}

impl FontStore {
    fn accepted_len(&self) -> usize {
        self.accepted.as_ref().map_or(0, |block| block.total_len)
    }

    fn local_index(&self, id: FontId) -> Option<usize> {
        (id.raw() as usize).checked_sub(self.accepted_len())
    }

    fn lookup_key(&self, key: &FontKey) -> Option<FontId> {
        self.by_key
            .get(key)
            .copied()
            .or_else(|| self.accepted.as_ref()?.by_key(key))
    }

    fn cached_fragment(&self, key: &FontHashFragmentKey) -> Option<StateHashFragment> {
        self.hash_fragments_by_key
            .get(key)
            .and_then(|index| self.hash_fragments.get(*index))
            .copied()
            .or_else(|| self.accepted.as_ref()?.cached_fragment(key))
    }

    fn complete_fragment(&self, id: FontId) -> StateHashFragment {
        if let Some((_, fragment)) = self.identifier_overrides.get(&id) {
            return *fragment;
        }
        if let Some(index) = self.local_index(id) {
            return self.complete_hash_fragments[index];
        }
        self.accepted
            .as_ref()
            .and_then(|block| block.complete_fragment(id.raw() as usize))
            .expect("live accepted font has a complete fragment")
    }

    #[must_use]
    pub(crate) fn new() -> Self {
        let null = LoadedFont::new(
            "nullfont",
            PathBuf::from("nullfont"),
            tex_fonts::font_content_hash(&[]),
            0,
            Scaled::from_raw(0),
            Scaled::from_raw(0),
            vec![Scaled::from_raw(0); 7],
            FontMetrics::default(),
        );
        let hash_fragment_key = FontHashFragmentKey::from(&null);
        let hash_fragment = font_hash_fragment(&null);
        let complete_hash_fragment = complete_font_hash_fragment(hash_fragment, None);
        Self {
            accepted: None,
            fonts: Arc::new(vec![null]),
            non_parameter_font_info_words: 0,
            identifiers: Arc::new(vec![None]),
            identifier_overrides: BTreeMap::new(),
            identifier_writes: Arc::new(Vec::new()),
            identifier_writes_base: 0,
            expansion_specs: Arc::new(vec![None]),
            expansion_overrides: BTreeMap::new(),
            expansion_writes: Arc::new(Vec::new()),
            expansion_writes_base: 0,
            by_key: Arc::new(BTreeMap::new()),
            hash_fragments: Arc::new(vec![hash_fragment]),
            hash_fragments_by_key: Arc::new(BTreeMap::from([(hash_fragment_key, 0)])),
            font_hash_fragments: Arc::new(vec![hash_fragment]),
            complete_hash_fragments: Arc::new(vec![complete_hash_fragment]),
            identities: IdentityAllocator::new(1),
        }
    }

    pub(crate) fn capture_format_fonts(
        &self,
        mut runtime: impl FnMut(
            FontId,
        )
            -> Result<crate::format::schema::FormatFontRuntime, &'static str>,
    ) -> Result<Vec<crate::format::schema::FormatFont>, &'static str> {
        use crate::format::schema::{FormatFont, FormatFontConstruction};

        (0..self.len())
            .map(|raw| {
                let id = self.id_at(raw as u32).expect("format font is live");
                let font = self.get(id);
                if font.opentype().is_some()
                    || font.encoding_map().is_some()
                    || font.layout_policy() != tex_fonts::FontLayoutPolicy::ClassicTfmExact
                    || font.mapping_fallback().is_some()
                {
                    return Err("format contains a non-classic font recipe");
                }
                let construction = match font.construction() {
                    FontConstruction::Loaded => FormatFontConstruction::Loaded,
                    FontConstruction::Copied { source } => FormatFontConstruction::Copied {
                        source: source.bytes(),
                    },
                    FontConstruction::Letterspaced {
                        source,
                        amount,
                        no_ligatures,
                    } => FormatFontConstruction::Letterspaced {
                        source: source.bytes(),
                        amount: *amount,
                        no_ligatures: *no_ligatures,
                    },
                    FontConstruction::Expanded { source, ratio } => {
                        FormatFontConstruction::Expanded {
                            source: source.bytes(),
                            ratio: *ratio,
                        }
                    }
                };
                Ok(FormatFont {
                    name: font.name().to_owned(),
                    content_hash: font.content_hash(),
                    checksum: font.checksum(),
                    design_size: font.design_size().raw(),
                    size: font.size().raw(),
                    parameters: font.parameters().iter().map(|v| v.raw()).collect(),
                    source_parameters: font.source_parameters().iter().map(|v| v.raw()).collect(),
                    font_info_words: font
                        .font_info_words()
                        .try_into()
                        .map_err(|_| "format font-info extent exceeds u32")?,
                    characters: font.metrics().characters().to_vec(),
                    lig_kern_program: font.metrics().lig_kern_program().to_vec(),
                    right_boundary_char: font.metrics().right_boundary_char(),
                    left_boundary_program: font.metrics().left_boundary_program(),
                    extensible_recipes: font.metrics().extensible_recipes().to_vec(),
                    identifier: self.identifier(id).map(SymbolId::raw),
                    expansion: self.expansion(id),
                    construction,
                    runtime: runtime(id)?,
                })
            })
            .collect()
    }

    pub(crate) fn restore_format_fonts(
        rows: Vec<crate::format::schema::FormatFont>,
        interner: &crate::interner::Interner,
    ) -> Result<(Self, Vec<crate::format::schema::FormatFontRuntime>), &'static str> {
        use crate::format::schema::{FormatFont, FormatFontConstruction};

        let row_count = rows.len();
        let mut restored = Vec::with_capacity(row_count);
        let mut runtimes = Vec::with_capacity(row_count);
        for row in rows {
            let FormatFont {
                name,
                content_hash,
                checksum,
                design_size,
                size,
                parameters,
                source_parameters,
                font_info_words,
                characters,
                lig_kern_program,
                right_boundary_char,
                left_boundary_program,
                extensible_recipes,
                identifier,
                expansion,
                construction,
                runtime,
            } = row;
            let construction = match construction {
                FormatFontConstruction::Loaded => FontConstruction::Loaded,
                FormatFontConstruction::Copied { source } => FontConstruction::Copied {
                    source: FontSourceIdentity::from_bytes(source),
                },
                FormatFontConstruction::Letterspaced {
                    source,
                    amount,
                    no_ligatures,
                } => FontConstruction::Letterspaced {
                    source: FontSourceIdentity::from_bytes(source),
                    amount,
                    no_ligatures,
                },
                FormatFontConstruction::Expanded { source, ratio } => FontConstruction::Expanded {
                    source: FontSourceIdentity::from_bytes(source),
                    ratio,
                },
            };
            let path = PathBuf::from(&name);
            let font = LoadedFont::new(
                name,
                path,
                content_hash,
                checksum,
                Scaled::from_raw(design_size),
                Scaled::from_raw(size),
                parameters.into_iter().map(Scaled::from_raw).collect(),
                FontMetrics::new(
                    characters,
                    lig_kern_program,
                    right_boundary_char,
                    left_boundary_program,
                    extensible_recipes,
                ),
            )
            .with_font_info_words(font_info_words as usize)
            .with_source_parameters(
                source_parameters
                    .into_iter()
                    .map(Scaled::from_raw)
                    .collect(),
            )
            .with_construction(construction);
            let identifier = identifier
                .map(|slot| {
                    interner
                        .symbol_at_slot(slot)
                        .ok_or("format font identifier is not live")
                })
                .transpose()?;
            restored.push((font, identifier, expansion));
            runtimes.push(runtime);
        }
        Ok((Self::from_frozen(restored, interner)?, runtimes))
    }

    pub(crate) fn from_frozen(
        rows: Vec<(LoadedFont, Option<SymbolId>, Option<FontExpansion>)>,
        interner: &crate::interner::Interner,
    ) -> Result<Self, &'static str> {
        let count = u32::try_from(rows.len()).map_err(|_| "frozen font count exceeds u32")?;
        if rows.is_empty() || rows.len() > MAX_FONT_COUNT {
            return Err("frozen font count is outside bank capacity");
        }
        let identities = IdentityAllocator::from_frozen_len(1, count);
        let mut fonts = Vec::with_capacity(rows.len());
        let mut identifiers = Vec::with_capacity(rows.len());
        let mut expansion_specs = Vec::with_capacity(rows.len());
        let mut by_key = BTreeMap::new();
        let mut hash_fragments = Vec::new();
        let mut hash_fragments_by_key = BTreeMap::new();
        let mut font_hash_fragments = Vec::with_capacity(rows.len());
        let mut complete_hash_fragments = Vec::with_capacity(rows.len());
        let mut non_parameter_font_info_words = 0_usize;
        for (raw, (font, identifier, expansion)) in rows.into_iter().enumerate() {
            if expansion.is_some()
                && matches!(font.construction(), FontConstruction::Expanded { .. })
            {
                return Err("frozen expanded font has an expansion specification");
            }
            let fragment_key = FontHashFragmentKey::from(&font);
            let fragment = match hash_fragments_by_key.get(&fragment_key).copied() {
                Some(fragment) => fragment,
                None => {
                    let fragment = hash_fragments.len();
                    hash_fragments.push(font_hash_fragment(&font));
                    hash_fragments_by_key.insert(fragment_key, fragment);
                    fragment
                }
            };
            let identifier_text = match identifier {
                Some(symbol) if interner.contains_id(symbol) => Some((
                    interner
                        .kind_id(symbol)
                        .map_err(|_| "frozen font identifier kind is unavailable")?,
                    interner
                        .resolve_id(symbol)
                        .map_err(|_| "frozen font identifier text is unavailable")?,
                )),
                Some(_) => return Err("frozen font identifier is not live"),
                None => None,
            };
            complete_hash_fragments.push(complete_font_hash_fragment(
                hash_fragments[fragment],
                identifier_text,
            ));
            non_parameter_font_info_words = non_parameter_font_info_words.saturating_add(
                font.font_info_words()
                    .saturating_sub(font.parameters().len()),
            );
            if raw != 0 && matches!(font.construction(), FontConstruction::Loaded) {
                let key = FontKey {
                    name: font.name().to_owned(),
                    size: font.size(),
                    content_hash: font.content_hash(),
                };
                let id = FontId::from_identity(
                    identities
                        .identity_at(raw as u32)
                        .expect("frozen font slot is live"),
                );
                if by_key.insert(key, id).is_some() {
                    return Err("duplicate frozen loaded-font key");
                }
            }
            fonts.push(font);
            identifiers.push(identifier);
            expansion_specs.push(expansion);
            font_hash_fragments.push(hash_fragments[fragment]);
        }
        Ok(Self {
            accepted: None,
            fonts: Arc::new(fonts),
            non_parameter_font_info_words,
            identifiers: Arc::new(identifiers),
            identifier_overrides: BTreeMap::new(),
            identifier_writes: Arc::new(Vec::new()),
            identifier_writes_base: 0,
            expansion_specs: Arc::new(expansion_specs),
            expansion_overrides: BTreeMap::new(),
            expansion_writes: Arc::new(Vec::new()),
            expansion_writes_base: 0,
            by_key: Arc::new(by_key),
            hash_fragments: Arc::new(hash_fragments),
            hash_fragments_by_key: Arc::new(hash_fragments_by_key),
            font_hash_fragments: Arc::new(font_hash_fragments),
            complete_hash_fragments: Arc::new(complete_hash_fragments),
            identities,
        })
    }

    pub(crate) fn would_allocate(&self, font: &LoadedFont) -> bool {
        !matches!(font.construction(), FontConstruction::Loaded)
            || self
                .lookup_key(&FontKey {
                    name: font.name().to_owned(),
                    size: font.size(),
                    content_hash: font.content_hash(),
                })
                .is_none()
    }

    pub(crate) fn intern(&mut self, font: LoadedFont) -> Result<FontId, FontStoreCapacityError> {
        let deduplicate = matches!(font.construction(), FontConstruction::Loaded);
        let key = FontKey {
            name: font.name().to_owned(),
            size: font.size(),
            content_hash: font.content_hash(),
        };
        if deduplicate && let Some(id) = self.lookup_key(&key) {
            return Ok(id);
        }
        if self.len() >= MAX_FONT_COUNT {
            return Err(FontStoreCapacityError);
        }
        let hash_fragment_key = FontHashFragmentKey::from(&font);
        let hash_fragment = match self.cached_fragment(&hash_fragment_key) {
            Some(fragment) => fragment,
            None => {
                let fragment = self.hash_fragments.len();
                let value = font_hash_fragment(&font);
                Arc::make_mut(&mut self.hash_fragments).push(value);
                Arc::make_mut(&mut self.hash_fragments_by_key).insert(hash_fragment_key, fragment);
                value
            }
        };
        let id = FontId::from_identity(
            self.identities
                .allocate()
                .expect("font store exceeds u32 ids"),
        );
        self.non_parameter_font_info_words = self.non_parameter_font_info_words.saturating_add(
            font.font_info_words()
                .saturating_sub(font.parameters().len()),
        );
        Arc::make_mut(&mut self.fonts).push(font);
        Arc::make_mut(&mut self.identifiers).push(None);
        Arc::make_mut(&mut self.expansion_specs).push(None);
        Arc::make_mut(&mut self.font_hash_fragments).push(hash_fragment);
        Arc::make_mut(&mut self.complete_hash_fragments)
            .push(complete_font_hash_fragment(hash_fragment, None));
        if deduplicate {
            Arc::make_mut(&mut self.by_key).insert(key, id);
        }
        Ok(id)
    }

    pub(crate) fn set_identifier(
        &mut self,
        id: FontId,
        symbol: SymbolId,
        complete_hash_fragment: StateHashFragment,
    ) -> bool {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        let previous = self.identifier(id);
        if previous != Some(symbol) {
            // TeX82 §1257 assigns `font_id_text(f):=t` at `common_ending`,
            // including when an already-loaded font is found. The identifier
            // is mutable independently of the immutable metric program.
            let previous_fragment = self.complete_fragment(id);
            Arc::make_mut(&mut self.identifier_writes).push((id, previous, previous_fragment));
            if let Some(local) = self.local_index(id) {
                Arc::make_mut(&mut self.identifiers)[local] = Some(symbol);
                Arc::make_mut(&mut self.complete_hash_fragments)[local] = complete_hash_fragment;
            } else {
                self.identifier_overrides
                    .insert(id, (Some(symbol), complete_hash_fragment));
            }
            true
        } else {
            false
        }
    }

    #[must_use]
    pub(crate) fn identifier(&self, id: FontId) -> Option<SymbolId> {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        if let Some((identifier, _)) = self.identifier_overrides.get(&id) {
            return *identifier;
        }
        if let Some(index) = self.local_index(id) {
            return self.identifiers.get(index).copied().flatten();
        }
        self.accepted
            .as_ref()
            .and_then(|block| block.identifier(id.raw() as usize))
            .flatten()
    }

    #[must_use]
    pub(crate) fn get(&self, id: FontId) -> &LoadedFont {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        if let Some(index) = self.local_index(id) {
            return self
                .fonts
                .get(index)
                .expect("font id is not live in this Universe timeline");
        }
        self.accepted
            .as_ref()
            .and_then(|block| block.font(id.raw() as usize))
            .expect("font id is not live in this Universe timeline")
    }

    pub(crate) fn artifact_recipe(&self, id: FontId) -> FontArtifactRecipe {
        let font = self.get(id);
        let construction = match font.construction() {
            FontConstruction::Loaded => FontArtifactConstructionRecipe::Loaded,
            FontConstruction::Copied { source } => FontArtifactConstructionRecipe::Copied {
                source_identity: *source,
            },
            FontConstruction::Letterspaced {
                source,
                amount,
                no_ligatures,
            } => FontArtifactConstructionRecipe::Letterspaced {
                source_identity: *source,
                amount: *amount,
                no_ligatures: *no_ligatures,
            },
            FontConstruction::Expanded { source, ratio } => {
                FontArtifactConstructionRecipe::Expanded {
                    source_identity: *source,
                    ratio: *ratio,
                }
            }
        };
        let opentype = font.opentype().map(|opentype| OpenTypeArtifactRecipe {
            program_identity: opentype.identity,
            object_identity: opentype.object_identity,
            instance_identity: font
                .opentype_instance_identity()
                .expect("OpenType font has an instance identity"),
            container: opentype.container,
            face_index: opentype.face_index,
            variation: opentype.variation.clone(),
            features: opentype.feature_policy.clone(),
            direction: opentype.direction,
            script: opentype.script,
            language: opentype.language.clone(),
            encoding_map_version: font.encoding_map().map(|map| map.version()),
            encoding_map_identity: font.encoding_map().map(|map| map.identity()),
            fontdimen_synthesis_version: font
                .encoding_map()
                .map(|_| tex_fonts::OPENTYPE_FONTDIMEN_SYNTHESIS_VERSION),
        });
        FontArtifactRecipe {
            name: font.name().to_owned(),
            tfm_content_hash: font.content_hash(),
            tfm_checksum: font.checksum(),
            design_size: font.design_size(),
            at_size: font.size(),
            layout_policy: font.layout_policy(),
            mapping_fallback: font.mapping_fallback(),
            opentype,
            semantic_identity: font.source_identity(),
            construction,
        }
    }

    pub(crate) fn expansion(&self, id: FontId) -> Option<FontExpansion> {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        if let Some(expansion) = self.expansion_overrides.get(&id) {
            return *expansion;
        }
        if let Some(index) = self.local_index(id) {
            return self.expansion_specs[index];
        }
        self.accepted
            .as_ref()
            .and_then(|block| block.expansion(id.raw() as usize))
            .flatten()
    }

    pub(crate) fn set_expansion(
        &mut self,
        id: FontId,
        expansion: FontExpansion,
    ) -> Result<bool, FontExpansionConfigError> {
        if matches!(
            self.get(id).construction(),
            FontConstruction::Expanded { .. }
        ) {
            return Err(FontExpansionConfigError::ExpandedBase);
        }
        if let Some(existing) = self.expansion(id) {
            if existing.step != expansion.step {
                return Err(FontExpansionConfigError::DifferentStep);
            }
            if existing.stretch != expansion.stretch {
                return Err(FontExpansionConfigError::DifferentStretch);
            }
            if existing.shrink != expansion.shrink {
                return Err(FontExpansionConfigError::DifferentShrink);
            }
            if existing.auto_expand != expansion.auto_expand {
                return Err(FontExpansionConfigError::DifferentAutoExpand);
            }
            return Ok(false);
        }
        let previous = self.expansion(id);
        Arc::make_mut(&mut self.expansion_writes).push((id, previous));
        if let Some(index) = self.local_index(id) {
            Arc::make_mut(&mut self.expansion_specs)[index] = Some(expansion);
        } else {
            self.expansion_overrides.insert(id, Some(expansion));
        }
        Ok(true)
    }

    #[must_use]
    pub(crate) fn by_source_identity(&self, identity: FontSourceIdentity) -> Option<FontId> {
        (0..self.len()).find_map(|raw| {
            let id = self
                .id_at(raw as u32)
                .expect("live font slot has an identity");
            (self.get(id).source_identity() == identity).then_some(id)
        })
    }

    pub(crate) fn hash_fragment(&self, id: FontId) -> &StateHashFragment {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        if let Some(index) = self.local_index(id) {
            return &self.font_hash_fragments[index];
        }
        self.accepted
            .as_ref()
            .and_then(|block| block.immutable_fragment(id.raw() as usize))
            .expect("live accepted font has an immutable fragment")
    }

    #[must_use]
    pub(crate) fn contains(&self, id: FontId) -> bool {
        self.identities.contains(id.identity())
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.accepted_len().saturating_add(self.fonts.len())
    }

    /// Returns TeX82 §549's first-unused `font_info` coordinate.
    ///
    /// Sections 552 and 565 allocate immutable table words and the initial
    /// parameter rows together. Section 580 can then extend only the mutable
    /// parameter suffix of the newest font. The two owners stay separate in
    /// Umber, so this aggregate joins them without recreating WEB storage.
    #[must_use]
    pub(crate) fn font_info_words(&self, parameter_words: usize) -> usize {
        self.non_parameter_font_info_words
            .saturating_add(parameter_words)
    }

    /// Resolves a validated dense format coordinate to this timeline's fresh
    /// live font identity.
    pub(crate) fn id_at(&self, slot: u32) -> Option<FontId> {
        self.identities.identity_at(slot).map(FontId::from_identity)
    }

    #[must_use]
    pub(crate) fn watermark(&self) -> FontStoreMark {
        FontStoreMark {
            len: u32::try_from(self.len()).expect("font store exceeds u32 ids"),
            identifier_writes_len: u32::try_from(
                self.identifier_writes_base + self.identifier_writes.len(),
            )
            .expect("font identifier write log exceeds u32 entries"),
            expansion_writes_len: u32::try_from(
                self.expansion_writes_base + self.expansion_writes.len(),
            )
            .expect("font expansion write log exceeds u32 entries"),
            non_parameter_font_info_words: self.non_parameter_font_info_words,
            identities: self.identities.watermark(),
        }
    }

    pub(crate) fn validates(&self, mark: FontStoreMark) -> bool {
        mark.len as usize >= self.accepted_len()
            && mark.len as usize <= self.len()
            && mark.identifier_writes_len as usize >= self.identifier_writes_base
            && mark.identifier_writes_len as usize
                <= self.identifier_writes_base + self.identifier_writes.len()
            && mark.expansion_writes_len as usize >= self.expansion_writes_base
            && mark.expansion_writes_len as usize
                <= self.expansion_writes_base + self.expansion_writes.len()
            && self.identities.validate_rollback(mark.identities).is_ok()
    }

    /// Returns whether an exact font identity survives rollback to `mark`.
    #[must_use]
    pub(crate) fn contains_at(&self, mark: FontStoreMark, id: FontId) -> bool {
        id.raw() < mark.len && self.contains(id)
    }

    pub(crate) fn truncate_to(&mut self, mark: FontStoreMark) {
        let accepted_len = self.accepted_len();
        self.identities
            .rollback(mark.identities)
            .expect("font-store mark is not an ancestor");
        let identifier_mark = mark.identifier_writes_len as usize - self.identifier_writes_base;
        for (id, identifier, fragment) in self.identifier_writes[identifier_mark..]
            .iter()
            .rev()
            .copied()
        {
            if id.raw() < mark.len {
                if let Some(index) = self.local_index(id) {
                    Arc::make_mut(&mut self.identifiers)[index] = identifier;
                    Arc::make_mut(&mut self.complete_hash_fragments)[index] = fragment;
                } else {
                    let accepted_identifier = self
                        .accepted
                        .as_ref()
                        .and_then(|block| block.identifier(id.raw() as usize))
                        .flatten();
                    if identifier == accepted_identifier {
                        self.identifier_overrides.remove(&id);
                    } else {
                        self.identifier_overrides.insert(id, (identifier, fragment));
                    }
                }
            }
        }
        Arc::make_mut(&mut self.identifier_writes).truncate(identifier_mark);
        let expansion_mark = mark.expansion_writes_len as usize - self.expansion_writes_base;
        for (id, previous) in self.expansion_writes[expansion_mark..]
            .iter()
            .rev()
            .copied()
        {
            if id.raw() < mark.len {
                if let Some(index) = self.local_index(id) {
                    Arc::make_mut(&mut self.expansion_specs)[index] = previous;
                } else {
                    let accepted_expansion = self
                        .accepted
                        .as_ref()
                        .and_then(|block| block.expansion(id.raw() as usize))
                        .flatten();
                    if previous == accepted_expansion {
                        self.expansion_overrides.remove(&id);
                    } else {
                        self.expansion_overrides.insert(id, previous);
                    }
                }
            }
        }
        Arc::make_mut(&mut self.expansion_writes).truncate(expansion_mark);
        self.non_parameter_font_info_words = mark.non_parameter_font_info_words;
        let local_len = mark.len as usize - accepted_len;
        Arc::make_mut(&mut self.fonts).truncate(local_len);
        Arc::make_mut(&mut self.identifiers).truncate(local_len);
        Arc::make_mut(&mut self.expansion_specs).truncate(local_len);
        Arc::make_mut(&mut self.font_hash_fragments).truncate(local_len);
        Arc::make_mut(&mut self.complete_hash_fragments).truncate(local_len);
        Arc::make_mut(&mut self.by_key).retain(|_, id| id.raw() < mark.len);
    }

    pub(crate) fn fork_at(&self, mark: FontStoreMark) -> Self {
        assert!(self.validates(mark));
        let parent_len = self.accepted_len();
        let len = mark.len as usize - parent_len;
        let identifier_writes_len =
            mark.identifier_writes_len as usize - self.identifier_writes_base;
        let expansion_writes_len = mark.expansion_writes_len as usize - self.expansion_writes_base;
        let accepted = if len == 0 {
            self.accepted.clone()
        } else {
            Some(Arc::new(AcceptedFontBlock {
                parent: self.accepted.clone(),
                fonts: Arc::clone(&self.fonts),
                identifiers: Arc::clone(&self.identifiers),
                identifier_writes: Arc::clone(&self.identifier_writes),
                identifier_writes_len,
                expansion_specs: Arc::clone(&self.expansion_specs),
                expansion_writes: Arc::clone(&self.expansion_writes),
                expansion_writes_len,
                by_key: Arc::clone(&self.by_key),
                hash_fragments: Arc::clone(&self.hash_fragments),
                hash_fragments_by_key: Arc::clone(&self.hash_fragments_by_key),
                font_hash_fragments: Arc::clone(&self.font_hash_fragments),
                complete_hash_fragments: Arc::clone(&self.complete_hash_fragments),
                len,
                total_len: mark.len as usize,
            }))
        };
        let identities = self
            .identities
            .fork_at(mark.identities)
            .expect("font-store fork mark is an ancestor");
        Self {
            accepted,
            fonts: Arc::new(Vec::new()),
            non_parameter_font_info_words: mark.non_parameter_font_info_words,
            identifiers: Arc::new(Vec::new()),
            identifier_overrides: BTreeMap::new(),
            identifier_writes: Arc::new(Vec::new()),
            identifier_writes_base: mark.identifier_writes_len as usize,
            expansion_specs: Arc::new(Vec::new()),
            expansion_overrides: BTreeMap::new(),
            expansion_writes: Arc::new(Vec::new()),
            expansion_writes_base: mark.expansion_writes_len as usize,
            by_key: Arc::new(BTreeMap::new()),
            hash_fragments: Arc::new(Vec::new()),
            hash_fragments_by_key: Arc::new(BTreeMap::new()),
            font_hash_fragments: Arc::new(Vec::new()),
            complete_hash_fragments: Arc::new(Vec::new()),
            identities,
        }
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn retained_payload_bytes(&self) -> usize {
        (0..self.len())
            .filter_map(|raw| self.id_at(raw as u32))
            .map(|id| {
                let font = self.get(id);
                std::mem::size_of::<LoadedFont>()
                    .saturating_add(font.name().len())
                    .saturating_add(
                        font.font_info_words()
                            .saturating_mul(std::mem::size_of::<Scaled>()),
                    )
            })
            .sum()
    }

    #[cfg(test)]
    pub(crate) fn testing_hash_fragment_counts(&self) -> (usize, usize, usize) {
        (self.hash_fragments.len(), self.len(), self.len())
    }
}

impl From<&LoadedFont> for FontHashFragmentKey {
    fn from(font: &LoadedFont) -> Self {
        Self {
            name: font.name().to_owned(),
            content_hash: font.content_hash(),
            checksum: font.checksum(),
            design_size: font.design_size(),
            size: font.size(),
            construction: font.construction().clone(),
        }
    }
}

fn font_hash_fragment(font: &LoadedFont) -> StateHashFragment {
    StateHashFragment::from_builder(IMMUTABLE_FONT_HASH_DOMAIN, |fragment| {
        fragment.str(font.name());
        fragment.bytes(&font.content_hash());
        fragment.u32(font.checksum());
        fragment.i32(font.design_size().raw());
        fragment.i32(font.size().raw());
        match font.construction() {
            FontConstruction::Loaded => fragment.u8(0),
            FontConstruction::Copied { source } => {
                fragment.u8(1);
                fragment.bytes(&source.bytes());
            }
            FontConstruction::Letterspaced {
                source,
                amount,
                no_ligatures,
            } => {
                fragment.u8(2);
                fragment.bytes(&source.bytes());
                fragment.i32(i32::from(*amount));
                fragment.bool(*no_ligatures);
            }
            FontConstruction::Expanded { source, ratio } => {
                fragment.u8(3);
                fragment.bytes(&source.bytes());
                fragment.i32(i32::from(*ratio));
            }
        }
    })
}

pub(crate) fn complete_font_hash_fragment(
    immutable: StateHashFragment,
    identifier: Option<(ControlSequenceKind, &str)>,
) -> StateHashFragment {
    StateHashFragment::from_builder(COMPLETE_FONT_HASH_DOMAIN, |fragment| {
        immutable.apply(fragment);
        match identifier {
            Some((kind, name)) => {
                fragment.bool(true);
                fragment.u8(match kind {
                    ControlSequenceKind::Null
                    | ControlSequenceKind::SingleCharacter
                    | ControlSequenceKind::Named => 0,
                    ControlSequenceKind::ActiveCharacter => 1,
                    ControlSequenceKind::Internal => 2,
                });
                fragment.str(name);
            }
            None => fragment.bool(false),
        }
    })
}

impl Default for FontStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_hash::StateHasher;

    #[test]
    fn expansion_configuration_is_idempotent_and_rollback_owned() {
        let mut fonts = FontStore::new();
        let mark = fonts.watermark();
        let expansion = FontExpansion {
            stretch: 20,
            shrink: 10,
            step: 5,
            auto_expand: true,
        };
        fonts
            .set_expansion(NULL_FONT, expansion)
            .expect("first expansion config is accepted");
        fonts
            .set_expansion(NULL_FONT, expansion)
            .expect("identical expansion config is idempotent");
        assert_eq!(fonts.expansion(NULL_FONT), Some(expansion));
        assert_eq!(
            fonts.set_expansion(
                NULL_FONT,
                FontExpansion {
                    step: 10,
                    ..expansion
                },
            ),
            Err(FontExpansionConfigError::DifferentStep)
        );

        fonts.truncate_to(mark);
        assert_eq!(fonts.expansion(NULL_FONT), None);
    }

    const TEST_DOMAIN: u64 = 0x666f_6e74_5f74_6573;

    #[test]
    fn cached_fragment_matches_canonical_immutable_font_fingerprint() {
        let font = test_font();
        let direct = font_hash_fragment(&font);

        let mut store = FontStore::new();
        let id = store.intern(font).expect("test font fits");
        let mut cached = StateHasher::new(TEST_DOMAIN);
        store.hash_fragment(id).apply(&mut cached);
        let mut expected = StateHasher::new(TEST_DOMAIN);
        direct.apply(&mut expected);

        assert_eq!(cached.finish(), expected.finish());
    }

    #[test]
    fn immutable_fragments_survive_rollback_and_are_reused() {
        let mut store = FontStore::new();
        let mark = store.watermark();
        let font = test_font();
        let first = store.intern(font.clone()).expect("test font fits");
        assert_eq!(store.testing_hash_fragment_counts(), (2, 2, 2));
        let first_fragment = {
            let mut hasher = StateHasher::new(TEST_DOMAIN);
            store.hash_fragment(first).apply(&mut hasher);
            hasher.finish()
        };

        store.truncate_to(mark);
        assert_eq!(store.testing_hash_fragment_counts(), (2, 1, 1));

        let replacement = store.intern(font).expect("test font fits");
        assert_eq!(replacement.raw(), first.raw());
        assert_ne!(replacement, first);
        assert_eq!(store.testing_hash_fragment_counts(), (2, 2, 2));
        let mut hasher = StateHasher::new(TEST_DOMAIN);
        store.hash_fragment(replacement).apply(&mut hasher);
        assert_eq!(hasher.finish(), first_fragment);

        let clone = store.clone();
        assert_eq!(clone.testing_hash_fragment_counts(), (2, 2, 2));
    }

    #[test]
    fn frozen_loaded_fonts_keep_slot_order_and_deduplicate_in_place() {
        let mut source = FontStore::new();
        let first_font = test_font();
        let second_font = LoadedFont::new(
            "second",
            PathBuf::from("second"),
            tex_fonts::font_content_hash(b"second"),
            0x8765_4321,
            first_font.design_size(),
            first_font.size(),
            first_font.parameters().to_vec(),
            first_font.metrics().clone(),
        );
        let first = source.intern(first_font.clone()).expect("first font");
        let second = source.intern(second_font.clone()).expect("second font");
        let rows = vec![
            (source.get(NULL_FONT).clone(), None, None),
            (source.get(first).clone(), None, None),
            (source.get(second).clone(), None, None),
        ];
        let interner = crate::interner::Interner::new(
            crate::interner::InternerBudget::new(16, 16, 256).expect("budget"),
        );
        let mut loaded = FontStore::from_frozen(rows, &interner).expect("loaded font prefix");

        assert_eq!(loaded.intern(first_font).expect("reused first").raw(), 1);
        assert_eq!(loaded.intern(second_font).expect("reused second").raw(), 2);
        let mark = loaded.watermark();
        let third = LoadedFont::new(
            "third",
            PathBuf::from("third"),
            tex_fonts::font_content_hash(b"third"),
            0x1020_3040,
            Scaled::from_raw(10 * Scaled::UNITY),
            Scaled::from_raw(12 * Scaled::UNITY),
            vec![Scaled::from_raw(0); 7],
            FontMetrics::default(),
        );
        assert_eq!(loaded.intern(third.clone()).expect("third font").raw(), 3);
        loaded.truncate_to(mark);
        assert_eq!(
            loaded
                .intern(third)
                .expect("third font after rollback")
                .raw(),
            3
        );
    }

    #[test]
    fn checkpoint_fork_shares_loaded_prefix_and_isolates_font_suffix_and_overrides() {
        let mut parent = FontStore::new();
        let inherited_font = test_font();
        let inherited = parent.intern(inherited_font).expect("inherited font");
        let mark = parent.watermark();
        let parent_only_font = LoadedFont::new(
            "parent-only",
            "/fonts/parent-only.tfm",
            tex_fonts::font_content_hash(b"parent-only metrics"),
            0x1020_3040,
            Scaled::from_raw(10 * Scaled::UNITY),
            Scaled::from_raw(10 * Scaled::UNITY),
            vec![Scaled::from_raw(0); 7],
            FontMetrics::default(),
        );
        let parent_only = parent
            .intern(parent_only_font.clone())
            .expect("parent-only font");

        let mut child = parent.fork_at(mark);
        assert_eq!(child.get(inherited).name(), "cmr10");
        assert!(!child.contains(parent_only));
        let child_only = child
            .intern(parent_only_font)
            .expect("candidate-private font");
        assert_eq!(child_only.raw(), parent_only.raw());
        assert_ne!(child_only, parent_only);
        assert!(parent.contains(parent_only));
        assert!(!parent.contains(child_only));

        let before_override = child.watermark();
        let expansion = FontExpansion {
            stretch: 20,
            shrink: 10,
            step: 5,
            auto_expand: true,
        };
        child
            .set_expansion(inherited, expansion)
            .expect("accepted font can receive a candidate-local override");
        assert_eq!(child.expansion(inherited), Some(expansion));
        assert_eq!(parent.expansion(inherited), None);
        child.truncate_to(before_override);
        assert_eq!(child.expansion(inherited), None);
    }

    fn test_font() -> LoadedFont {
        LoadedFont::new(
            "cmr10",
            "/fonts/cmr10.tfm",
            tex_fonts::font_content_hash(b"cmr10 metrics"),
            0x1234_5678,
            Scaled::from_raw(10 * Scaled::UNITY),
            Scaled::from_raw(12 * Scaled::UNITY),
            vec![Scaled::from_raw(0); 7],
            FontMetrics::default(),
        )
    }
}
