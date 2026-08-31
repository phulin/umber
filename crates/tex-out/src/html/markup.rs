//! Bounded lexical and page-markup emission for standalone HTML output.

#[cfg(test)]
mod tests;

use tex_arith::Scaled;

use super::incremental;
use super::{HtmlError, HtmlOptions};
use crate::positioned::{BoxKind, TextUnit};

pub(super) const BASE_CSS: &str = concat!(
    ".umber-document{margin:0;padding:0;background:#777}\n",
    ".umber-page{position:relative;contain:strict;overflow:hidden;background:#fff;margin:0 auto 1rem;isolation:isolate}\n",
    ".umber-page-content{position:absolute;width:0;height:0;overflow:visible}\n",
    ".umber-box{position:absolute;pointer-events:none}\n",
    ".umber-rule{position:absolute;background:currentColor}\n",
    ".umber-run{position:absolute;left:0;top:0;width:0;height:0;overflow:visible;white-space:pre;unicode-bidi:isolate-override;font-kerning:normal;font-variant-ligatures:common-ligatures;font-synthesis:none;font-optical-sizing:none}\n",
    ".umber-run-text{white-space:pre;fill:currentColor}\n",
    ".umber-baseline{fill:transparent;pointer-events:none}\n",
    ".umber-math{position:absolute;left:0;top:0;width:0;height:0;overflow:visible;color:currentColor}\n",
    ".umber-math-text,.umber-math-outline,.umber-math-rule{fill:currentColor}\n",
    ".umber-math-baseline{fill:transparent;pointer-events:none}\n",
    ".umber-special{position:absolute;width:0;height:0;overflow:hidden;pointer-events:none}\n",
    ".umber-a11y{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}\n",
    ".umber-a11y-line{display:block;margin:0;padding:0}\n",
    "@media print{.umber-document{background:#fff}.umber-page{break-after:page;margin:0}}\n",
);

const ASCII_SCALAR_CAPACITY: usize = 64;

struct AsciiScalar {
    bytes: [u8; ASCII_SCALAR_CAPACITY],
    start: usize,
}

impl AsciiScalar {
    fn unsigned_decimal(value: u128) -> Self {
        let mut scalar = Self::empty();
        scalar.prepend_digits(value, 10, 1);
        scalar
    }

    fn signed_decimal(value: i128) -> Self {
        let mut scalar = Self::empty();
        scalar.prepend_digits(value.unsigned_abs(), 10, 1);
        if value < 0 {
            scalar.prepend(b'-');
        }
        scalar
    }

    fn lower_hex(value: u32) -> Self {
        let mut scalar = Self::empty();
        scalar.prepend_digits(u128::from(value), 16, 1);
        scalar
    }

    fn rounded_decimal(value: u128, negative: bool, fraction_places: u32) -> Self {
        let mut scalar = Self::empty();
        let fraction_scale = 10_u128.pow(fraction_places);
        scalar.prepend_digits(
            value % fraction_scale,
            10,
            usize::try_from(fraction_places).expect("fraction width fits in usize"),
        );
        scalar.prepend(b'.');
        scalar.prepend_digits(value / fraction_scale, 10, 1);
        if negative && value != 0 {
            scalar.prepend(b'-');
        }
        scalar
    }

    fn variation_decimal(value: i32) -> Self {
        const BINARY_DENOMINATOR: u128 = 65_536;
        const DECIMAL_FRACTION_FACTOR: u128 = 152_587_890_625; // 5^16

        let mut scalar = Self::empty();
        let magnitude = u128::from(value.unsigned_abs());
        let integer = magnitude / BINARY_DENOMINATOR;
        let remainder = magnitude % BINARY_DENOMINATOR;
        if remainder == 0 {
            scalar.prepend_digits(integer, 10, 1);
        } else {
            let mut fraction = remainder * DECIMAL_FRACTION_FACTOR;
            let mut width = 16;
            while fraction.is_multiple_of(10) {
                fraction /= 10;
                width -= 1;
            }
            scalar.prepend_digits(fraction, 10, width);
            scalar.prepend(b'.');
            scalar.prepend_digits(integer, 10, 1);
        }
        if value < 0 {
            scalar.prepend(b'-');
        }
        scalar
    }

    fn empty() -> Self {
        Self {
            bytes: [0; ASCII_SCALAR_CAPACITY],
            start: ASCII_SCALAR_CAPACITY,
        }
    }

    fn prepend_digits(&mut self, mut value: u128, radix: u8, mut minimum_width: usize) {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let radix = u128::from(radix);
        loop {
            let digit = usize::try_from(value % radix).expect("digit fits in usize");
            self.prepend(DIGITS[digit]);
            value /= radix;
            minimum_width = minimum_width.saturating_sub(1);
            if value == 0 && minimum_width == 0 {
                break;
            }
        }
    }

    fn prepend(&mut self, byte: u8) {
        self.start = self
            .start
            .checked_sub(1)
            .expect("ASCII scalar buffer has enough capacity");
        self.bytes[self.start] = byte;
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[self.start..])
            .expect("ASCII scalar encoder only initializes ASCII bytes")
    }
}

pub(super) fn write_render_page(
    out: &mut String,
    page: &incremental::RenderPage,
    options: &HtmlOptions,
) -> Result<(), HtmlError> {
    MarkupWriter::new(out, options.max_html_bytes).write_page(page, options)
}

struct MarkupWriter<'a> {
    out: &'a mut String,
    max_bytes: usize,
}

impl<'a> MarkupWriter<'a> {
    fn new(out: &'a mut String, max_bytes: usize) -> Self {
        Self { out, max_bytes }
    }

    fn write_page(
        &mut self,
        page: &incremental::RenderPage,
        options: &HtmlOptions,
    ) -> Result<(), HtmlError> {
        self.out
            .push_str("<section class=\"umber-page\" data-umber-page=\"");
        self.unsigned_decimal(u128::from(page.ordinal));
        self.out.push_str("\" data-umber-revision=\"");
        self.unsigned_decimal(u128::from(options.revision));
        self.out.push_str("\" data-umber-output=\"");
        self.write_hex(&options.output_id.as_bytes());
        self.out.push('"');
        self.attr_sp("width", page.width);
        self.attr_sp("height", page.height);
        self.attr_sp("origin-x", page.origin_x);
        self.attr_sp("origin-y", page.origin_y);
        self.out.push_str(" data-umber-mag=\"");
        self.signed_decimal(i128::from(page.mag));
        self.out.push_str("\" style=\"width:");
        self.css_px(page.width, page.mag);
        self.out.push_str(";height:");
        self.css_px(page.height, page.mag);
        self.out
            .push_str("\">\n<div class=\"umber-page-content\" style=\"left:");
        self.css_px(page.origin_x, page.mag);
        self.out.push_str(";top:");
        self.css_px(page.origin_y, page.mag);
        self.out.push_str("\">\n");

        let mut math_open = false;
        for node in &page.nodes {
            match &node.value {
                incremental::RenderNodeValue::Box(value) => {
                    self.out.push_str(
                        "<div class=\"umber-box\" aria-hidden=\"true\" data-umber-event=\"",
                    );
                    self.unsigned_decimal(u128::from(node.event_ordinal));
                    self.out.push_str("\" data-umber-kind=\"");
                    self.out.push_str(match value.kind {
                        BoxKind::Horizontal => "hbox",
                        BoxKind::Vertical => "vbox",
                    });
                    self.out.push('"');
                    self.geometry_attrs(value.x, value.y, value.width, value.height);
                    self.attr_sp("baseline", value.baseline);
                    self.geometry_style(value.x, value.y, value.width, value.height, page.mag);
                    self.out.push_str("\"></div>\n");
                }
                incremental::RenderNodeValue::Rule(value) => {
                    self.out.push_str(
                        "<div class=\"umber-rule\" aria-hidden=\"true\" data-umber-event=\"",
                    );
                    self.unsigned_decimal(u128::from(node.event_ordinal));
                    self.out.push('"');
                    self.geometry_attrs(value.x, value.y, value.width, value.height);
                    self.geometry_style(value.x, value.y, value.width, value.height, page.mag);
                    if let Some(color) = &value.color {
                        self.out.push_str(";color:");
                        self.out.push_str(color);
                    }
                    self.out.push_str("\"></div>\n");
                }
                incremental::RenderNodeValue::Text(value) => {
                    self.write_text(value, node.event_ordinal, page.mag);
                }
                incremental::RenderNodeValue::Special(value) => {
                    self.out.push_str(
                        "<span class=\"umber-special\" aria-hidden=\"true\" data-umber-event=\"",
                    );
                    self.unsigned_decimal(u128::from(node.event_ordinal));
                    self.out.push('"');
                    self.attr_sp("x", value.x);
                    self.attr_sp("y", value.y);
                    self.out.push_str(" data-umber-special-class=\"");
                    self.escape_attr(&value.class);
                    self.out.push_str("\" data-umber-special-hex=\"");
                    self.write_hex(&value.payload);
                    match &value.action {
                        incremental::RenderSpecialAction::Destination(id) => {
                            self.out.push_str("\" id=\"");
                            self.escape_attr(id);
                        }
                        incremental::RenderSpecialAction::Inert => {
                            self.out.push_str("\" data-umber-special-policy=\"inert");
                        }
                        _ => self.out.push_str("\" data-umber-special-policy=\"applied"),
                    }
                    self.out.push_str("\" style=\"left:");
                    self.css_px(value.x, page.mag);
                    self.out.push_str(";top:");
                    self.css_px(value.y, page.mag);
                    self.out.push_str("\"></span>\n");
                }
                incremental::RenderNodeValue::MathStart(value) => {
                    math_open = true;
                    self.out.push_str(
                        "<svg class=\"umber-math\" aria-hidden=\"true\" data-umber-math=\"",
                    );
                    self.unsigned_decimal(u128::from(value.id));
                    self.out.push('"');
                    self.attr_sp("x", value.x);
                    self.attr_sp("baseline", value.baseline);
                    self.attr_sp("width", value.width);
                    self.attr_sp("height", value.height);
                    self.attr_sp("depth", value.depth);
                    self.out
                        .push_str("><rect class=\"umber-math-baseline\" x=\"");
                    self.css_px(value.x, page.mag);
                    self.out.push_str("\" y=\"");
                    self.css_px(value.baseline, page.mag);
                    self.out.push_str("\" width=\"1\" height=\"1\"></rect>");
                }
                incremental::RenderNodeValue::MathGlyph(value) => {
                    self.write_math_glyph(value, node.event_ordinal, page.mag);
                }
                incremental::RenderNodeValue::MathRule(value) => {
                    self.out
                        .push_str("<rect class=\"umber-math-rule\" data-umber-math-event=\"");
                    self.unsigned_decimal(u128::from(node.event_ordinal));
                    self.out.push('"');
                    self.geometry_attrs(value.x, value.y, value.width, value.height);
                    self.out.push_str(" x=\"");
                    self.css_px(value.x, page.mag);
                    self.out.push_str("\" y=\"");
                    self.css_px(value.y, page.mag);
                    self.out.push_str("\" width=\"");
                    self.css_px(value.width, page.mag);
                    self.out.push_str("\" height=\"");
                    self.css_px(value.height, page.mag);
                    self.out.push_str("\"></rect>");
                }
                incremental::RenderNodeValue::MathEnd => {
                    math_open = false;
                    self.out.push_str("</svg>\n");
                }
            }
            self.check_size()?;
        }
        debug_assert!(!math_open);
        self.out
            .push_str("</div><div class=\"umber-a11y\" role=\"group\" aria-label=\"Page ");
        self.unsigned_decimal(u128::from(page.ordinal));
        self.out.push_str("\">");
        self.write_accessibility(page);
        self.out.push_str("</div></section>\n");
        self.check_size()
    }

    fn write_text(&mut self, value: &incremental::RenderText, ordinal: u32, mag: i32) {
        self.out
            .push_str("<svg class=\"umber-run\" aria-hidden=\"true\" data-umber-event=\"");
        self.unsigned_decimal(u128::from(ordinal));
        self.out.push('"');
        self.attr_sp("x", value.x);
        self.attr_sp("baseline", value.baseline);
        self.out.push_str(" data-umber-font=\"");
        self.unsigned_decimal(u128::from(value.font_id));
        if let Some(face_index) = value.face_index {
            self.out.push_str("\" data-umber-face-index=\"");
            self.unsigned_decimal(u128::from(face_index));
            if let Some(script) = value.script {
                self.out.push_str("\" data-umber-script=\"");
                self.escape_attr(&String::from_utf8_lossy(&script));
            }
        }
        self.out.push_str("\" data-umber-codes=\"");
        self.write_codes(&value.units);
        self.out.push_str("\" data-umber-text-kind=\"");
        self.out.push_str(if value.mapped_encoding {
            "encoding"
        } else {
            "unicode"
        });
        self.out.push_str("\" style=\"font-family:'");
        self.out.push_str(&value.family);
        self.out.push_str("';font-size:");
        self.css_px(Scaled::from_raw(value.font.at_size_raw), mag);
        if value.face_index.is_some() {
            self.out.push_str(";font-feature-settings:");
            self.write_feature_settings(&value.features);
            self.out.push_str(";font-variation-settings:");
            self.write_variation_settings(&value.variations);
        }
        if let Some(color) = &value.color {
            self.out.push_str(";color:");
            self.out.push_str(color);
        }
        self.out.push_str("\"><rect class=\"umber-baseline\" x=\"");
        self.css_px(value.x, mag);
        self.out.push_str("\" y=\"");
        self.css_px(value.baseline, mag);
        self.out.push_str("\" width=\"1\" height=\"1\"></rect>");
        if let Some(link) = &value.link {
            self.out.push_str("<a href=\"");
            self.escape_attr(link);
            self.out.push_str("\" rel=\"noreferrer noopener\">");
        }
        self.out
            .push_str("<text class=\"umber-run-text\" direction=\"");
        self.out.push_str(match value.direction {
            incremental::RenderDirection::LeftToRight => "ltr",
            incremental::RenderDirection::RightToLeft => "rtl",
        });
        if let Some(language) = &value.language {
            self.out.push_str("\" lang=\"");
            self.escape_attr(language);
        }
        self.out.push_str("\" x=\"");
        if value.exact_character_positions {
            for (index, position) in value.positions.iter().enumerate() {
                if index > 0 {
                    self.out.push(' ');
                }
                self.css_px(*position, mag);
            }
        } else {
            self.css_px(value.x, mag);
        }
        self.out.push_str("\" y=\"");
        self.css_px(value.baseline, mag);
        self.out.push_str("\">");
        self.escape_text(&value.text);
        self.out.push_str("</text>");
        if value.link.is_some() {
            self.out.push_str("</a>");
        }
        self.out.push_str("</svg>\n");
    }

    fn write_math_glyph(&mut self, value: &incremental::RenderMathGlyph, ordinal: u32, mag: i32) {
        let glyph = value.glyph;
        self.out
            .push_str("<g class=\"umber-math-glyph\" data-umber-math-event=\"");
        self.unsigned_decimal(u128::from(ordinal));
        self.out.push_str("\" data-umber-glyph-id=\"");
        self.unsigned_decimal(u128::from(glyph.glyph_id));
        self.out.push_str("\" data-umber-font-instance=\"");
        self.write_hex(&glyph.font_instance.bytes());
        self.out.push_str("\" data-umber-ssty=\"");
        self.unsigned_decimal(u128::from(glyph.ssty));
        self.out.push('"');
        self.attr_sp("x", glyph.x);
        self.attr_sp("baseline", glyph.baseline);
        self.attr_sp("width", glyph.width);
        self.attr_sp("height", glyph.height);
        self.attr_sp("depth", glyph.depth);
        self.out.push('>');
        match &value.drawing {
            incremental::RenderMathDrawing::Text {
                scalar,
                family,
                font_size_raw,
                variations,
            } => {
                self.out
                    .push_str("<text class=\"umber-math-text\" direction=\"ltr\" x=\"");
                self.css_px(glyph.x, mag);
                self.out.push_str("\" y=\"");
                self.css_px(glyph.baseline, mag);
                self.out.push_str("\" style=\"font-family:'");
                self.out.push_str(family);
                self.out.push_str("';font-size:");
                self.css_px(Scaled::from_raw(*font_size_raw), mag);
                self.out.push_str(";font-feature-settings:'ssty' ");
                self.unsigned_decimal(u128::from(glyph.ssty));
                self.out.push_str(";font-variation-settings:");
                self.write_variation_settings(variations);
                self.out.push_str("\">");
                self.escape_char(*scalar);
                self.out.push_str("</text>");
            }
            incremental::RenderMathDrawing::Outline {
                path,
                units_per_em,
                font_size_raw,
            } => {
                self.out.push_str("<path class=\"umber-math-outline\" d=\"");
                self.out.push_str(path);
                self.out.push_str("\" transform=\"translate(");
                self.css_number(glyph.x, mag, 1);
                self.out.push(' ');
                self.css_number(glyph.baseline, mag, 1);
                self.out.push_str(") scale(");
                self.css_number(
                    Scaled::from_raw(*font_size_raw),
                    mag,
                    i128::from(*units_per_em),
                );
                self.out.push(' ');
                self.css_number(
                    Scaled::from_raw(-*font_size_raw),
                    mag,
                    i128::from(*units_per_em),
                );
                self.out.push_str(")\"></path>");
            }
        }
        self.out.push_str("</g>");
    }

    fn write_feature_settings(&mut self, settings: &[([u8; 4], u32)]) {
        if settings.is_empty() {
            self.out.push_str("normal");
            return;
        }
        for (index, (tag, value)) in settings.iter().enumerate() {
            if index > 0 {
                self.out.push(',');
            }
            self.out.push('\'');
            self.write_css_tag(*tag);
            self.out.push_str("' ");
            self.unsigned_decimal(u128::from(*value));
        }
    }

    fn write_variation_settings(&mut self, settings: &[([u8; 4], i32)]) {
        if settings.is_empty() {
            self.out.push_str("normal");
            return;
        }
        for (index, (tag, value)) in settings.iter().enumerate() {
            if index > 0 {
                self.out.push(',');
            }
            self.out.push('\'');
            self.write_css_tag(*tag);
            self.out.push_str("' ");
            self.out
                .push_str(AsciiScalar::variation_decimal(*value).as_str());
        }
    }

    fn write_css_tag(&mut self, tag: [u8; 4]) {
        for byte in tag {
            match byte {
                b'\'' => self.out.push_str("\\27 "),
                b'\\' => self.out.push_str("\\5c "),
                _ => self.out.push(char::from(byte)),
            }
        }
    }

    fn write_accessibility(&mut self, page: &incremental::RenderPage) {
        let mut open_line = None;
        let mut line_is_open = false;
        for node in &page.nodes {
            let incremental::RenderNodeValue::Text(value) = &node.value else {
                continue;
            };
            if !line_is_open || open_line != value.accessibility_line {
                if line_is_open {
                    self.out.push_str("</p>");
                }
                self.out.push_str("<p class=\"umber-a11y-line\">");
                open_line = value.accessibility_line;
                line_is_open = true;
            }
            if let Some(link) = &value.link {
                self.out.push_str("<a href=\"");
                self.escape_attr(link);
                self.out.push_str("\" rel=\"noreferrer noopener\">");
            }
            self.escape_text(&value.text);
            if value.link.is_some() {
                self.out.push_str("</a>");
            }
        }
        if line_is_open {
            self.out.push_str("</p>");
        }
    }

    fn geometry_attrs(&mut self, x: Scaled, y: Scaled, width: Scaled, height: Scaled) {
        self.attr_sp("x", x);
        self.attr_sp("y", y);
        self.attr_sp("width", width);
        self.attr_sp("height", height);
    }

    fn attr_sp(&mut self, name: &str, value: Scaled) {
        self.out.push_str(" data-umber-");
        self.out.push_str(name);
        self.out.push_str("-sp=\"");
        self.signed_decimal(i128::from(value.raw()));
        self.out.push('"');
    }

    fn geometry_style(&mut self, x: Scaled, y: Scaled, width: Scaled, height: Scaled, mag: i32) {
        self.out.push_str(" style=\"left:");
        self.css_px(x, mag);
        self.out.push_str(";top:");
        self.css_px(y, mag);
        self.out.push_str(";width:");
        self.css_px(width, mag);
        self.out.push_str(";height:");
        self.css_px(height, mag);
    }

    fn css_px(&mut self, value: Scaled, mag: i32) {
        self.css_number(value, mag, 1);
        self.out.push_str("px");
    }

    fn css_number(&mut self, value: Scaled, mag: i32, extra_denominator: i128) {
        const DENOMINATOR: i128 = 65_536 * 5 * 7_227;
        const PLACES: i128 = 100_000_000;
        let numerator = i128::from(value.raw()) * i128::from(mag) * 48;
        let negative = numerator < 0;
        let magnitude = numerator.abs();
        let denominator = DENOMINATOR * extra_denominator;
        let mut scaled = magnitude * PLACES / denominator;
        let remainder = magnitude * PLACES % denominator;
        if remainder * 2 >= denominator {
            scaled += 1;
        }
        self.out.push_str(
            AsciiScalar::rounded_decimal(
                u128::try_from(scaled).expect("rounded CSS magnitude is nonnegative"),
                negative,
                8,
            )
            .as_str(),
        );
    }

    fn write_codes(&mut self, units: &[TextUnit]) {
        for (index, unit) in units.iter().enumerate() {
            if index != 0 {
                self.out.push(',');
            }
            match unit {
                TextUnit::Code(code) => {
                    self.out.push_str("0x");
                    self.out.push_str(AsciiScalar::lower_hex(*code).as_str());
                }
                TextUnit::Space => self.out.push_str("space"),
            }
        }
    }

    fn signed_decimal(&mut self, value: i128) {
        self.out
            .push_str(AsciiScalar::signed_decimal(value).as_str());
    }

    fn unsigned_decimal(&mut self, value: u128) {
        self.out
            .push_str(AsciiScalar::unsigned_decimal(value).as_str());
    }

    fn write_hex(&mut self, bytes: &[u8]) {
        write_hex(bytes, self.out);
    }

    fn escape_text(&mut self, value: &str) {
        escape_text(value, self.out);
    }

    fn escape_char(&mut self, value: char) {
        match value {
            '&' => self.out.push_str("&amp;"),
            '<' => self.out.push_str("&lt;"),
            '>' => self.out.push_str("&gt;"),
            _ => self.out.push(value),
        }
    }

    fn escape_attr(&mut self, value: &str) {
        escape_attr(value, self.out);
    }

    fn check_size(&self) -> Result<(), HtmlError> {
        check_html_size(self.out, self.max_bytes)
    }
}

pub(super) fn check_html_size(out: &str, max_bytes: usize) -> Result<(), HtmlError> {
    if out.len() > max_bytes {
        Err(HtmlError::HtmlTooLarge {
            bytes: out.len(),
            limit: max_bytes,
        })
    } else {
        Ok(())
    }
}

pub(super) fn escape_text(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

pub(super) fn escape_attr(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
}

pub(super) fn write_hex(bytes: &[u8], out: &mut String) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    write_hex(bytes, &mut value);
    value
}

pub(super) fn base64(bytes: &[u8], out: &mut String) {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(char::from(TABLE[usize::from(a >> 2)]));
        out.push(char::from(TABLE[usize::from((a & 3) << 4 | b >> 4)]));
        if chunk.len() > 1 {
            out.push(char::from(TABLE[usize::from((b & 15) << 2 | c >> 6)]));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(TABLE[usize::from(c & 63)]));
        } else {
            out.push('=');
        }
    }
}
