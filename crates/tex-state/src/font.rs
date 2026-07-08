//! Stateful font handles and rollback storage.

use crate::ids::FontId;
use crate::scaled::Scaled;
use crate::world::ContentHash;
use std::collections::BTreeMap;
use std::path::PathBuf;
pub use tex_fonts::metrics::{
    CharMetrics, CharTag, ExtensibleRecipe, FontContentHash, FontMetrics, LigKernChar,
    LigKernCommand, LigKernInstruction, LigKernIter, LigKernStep, LigatureCommand, LoadedFont,
};

/// TeX's predefined null font.
pub const NULL_FONT: FontId = FontId::new(0);

/// A missing-character event for consumers to report according to policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MissingCharacter {
    pub font: FontId,
    pub code: u8,
}

/// Rollback watermark for loaded fonts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FontStoreMark {
    len: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FontKey {
    name: String,
    size: Scaled,
    content_hash: FontContentHash,
}

/// Immutable font store with dense ids and hash-consed load identity.
#[derive(Clone, Debug)]
pub(crate) struct FontStore {
    fonts: Vec<LoadedFont>,
    by_key: BTreeMap<FontKey, FontId>,
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
        Self {
            fonts: vec![null],
            by_key: BTreeMap::new(),
        }
    }

    pub(crate) fn intern(&mut self, font: LoadedFont) -> FontId {
        let key = FontKey {
            name: font.name().to_owned(),
            size: font.size(),
            content_hash: font.content_hash(),
        };
        if let Some(id) = self.by_key.get(&key).copied() {
            return id;
        }
        let raw = u32::try_from(self.fonts.len()).expect("font store exceeds u32 ids");
        let id = FontId::new(raw);
        self.fonts.push(font);
        self.by_key.insert(key, id);
        id
    }

    #[must_use]
    pub(crate) fn get(&self, id: FontId) -> &LoadedFont {
        self.fonts
            .get(id.raw() as usize)
            .expect("font id is not live in this Universe timeline")
    }

    #[must_use]
    pub(crate) fn contains(&self, id: FontId) -> bool {
        (id.raw() as usize) < self.fonts.len()
    }

    #[must_use]
    pub(crate) fn watermark(&self) -> FontStoreMark {
        FontStoreMark {
            len: u32::try_from(self.fonts.len()).expect("font store exceeds u32 ids"),
        }
    }

    pub(crate) fn truncate_to(&mut self, mark: FontStoreMark) {
        self.fonts.truncate(mark.len as usize);
        self.by_key.retain(|_, id| id.raw() < mark.len);
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn testing_state_hash(&self, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash as _;

        for font in &self.fonts {
            font.name().hash(hasher);
            font.path().hash(hasher);
            font.content_hash().hash(hasher);
            font.checksum().hash(hasher);
            font.design_size().raw().hash(hasher);
            font.size().raw().hash(hasher);
            for parameter in font.parameters() {
                parameter.raw().hash(hasher);
            }
            font.metrics().hash(hasher);
        }
    }
}

impl Default for FontStore {
    fn default() -> Self {
        Self::new()
    }
}
