//! Pure, backend-neutral Unicode/OpenType shaping.

use rustybuzz::{Feature, UnicodeBuffer};
use tex_arith::{Scaled, font_units_to_scaled};
use tex_fonts::{FontFeaturePolicy, OpenTypeTag, ShapingFont};
pub use unicode_script::Script;
use unicode_script::UnicodeScript;

/// The logical direction of one already-itemized text run.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Direction {
    LeftToRight,
    RightToLeft,
}

impl Direction {
    /// Infers a base direction from the first strong Unicode bidi character.
    ///
    /// This is only a convenience for callers preparing a single run; full
    /// Unicode bidi run reordering remains a later integration stage.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        text.chars()
            .find_map(|character| match unicode_bidi::bidi_class(character) {
                unicode_bidi::BidiClass::L => Some(Self::LeftToRight),
                unicode_bidi::BidiClass::R | unicode_bidi::BidiClass::AL => Some(Self::RightToLeft),
                _ => None,
            })
            .unwrap_or(Self::LeftToRight)
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
    pub direction: Direction,
    pub script: Script,
}

/// Shapes one logical, single-direction text run.
///
/// Cluster values are rustybuzz's UTF-8 byte offsets into `text`. Run
/// segmentation and line-breaking integration deliberately remain outside
/// this Stage 2 kernel.
pub fn shape_run(
    font: ShapingFont<'_>,
    text: &str,
    features: &FontFeaturePolicy,
    direction: Direction,
) -> ShapedRun {
    shape_run_with_breaks(font, text, features, direction, &[])
}

/// Shapes a run while suppressing optional ligatures across candidate breaks.
///
/// Breaks are UTF-8 byte offsets between source characters. The range-limited
/// feature toggles are the standard HarfBuzz mechanism used by paragraph
/// builders to keep shaped clusters from straddling legal hyphenation points.
pub fn shape_run_with_breaks(
    font: ShapingFont<'_>,
    text: &str,
    features: &FontFeaturePolicy,
    direction: Direction,
    breaks: &[usize],
) -> ShapedRun {
    let (font, size) = font.parts();
    let script = run_script(text);
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(to_rustybuzz_direction(direction));
    buffer.set_script(to_rustybuzz_script(script));
    let mut features = features
        .settings()
        .iter()
        .map(|setting| {
            Feature::new(
                to_rustybuzz_tag(setting.tag),
                u32::from(setting.enabled),
                ..,
            )
        })
        .collect::<Vec<_>>();
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
        direction,
        script,
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

fn to_rustybuzz_direction(direction: Direction) -> rustybuzz::Direction {
    match direction {
        Direction::LeftToRight => rustybuzz::Direction::LeftToRight,
        Direction::RightToLeft => rustybuzz::Direction::RightToLeft,
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
