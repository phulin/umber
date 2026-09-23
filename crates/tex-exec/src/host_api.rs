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
