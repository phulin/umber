//! Stateful font handles and rollback storage.

use crate::ids::FontId;
use crate::interner::Symbol;
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
    pub(crate) len: u32,
    identifier_writes_len: u32,
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
    identifiers: Vec<Option<Symbol>>,
    identifier_writes: Vec<FontId>,
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
            identifiers: vec![None],
            identifier_writes: Vec::new(),
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
        self.identifiers.push(None);
        self.by_key.insert(key, id);
        id
    }

    pub(crate) fn set_identifier(&mut self, id: FontId, symbol: Symbol) {
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
    pub(crate) fn identifier(&self, id: FontId) -> Option<Symbol> {
        self.identifiers.get(id.raw() as usize).copied().flatten()
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
            identifier_writes_len: u32::try_from(self.identifier_writes.len())
                .expect("font identifier write log exceeds u32 entries"),
        }
    }

    pub(crate) fn truncate_to(&mut self, mark: FontStoreMark) {
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
        self.by_key.retain(|_, id| id.raw() < mark.len);
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn testing_state_hash(&self, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash as _;

        for (font, identifier) in self.fonts.iter().zip(&self.identifiers) {
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
            identifier.hash(hasher);
        }
    }
}

impl Default for FontStore {
    fn default() -> Self {
        Self::new()
    }
}
