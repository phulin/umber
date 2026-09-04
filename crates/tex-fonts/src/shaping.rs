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

/// Metadata describing one borrowed shaping result.
///
/// The hot execution path only needs to visit the glyphs while rustybuzz's
/// output buffer is borrowed.  Keeping the two properties separate from the
/// glyph storage lets that path avoid constructing an owned [`ShapedRun`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShapingMetadata {
    pub direction: WritingDirection,
    pub script: Script,
}

/// Caller-owned storage for repeated OpenType shaping operations.
///
/// rustybuzz consumes a [`UnicodeBuffer`] and returns a [`rustybuzz::GlyphBuffer`].
/// Calling [`rustybuzz::GlyphBuffer::clear`] on the latter returns the same
/// allocation as a Unicode buffer, so retaining that buffer here keeps both
/// input and output storage warm without allowing a shaped glyph vector to
/// escape the shaping boundary.  Feature records are similarly rebuilt in
/// place for each request.
pub struct ShapingScratch {
    input: Option<UnicodeBuffer>,
    features: Vec<Feature>,
}

impl ShapingScratch {
    /// Clears logical contents while retaining rustybuzz and feature
    /// capacities for the next operation.
    pub fn clear(&mut self) {
        if let Some(input) = &mut self.input {
            input.clear();
        }
        self.features.clear();
    }

    fn take_input(&mut self) -> UnicodeBuffer {
        self.input.take().unwrap_or_default()
    }

    fn return_input(&mut self, input: UnicodeBuffer) {
        debug_assert!(self.input.is_none());
        self.input = Some(input);
    }
}

impl Default for ShapingScratch {
    fn default() -> Self {
        Self {
            input: Some(UnicodeBuffer::new()),
            features: Vec::new(),
        }
    }
}

pub(crate) fn shape_run(
    font: &OpenTypeFont,
    size: Scaled,
    request: ShapingRequest<'_>,
) -> ShapedRun {
    let mut scratch = ShapingScratch::default();
    let mut glyphs = Vec::new();
    let metadata = shape_run_with_scratch(font, size, request, &mut scratch, |glyph| {
        glyphs.push(glyph);
    });
    ShapedRun {
        glyphs,
        direction: metadata.direction,
        script: metadata.script,
    }
}

/// Shapes one caller-delimited run and visits each glyph before the reusable
/// rustybuzz output buffer is returned to `scratch`.
///
/// This is the allocation-free boundary used by execution.  The callback is
/// intentionally passed a copyable glyph projection rather than rustybuzz's
/// borrowed records, so callers do not need to depend on rustybuzz internals.
pub(crate) fn shape_run_with_scratch<F>(
    font: &OpenTypeFont,
    size: Scaled,
    request: ShapingRequest<'_>,
    scratch: &mut ShapingScratch,
    mut visit: F,
) -> ShapingMetadata
where
    F: FnMut(ShapedGlyph),
{
    let text = request.text;
    let script = run_script(text);
    let mut buffer = scratch.take_input();
    buffer.clear();
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
    scratch.features.clear();
    scratch
        .features
        .reserve(font.feature_policy.settings().len());
    scratch.features.extend(
        font.feature_policy
            .settings()
            .iter()
            .map(|setting| Feature::new(to_rustybuzz_tag(setting.tag), setting.value, ..)),
    );
    suppress_ligatures_at_breaks(&mut scratch.features, text, request.break_offsets);

    let output = font.with_shaping_face(|face| rustybuzz::shape(face, &scratch.features, buffer));
    for (info, position) in output.glyph_infos().iter().zip(output.glyph_positions()) {
        visit(ShapedGlyph {
            glyph_id: info.glyph_id,
            cluster: info.cluster,
            x_advance: project(position.x_advance, size, font.metrics.units_per_em),
            y_advance: project(position.y_advance, size, font.metrics.units_per_em),
            x_offset: project(position.x_offset, size, font.metrics.units_per_em),
            y_offset: project(position.y_offset, size, font.metrics.units_per_em),
        });
    }
    scratch.return_input(output.clear());
    scratch.features.clear();
    ShapingMetadata {
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
    features.reserve(breaks.len().saturating_mul(4));
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
