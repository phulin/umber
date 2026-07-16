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

/// Largest dense font id representable in a fontdimen cell key.
pub const MAX_FONT_DIMEN_FONT_ID: u32 = (1 << 15) - 1;

/// Maximum number of loaded fonts, including `nullfont`.
pub(crate) const MAX_FONT_COUNT: usize = 1 << 15;
const IMMUTABLE_FONT_HASH_DOMAIN: u64 = 0x666f_6e74_5f69_6d6d;
const COMPLETE_FONT_HASH_DOMAIN: u64 = 0x666f_6e74_5f63_6d70;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FontStoreCapacityError;

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
    identifier_writes_len: u32,
    semantic_seal_writes_len: u32,
    expansion_writes_len: u32,
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
    identifier_writes: Vec<FontId>,
    semantic_sealed: Vec<bool>,
    semantic_seal_writes: Vec<FontId>,
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
            semantic_sealed: self.semantic_sealed.clone(),
            semantic_seal_writes: self.semantic_seal_writes.clone(),
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
            semantic_sealed: vec![false],
            semantic_seal_writes: Vec::new(),
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
        self.semantic_sealed.push(false);
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
    ) {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        let identifier = self
            .identifiers
            .get_mut(id.raw() as usize)
            .expect("font id is not live in this Universe timeline");
        if identifier.is_none() {
            assert!(
                !self.semantic_sealed[id.raw() as usize],
                "font identifier must be assigned before the font enters a frozen node list"
            );
            *identifier = Some(symbol);
            self.complete_hash_fragments[id.raw() as usize] = complete_hash_fragment;
            self.identifier_writes.push(id);
        }
    }

    /// Prevents the rollback-coupled identifier from changing after its
    /// complete semantics have been captured by a frozen node-list identity.
    pub(crate) fn seal_semantic_identity(&mut self, id: FontId) {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        let sealed = &mut self.semantic_sealed[id.raw() as usize];
        if !*sealed {
            *sealed = true;
            self.semantic_seal_writes.push(id);
        }
    }

    pub(crate) fn truncate_semantic_seals_to(&mut self, mark: usize) {
        assert!(
            mark <= self.semantic_seal_writes.len(),
            "font semantic-seal mark is not an ancestor"
        );
        for id in self.semantic_seal_writes[mark..].iter().copied() {
            if self.contains(id) {
                self.semantic_sealed[id.raw() as usize] = false;
            }
        }
        self.semantic_seal_writes.truncate(mark);
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
    ) -> Result<(), FontExpansionConfigError> {
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
            return Ok(());
        }
        self.expansion_writes.push((id, None));
        self.expansion_specs[id.raw() as usize] = Some(expansion);
        Ok(())
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

    /// Resolves a live or stored font handle and returns its cached complete
    /// semantic fragment with a single identity lookup.
    pub(crate) fn resolve_complete_hash_fragment(&self, id: FontId) -> Option<&StateHashFragment> {
        let id = self.resolve_stored(id)?;
        self.complete_hash_fragments.get(id.raw() as usize)
    }

    #[must_use]
    pub(crate) fn contains(&self, id: FontId) -> bool {
        self.identities.contains(id.identity())
    }

    #[must_use]
    pub(crate) fn resolve_stored(&self, id: FontId) -> Option<FontId> {
        if self.contains(id) {
            return Some(id);
        }
        if !id.is_stored() {
            return None;
        }
        self.identities
            .identity_at(id.raw())
            .map(FontId::from_identity)
    }

    #[must_use]
    pub(crate) fn watermark(&self) -> FontStoreMark {
        FontStoreMark {
            len: u32::try_from(self.fonts.len()).expect("font store exceeds u32 ids"),
            identifier_writes_len: u32::try_from(self.identifier_writes.len())
                .expect("font identifier write log exceeds u32 entries"),
            semantic_seal_writes_len: u32::try_from(self.semantic_seal_writes.len())
                .expect("font semantic-seal write log exceeds u32 entries"),
            expansion_writes_len: u32::try_from(self.expansion_writes.len())
                .expect("font expansion write log exceeds u32 entries"),
            identities: self.identities.watermark(),
        }
    }

    pub(crate) fn truncate_to(&mut self, mark: FontStoreMark) {
        self.identities
            .rollback(mark.identities)
            .expect("font-store mark is not an ancestor");
        for id in self.identifier_writes[mark.identifier_writes_len as usize..]
            .iter()
            .copied()
        {
            if id.raw() < mark.len {
                self.identifiers[id.raw() as usize] = None;
                let immutable = self.hash_fragments[self.font_hash_fragments[id.raw() as usize]];
                self.complete_hash_fragments[id.raw() as usize] =
                    complete_font_hash_fragment(immutable, None);
            }
        }
        self.identifier_writes
            .truncate(mark.identifier_writes_len as usize);
        self.truncate_semantic_seals_to(mark.semantic_seal_writes_len as usize);
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
        self.semantic_sealed.truncate(mark.len as usize);
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

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn testing_state_hash(&self, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash as _;

        for ((font, identifier), expansion) in self
            .fonts
            .iter()
            .zip(&self.identifiers)
            .zip(&self.expansion_specs)
        {
            font.name().hash(hasher);
            font.content_hash().hash(hasher);
            font.checksum().hash(hasher);
            font.design_size().raw().hash(hasher);
            font.size().raw().hash(hasher);
            font.construction().hash(hasher);
            for parameter in font.parameters() {
                parameter.raw().hash(hasher);
            }
            for parameter in font.source_parameters() {
                parameter.raw().hash(hasher);
            }
            font.metrics_source().hash(hasher);
            identifier.hash(hasher);
            expansion.hash(hasher);
        }
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
                    ControlSequenceKind::Named => 0,
                    ControlSequenceKind::ActiveCharacter => 1,
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
                }
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
