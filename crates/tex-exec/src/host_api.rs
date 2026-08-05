use std::path::Path;

use tex_state::{FileContent, InputReadState};

#[derive(Debug)]
pub enum ResourceLookup<T> {
    Available(T),
    Unavailable,
    NeedResource(ResolverResourceNeed),
}

impl<T> ResourceLookup<T> {
    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> ResourceLookup<U> {
        match self {
            Self::Available(value) => ResourceLookup::Available(f(value)),
            Self::Unavailable => ResourceLookup::Unavailable,
            Self::NeedResource(need) => ResourceLookup::NeedResource(need),
        }
    }
}

pub type ResourceResult<T> = Result<ResourceLookup<T>, String>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ResolverResourceNeed {
    request_index: u64,
}

impl ResolverResourceNeed {
    #[must_use]
    pub const fn new(request_index: u64) -> Self {
        Self { request_index }
    }

    #[must_use]
    pub const fn request_index(self) -> u64 {
        self.request_index
    }
}

impl From<tex_state::ResourceNeed> for ResolverResourceNeed {
    fn from(value: tex_state::ResourceNeed) -> Self {
        Self::new(value.request_index())
    }
}

pub trait FontResolver {
    fn open_font(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
        request_index: u64,
    ) -> ResourceResult<FontSource>;
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PdfImagePageBox {
    #[default]
    Crop,
    Media,
    Bleed,
    Trim,
    Art,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum PdfImagePageSelection {
    Number(u32),
    Named(Vec<u8>),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdfImageRequest {
    pub name: String,
    pub page: PdfImagePageSelection,
    pub color_space_object: i32,
    pub page_box: PdfImagePageBox,
    pub resolution: u32,
}

pub trait PdfImageResolver {
    fn open_image(
        &mut self,
        input: &mut dyn InputReadState,
        request: &PdfImageRequest,
        request_index: u64,
    ) -> ResourceResult<tex_state::PdfExternalImageSource>;
}

pub enum FontSource {
    Tfm {
        metrics: FileContent,
        opentype: Option<tex_fonts::OpenTypeFont>,
    },
    MappedTfm {
        metrics: FileContent,
        opentype: tex_fonts::OpenTypeFont,
        encoding_map: tex_fonts::LegacyEncodingMap,
    },
    ClassicTfmFallback {
        metrics: FileContent,
    },
    OpenType(tex_fonts::OpenTypeFont),
}
