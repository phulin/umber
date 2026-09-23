//! Annotations lowering for detached PDF navigation.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::pdf::finalize) struct ShippedAnnotation {
    pub(in crate::pdf::finalize) source_object: u32,
    pub(in crate::pdf::finalize) object: u32,
    pub(in crate::pdf::finalize) kind: ShippedAnnotationKind,
    pub(in crate::pdf::finalize) rect: ShippedAnnotationRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::pdf::finalize) enum ShippedAnnotationKind {
    Annotation,
    Link,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::pdf::finalize) struct ShippedAnnotationRect {
    pub(in crate::pdf::finalize) left: Scaled,
    pub(in crate::pdf::finalize) top: Scaled,
    pub(in crate::pdf::finalize) right: Scaled,
    pub(in crate::pdf::finalize) bottom: Scaled,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::pdf::finalize) struct ActiveShippedLink<'a> {
    record: &'a crate::pdf::PdfLinkInput,
    depth: u32,
    candidate: Option<(u32, Scaled)>,
}

pub(in crate::pdf::finalize) fn lower_page_annotations(
    stores: &PdfFinalizationInput,
    pages: &[PositionedPage],
    link_margins: &[Scaled],
) -> Result<Vec<Vec<ShippedAnnotation>>, PdfBuildError> {
    let annotations = stores
        .pdf_annotations()
        .iter()
        .map(|record| (record.object(), record))
        .collect::<BTreeMap<_, _>>();
    let links = stores
        .pdf_links()
        .iter()
        .map(|record| (record.object(), record))
        .collect::<BTreeMap<_, _>>();
    let mut active = Vec::<ActiveShippedLink<'_>>::new();
    let mut result = Vec::with_capacity(pages.len());
    // pdftex.web §1597 initializes `gen_running_link` once, while
    // §§37031–37034/37116–37119 mutate it as ordered whatsits are shipped.
    // It therefore persists across pages rather than resetting per shipout.
    let mut running = true;

    for (page, link_margin) in pages.iter().zip(link_margins.iter().copied()) {
        let mut shipped = Vec::new();
        let mut boxes = BTreeMap::<u32, PositionedBox>::new();
        for event in &page.events {
            match event {
                PositionedEvent::Box(positioned_box) => {
                    boxes.insert(positioned_box.id, *positioned_box);
                    if running && positioned_box.kind == BoxKind::Horizontal {
                        for link in &mut active {
                            if link.depth == positioned_box.depth
                                && link.record.dimensions().width.is_none()
                            {
                                link.candidate = Some((positioned_box.id, positioned_box.x));
                            }
                        }
                    }
                }
                PositionedEvent::BoxEnd(end) => {
                    let positioned_box = boxes[&end.id];
                    for link in &mut active {
                        if let Some((box_id, left)) = link.candidate
                            && box_id == end.id
                        {
                            shipped.push(link_segment(
                                link.record,
                                positioned_box,
                                left,
                                positioned_box
                                    .x
                                    .checked_add(positioned_box.width)
                                    .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                link_margin,
                            )?);
                            link.candidate = None;
                        }
                    }
                }
                PositionedEvent::PdfAnnotation(marker) => {
                    let positioned_box = boxes[&marker.containing_box];
                    match marker.marker {
                        crate::PdfAnnotationEffect::Annotation { object } => {
                            let record = annotations
                                .get(&object)
                                .copied()
                                .ok_or(PdfBuildError::MissingAnnotationRecord(object))?;
                            let data = record
                                .data
                                .as_ref()
                                .ok_or(PdfBuildError::UninitializedAnnotation(object))?;
                            shipped.push(ShippedAnnotation {
                                source_object: object,
                                object,
                                kind: ShippedAnnotationKind::Annotation,
                                rect: marker_rect(
                                    marker.x,
                                    marker.y,
                                    positioned_box,
                                    data.0,
                                    Scaled::from_raw(0),
                                )?,
                            });
                        }
                        crate::PdfAnnotationEffect::LinkStart { object } => {
                            let record = links
                                .get(&object)
                                .copied()
                                .ok_or(PdfBuildError::MissingLinkRecord(object))?;
                            let mut link = ActiveShippedLink {
                                record,
                                depth: marker.depth,
                                candidate: None,
                            };
                            if let Some(width) = record.dimensions().width {
                                shipped.push(link_segment(
                                    record,
                                    positioned_box,
                                    marker.x,
                                    marker
                                        .x
                                        .checked_add(width)
                                        .ok_or(PdfBuildError::PageGeometryOverflow)?,
                                    link_margin,
                                )?);
                            } else {
                                link.candidate = Some((marker.containing_box, marker.x));
                            }
                            active.push(link);
                        }
                        crate::PdfAnnotationEffect::LinkEnd { object } => {
                            let index = active
                                .iter()
                                .rposition(|link| link.record.object() == object)
                                .ok_or(PdfBuildError::MissingOpenLink(object))?;
                            let link = active.remove(index);
                            if link.record.dimensions().width.is_none() {
                                let left = link
                                    .candidate
                                    .filter(|(box_id, _)| *box_id == marker.containing_box)
                                    .map_or(positioned_box.x, |(_, left)| left);
                                shipped.push(link_segment(
                                    link.record,
                                    positioned_box,
                                    left,
                                    marker.x,
                                    link_margin,
                                )?);
                            }
                        }
                        crate::PdfAnnotationEffect::RunningLink(enabled) => running = enabled,
                    }
                }
                PositionedEvent::TextRun(_)
                | PositionedEvent::Rule(_)
                | PositionedEvent::Special(_)
                | PositionedEvent::PdfAccessibility(_)
                | PositionedEvent::PdfGraphics(_)
                | PositionedEvent::PdfDestination(_)
                | PositionedEvent::PdfThread(_)
                | PositionedEvent::PdfEndThread { .. } => {}
            }
        }
        result.push(shipped);
    }
    Ok(result)
}

pub(in crate::pdf::finalize) fn link_segment(
    record: &crate::pdf::PdfLinkInput,
    positioned_box: PositionedBox,
    left: Scaled,
    right: Scaled,
    margin: Scaled,
) -> Result<ShippedAnnotation, PdfBuildError> {
    let dimensions = record.dimensions();
    let baseline = positioned_box.baseline;
    Ok(ShippedAnnotation {
        source_object: record.object(),
        object: record.object(),
        kind: ShippedAnnotationKind::Link,
        rect: marker_rect_with_right(left, right, baseline, positioned_box, dimensions, margin)?,
    })
}

pub(in crate::pdf::finalize) fn marker_rect(
    left: Scaled,
    baseline: Scaled,
    positioned_box: PositionedBox,
    dimensions: crate::pdf::PdfAnnotationDimensionsInput,
    margin: Scaled,
) -> Result<ShippedAnnotationRect, PdfBuildError> {
    let right = left
        .checked_add(dimensions.width.unwrap_or_else(|| {
            positioned_box
                .x
                .checked_add(positioned_box.width)
                .and_then(|right| right.checked_sub(left))
                .unwrap_or(Scaled::from_raw(0))
        }))
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    marker_rect_with_right(left, right, baseline, positioned_box, dimensions, margin)
}

pub(in crate::pdf::finalize) fn marker_rect_with_right(
    left: Scaled,
    right: Scaled,
    baseline: Scaled,
    positioned_box: PositionedBox,
    dimensions: crate::pdf::PdfAnnotationDimensionsInput,
    margin: Scaled,
) -> Result<ShippedAnnotationRect, PdfBuildError> {
    let top = match dimensions.height {
        Some(height) => baseline
            .checked_sub(height)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
        None => positioned_box.y,
    };
    let bottom = match dimensions.depth {
        Some(depth) => baseline
            .checked_add(depth)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
        None => positioned_box
            .y
            .checked_add(positioned_box.height)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
    };
    Ok(ShippedAnnotationRect {
        left: left
            .checked_sub(margin)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
        top: top
            .checked_sub(margin)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
        right: right
            .checked_add(margin)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
        bottom: bottom
            .checked_add(margin)
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
    })
}

pub(in crate::pdf::finalize) fn assign_annotation_objects(
    pages: &mut [Vec<ShippedAnnotation>],
    next_object: &mut u32,
) -> Result<(), PdfBuildError> {
    let mut used = BTreeSet::new();
    for annotation in pages.iter_mut().flatten() {
        annotation.object = if used.insert(annotation.source_object) {
            annotation.source_object
        } else {
            let object = *next_object;
            *next_object = next_object
                .checked_add(1)
                .ok_or(PdfBuildError::ObjectCapacity)?;
            object
        };
    }
    Ok(())
}

pub(in crate::pdf::finalize) fn annotation_object(
    stores: &PdfFinalizationInput,
    shipped: ShippedAnnotation,
    page: &crate::pdf::PdfCommittedPageInput,
    page_height: Scaled,
    pages: &[crate::pdf::PdfCommittedPageInput],
    decimal_digits: i32,
) -> Result<PdfIndirectObject, PdfBuildError> {
    let left = shipped
        .rect
        .left
        .checked_add(page.h_origin())
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let right = shipped
        .rect
        .right
        .checked_add(page.h_origin())
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let bottom = page_height
        .checked_sub(shipped.rect.bottom)
        .and_then(|value| value.checked_sub(page.v_origin()))
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let top = page_height
        .checked_sub(shipped.rect.top)
        .and_then(|value| value.checked_sub(page.v_origin()))
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let (subtype, action, raw_entries) = match shipped.kind {
        ShippedAnnotationKind::Annotation => {
            let record = stores
                .pdf_annotations()
                .iter()
                .find(|record| record.object() == shipped.source_object)
                .and_then(|record| record.data.as_ref())
                .ok_or(PdfBuildError::MissingAnnotationRecord(
                    shipped.source_object,
                ))?;
            (None, None, record.1.clone())
        }
        ShippedAnnotationKind::Link => {
            let record = stores
                .pdf_links()
                .iter()
                .find(|record| record.object() == shipped.source_object)
                .ok_or(PdfBuildError::MissingLinkRecord(shipped.source_object))?;
            let raw_entries = record.entries.clone();
            let action = detached_link_action(stores, &record.action, pages)?;
            let subtype = (!matches!(action, PdfAnnotationAction::UserEntries(_)))
                .then_some(PdfAnnotationType::Link);
            (subtype, Some(action), raw_entries)
        }
    };
    Ok(PdfIndirectObject {
        id: object_id(shipped.object)?,
        object: PdfObject::Annotation(PdfAnnotationObject {
            rect: [
                scaled_to_bp_number(left, decimal_digits)?,
                scaled_to_bp_number(bottom, decimal_digits)?,
                scaled_to_bp_number(right, decimal_digits)?,
                scaled_to_bp_number(top, decimal_digits)?,
            ],
            subtype,
            action,
            raw_entries,
        }),
    })
}

pub(in crate::pdf::finalize) fn detached_link_action(
    stores: &PdfFinalizationInput,
    spec: &crate::pdf::PdfActionInput,
    pages: &[crate::pdf::PdfCommittedPageInput],
) -> Result<PdfAnnotationAction, PdfBuildError> {
    let (kind, file, structure_identity, target_input, new_window) = match spec {
        crate::pdf::PdfActionInput::User(bytes) => {
            return Ok(PdfAnnotationAction::UserEntries(bytes.clone()));
        }
        crate::pdf::PdfActionInput::GoTo {
            file,
            structure,
            target,
            new_window,
        } => (
            PdfDestinationActionKind::GoTo,
            file,
            structure,
            target,
            *new_window,
        ),
        crate::pdf::PdfActionInput::Thread {
            file,
            structure,
            target,
            new_window,
        } => (
            PdfDestinationActionKind::Thread,
            file,
            structure,
            target,
            *new_window,
        ),
    };
    let external = file.is_some();
    let target = match target_input {
        crate::pdf::PdfActionTargetInput::Page { number, view } => {
            let page = if external {
                PdfDestinationPage::External(number.saturating_sub(1))
            } else {
                PdfDestinationPage::Internal(object_id(
                    pages
                        .get((*number - 1) as usize)
                        .ok_or(PdfBuildError::OpenActionPageNotFound(*number))?
                        .page_object(),
                )?)
            };
            PdfDestinationTarget::Page {
                page,
                view: view.clone(),
            }
        }
        crate::pdf::PdfActionTargetInput::Destination(
            crate::pdf::PdfDestinationIdentityInput::Name(name),
        )
        | crate::pdf::PdfActionTargetInput::Destination(
            crate::pdf::PdfDestinationIdentityInput::Raw(name),
        ) => PdfDestinationTarget::Name(name.clone()),
        crate::pdf::PdfActionTargetInput::Destination(
            crate::pdf::PdfDestinationIdentityInput::Number(number),
        ) => {
            if external {
                PdfDestinationTarget::Number(*number)
            } else {
                let identity = crate::pdf::PdfDestinationIdentityInput::Number(*number);
                PdfDestinationTarget::Reference(object_id(
                    if kind == PdfDestinationActionKind::Thread {
                        stores
                            .pdf_threads()
                            .iter()
                            .find(|thread| thread.identity() == &identity)
                            .expect("local numeric thread action reserves its thread")
                            .object()
                    } else {
                        stores
                            .pdf_destination(&identity, false)
                            .expect("local numeric action reserves its destination")
                            .object()
                    },
                )?)
            }
        }
    };
    let structure = structure_identity.as_ref().and_then(|identifier| {
        if external {
            Some(match identifier {
                crate::pdf::PdfDestinationIdentityInput::Name(bytes)
                | crate::pdf::PdfDestinationIdentityInput::Raw(bytes) => {
                    PdfDestinationStructure::External(bytes.clone())
                }
                crate::pdf::PdfDestinationIdentityInput::Number(number) => {
                    PdfDestinationStructure::External(number.to_string().into_bytes())
                }
            })
        } else {
            let identity = identifier.clone();
            stores
                .pdf_destination(&identity, true)
                .filter(|record| record.defined)
                .map(|record| {
                    PdfDestinationStructure::Internal(
                        object_id(record.object()).expect("valid reserved destination object"),
                    )
                })
        }
    });
    Ok(PdfAnnotationAction::Destination(PdfDestinationAction {
        kind,
        file: file.clone(),
        target,
        structure,
        new_window,
    }))
}
