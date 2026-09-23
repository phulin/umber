//! Destinations lowering for detached PDF navigation.

use super::*;
#[derive(Clone, Debug)]
pub(in crate::pdf::finalize) struct ShippedDestination {
    object: u32,
    target: PdfObjectId,
    view: PdfDestinationView,
}

pub(in crate::pdf::finalize) struct OutlineObjects {
    pub(in crate::pdf::finalize) objects: Vec<PdfIndirectObject>,
    pub(in crate::pdf::finalize) root: Option<PdfObjectId>,
}

pub(in crate::pdf::finalize) fn outline_objects(
    stores: &PdfFinalizationInput,
    pages: &[crate::pdf::PdfCommittedPageInput],
    next_object: &mut u32,
) -> Result<OutlineObjects, PdfBuildError> {
    let records = stores.pdf_outlines();
    if records.is_empty() {
        return Ok(OutlineObjects {
            objects: Vec::new(),
            root: None,
        });
    }
    let root = object_id(*next_object)?;
    *next_object = next_object
        .checked_add(1)
        .ok_or(PdfBuildError::ObjectCapacity)?;
    let mut parents = vec![None; records.len()];
    let mut children = vec![Vec::new(); records.len()];
    let mut roots = Vec::new();
    let mut stack = Vec::<(usize, usize)>::new();
    for (index, record) in records.iter().enumerate() {
        while stack.last().is_some_and(|(_, remaining)| *remaining == 0) {
            stack.pop();
        }
        if let Some((parent, remaining)) = stack.last_mut() {
            parents[index] = Some(*parent);
            children[*parent].push(index);
            *remaining -= 1;
        } else {
            roots.push(index);
        }
        if record.count() != 0 {
            stack.push((index, record.count().unsigned_abs() as usize));
        }
    }
    while stack.last().is_some_and(|(_, remaining)| *remaining == 0) {
        stack.pop();
    }
    if let Some(&(parent, remaining)) = stack.last() {
        return Err(PdfBuildError::OutlineCountIncomplete {
            object: records[parent].item_object(),
            missing: remaining,
        });
    }
    let descendants = (0..records.len())
        .map(|index| outline_descendants(index, &children))
        .collect::<Vec<_>>();
    let visible_count: usize = roots
        .iter()
        .map(|&index| outline_visible(index, records, &children))
        .sum();
    let mut previous = vec![None; records.len()];
    let mut next = vec![None; records.len()];
    for siblings in std::iter::once(&roots).chain(children.iter()) {
        for pair in siblings.windows(2) {
            next[pair[0]] = Some(pair[1]);
            previous[pair[1]] = Some(pair[0]);
        }
    }
    let mut objects = Vec::with_capacity(records.len() * 3 + 1);
    for (index, record) in records.iter().enumerate() {
        objects.push(PdfIndirectObject {
            id: object_id(record.action_object())?,
            object: PdfObject::Action(detached_link_action(stores, record.action(), pages)?),
        });
        objects.push(PdfIndirectObject {
            id: object_id(record.title_object())?,
            object: PdfObject::PdfStringSyntax(record.title.clone()),
        });
        let child_ids =
            if let Some((&first, &last)) = children[index].first().zip(children[index].last()) {
                Some((
                    object_id(records[first].item_object())?,
                    object_id(records[last].item_object())?,
                ))
            } else {
                None
            };
        let signed_count = (!children[index].is_empty()).then(|| {
            let count = i32::try_from(descendants[index]).unwrap_or(i32::MAX);
            if record.count() < 0 { -count } else { count }
        });
        objects.push(PdfIndirectObject {
            id: object_id(record.item_object())?,
            object: PdfObject::OutlineItem(PdfOutlineItemObject {
                title: object_id(record.title_object())?,
                action: object_id(record.action_object())?,
                parent: parents[index]
                    .map_or(Ok(root), |parent| object_id(records[parent].item_object()))?,
                previous: previous[index]
                    .map(|sibling| object_id(records[sibling].item_object()))
                    .transpose()?,
                next: next[index]
                    .map(|sibling| object_id(records[sibling].item_object()))
                    .transpose()?,
                first: child_ids.map(|ids| ids.0),
                last: child_ids.map(|ids| ids.1),
                count: signed_count,
                raw_entries: record.entries.clone(),
            }),
        });
    }
    objects.push(PdfIndirectObject {
        id: root,
        object: PdfObject::Outline(PdfOutlineObject {
            first: object_id(records[*roots.first().expect("outline has root")].item_object())?,
            last: object_id(records[*roots.last().expect("outline has root")].item_object())?,
            visible_count: i32::try_from(visible_count).unwrap_or(i32::MAX),
        }),
    });
    Ok(OutlineObjects {
        objects,
        root: Some(root),
    })
}

pub(in crate::pdf::finalize) fn outline_descendants(
    index: usize,
    children: &[Vec<usize>],
) -> usize {
    children[index]
        .iter()
        .map(|&child| 1 + outline_descendants(child, children))
        .sum()
}

pub(in crate::pdf::finalize) fn outline_visible(
    index: usize,
    records: &[crate::pdf::PdfOutlineInput],
    children: &[Vec<usize>],
) -> usize {
    1 + if records[index].count() > 0 {
        children[index]
            .iter()
            .map(|&child| outline_visible(child, records, children))
            .sum()
    } else {
        0
    }
}

pub(in crate::pdf::finalize) fn lower_page_destinations(
    _input: &PdfFinalizationInput,
    records: &[crate::pdf::PdfCommittedPageInput],
    pages: &[PositionedPage],
    decimal_digits: i32,
) -> Result<Vec<ShippedDestination>, PdfBuildError> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for (page, record) in pages.iter().zip(records) {
        let artifact = PageArtifact::from_bytes(&record.artifact_bytes)?;
        let (_, page_height) = pdf_page_extents(&artifact, record)?;
        let page_object = object_id(record.page_object())?;
        let mut boxes = BTreeMap::new();
        for event in &page.events {
            match event {
                PositionedEvent::Box(positioned_box) => {
                    boxes.insert(positioned_box.id, *positioned_box);
                }
                PositionedEvent::PdfDestination(destination) => {
                    if !seen.insert(destination.marker.object) {
                        continue;
                    }
                    let target = destination
                        .marker
                        .structure
                        .map(object_id)
                        .transpose()?
                        .unwrap_or(page_object);
                    let x = destination
                        .x
                        .checked_add(record.h_origin())
                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                    let y = page_height
                        .checked_sub(destination.y)
                        .and_then(|value| value.checked_sub(record.v_origin()))
                        .ok_or(PdfBuildError::PageGeometryOverflow)?;
                    let number = |value| scaled_to_bp_number(value, decimal_digits);
                    let view = match destination.marker.kind {
                        crate::PdfDestinationKind::Xyz { zoom } => PdfDestinationView::Xyz {
                            left: number(x)?,
                            top: number(y)?,
                            zoom: zoom
                                .map(|zoom| PdfNumber::new(i64::from(zoom), 3))
                                .transpose()?,
                        },
                        crate::PdfDestinationKind::FitBoundingBoxHorizontal => {
                            PdfDestinationView::FitBoundingBoxHorizontal { top: number(y)? }
                        }
                        crate::PdfDestinationKind::FitBoundingBoxVertical => {
                            PdfDestinationView::FitBoundingBoxVertical { left: number(x)? }
                        }
                        crate::PdfDestinationKind::FitBoundingBox => {
                            PdfDestinationView::FitBoundingBox
                        }
                        crate::PdfDestinationKind::FitHorizontal => {
                            PdfDestinationView::FitHorizontal { top: number(y)? }
                        }
                        crate::PdfDestinationKind::FitVertical => {
                            PdfDestinationView::FitVertical { left: number(x)? }
                        }
                        crate::PdfDestinationKind::FitRectangle {
                            width,
                            height,
                            depth,
                        } => {
                            let positioned_box = boxes[&destination.containing_box];
                            let margin = destination.marker.margin;
                            let left = destination
                                .x
                                .checked_sub(margin)
                                .and_then(|value| value.checked_add(record.h_origin()))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let right = destination
                                .x
                                .checked_add(width.unwrap_or_else(|| {
                                    positioned_box
                                        .x
                                        .checked_add(positioned_box.width)
                                        .and_then(|right| right.checked_sub(destination.x))
                                        .unwrap_or(Scaled::from_raw(0))
                                }))
                                .and_then(|value| value.checked_add(margin))
                                .and_then(|value| value.checked_add(record.h_origin()))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let top_tex = height.map_or(positioned_box.y, |height| {
                                destination.y.checked_sub(height).unwrap_or(destination.y)
                            });
                            let bottom_tex = depth.map_or(
                                positioned_box
                                    .y
                                    .checked_add(positioned_box.height)
                                    .unwrap_or(positioned_box.y),
                                |depth| destination.y.checked_add(depth).unwrap_or(destination.y),
                            );
                            let top = page_height
                                .checked_sub(top_tex)
                                .and_then(|value| value.checked_sub(record.v_origin()))
                                .and_then(|value| value.checked_add(margin))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            let bottom = page_height
                                .checked_sub(bottom_tex)
                                .and_then(|value| value.checked_sub(record.v_origin()))
                                .and_then(|value| value.checked_sub(margin))
                                .ok_or(PdfBuildError::PageGeometryOverflow)?;
                            PdfDestinationView::FitRectangle {
                                left: number(left)?,
                                bottom: number(bottom)?,
                                right: number(right)?,
                                top: number(top)?,
                            }
                        }
                        crate::PdfDestinationKind::Fit => PdfDestinationView::Fit,
                    };
                    result.push(ShippedDestination {
                        object: destination.marker.object,
                        target,
                        view,
                    });
                }
                _ => {}
            }
        }
    }
    Ok(result)
}

pub(in crate::pdf::finalize) fn destination_objects(
    stores: &PdfFinalizationInput,
    pages: &[crate::pdf::PdfCommittedPageInput],
    shipped: Vec<ShippedDestination>,
    next_object: &mut u32,
) -> Result<DestinationObjects, PdfBuildError> {
    let first_page = pages
        .first()
        .map(|page| object_id(page.page_object()))
        .transpose()?;
    let shipped = shipped
        .into_iter()
        .map(|value| (value.object, value))
        .collect::<BTreeMap<_, _>>();
    let mut objects = Vec::new();
    let mut names = Vec::new();
    for record in stores.pdf_destinations(false) {
        let explicit = if let Some(value) = shipped.get(&record.object()) {
            PdfExplicitDestination {
                page: value.target,
                view: value.view.clone(),
            }
        } else if let Some(page) = first_page {
            PdfExplicitDestination {
                page,
                view: PdfDestinationView::Fit,
            }
        } else {
            continue;
        };
        let named = match record.identity() {
            crate::pdf::PdfDestinationIdentityInput::Name(name)
            | crate::pdf::PdfDestinationIdentityInput::Raw(name) => {
                names.push((decode_pdf_string(name), object_id(record.object())?));
                true
            }
            crate::pdf::PdfDestinationIdentityInput::Number(_) => false,
        };
        objects.push(PdfIndirectObject {
            id: object_id(record.object())?,
            object: if named {
                PdfObject::NamedDestination(explicit)
            } else {
                PdfObject::Destination(explicit)
            },
        });
    }
    for record in stores.pdf_destinations(true) {
        let Some(value) = shipped.get(&record.object()) else {
            continue;
        };
        objects.push(PdfIndirectObject {
            id: object_id(record.object())?,
            object: PdfObject::Destination(PdfExplicitDestination {
                page: value.target,
                view: value.view.clone(),
            }),
        });
    }
    names.sort_by(|left, right| left.0.cmp(&right.0));
    let (tree, root) = build_destination_name_tree(names, next_object)?;
    Ok(DestinationObjects {
        destinations: objects,
        name_tree: tree,
        name_tree_root: root,
    })
}

pub(in crate::pdf::finalize) struct DestinationObjects {
    pub(in crate::pdf::finalize) destinations: Vec<PdfIndirectObject>,
    pub(in crate::pdf::finalize) name_tree: Vec<PdfIndirectObject>,
    pub(in crate::pdf::finalize) name_tree_root: Option<PdfObjectId>,
}

pub(in crate::pdf::finalize) fn decode_pdf_string(source: &[u8]) -> Vec<u8> {
    if source.len() >= 2 && source[0] == b'<' && source[source.len() - 1] == b'>' {
        let hex = &source[1..source.len() - 1];
        if hex.iter().all(u8::is_ascii_hexdigit) {
            let mut result = Vec::with_capacity(hex.len().div_ceil(2));
            for pair in hex.chunks(2) {
                let high = (pair[0] as char).to_digit(16).expect("hex digit") as u8;
                let low = pair.get(1).map_or(0, |byte| {
                    (*byte as char).to_digit(16).expect("hex digit") as u8
                });
                result.push((high << 4) | low);
            }
            return result;
        }
    }
    let body = if source.len() >= 2 && source[0] == b'(' && source[source.len() - 1] == b')' {
        &source[1..source.len() - 1]
    } else {
        source
    };
    let mut result = Vec::with_capacity(body.len());
    let mut index = 0;
    while index < body.len() {
        if body[index] != b'\\' {
            result.push(body[index]);
            index += 1;
            continue;
        }
        index += 1;
        let Some(&escaped) = body.get(index) else {
            break;
        };
        if escaped.is_ascii_digit() && escaped < b'8' {
            let mut value = 0_u16;
            let mut count = 0;
            while count < 3 && index < body.len() && matches!(body[index], b'0'..=b'7') {
                value = value * 8 + u16::from(body[index] - b'0');
                index += 1;
                count += 1;
            }
            result.push(value as u8);
            continue;
        }
        match escaped {
            b'n' => result.push(b'\n'),
            b'r' => result.push(b'\r'),
            b't' => result.push(b'\t'),
            b'b' => result.push(8),
            b'f' => result.push(12),
            b'\n' => {}
            b'\r' => {
                if body.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
            }
            byte => result.push(byte),
        }
        index += 1;
    }
    result
}

pub(in crate::pdf::finalize) fn build_destination_name_tree(
    names: Vec<(Vec<u8>, PdfObjectId)>,
    next_object: &mut u32,
) -> Result<(Vec<PdfIndirectObject>, Option<PdfObjectId>), PdfBuildError> {
    if names.is_empty() {
        return Ok((Vec::new(), None));
    }
    let mut objects = Vec::new();
    let mut level = Vec::new();
    for chunk in names.chunks(6) {
        let id = object_id(*next_object)?;
        *next_object = next_object
            .checked_add(1)
            .ok_or(PdfBuildError::ObjectCapacity)?;
        let min = chunk.first().expect("nonempty chunk").0.clone();
        let max = chunk.last().expect("nonempty chunk").0.clone();
        objects.push(PdfIndirectObject {
            id,
            object: PdfObject::DestinationNameTree(PdfDestinationNameTree {
                limits: Some((min.clone(), max.clone())),
                children: PdfDestinationNameTreeChildren::Names(chunk.to_vec()),
            }),
        });
        level.push((id, min, max));
    }
    while level.len() > 1 {
        let mut parent = Vec::new();
        for chunk in level.chunks(6) {
            let id = object_id(*next_object)?;
            *next_object = next_object
                .checked_add(1)
                .ok_or(PdfBuildError::ObjectCapacity)?;
            let min = chunk.first().expect("nonempty chunk").1.clone();
            let max = chunk.last().expect("nonempty chunk").2.clone();
            objects.push(PdfIndirectObject {
                id,
                object: PdfObject::DestinationNameTree(PdfDestinationNameTree {
                    limits: Some((min.clone(), max.clone())),
                    children: PdfDestinationNameTreeChildren::Kids(
                        chunk.iter().map(|entry| entry.0).collect(),
                    ),
                }),
            });
            parent.push((id, min, max));
        }
        level = parent;
    }
    let root = level[0].0;
    Ok((objects, Some(root)))
}
