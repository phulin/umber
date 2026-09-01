use pdf_writer::{Name, Str};

use super::{PdfContentOperation, PdfContentRectangle};

pub(super) enum PdfPaintProgram<'a> {
    Rectangles(&'a [PdfContentRectangle]),
    Compact(&'a [PdfContentOperation]),
    Ordered(&'a [PdfContentOperation]),
}

impl<'a> PdfPaintProgram<'a> {
    pub(super) fn rectangles(rectangles: &'a [PdfContentRectangle]) -> Self {
        Self::Rectangles(rectangles)
    }

    pub(super) fn compact(operations: &'a [PdfContentOperation]) -> Self {
        Self::Compact(operations)
    }

    pub(super) fn ordered(operations: &'a [PdfContentOperation]) -> Self {
        Self::Ordered(operations)
    }

    pub(super) fn finish(self) -> Vec<u8> {
        let mut painter = PdfPainter::new();
        match self {
            Self::Rectangles(rectangles) => painter.compact_rectangles(rectangles),
            Self::Compact(operations) => {
                painter.compact_rectangles(operations.iter().filter_map(
                    |operation| match operation {
                        PdfContentOperation::Rectangle(rectangle) => Some(rectangle),
                        _ => None,
                    },
                ));
                for operation in operations {
                    if let PdfContentOperation::Text(run) = operation {
                        painter.text(run);
                    }
                }
            }
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
    saved_origins: Vec<(f32, f32)>,
    in_text: bool,
    text_cursor: Option<PdfTextCursor>,
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
            saved_origins: Vec::new(),
            in_text: false,
            text_cursor: None,
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
            PdfContentOperation::Text(run) => self.text(run),
            PdfContentOperation::Literal { mode, x, y, bytes } => {
                self.prepare_literal(*mode, *x, *y);
                self.content.verbatim_operations(bytes);
            }
            PdfContentOperation::ColorStack { mode, x, y, bytes } => {
                self.prepare_literal(*mode, *x, *y);
                self.content.color_stack_operations(bytes);
            }
            PdfContentOperation::SetMatrix { x, y, matrix } => {
                self.end_text();
                self.set_origin(*x, *y);
                self.content
                    .transform([matrix[0], matrix[1], matrix[2], matrix[3], 0.0, 0.0]);
            }
            PdfContentOperation::Save { x, y } => {
                self.end_text();
                self.set_origin(*x, *y);
                self.save();
            }
            PdfContentOperation::Restore { x, y } => {
                self.end_text();
                self.set_origin(*x, *y);
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
        }
    }

    fn rectangle(&mut self, rectangle: &PdfContentRectangle) {
        let (x, y) = self.relative_position(rectangle.x, rectangle.y);
        self.content
            .rect(x, y, rectangle.width, rectangle.height)
            .fill_nonzero();
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
            self.set_origin(0.0, 0.0);
            self.content.begin_text();
            self.in_text = true;
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
        self.content.set_font(Name(&run.font_name), run.font_size);
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
                            self.content.show(Str(&run.bytes));
                        } else {
                            self.content
                                .show_positioned()
                                .items()
                                .adjust(adjustment as f32)
                                .show(Str(&run.bytes));
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
        self.content
            .set_text_matrix([run.horizontal_scale, 0.0, 0.0, 1.0, x, baseline]);
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
                serialized_x,
                None,
                0.0,
                text_unit,
                f64::from(self.origin.0),
            )
        } else {
            self.content.show(Str(&run.bytes));
            None
        };
        self.text_cursor = run.advance.map(|advance| PdfTextCursor {
            x: raster_cursor
                .map(|cursor| cursor.x)
                .unwrap_or(serialized_x + advance),
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
            let mut cursor = exact_cursor.unwrap_or(PdfExactTextCursor {
                tj_start_h: exact.serialized_h,
                delta_h: 0,
            });
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
            self.content.show(Str(&run.bytes));
            return Some(PdfRasteredTextCursor {
                x: cursor_x,
                exact: exact_cursor.take(),
            });
        }

        let mut operation = self.content.show_positioned();
        let mut items = operation.items();
        let mut string_start = 0;
        for (index, adjustment) in adjustments.iter().copied().enumerate() {
            if adjustment == 0.0 {
                continue;
            }
            if string_start < index {
                items.show(Str(&run.bytes[string_start..index]));
            }
            items.adjust(adjustment as f32);
            string_start = index;
        }
        if string_start < run.bytes.len() {
            items.show(Str(&run.bytes[string_start..]));
        }
        Some(PdfRasteredTextCursor {
            x: cursor_x,
            exact: exact_cursor,
        })
    }

    fn prepare_literal(&mut self, mode: crate::PdfLiteralMode, x: f32, y: f32) {
        if mode != crate::PdfLiteralMode::Direct {
            self.end_text();
        }
        if mode == crate::PdfLiteralMode::Origin {
            self.set_origin(x, y);
        }
    }

    fn xobject(&mut self, mut matrix: [f32; 6], name: &[u8]) {
        self.end_text();
        (matrix[4], matrix[5]) = self.relative_position(matrix[4], matrix[5]);
        self.save();
        self.content.transform(matrix).x_object(Name(name));
        self.restore();
    }

    fn relative_position(&self, x: f32, y: f32) -> (f32, f32) {
        // pdfTeX §690 retains its translated PDF origin, so every later
        // absolute page position is emitted as a delta from that origin.
        (x - self.origin.0, y - self.origin.1)
    }

    fn set_origin(&mut self, x: f32, y: f32) {
        let (dx, dy) = (x - self.origin.0, y - self.origin.1);
        if dx != 0.0 || dy != 0.0 {
            self.content.transform([1.0, 0.0, 0.0, 1.0, dx, dy]);
            self.origin = (x, y);
        }
    }

    fn save(&mut self) {
        self.saved_origins.push(self.origin);
        self.content.save_state();
    }

    fn restore(&mut self) {
        self.content.restore_state();
        if let Some(origin) = self.saved_origins.pop() {
            self.origin = origin;
        }
    }

    fn end_text(&mut self) {
        if self.in_text {
            self.content.end_text();
            self.in_text = false;
        }
        self.text_cursor = None;
    }

    fn finish(mut self) -> Vec<u8> {
        self.end_text();
        self.content.finish().to_vec()
    }
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
