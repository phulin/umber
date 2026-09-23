//! Threads lowering for detached PDF navigation.

use super::*;
pub(in crate::pdf::finalize) struct ThreadOutput {
    pub(in crate::pdf::finalize) objects: Vec<PdfIndirectObject>,
    pub(in crate::pdf::finalize) list: Option<PdfObjectId>,
    pub(in crate::pdf::finalize) page_beads: Vec<Vec<PdfObjectId>>,
}

#[derive(Clone)]
pub(in crate::pdf::finalize) struct ShippedBead {
    thread: PdfObjectId,
    bead: PdfObjectId,
    rectangle: PdfObjectId,
    page: PdfObjectId,
    rect: ShippedAnnotationRect,
    attributes: Vec<u8>,
    title: Vec<u8>,
    margin: Scaled,
}

pub(in crate::pdf::finalize) fn thread_objects(
    thread_records: &[crate::pdf::PdfThreadInput],
    pages: &[PositionedPage],
    page_records: &[crate::pdf::PdfCommittedPageInput],
    decimal_digits: i32,
    next_object: &mut u32,
) -> Result<ThreadOutput, PdfBuildError> {
    let mut thread_beads = BTreeMap::<u32, BTreeSet<(u32, u32)>>::new();
    for thread in thread_records {
        if thread_beads.contains_key(&thread.object()) {
            return Err(PdfBuildError::DuplicateThreadObject(thread.object()));
        }
        thread_beads.insert(
            thread.object(),
            thread
                .beads()
                .iter()
                .map(|bead| (bead.bead_object(), bead.rectangle_object()))
                .collect(),
        );
    }
    let mut beads = Vec::<ShippedBead>::new();
    let mut shipped_beads = BTreeSet::new();
    let mut page_beads = vec![Vec::new(); pages.len()];
    for (page_index, (page, record)) in pages.iter().zip(page_records).enumerate() {
        let mut boxes = BTreeMap::<u32, PositionedBox>::new();
        let mut running_bead: Option<usize> = None;
        let mut running_parent_depth = None;
        for event in &page.events {
            match event {
                PositionedEvent::Box(positioned) => {
                    boxes.insert(positioned.id, *positioned);
                    if running_parent_depth.is_some_and(|depth| positioned.depth == depth + 1)
                        && positioned.kind == BoxKind::Vertical
                        && let Some(previous) = running_bead
                    {
                        let bead = object_id(*next_object)?;
                        *next_object = next_object
                            .checked_add(1)
                            .ok_or(PdfBuildError::ObjectCapacity)?;
                        let rectangle = object_id(*next_object)?;
                        *next_object = next_object
                            .checked_add(1)
                            .ok_or(PdfBuildError::ObjectCapacity)?;
                        let source = beads[previous].clone();
                        page_beads[page_index].push(bead);
                        beads.push(ShippedBead {
                            thread: source.thread,
                            bead,
                            rectangle,
                            page: source.page,
                            rect: marker_rect(
                                positioned.x,
                                positioned.baseline,
                                *positioned,
                                crate::pdf::PdfAnnotationDimensionsInput {
                                    width: None,
                                    height: None,
                                    depth: None,
                                },
                                source.margin,
                            )?,
                            attributes: Vec::new(),
                            title: source.title,
                            margin: source.margin,
                        });
                        running_bead = Some(beads.len() - 1);
                    }
                }
                PositionedEvent::PdfThread(positioned) => {
                    let marker = &positioned.marker;
                    let thread = object_id(marker.thread_object)?;
                    let bead = object_id(marker.bead_object)?;
                    let rectangle = object_id(marker.rectangle_object)?;
                    let Some(owned_beads) = thread_beads.get(&marker.thread_object) else {
                        return Err(PdfBuildError::MissingThreadRecord(marker.thread_object));
                    };
                    if !owned_beads.contains(&(marker.bead_object, marker.rectangle_object)) {
                        return Err(PdfBuildError::ThreadBeadOwnership {
                            thread: marker.thread_object,
                            bead: marker.bead_object,
                            rectangle: marker.rectangle_object,
                        });
                    }
                    if !shipped_beads.insert(marker.bead_object) {
                        return Err(PdfBuildError::DuplicateThreadBead(marker.bead_object));
                    }
                    let positioned_box = boxes.get(&positioned.containing_box).copied().ok_or(
                        PdfBuildError::MissingThreadContainingBox(positioned.containing_box),
                    )?;
                    let dimensions = crate::pdf::PdfAnnotationDimensionsInput {
                        width: marker.width,
                        height: marker.height,
                        depth: marker.depth,
                    };
                    let rect = marker_rect(
                        positioned.x,
                        positioned.y,
                        positioned_box,
                        dimensions,
                        marker.margin,
                    )?;
                    let title = match &marker.identifier {
                        crate::PdfDestinationIdentifier::Name(name) => name.clone(),
                        crate::PdfDestinationIdentifier::Number(number) => {
                            number.to_string().into_bytes()
                        }
                    };
                    page_beads[page_index].push(bead);
                    beads.push(ShippedBead {
                        thread,
                        bead,
                        rectangle,
                        page: object_id(record.page_object())?,
                        rect,
                        attributes: marker.attributes.clone(),
                        title,
                        margin: marker.margin,
                    });
                    running_bead = positioned.running.then_some(beads.len() - 1);
                    running_parent_depth = positioned.running.then_some(positioned_box.depth);
                }
                PositionedEvent::PdfEndThread { y, .. } => {
                    let index = running_bead
                        .take()
                        .ok_or(PdfBuildError::UnmatchedThreadEnd { page: page_index })?;
                    beads[index].rect.bottom = y
                        .checked_add(beads[index].margin)
                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                    running_parent_depth = None;
                }
                _ => {}
            }
        }
        if let Some(index) = running_bead {
            return Err(PdfBuildError::UnfinishedThread {
                page: page_index,
                thread: beads[index].thread.get(),
            });
        }
    }
    if let Some((page, page_record)) = pages.first().zip(page_records.first()) {
        for thread in thread_records {
            let thread_id = object_id(thread.object())?;
            if beads.iter().any(|bead| bead.thread == thread_id) {
                continue;
            }
            let bead = object_id(*next_object)?;
            *next_object = next_object
                .checked_add(1)
                .ok_or(PdfBuildError::ObjectCapacity)?;
            let rectangle = object_id(*next_object)?;
            *next_object = next_object
                .checked_add(1)
                .ok_or(PdfBuildError::ObjectCapacity)?;
            page_beads[0].push(bead);
            let title = match thread.identity() {
                crate::pdf::PdfDestinationIdentityInput::Name(name)
                | crate::pdf::PdfDestinationIdentityInput::Raw(name) => name.clone(),
                crate::pdf::PdfDestinationIdentityInput::Number(number) => {
                    number.to_string().into_bytes()
                }
            };
            beads.push(ShippedBead {
                thread: thread_id,
                bead,
                rectangle,
                page: object_id(page_record.page_object())?,
                rect: ShippedAnnotationRect {
                    left: Scaled::from_raw(0),
                    bottom: Scaled::from_raw(0),
                    right: page.width,
                    top: page.height,
                },
                attributes: Vec::new(),
                title,
                margin: Scaled::from_raw(0),
            });
        }
    }
    if beads.is_empty() {
        return Ok(ThreadOutput {
            objects: Vec::new(),
            list: None,
            page_beads,
        });
    }
    let mut by_thread = BTreeMap::<PdfObjectId, Vec<usize>>::new();
    for (index, bead) in beads.iter().enumerate() {
        by_thread.entry(bead.thread).or_default().push(index);
    }
    let list = object_id(*next_object)?;
    *next_object = next_object
        .checked_add(1)
        .ok_or(PdfBuildError::ObjectCapacity)?;
    let mut objects = vec![PdfIndirectObject {
        id: list,
        object: PdfObject::ThreadList(by_thread.keys().copied().collect()),
    }];
    for (&thread, indices) in &by_thread {
        let attributes = indices
            .iter()
            .rev()
            .find_map(|&index| {
                (!beads[index].attributes.is_empty()).then(|| beads[index].attributes.clone())
            })
            .unwrap_or_default();
        let default_title = attributes.is_empty().then(|| {
            let mut title = vec![b'('];
            title.extend_from_slice(&beads[indices[0]].title);
            title.push(b')');
            title
        });
        objects.push(PdfIndirectObject {
            id: thread,
            object: PdfObject::Thread(PdfThreadObject {
                first_bead: beads[indices[0]].bead,
                default_title,
                raw_entries: attributes,
            }),
        });
        for (position, &index) in indices.iter().enumerate() {
            let bead = &beads[index];
            let previous = beads[indices[(position + indices.len() - 1) % indices.len()]].bead;
            let next = beads[indices[(position + 1) % indices.len()]].bead;
            objects.push(PdfIndirectObject {
                id: bead.bead,
                object: PdfObject::Bead(PdfBeadObject {
                    thread: (position == 0).then_some(thread),
                    previous,
                    next,
                    page: bead.page,
                    rectangle: bead.rectangle,
                }),
            });
            let page_index = page_records
                .iter()
                .position(|record| object_id(record.page_object()).ok() == Some(bead.page))
                .expect("bead page belongs to page ledger");
            let page_height = pages[page_index].height;
            let rect = &bead.rect;
            objects.push(PdfIndirectObject {
                id: bead.rectangle,
                object: PdfObject::Value(PdfValue::Array(vec![
                    PdfValue::Number(scaled_to_bp_number(rect.left, decimal_digits)?),
                    PdfValue::Number(scaled_to_bp_number(
                        page_height
                            .checked_sub(rect.bottom)
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        decimal_digits,
                    )?),
                    PdfValue::Number(scaled_to_bp_number(rect.right, decimal_digits)?),
                    PdfValue::Number(scaled_to_bp_number(
                        page_height
                            .checked_sub(rect.top)
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        decimal_digits,
                    )?),
                ])),
            });
        }
    }
    Ok(ThreadOutput {
        objects,
        list: Some(list),
        page_beads,
    })
}
