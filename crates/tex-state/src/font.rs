//! Stateful font handles and rollback storage.

use crate::identity::{IdentityAllocator, IdentityMark};
use crate::ids::FontId;
use crate::interner::{ControlSequenceKind, SymbolId};
use crate::scaled::Scaled;
use crate::state_hash::StateHashFragment;
use crate::world::ContentHash;
use std::collections::BTreeMap;
use std::path::PathBuf;
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
    pub encoding_map_identity: Option<[u8; 32]>,
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
    identities: IdentityMark,
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

/// Immutable font store with dense ids and hash-consed load identity.
#[derive(Debug)]
pub(crate) struct FontStore {
    fonts: Vec<LoadedFont>,
    identifiers: Vec<Option<SymbolId>>,
    identifier_writes: Vec<(FontId, Option<SymbolId>, StateHashFragment)>,
    expansion_specs: Vec<Option<FontExpansion>>,
    expansion_writes: Vec<(FontId, Option<FontExpansion>)>,
    by_key: BTreeMap<FontKey, FontId>,
    /// Append-only derived fragments keyed by semantic content. Rollback only
    /// truncates the live slot-to-fragment mapping, so a later equivalent load
    /// can reuse its domain-separated fingerprint.
    hash_fragments: Vec<StateHashFragment>,
    hash_fragments_by_key: BTreeMap<FontHashFragmentKey, usize>,
    font_hash_fragments: Vec<usize>,
    complete_hash_fragments: Vec<StateHashFragment>,
    identities: IdentityAllocator,
}

impl Clone for FontStore {
    fn clone(&self) -> Self {
        Self {
            fonts: self.fonts.clone(),
            identifiers: self.identifiers.clone(),
            identifier_writes: self.identifier_writes.clone(),
            expansion_specs: self.expansion_specs.clone(),
            expansion_writes: self.expansion_writes.clone(),
            by_key: self.by_key.clone(),
            hash_fragments: self.hash_fragments.clone(),
            hash_fragments_by_key: self.hash_fragments_by_key.clone(),
            font_hash_fragments: self.font_hash_fragments.clone(),
            complete_hash_fragments: self.complete_hash_fragments.clone(),
            identities: self.identities.fork(),
        }
    }
}

impl FontStore {
    #[must_use]
    pub(crate) fn new() -> Self {
        let null = LoadedFont::new(
            "nullfont",
            PathBuf::from("nullfont"),
            ContentHash::from_bytes(&[]).bytes(),
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
            fonts: vec![null],
            identifiers: vec![None],
            identifier_writes: Vec::new(),
            expansion_specs: vec![None],
            expansion_writes: Vec::new(),
            by_key: BTreeMap::new(),
            hash_fragments: vec![hash_fragment],
            hash_fragments_by_key: BTreeMap::from([(hash_fragment_key, 0)]),
            font_hash_fragments: vec![0],
            complete_hash_fragments: vec![complete_hash_fragment],
            identities: IdentityAllocator::new(1),
        }
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
            font_hash_fragments.push(fragment);
        }
        Ok(Self {
            fonts,
            identifiers,
            identifier_writes: Vec::new(),
            expansion_specs,
            expansion_writes: Vec::new(),
            by_key,
            hash_fragments,
            hash_fragments_by_key,
            font_hash_fragments,
            complete_hash_fragments,
            identities,
        })
    }

    pub(crate) fn would_allocate(&self, font: &LoadedFont) -> bool {
        !matches!(font.construction(), FontConstruction::Loaded)
            || !self.by_key.contains_key(&FontKey {
                name: font.name().to_owned(),
                size: font.size(),
                content_hash: font.content_hash(),
            })
    }

    pub(crate) fn intern(&mut self, font: LoadedFont) -> Result<FontId, FontStoreCapacityError> {
        let deduplicate = matches!(font.construction(), FontConstruction::Loaded);
        let key = FontKey {
            name: font.name().to_owned(),
            size: font.size(),
            content_hash: font.content_hash(),
        };
        if deduplicate && let Some(id) = self.by_key.get(&key).copied() {
            return Ok(id);
        }
        if self.fonts.len() >= MAX_FONT_COUNT {
            return Err(FontStoreCapacityError);
        }
        let hash_fragment_key = FontHashFragmentKey::from(&font);
        let hash_fragment = match self.hash_fragments_by_key.get(&hash_fragment_key) {
            Some(&fragment) => fragment,
            None => {
                let fragment = self.hash_fragments.len();
                self.hash_fragments.push(font_hash_fragment(&font));
                self.hash_fragments_by_key
                    .insert(hash_fragment_key, fragment);
                fragment
            }
        };
        let id = FontId::from_identity(
            self.identities
                .allocate()
                .expect("font store exceeds u32 ids"),
        );
        self.fonts.push(font);
        self.identifiers.push(None);
        self.expansion_specs.push(None);
        self.font_hash_fragments.push(hash_fragment);
        self.complete_hash_fragments
            .push(complete_font_hash_fragment(
                self.hash_fragments[hash_fragment],
                None,
            ));
        if deduplicate {
            self.by_key.insert(key, id);
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
        let index = id.raw() as usize;
        let identifier = self
            .identifiers
            .get_mut(index)
            .expect("font id is not live in this Universe timeline");
        if *identifier != Some(symbol) {
            // TeX82 §1257 assigns `font_id_text(f):=t` at `common_ending`,
            // including when an already-loaded font is found. The identifier
            // is mutable independently of the immutable metric program.
            self.identifier_writes
                .push((id, *identifier, self.complete_hash_fragments[index]));
            *identifier = Some(symbol);
            self.complete_hash_fragments[index] = complete_hash_fragment;
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
        self.identifiers.get(id.raw() as usize).copied().flatten()
    }

    #[must_use]
    pub(crate) fn get(&self, id: FontId) -> &LoadedFont {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        self.fonts
            .get(id.raw() as usize)
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

    pub(crate) fn iter(&self) -> impl Iterator<Item = &LoadedFont> {
        self.fonts.iter()
    }

    pub(crate) fn expansion(&self, id: FontId) -> Option<FontExpansion> {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        self.expansion_specs[id.raw() as usize]
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
        self.expansion_writes.push((id, None));
        self.expansion_specs[id.raw() as usize] = Some(expansion);
        Ok(true)
    }

    #[must_use]
    pub(crate) fn by_source_identity(&self, identity: FontSourceIdentity) -> Option<FontId> {
        self.fonts.iter().enumerate().find_map(|(raw, font)| {
            (font.source_identity() == identity).then(|| {
                FontId::from_identity(
                    self.identities
                        .identity_at(raw as u32)
                        .expect("live font slot has an identity"),
                )
            })
        })
    }

    pub(crate) fn hash_fragment(&self, id: FontId) -> &StateHashFragment {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        let fragment = self.font_hash_fragments[id.raw() as usize];
        &self.hash_fragments[fragment]
    }

    pub(crate) fn complete_hash_fragment(&self, id: FontId) -> &StateHashFragment {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        &self.complete_hash_fragments[id.raw() as usize]
    }

    #[must_use]
    pub(crate) fn contains(&self, id: FontId) -> bool {
        self.identities.contains(id.identity())
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.fonts.len()
    }

    #[must_use]
    pub(crate) fn watermark(&self) -> FontStoreMark {
        FontStoreMark {
            len: u32::try_from(self.fonts.len()).expect("font store exceeds u32 ids"),
            identifier_writes_len: u32::try_from(self.identifier_writes.len())
                .expect("font identifier write log exceeds u32 entries"),
            expansion_writes_len: u32::try_from(self.expansion_writes.len())
                .expect("font expansion write log exceeds u32 entries"),
            identities: self.identities.watermark(),
        }
    }

    pub(crate) fn validates(&self, mark: FontStoreMark) -> bool {
        mark.len as usize <= self.fonts.len()
            && mark.len as usize <= self.identifiers.len()
            && mark.len as usize <= self.expansion_specs.len()
            && mark.identifier_writes_len as usize <= self.identifier_writes.len()
            && mark.expansion_writes_len as usize <= self.expansion_writes.len()
            && self.identities.validate_rollback(mark.identities).is_ok()
    }

    /// Returns whether an exact font identity survives rollback to `mark`.
    #[must_use]
    pub(crate) fn contains_at(&self, mark: FontStoreMark, id: FontId) -> bool {
        id.raw() < mark.len && self.contains(id)
    }

    pub(crate) fn truncate_to(&mut self, mark: FontStoreMark) {
        self.identities
            .rollback(mark.identities)
            .expect("font-store mark is not an ancestor");
        for (id, identifier, fragment) in self.identifier_writes
            [mark.identifier_writes_len as usize..]
            .iter()
            .rev()
            .copied()
        {
            if id.raw() < mark.len {
                self.identifiers[id.raw() as usize] = identifier;
                self.complete_hash_fragments[id.raw() as usize] = fragment;
            }
        }
        self.identifier_writes
            .truncate(mark.identifier_writes_len as usize);
        for (id, previous) in self.expansion_writes[mark.expansion_writes_len as usize..]
            .iter()
            .rev()
            .copied()
        {
            if id.raw() < mark.len {
                self.expansion_specs[id.raw() as usize] = previous;
            }
        }
        self.expansion_writes
            .truncate(mark.expansion_writes_len as usize);
        self.fonts.truncate(mark.len as usize);
        self.identifiers.truncate(mark.len as usize);
        self.expansion_specs.truncate(mark.len as usize);
        self.font_hash_fragments.truncate(mark.len as usize);
        self.complete_hash_fragments.truncate(mark.len as usize);
        self.by_key.retain(|_, id| id.raw() < mark.len);
    }

    #[cfg(test)]
    pub(crate) fn testing_hash_fragment_counts(&self) -> (usize, usize, usize) {
        (
            self.hash_fragments.len(),
            self.font_hash_fragments.len(),
            self.complete_hash_fragments.len(),
        )
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
            ContentHash::from_bytes(b"second").bytes(),
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
            ContentHash::from_bytes(b"third").bytes(),
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

    fn test_font() -> LoadedFont {
        LoadedFont::new(
            "cmr10",
            "/fonts/cmr10.tfm",
            ContentHash::from_bytes(b"cmr10 metrics").bytes(),
            0x1234_5678,
            Scaled::from_raw(10 * Scaled::UNITY),
            Scaled::from_raw(12 * Scaled::UNITY),
            vec![Scaled::from_raw(0); 7],
            FontMetrics::default(),
        )
    }
}
