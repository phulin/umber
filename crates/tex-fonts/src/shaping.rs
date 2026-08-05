use crate::{FontLanguage, OpenTypeFont, OpenTypeTag, WritingDirection};
use rustybuzz::{Feature, UnicodeBuffer};
use std::str::FromStr;
use tex_arith::{Scaled, font_units_to_scaled};
pub use unicode_script::Script;
use unicode_script::UnicodeScript;

/// One typed request to shape a caller-delimited text run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShapingRequest<'a> {
    pub text: &'a str,
    /// UTF-8 byte offsets where optional ligatures must not cross.
    pub break_offsets: &'a [usize],
}

impl<'a> ShapingRequest<'a> {
    #[must_use]
    pub const fn new(text: &'a str) -> Self {
        Self {
            text,
            break_offsets: &[],
        }
    }

    #[must_use]
    pub const fn with_breaks(text: &'a str, break_offsets: &'a [usize]) -> Self {
        Self {
            text,
            break_offsets,
        }
    }
}

/// One positioned glyph produced by the shaping engine.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ShapedGlyph {
    pub glyph_id: u32,
    /// UTF-8 byte offset into the source run.
    pub cluster: u32,
    pub x_advance: Scaled,
    pub y_advance: Scaled,
    pub x_offset: Scaled,
    pub y_offset: Scaled,
}

/// The shaped output for one caller-delimited text run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
    pub direction: WritingDirection,
    pub script: Script,
}

pub(crate) fn shape_run(
    font: &OpenTypeFont,
    size: Scaled,
    request: ShapingRequest<'_>,
) -> ShapedRun {
    let text = request.text;
    let script = run_script(text);
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(to_rustybuzz_direction(font.direction));
    buffer.set_script(font.script.map_or_else(
        || to_rustybuzz_script(script),
        |tag| {
            rustybuzz::Script::from_iso15924_tag(to_rustybuzz_tag(tag))
                .unwrap_or(rustybuzz::script::UNKNOWN)
        },
    ));
    set_language(&mut buffer, font.language.as_ref());
    let mut features = font
        .feature_policy
        .settings()
        .iter()
        .map(|setting| Feature::new(to_rustybuzz_tag(setting.tag), setting.value, ..))
        .collect::<Vec<_>>();
    suppress_ligatures_at_breaks(&mut features, text, request.break_offsets);

    let glyphs = font.with_shaping_face(|face| {
        let output = rustybuzz::shape(face, &features, buffer);
        output
            .glyph_infos()
            .iter()
            .zip(output.glyph_positions())
            .map(|(info, position)| ShapedGlyph {
                glyph_id: info.glyph_id,
                cluster: info.cluster,
                x_advance: project(position.x_advance, size, font.metrics.units_per_em),
                y_advance: project(position.y_advance, size, font.metrics.units_per_em),
                x_offset: project(position.x_offset, size, font.metrics.units_per_em),
                y_offset: project(position.y_offset, size, font.metrics.units_per_em),
            })
            .collect()
    });

    ShapedRun {
        glyphs,
        direction: font.direction,
        script,
    }
}

fn set_language(buffer: &mut UnicodeBuffer, language: Option<&FontLanguage>) {
    if let Some(language) = language
        && let Ok(language) = rustybuzz::Language::from_str(language.as_str())
    {
        buffer.set_language(language);
    }
}

fn suppress_ligatures_at_breaks(features: &mut Vec<Feature>, text: &str, breaks: &[usize]) {
    for &boundary in breaks {
        if boundary > text.len() || !text.is_char_boundary(boundary) {
            continue;
        }
        let Some(start) = text[..boundary]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
        else {
            continue;
        };
        for tag in [*b"liga", *b"clig", *b"dlig", *b"hlig"] {
            features.push(Feature {
                tag: rustybuzz::ttf_parser::Tag::from_bytes(&tag),
                value: 0,
                start: start as u32,
                end: boundary as u32,
            });
        }
    }
}

fn project(units: i32, size: Scaled, units_per_em: u16) -> Scaled {
    font_units_to_scaled(units, size, units_per_em)
        .expect("validated font units and TeX font size fit scaled arithmetic")
}

#[must_use]
pub fn run_script(text: &str) -> Script {
    text.chars()
        .map(|character| character.script())
        .find(|script| !matches!(script, Script::Common | Script::Inherited))
        .unwrap_or(Script::Common)
}

/// Returns the Unicode script property used by execution-side run itemization.
#[must_use]
pub fn character_script(character: char) -> Script {
    character.script()
}

#[must_use]
pub fn text_direction(text: &str) -> WritingDirection {
    text.chars()
        .find_map(|character| match unicode_bidi::bidi_class(character) {
            unicode_bidi::BidiClass::L => Some(WritingDirection::LeftToRight),
            unicode_bidi::BidiClass::R | unicode_bidi::BidiClass::AL => {
                Some(WritingDirection::RightToLeft)
            }
            _ => None,
        })
        .unwrap_or(WritingDirection::LeftToRight)
}

fn to_rustybuzz_direction(direction: WritingDirection) -> rustybuzz::Direction {
    match direction {
        WritingDirection::LeftToRight => rustybuzz::Direction::LeftToRight,
        WritingDirection::RightToLeft => rustybuzz::Direction::RightToLeft,
    }
}

fn to_rustybuzz_tag(tag: OpenTypeTag) -> rustybuzz::ttf_parser::Tag {
    rustybuzz::ttf_parser::Tag::from_bytes(&tag.bytes())
}

fn to_rustybuzz_script(script: Script) -> rustybuzz::Script {
    let tag = script.as_iso15924_tag().to_be_bytes();
    rustybuzz::Script::from_iso15924_tag(rustybuzz::ttf_parser::Tag::from_bytes(&tag))
        .unwrap_or(rustybuzz::script::UNKNOWN)
}

#[cfg(test)]
mod tests;
