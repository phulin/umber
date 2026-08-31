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
}

impl PdfPainter {
    fn new() -> Self {
        Self {
            content: pdf_writer::Content::new(),
            origin: (0.0, 0.0),
            saved_origins: Vec::new(),
            in_text: false,
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
            self.content.begin_text();
            self.in_text = true;
        }
        let (x, baseline) = self.relative_position(run.x, run.baseline);
        self.content
            .set_font(Name(&run.font_name), run.font_size)
            .set_text_matrix([run.horizontal_scale, 0.0, 0.0, 1.0, x, baseline])
            .show(Str(&run.bytes));
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
    }

    fn finish(mut self) -> Vec<u8> {
        self.end_text();
        self.content.finish().to_vec()
    }
}
