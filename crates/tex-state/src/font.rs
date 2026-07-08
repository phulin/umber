//! Immutable loaded font records owned by the state layer.

use crate::ids::FontId;
use crate::scaled::Scaled;
use crate::world::ContentHash;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// TeX's predefined null font.
pub const NULL_FONT: FontId = FontId::new(0);

/// Immutable data captured when a TFM font is loaded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedFont {
    name: String,
    path: PathBuf,
    content_hash: ContentHash,
    checksum: u32,
    design_size: Scaled,
    size: Scaled,
    parameters: Vec<Scaled>,
}

impl LoadedFont {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        path: impl Into<PathBuf>,
        content_hash: ContentHash,
        checksum: u32,
        design_size: Scaled,
        size: Scaled,
        parameters: Vec<Scaled>,
    ) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            content_hash,
            checksum,
            design_size,
            size,
            parameters,
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
    pub const fn content_hash(&self) -> ContentHash {
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
    pub fn fontname_text(&self) -> String {
        if self.size == self.design_size {
            self.name.clone()
        } else {
            format!("{} at {}", self.name, format_scaled(self.size))
        }
    }
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
    content_hash: ContentHash,
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
            ContentHash::from_bytes(&[]),
            0,
            Scaled::from_raw(0),
            Scaled::from_raw(0),
            vec![Scaled::from_raw(0); 7],
        );
        Self {
            fonts: vec![null],
            by_key: BTreeMap::new(),
        }
    }

    pub(crate) fn intern(&mut self, font: LoadedFont) -> FontId {
        let key = FontKey {
            name: font.name.clone(),
            size: font.size,
            content_hash: font.content_hash,
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
            font.name.hash(hasher);
            font.path.hash(hasher);
            font.content_hash.hash(hasher);
            font.checksum.hash(hasher);
            font.design_size.raw().hash(hasher);
            font.size.raw().hash(hasher);
            for parameter in &font.parameters {
                parameter.raw().hash(hasher);
            }
        }
    }
}

impl Default for FontStore {
    fn default() -> Self {
        Self::new()
    }
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
