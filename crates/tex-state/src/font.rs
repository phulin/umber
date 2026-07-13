//! Stateful font handles and rollback storage.

use crate::identity::{IdentityAllocator, IdentityMark};
use crate::ids::FontId;
use crate::interner::SymbolId;
use crate::scaled::Scaled;
use crate::state_hash::StateHashFragment;
use crate::world::ContentHash;
use std::collections::BTreeMap;
use std::path::PathBuf;
pub use tex_fonts::metrics::{
    CharMetrics, CharTag, ExtensibleRecipe, FontContentHash, FontMetrics,
    FontMetricsValidationError, LigKernChar, LigKernCommand, LigKernInstruction, LigKernIter,
    LigKernStep, LigatureCommand, LoadedFont,
};

/// TeX's predefined null font.
pub const NULL_FONT: FontId = FontId::builtin(0);

/// Largest TeX font-parameter number representable in a fontdimen cell key.
pub const MAX_FONT_DIMEN: u16 = 1 << 15;

/// Largest dense font id representable in a fontdimen cell key.
pub const MAX_FONT_DIMEN_FONT_ID: u32 = (1 << 15) - 1;

/// Maximum number of loaded fonts, including `nullfont`.
pub(crate) const MAX_FONT_COUNT: usize = 1 << 15;
const IMMUTABLE_FONT_HASH_DOMAIN: u64 = 0x666f_6e74_5f69_6d6d;

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
}

/// Immutable font store with dense ids and hash-consed load identity.
#[derive(Debug)]
pub(crate) struct FontStore {
    fonts: Vec<LoadedFont>,
    identifiers: Vec<Option<SymbolId>>,
    identifier_writes: Vec<FontId>,
    by_key: BTreeMap<FontKey, FontId>,
    /// Append-only derived fragments keyed by semantic content. Rollback only
    /// truncates the live slot-to-fragment mapping, so a later equivalent load
    /// can reuse its domain-separated fingerprint.
    hash_fragments: Vec<StateHashFragment>,
    hash_fragments_by_key: BTreeMap<FontHashFragmentKey, usize>,
    font_hash_fragments: Vec<usize>,
    identities: IdentityAllocator,
}

impl Clone for FontStore {
    fn clone(&self) -> Self {
        Self {
            fonts: self.fonts.clone(),
            identifiers: self.identifiers.clone(),
            identifier_writes: self.identifier_writes.clone(),
            by_key: self.by_key.clone(),
            hash_fragments: self.hash_fragments.clone(),
            hash_fragments_by_key: self.hash_fragments_by_key.clone(),
            font_hash_fragments: self.font_hash_fragments.clone(),
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
        Self {
            fonts: vec![null],
            identifiers: vec![None],
            identifier_writes: Vec::new(),
            by_key: BTreeMap::new(),
            hash_fragments: vec![hash_fragment],
            hash_fragments_by_key: BTreeMap::from([(hash_fragment_key, 0)]),
            font_hash_fragments: vec![0],
            identities: IdentityAllocator::new(1),
        }
    }

    pub(crate) fn intern(&mut self, font: LoadedFont) -> Result<FontId, FontStoreCapacityError> {
        let key = FontKey {
            name: font.name().to_owned(),
            size: font.size(),
            content_hash: font.content_hash(),
        };
        if let Some(id) = self.by_key.get(&key).copied() {
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
        self.font_hash_fragments.push(hash_fragment);
        self.by_key.insert(key, id);
        Ok(id)
    }

    pub(crate) fn set_identifier(&mut self, id: FontId, symbol: SymbolId) {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        let identifier = self
            .identifiers
            .get_mut(id.raw() as usize)
            .expect("font id is not live in this Universe timeline");
        if identifier.is_none() {
            *identifier = Some(symbol);
            self.identifier_writes.push(id);
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

    pub(crate) fn hash_fragment(&self, id: FontId) -> &StateHashFragment {
        assert!(
            self.contains(id),
            "font id is not live in this Universe timeline"
        );
        let fragment = self.font_hash_fragments[id.raw() as usize];
        &self.hash_fragments[fragment]
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
            }
        }
        self.identifier_writes
            .truncate(mark.identifier_writes_len as usize);
        self.fonts.truncate(mark.len as usize);
        self.identifiers.truncate(mark.len as usize);
        self.font_hash_fragments.truncate(mark.len as usize);
        self.by_key.retain(|_, id| id.raw() < mark.len);
    }

    #[cfg(test)]
    pub(crate) fn testing_hash_fragment_counts(&self) -> (usize, usize) {
        (self.hash_fragments.len(), self.font_hash_fragments.len())
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn testing_state_hash(&self, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash as _;

        for (font, identifier) in self.fonts.iter().zip(&self.identifiers) {
            font.name().hash(hasher);
            font.content_hash().hash(hasher);
            font.checksum().hash(hasher);
            font.design_size().raw().hash(hasher);
            font.size().raw().hash(hasher);
            for parameter in font.parameters() {
                parameter.raw().hash(hasher);
            }
            font.metrics().hash(hasher);
            identifier.hash(hasher);
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
        assert_eq!(store.testing_hash_fragment_counts(), (2, 2));
        let first_fragment = {
            let mut hasher = StateHasher::new(TEST_DOMAIN);
            store.hash_fragment(first).apply(&mut hasher);
            hasher.finish()
        };

        store.truncate_to(mark);
        assert_eq!(store.testing_hash_fragment_counts(), (2, 1));

        let replacement = store.intern(font).expect("test font fits");
        assert_eq!(replacement.raw(), first.raw());
        assert_ne!(replacement, first);
        assert_eq!(store.testing_hash_fragment_counts(), (2, 2));
        let mut hasher = StateHasher::new(TEST_DOMAIN);
        store.hash_fragment(replacement).apply(&mut hasher);
        assert_eq!(hasher.finish(), first_fragment);

        let clone = store.clone();
        assert_eq!(clone.testing_hash_fragment_counts(), (2, 2));
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
