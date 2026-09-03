use pdf_writer::types::LineCapStyle;
use pdf_writer::{Name, Raw, Str};
use tex_arith::Scaled;

use super::{
    PdfContentOperation, PdfContentRectangle, PdfContentRule, PdfNumber, fixed_number_bytes,
};

pub(super) enum PdfPaintProgram<'a> {
    Rectangles(&'a [PdfContentRectangle]),
    Ordered(&'a [PdfContentOperation]),
}

impl<'a> PdfPaintProgram<'a> {
    pub(super) fn rectangles(rectangles: &'a [PdfContentRectangle]) -> Self {
        Self::Rectangles(rectangles)
    }

    pub(super) fn ordered(operations: &'a [PdfContentOperation]) -> Self {
        Self::Ordered(operations)
    }

    pub(super) fn finish(self) -> Vec<u8> {
        let mut painter = PdfPainter::new();
        match self {
            Self::Rectangles(rectangles) => painter.compact_rectangles(rectangles),
            Self::Ordered(operations) => {
                for operation in operations {
                    painter.ordered(operation);
                }
            }
        }
        painter.finish()
    }
}

struct PdfPainter {
    content: pdf_writer::Content,
    origin: (f32, f32),
    exact_origin: Option<PdfExactTextPosition>,
    fixed_origin: Option<(PdfNumber, PdfNumber)>,
    saved_origins: Vec<PdfSavedOrigin>,
    in_text: bool,
    current_font: Option<PdfTextFont>,
    text_matrix: Option<PdfTextMatrix>,
    text_cursor: Option<PdfTextCursor>,
    pending_text: Vec<PdfTextItem>,
}

type PdfSavedOrigin = (
    (f32, f32),
    Option<PdfExactTextPosition>,
    Option<(PdfNumber, PdfNumber)>,
);

struct PdfTextFont {
    name: Vec<u8>,
    size: f32,
}

#[derive(Clone, Copy)]
struct PdfTextMatrix {
    x: f64,
    baseline: f64,
    horizontal_scale: f32,
    exact: Option<PdfExactTextPosition>,
}

#[derive(Clone, Copy)]
struct PdfExactTextPosition {
    h: i64,
    v: i64,
}

enum PdfTextItem {
    Text(Vec<u8>),
    Adjustment(f32),
}

#[derive(Clone, Copy)]
struct PdfTextCursor {
    x: f64,
    baseline: f32,
    horizontal_scale: f32,
    exact: Option<PdfExactTextCursor>,
}

#[derive(Clone, Copy)]
struct PdfExactTextCursor {
    tj_start_h: i64,
    delta_h: i64,
}

impl PdfPainter {
    fn new() -> Self {
        Self {
            content: pdf_writer::Content::new(),
            origin: (0.0, 0.0),
            exact_origin: Some(PdfExactTextPosition { h: 0, v: 0 }),
            fixed_origin: Some((
                PdfNumber::new(0, 0).expect("zero has valid PDF precision"),
                PdfNumber::new(0, 0).expect("zero has valid PDF precision"),
            )),
            saved_origins: Vec::new(),
            in_text: false,
            current_font: None,
            text_matrix: None,
            text_cursor: None,
            pending_text: Vec::new(),
        }
    }

    fn compact_rectangles<'a>(
        &mut self,
        rectangles: impl IntoIterator<Item = &'a PdfContentRectangle>,
    ) {
        self.save();
        for rectangle in rectangles {
            self.rectangle(rectangle);
        }
        self.restore();
    }

    fn ordered(&mut self, operation: &PdfContentOperation) {
        match operation {
            PdfContentOperation::Rectangle(rectangle) => {
                self.end_text();
                self.save();
                self.rectangle(rectangle);
                self.restore();
            }
            PdfContentOperation::Rule(rule) => self.rule(rule),
            PdfContentOperation::Text(run) => self.text(run),
            PdfContentOperation::Literal {
                mode,
                x,
                y,
                exact_position,
                bytes,
            } => {
                self.prepare_literal(*mode, *x, *y, *exact_position);
                self.content.verbatim_operations(bytes);
            }
            PdfContentOperation::ColorStack {
                mode,
                x,
                y,
                exact_position,
                bytes,
            } => {
                self.prepare_literal(*mode, *x, *y, *exact_position);
                self.content.color_stack_operations(bytes);
            }
            PdfContentOperation::SetMatrix {
                x,
                y,
                exact_position,
                matrix,
            } => {
                self.end_text();
                self.set_origin(*x, *y, *exact_position);
                self.content
                    .transform([matrix[0], matrix[1], matrix[2], matrix[3], 0.0, 0.0]);
            }
            PdfContentOperation::Save {
                x,
                y,
                exact_position,
            } => {
                self.end_text();
                self.set_origin(*x, *y, *exact_position);
                self.save();
            }
            PdfContentOperation::Restore {
                x,
                y,
                exact_position,
            } => {
                self.end_text();
                self.set_origin(*x, *y, *exact_position);
                self.restore();
            }
            PdfContentOperation::FormXObject { x, y, name } => {
                self.xobject([1.0, 0.0, 0.0, 1.0, *x, *y], name);
            }
            PdfContentOperation::ImageXObject {
                x,
                y,
                width,
                height,
                name,
            } => self.xobject([*width, 0.0, 0.0, *height, *x, *y], name),
            PdfContentOperation::ImportedPdfPage { matrix, name } => {
                self.fixed_xobject(matrix, name);
            }
        }
    }

    fn rectangle(&mut self, rectangle: &PdfContentRectangle) {
        let (x, y) = self.relative_position(rectangle.x, rectangle.y);
        self.content
            .rect(x, y, rectangle.width, rectangle.height)
            .fill_nonzero();
    }

    fn rule(&mut self, rule: &PdfContentRule) {
        // pdftex.web §691 (`pdf_set_rule`) uses strokes for rules no thicker
        // than one bp. It selects horizontal strokes first, centers the path
        // with scaled integer arithmetic, and uses a filled rectangle only
        // when both dimensions exceed that threshold.
        const ONE_BP: i32 = 65_782;

        self.end_text();
        self.save();
        if rule.height.raw() <= ONE_BP {
            let center_y = i64::from(rule.y.raw()) + (i64::from(rule.height.raw()) + 1) / 2;
            self.rule_origin(i64::from(rule.x.raw()), center_y, rule.decimal_digits);
            self.stroke_rule(
                scaled_to_bp(rule.height, rule.decimal_digits),
                scaled_to_bp(rule.width, rule.decimal_digits),
                0.0,
            );
        } else if rule.width.raw() <= ONE_BP {
            let center_x = i64::from(rule.x.raw()) + (i64::from(rule.width.raw()) + 1) / 2;
            self.rule_origin(center_x, i64::from(rule.y.raw()), rule.decimal_digits);
            self.stroke_rule(
                scaled_to_bp(rule.width, rule.decimal_digits),
                0.0,
                scaled_to_bp(rule.height, rule.decimal_digits),
            );
        } else {
            self.rule_origin(
                i64::from(rule.x.raw()),
                i64::from(rule.y.raw()),
                rule.decimal_digits,
            );
            self.content
                .rect(
                    0.0,
                    0.0,
                    scaled_to_bp(rule.width, rule.decimal_digits),
                    scaled_to_bp(rule.height, rule.decimal_digits),
                )
                .fill_nonzero();
        }
        self.restore();
    }

    fn rule_origin(&mut self, x: i64, y: i64, decimal_digits: u8) {
        let (x, y) = self.relative_position(
            scaled_raw_to_bp(x, decimal_digits),
            scaled_raw_to_bp(y, decimal_digits),
        );
        if x != 0.0 || y != 0.0 {
            self.content.transform([1.0, 0.0, 0.0, 1.0, x, y]);
        }
    }

    fn stroke_rule(&mut self, width: f32, x: f32, y: f32) {
        self.content
            .set_dash_pattern([], 0.0)
            .set_line_cap(LineCapStyle::ButtCap)
            .set_line_width(width)
            .move_to(0.0, 0.0)
            .line_to(x, y)
            .stroke();
    }

    fn text(&mut self, run: &super::PdfContentTextRun) {
        if !self.in_text {
            // pdftex.web §690 (`pdf_begin_text`) restores the page/form
            // origin before opening every text object. Paint-program
            // coordinates are already PDF-oriented, so that logical origin is
            // `(0, 0)` here. In particular, an origin-mode literal must not
            // leave later text expressed through its translated CTM: although
            // the affine positions are equivalent, PDF consumers raster and
            // extract the composed floating-point coordinates differently.
            self.set_origin(
                0.0,
                0.0,
                run.exact_position
                    .map(|position| super::PdfContentTextPosition {
                        h: 0,
                        v: 0,
                        decimal_digits: position.decimal_digits,
                    }),
            );
            self.content.begin_text();
            self.in_text = true;
            self.current_font = None;
            self.text_matrix = Some(PdfTextMatrix {
                x: 0.0,
                baseline: 0.0,
                horizontal_scale: 1.0,
                exact: self.exact_origin,
            });
        }
        let (x, baseline) = self.relative_position(run.x, run.baseline);
        let serialized_x = run
            .raster
            .as_ref()
            .map(|raster| raster.serialized_x - f64::from(self.origin.0))
            .unwrap_or_else(|| f64::from(x));
        let positioning_x = run
            .raster
            .as_ref()
            .map(|raster| raster.position_x - f64::from(self.origin.0))
            .unwrap_or_else(|| f64::from(x));
        if let (Some(cursor), Some(advance)) = (self.text_cursor, run.advance)
            && cursor.baseline == baseline
            && cursor.horizontal_scale == run.horizontal_scale
        {
            // pdftex.web §690 (`pdf_begin_string`) keeps the PDF text
            // position across character, kern, glue, and direct-color nodes.
            // Express the next TeX anchor as a TJ adjustment in the current
            // font raster instead of resetting Tm for every positioned run.
            let text_unit = run
                .raster
                .as_ref()
                .map(|raster| raster.font_size)
                .unwrap_or_else(|| f64::from(run.font_size))
                * f64::from(run.horizontal_scale)
                / 1000.0;
            if text_unit > 0.0 {
                let adjustment = exact_first_adjustment(run, cursor)
                    .unwrap_or_else(|| (-(positioning_x - cursor.x) / text_unit).round());
                if adjustment.abs() < 32_768.0 {
                    self.select_font(run);
                    let raster_cursor = if has_glyph_raster(run) {
                        self.show_rastered_text(
                            run,
                            cursor.x,
                            cursor.exact,
                            adjustment,
                            text_unit,
                            f64::from(self.origin.0),
                        )
                    } else {
                        if adjustment == 0.0 {
                            self.append_text(&run.bytes);
                        } else {
                            self.append_adjustment(adjustment as f32);
                            self.append_text(&run.bytes);
                        }
                        None
                    };
                    self.text_cursor = Some(PdfTextCursor {
                        x: raster_cursor
                            .map(|cursor| cursor.x)
                            .unwrap_or(cursor.x - adjustment * text_unit + advance),
                        baseline,
                        horizontal_scale: run.horizontal_scale,
                        exact: raster_cursor.and_then(|cursor| cursor.exact),
                    });
                    return;
                }
            }
        }
        self.flush_text();
        self.select_font(run);
        let start = self.set_text_position(
            serialized_x,
            f64::from(baseline),
            run.horizontal_scale,
            run.exact_position,
        );
        let text_unit = run
            .raster
            .as_ref()
            .map(|raster| raster.font_size)
            .unwrap_or_else(|| f64::from(run.font_size))
            * f64::from(run.horizontal_scale)
            / 1000.0;
        let raster_cursor = if has_glyph_raster(run) {
            self.show_rastered_text(
                run,
                start.x,
                start.exact_h.map(|tj_start_h| PdfExactTextCursor {
                    tj_start_h,
                    delta_h: 0,
                }),
                0.0,
                text_unit,
                f64::from(self.origin.0),
            )
        } else {
            self.flush_text();
            self.content.show(Str(&run.bytes));
            None
        };
        self.text_cursor = run.advance.map(|advance| PdfTextCursor {
            x: raster_cursor
                .map(|cursor| cursor.x)
                .unwrap_or(start.x + advance),
            baseline,
            horizontal_scale: run.horizontal_scale,
            exact: raster_cursor.and_then(|cursor| cursor.exact),
        });
    }

    fn show_rastered_text(
        &mut self,
        run: &super::PdfContentTextRun,
        cursor_x: f64,
        exact_cursor: Option<PdfExactTextCursor>,
        first_adjustment: f64,
        text_unit: f64,
        origin_x: f64,
    ) -> Option<PdfRasteredTextCursor> {
        let raster = run
            .raster
            .as_ref()
            .expect("glyph raster was checked by the caller");
        debug_assert_eq!(raster.glyphs.len(), run.bytes.len());
        debug_assert!(text_unit > 0.0);

        // pdftex.web §690 calls `pdf_begin_string` before every character.
        // `/Widths` advances live on their rounded PDF raster, so even
        // adjacent TeX character nodes can require an integer TJ correction.
        let mut cursor_x = cursor_x;
        let mut adjustments = Vec::with_capacity(run.bytes.len());
        let mut exact_cursor = raster.exact.map(|exact| {
            let mut cursor =
                exact_cursor.expect("exact text raster has an exact positioned anchor");
            for glyph in &raster.glyphs {
                let (movement, movement_out) = pdftex_text_movement(
                    glyph.position_raw - (cursor.tj_start_h + cursor.delta_h),
                    exact.font_size,
                    exact.expansion_ratio,
                );
                adjustments.push(-movement as f64);
                cursor.delta_h += movement_out;
                cursor.delta_h +=
                    pdftex_glyph_advance(glyph.width_raw, exact.font_size, exact.expansion_ratio);
            }
            cursor
        });
        if exact_cursor.is_none() {
            for (index, glyph) in raster.glyphs.iter().enumerate() {
                let adjustment = if index == 0 {
                    first_adjustment
                } else {
                    (-((glyph.position_x - origin_x) - cursor_x) / text_unit).round()
                };
                adjustments.push(adjustment);
                cursor_x = cursor_x - adjustment * text_unit + glyph.advance;
            }
        } else {
            for (adjustment, glyph) in adjustments.iter().zip(&raster.glyphs) {
                cursor_x = cursor_x - adjustment * text_unit + glyph.advance;
            }
        }
        if adjustments.iter().all(|adjustment| *adjustment == 0.0) {
            self.append_text(&run.bytes);
            return Some(PdfRasteredTextCursor {
                x: cursor_x,
                exact: exact_cursor.take(),
            });
        }

        let mut string_start = 0;
        for (index, adjustment) in adjustments.iter().copied().enumerate() {
            if adjustment == 0.0 {
                continue;
            }
            if string_start < index {
                self.append_text(&run.bytes[string_start..index]);
            }
            self.append_adjustment(adjustment as f32);
            string_start = index;
        }
        if string_start < run.bytes.len() {
            self.append_text(&run.bytes[string_start..]);
        }
        Some(PdfRasteredTextCursor {
            x: cursor_x,
            exact: exact_cursor,
        })
    }

    fn prepare_literal(
        &mut self,
        mode: crate::PdfLiteralMode,
        x: f32,
        y: f32,
        exact_position: Option<super::PdfContentTextPosition>,
    ) {
        if mode != crate::PdfLiteralMode::Direct {
            self.end_text();
        } else {
            self.flush_text();
        }
        if mode == crate::PdfLiteralMode::Origin {
            self.set_origin(x, y, exact_position);
        }
    }

    fn xobject(&mut self, mut matrix: [f32; 6], name: &[u8]) {
        self.end_text();
        (matrix[4], matrix[5]) = self.relative_position(matrix[4], matrix[5]);
        self.save();
        self.content.transform(matrix).x_object(Name(name));
        self.restore();
    }

    fn fixed_xobject(&mut self, matrix: &[super::PdfNumber; 6], name: &[u8]) {
        self.end_text();
        self.save();
        let mut matrix = *matrix;
        if let Some((origin_x, origin_y)) = self.fixed_origin {
            if let Some(relative_x) = subtract_fixed_numbers(matrix[4], origin_x) {
                matrix[4] = relative_x;
            }
            if let Some(relative_y) = subtract_fixed_numbers(matrix[5], origin_y) {
                matrix[5] = relative_y;
            }
        }
        let mut operation = self.content.op("cm");
        let mut buffer = [0_u8; 32];
        for number in &matrix {
            operation.operand(Raw(fixed_number_bytes(*number, &mut buffer)));
        }
        drop(operation);
        self.content.x_object(Name(name));
        self.restore();
    }

    fn relative_position(&self, x: f32, y: f32) -> (f32, f32) {
        // pdfTeX §690 retains its translated PDF origin, so every later
        // absolute page position is emitted as a delta from that origin.
        (x - self.origin.0, y - self.origin.1)
    }

    fn set_origin(
        &mut self,
        x: f32,
        y: f32,
        exact_position: Option<super::PdfContentTextPosition>,
    ) {
        let (dx, dy) = (x - self.origin.0, y - self.origin.1);
        if dx != 0.0 || dy != 0.0 {
            self.content.transform([1.0, 0.0, 0.0, 1.0, dx, dy]);
            self.origin = (x, y);
        }
        self.exact_origin = match (self.exact_origin, exact_position) {
            (Some(origin), Some(position)) => {
                let (_, h_out) =
                    pdftex_text_coordinate(position.h - origin.h, position.decimal_digits);
                let (_, v_out) =
                    pdftex_text_coordinate(position.v - origin.v, position.decimal_digits);
                Some(PdfExactTextPosition {
                    h: origin.h + h_out,
                    v: origin.v + v_out,
                })
            }
            _ => None,
        };
        self.fixed_origin = exact_position.and_then(|position| {
            let exact_origin = self.exact_origin;
            let h = exact_origin.map(|origin| origin.h).unwrap_or(position.h);
            let v = exact_origin.map(|origin| origin.v).unwrap_or(position.v);
            Some((
                fixed_scaled_number(h, position.decimal_digits)?,
                fixed_scaled_number(v, position.decimal_digits)?,
            ))
        });
    }

    fn save(&mut self) {
        self.saved_origins
            .push((self.origin, self.exact_origin, self.fixed_origin));
        self.content.save_state();
    }

    fn restore(&mut self) {
        self.content.restore_state();
        if let Some((origin, exact_origin, fixed_origin)) = self.saved_origins.pop() {
            self.origin = origin;
            self.exact_origin = exact_origin;
            self.fixed_origin = fixed_origin;
        }
    }

    fn end_text(&mut self) {
        if self.in_text {
            self.flush_text();
            self.content.end_text();
            self.in_text = false;
        }
        self.current_font = None;
        self.text_matrix = None;
        self.text_cursor = None;
    }

    fn select_font(&mut self, run: &super::PdfContentTextRun) {
        if self
            .current_font
            .as_ref()
            .is_some_and(|font| font.name == run.font_name && font.size == run.font_size)
        {
            return;
        }
        self.flush_text();
        self.content.set_font(Name(&run.font_name), run.font_size);
        self.current_font = Some(PdfTextFont {
            name: run.font_name.clone(),
            size: run.font_size,
        });
    }

    fn set_text_position(
        &mut self,
        x: f64,
        baseline: f64,
        horizontal_scale: f32,
        exact_position: Option<super::PdfContentTextPosition>,
    ) -> PdfSetTextPosition {
        let matrix = self
            .text_matrix
            .expect("an open PDF text object has a text matrix");
        if horizontal_scale != 1.0 || matrix.horizontal_scale != 1.0 {
            let (x, baseline, exact) = exact_position
                .map(pdftex_absolute_text_position)
                .unwrap_or((x, baseline, None));
            self.content.set_text_matrix([
                horizontal_scale,
                0.0,
                0.0,
                1.0,
                x as f32,
                baseline as f32,
            ]);
            self.text_matrix = Some(PdfTextMatrix {
                x,
                baseline,
                horizontal_scale,
                exact,
            });
        } else {
            let (dx, dy, exact) = match (matrix.exact, exact_position) {
                (Some(previous), Some(position)) => {
                    let (dx, h_out) =
                        pdftex_text_coordinate(position.h - previous.h, position.decimal_digits);
                    let (dy, v_out) =
                        pdftex_text_coordinate(position.v - previous.v, position.decimal_digits);
                    (
                        dx,
                        dy,
                        Some(PdfExactTextPosition {
                            h: previous.h + h_out,
                            v: previous.v + v_out,
                        }),
                    )
                }
                _ => (x - matrix.x, baseline - matrix.baseline, None),
            };
            self.content.next_line(dx as f32, dy as f32);
            self.text_matrix = Some(PdfTextMatrix {
                x: matrix.x + dx,
                baseline: matrix.baseline + dy,
                horizontal_scale,
                exact,
            });
        }
        let matrix = self
            .text_matrix
            .expect("positioning establishes a PDF text matrix");
        PdfSetTextPosition {
            x: matrix.x,
            exact_h: matrix.exact.map(|exact| exact.h),
        }
    }

    fn append_text(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if let Some(PdfTextItem::Text(text)) = self.pending_text.last_mut() {
            text.extend_from_slice(bytes);
        } else {
            self.pending_text.push(PdfTextItem::Text(bytes.to_vec()));
        }
    }

    fn append_adjustment(&mut self, adjustment: f32) {
        if adjustment != 0.0 {
            self.pending_text.push(PdfTextItem::Adjustment(adjustment));
        }
    }

    fn flush_text(&mut self) {
        if self.pending_text.is_empty() {
            return;
        }
        let mut operation = self.content.show_positioned();
        let mut items = operation.items();
        for item in self.pending_text.drain(..) {
            match item {
                PdfTextItem::Text(text) => {
                    items.show(Str(&text));
                }
                PdfTextItem::Adjustment(adjustment) => {
                    items.adjust(adjustment);
                }
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        self.end_text();
        self.content.finish().to_vec()
    }
}

fn fixed_scaled_number(value: i64, decimal_digits: u8) -> Option<PdfNumber> {
    if decimal_digits > 9 {
        return None;
    }
    let scale = 10_i128.checked_pow(u32::from(decimal_digits))?;
    let numerator = i128::from(value).checked_mul(7_200)?.checked_mul(scale)?;
    let denominator = 7_227_i128.checked_mul(65_536)?;
    let half = denominator / 2;
    let adjusted = if numerator >= 0 {
        numerator.checked_add(half)?
    } else {
        numerator.checked_sub(half)?
    };
    let coefficient = i64::try_from(adjusted / denominator).ok()?;
    PdfNumber::new(coefficient, decimal_digits).ok()
}

fn subtract_fixed_numbers(left: PdfNumber, right: PdfNumber) -> Option<PdfNumber> {
    let decimal_places = left.decimal_places().max(right.decimal_places());
    let left_scale = 10_i128.checked_pow(u32::from(decimal_places - left.decimal_places()))?;
    let right_scale = 10_i128.checked_pow(u32::from(decimal_places - right.decimal_places()))?;
    let coefficient = i128::from(left.coefficient())
        .checked_mul(left_scale)?
        .checked_sub(i128::from(right.coefficient()).checked_mul(right_scale)?)?;
    PdfNumber::new(i64::try_from(coefficient).ok()?, decimal_places).ok()
}

#[cfg(test)]
pub(super) fn retained_origin_after_save_restore(
    saved: super::PdfContentTextPosition,
    nested: super::PdfContentTextPosition,
) -> (i64, i64) {
    let mut painter = PdfPainter::new();
    painter.set_origin(48.964, 0.0, Some(saved));
    painter.save();
    painter.set_origin(58.927, 0.0, Some(nested));
    painter.restore();
    let exact = painter
        .exact_origin
        .expect("exact graphics positions retain an exact origin");
    (exact.h, exact.v)
}

fn scaled_to_bp(value: Scaled, decimal_digits: u8) -> f32 {
    scaled_raw_to_bp(i64::from(value.raw()), decimal_digits)
}

fn scaled_raw_to_bp(value: i64, decimal_digits: u8) -> f32 {
    let scale = 10_i128.pow(u32::from(decimal_digits));
    const NUMERATOR: i128 = 7_200;
    const DENOMINATOR: i128 = 7_227 * 65_536;
    let numerator = i128::from(value) * NUMERATOR * scale;
    let rounded = if numerator >= 0 {
        (numerator + DENOMINATOR / 2) / DENOMINATOR
    } else {
        (numerator - DENOMINATOR / 2) / DENOMINATOR
    };
    rounded as f32 / 10_f32.powi(i32::from(decimal_digits))
}

fn has_glyph_raster(run: &super::PdfContentTextRun) -> bool {
    run.raster
        .as_ref()
        .is_some_and(|raster| !run.bytes.is_empty() && raster.glyphs.len() == run.bytes.len())
}

fn exact_first_adjustment(run: &super::PdfContentTextRun, cursor: PdfTextCursor) -> Option<f64> {
    let exact_cursor = cursor.exact?;
    let raster = run.raster.as_ref()?;
    let exact = raster.exact?;
    let glyph = raster.glyphs.first()?;
    Some(
        -pdftex_text_movement(
            glyph.position_raw - (exact_cursor.tj_start_h + exact_cursor.delta_h),
            exact.font_size,
            exact.expansion_ratio,
        )
        .0 as f64,
    )
}

#[derive(Clone, Copy)]
struct PdfRasteredTextCursor {
    x: f64,
    exact: Option<PdfExactTextCursor>,
}

#[derive(Clone, Copy)]
struct PdfSetTextPosition {
    x: f64,
    exact_h: Option<i64>,
}

fn pdftex_absolute_text_position(
    position: super::PdfContentTextPosition,
) -> (f64, f64, Option<PdfExactTextPosition>) {
    let (x, h) = pdftex_text_coordinate(position.h, position.decimal_digits);
    let (baseline, v) = pdftex_text_coordinate(position.v, position.decimal_digits);
    (x, baseline, Some(PdfExactTextPosition { h, v }))
}

fn pdftex_text_coordinate(delta: i64, decimal_digits: u8) -> (f64, i64) {
    // pdftex.web §690 (`pdf_begin_string` and `pdf_set_text_pos`) subtracts
    // retained scaled coordinates first. `divide_scaled` then rounds the
    // delta and returns both the printed coefficient and the scaled position
    // actually represented by that coefficient.
    const ONE_HUNDRED_BP: i64 = 6_578_176;
    let (coefficient, scaled_out) =
        pdftex_divide_scaled(delta, ONE_HUNDRED_BP, u32::from(decimal_digits) + 2);
    let scale = 10_i64.pow(u32::from(decimal_digits)) as f64;
    (coefficient as f64 / scale, scaled_out)
}

fn pdftex_text_movement(delta: i64, font_size: i64, expansion_ratio: i16) -> (i64, i64) {
    let ratio = i64::from(expansion_ratio);
    let delta = if ratio == 0 {
        delta
    } else {
        pdftex_round_xn_over_d(delta, 1000, 1000 + ratio)
    };
    let (movement, unexpanded_out) = pdftex_divide_scaled(delta, font_size, 3);
    if ratio == 0 {
        return (movement, unexpanded_out);
    }
    let sign = movement.signum();
    let movement_out = pdftex_round_xn_over_d(
        pdftex_round_xn_over_d(font_size, movement.abs(), 1000),
        1000 + ratio,
        1000,
    ) * sign;
    (movement, movement_out)
}

fn pdftex_glyph_advance(width: i64, font_size: i64, expansion_ratio: i16) -> i64 {
    let ratio = i64::from(expansion_ratio);
    if ratio == 0 {
        return pdftex_divide_scaled(width, font_size, 4).1;
    }
    let unexpanded_width = pdftex_round_xn_over_d(width, 1000, 1000 + ratio);
    let width_coefficient = pdftex_divide_scaled(unexpanded_width, font_size, 4).0;
    let sign = width_coefficient.signum();
    pdftex_round_xn_over_d(
        pdftex_round_xn_over_d(font_size, width_coefficient.abs(), 10_000),
        1000 + ratio,
        1000,
    ) * sign
}

fn pdftex_divide_scaled(value: i64, divisor: i64, decimal_digits: u32) -> (i64, i64) {
    debug_assert!(divisor > 0);
    let sign = value.signum();
    let value = i128::from(value.abs());
    let divisor = i128::from(divisor);
    let scale = 10_i128.pow(decimal_digits);
    let quotient = (value * scale + divisor / 2) / divisor;
    let remainder = value * scale - quotient * divisor;
    let scaled_out = value - remainder / scale;
    (
        i64::try_from(quotient).expect("PDF text movement fits i64") * sign,
        i64::try_from(scaled_out).expect("PDF raster movement fits i64") * sign,
    )
}

fn pdftex_round_xn_over_d(value: i64, numerator: i64, denominator: i64) -> i64 {
    debug_assert!(numerator >= 0);
    debug_assert!(denominator > 0);
    let sign = value.signum();
    let product = i128::from(value.abs()) * i128::from(numerator);
    i64::try_from((product + i128::from(denominator) / 2) / i128::from(denominator))
        .expect("PDF raster multiplication fits i64")
        * sign
}
