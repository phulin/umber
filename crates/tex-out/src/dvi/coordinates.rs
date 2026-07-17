//! Exact coordinate oracle recorded by the canonical DVI traversal.

use tex_arith::Scaled;

use crate::positioned::{BoxKind, PositionedEvent, PositionedPage, TextUnit};
use crate::{PageArtifact, PageNode};

use super::{DviError, DviWriter};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DviCoordinateEvent {
    Box {
        vertical: bool,
        x: Scaled,
        y: Scaled,
        width: Scaled,
        height: Scaled,
        baseline: Scaled,
    },
    Glyph {
        x: Scaled,
        baseline: Scaled,
        font_id: u32,
        source_codes: Vec<u8>,
    },
    Rule {
        x: Scaled,
        y: Scaled,
        width: Scaled,
        height: Scaled,
    },
    Special {
        x: Scaled,
        y: Scaled,
        payload: Vec<u8>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoordinateError {
    Dvi(DviError),
    Mismatch {
        page: u32,
        event: usize,
        message: String,
    },
}

impl std::fmt::Display for CoordinateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dvi(error) => error.fmt(f),
            Self::Mismatch {
                page,
                event,
                message,
            } => write!(f, "page {page} coordinate event {event}: {message}"),
        }
    }
}

impl std::error::Error for CoordinateError {}

impl From<DviError> for CoordinateError {
    fn from(value: DviError) -> Self {
        Self::Dvi(value)
    }
}

pub fn trace_page(page: &PageArtifact) -> Result<Vec<DviCoordinateEvent>, DviError> {
    let mut writer = DviWriter::new(Vec::new());
    writer.coordinate_trace = Some(Vec::new());
    writer.index_page_fonts(page)?;
    writer.reset_page_state();
    writer.ship_box(page, &page.root)?;
    Ok(writer
        .coordinate_trace
        .take()
        .expect("coordinate tracing enabled"))
}

/// Compares the driver-neutral stream with the canonical DVI traversal.
///
/// All page, box, rule, special, and text-run anchor coordinates are exact.
/// Glyph positions after a run's first glyph and run width are excluded.
pub fn compare_page(
    artifact: &PageArtifact,
    positioned: &PositionedPage,
) -> Result<(), CoordinateError> {
    compare_media(artifact, positioned)?;
    let dvi = trace_page(artifact)?;
    let mut cursor = 0usize;
    for (ordinal, event) in positioned.events.iter().enumerate() {
        match event {
            PositionedEvent::Box(actual) => {
                let wanted = DviCoordinateEvent::Box {
                    vertical: actual.kind == BoxKind::Vertical,
                    x: actual.x,
                    y: actual.y,
                    width: actual.width,
                    height: actual.height,
                    baseline: actual.baseline,
                };
                compare_event(positioned, ordinal, dvi.get(cursor), &wanted, "box")?;
                cursor += 1;
            }
            PositionedEvent::Rule(actual) => {
                let wanted = DviCoordinateEvent::Rule {
                    x: actual.x,
                    y: actual.y,
                    width: actual.width,
                    height: actual.height,
                };
                compare_event(positioned, ordinal, dvi.get(cursor), &wanted, "rule")?;
                cursor += 1;
            }
            PositionedEvent::Special(actual) => {
                let wanted = DviCoordinateEvent::Special {
                    x: actual.x,
                    y: actual.y,
                    payload: actual.payload.clone(),
                };
                compare_event(positioned, ordinal, dvi.get(cursor), &wanted, "special")?;
                cursor += 1;
            }
            PositionedEvent::PdfGraphics(_) => {}
            PositionedEvent::TextRun(actual) => {
                let first_code_x = actual
                    .units
                    .iter()
                    .position(|unit| matches!(unit, TextUnit::Code(_)))
                    .map(|index| actual.positions[index]);
                let wanted_codes = actual
                    .units
                    .iter()
                    .filter_map(|unit| match unit {
                        TextUnit::Code(code) => Some(*code),
                        TextUnit::Space => None,
                    })
                    .collect::<Vec<_>>();
                let mut found_codes = Vec::new();
                let mut first = true;
                while found_codes.len() < wanted_codes.len() {
                    let Some(DviCoordinateEvent::Glyph {
                        x,
                        baseline,
                        font_id,
                        source_codes,
                    }) = dvi.get(cursor)
                    else {
                        return mismatch(positioned, ordinal, "DVI text sequence ended early");
                    };
                    if *font_id != actual.font_id {
                        return mismatch(positioned, ordinal, "DVI text font differs");
                    }
                    if first
                        && (*x != first_code_x.expect("nonempty code sequence has an anchor")
                            || *baseline != actual.baseline)
                    {
                        return mismatch(
                            positioned,
                            ordinal,
                            format!(
                                "text anchor differs: DVI=({}, {}), HTML=({}, {})",
                                x.raw(),
                                baseline.raw(),
                                first_code_x
                                    .expect("nonempty code sequence has an anchor")
                                    .raw(),
                                actual.baseline.raw()
                            ),
                        );
                    }
                    first = false;
                    found_codes.extend_from_slice(source_codes);
                    cursor += 1;
                }
                if found_codes != wanted_codes {
                    return mismatch(positioned, ordinal, "browser-shaped source codes differ");
                }
            }
            PositionedEvent::PdfAccessibility(_) => {}
            PositionedEvent::PdfAnnotation(_) | PositionedEvent::PdfDestination(_) => {}
            PositionedEvent::BoxEnd(_) => {}
            PositionedEvent::PdfThread(_) | PositionedEvent::PdfEndThread { .. } => {}
        }
    }
    if cursor != dvi.len() {
        return mismatch(
            positioned,
            positioned.events.len(),
            format!("DVI has {} trailing coordinate events", dvi.len() - cursor),
        );
    }
    Ok(())
}

fn compare_media(
    artifact: &PageArtifact,
    positioned: &PositionedPage,
) -> Result<(), CoordinateError> {
    let root = match &artifact.root {
        PageNode::HList(root) | PageNode::VList(root) => root,
        _ => unreachable!("validated page root is a box"),
    };
    let width = if artifact.job.page_width.raw() > 0 {
        artifact.job.page_width.raw()
    } else {
        let inset = artifact
            .job
            .page_origin_x
            .checked_add(artifact.job.h_offset)
            .ok_or(DviError::PositionOverflow)?;
        inset
            .checked_add(root.width)
            .and_then(|value| value.checked_add(inset))
            .ok_or(DviError::PositionOverflow)?
            .raw()
            .max(0)
    };
    let height = if artifact.job.page_height.raw() > 0 {
        artifact.job.page_height.raw()
    } else {
        let inset = artifact
            .job
            .page_origin_y
            .checked_add(artifact.job.v_offset)
            .ok_or(DviError::PositionOverflow)?;
        inset
            .checked_add(root.height)
            .and_then(|value| value.checked_add(root.depth))
            .and_then(|value| value.checked_add(inset))
            .ok_or(DviError::PositionOverflow)?
            .raw()
            .max(0)
    };
    if positioned.width.raw() != width
        || positioned.height.raw() != height
        || positioned.mag != artifact.job.mag
    {
        return mismatch(positioned, 0, "page media box or magnification differs");
    }
    Ok(())
}

fn compare_event(
    page: &PositionedPage,
    ordinal: usize,
    dvi: Option<&DviCoordinateEvent>,
    html: &DviCoordinateEvent,
    kind: &str,
) -> Result<(), CoordinateError> {
    if dvi != Some(html) {
        return mismatch(
            page,
            ordinal,
            format!("{kind} differs: DVI={dvi:?}, HTML={html:?}"),
        );
    }
    Ok(())
}

fn mismatch<T>(
    page: &PositionedPage,
    event: usize,
    message: impl Into<String>,
) -> Result<T, CoordinateError> {
    Err(CoordinateError::Mismatch {
        page: page.page_index,
        event,
        message: message.into(),
    })
}
