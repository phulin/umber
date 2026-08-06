use std::collections::BTreeMap;

use tex_arith::Scaled;

use crate::dvi::glue::adjusted_glue_width;
use crate::node_cursor::{ArtifactNodeCursor, ArtifactNodeEvent};
use crate::{BoxNode, GlueKind, PageEffect, PageNode};

pub(crate) const LEADER_ROUNDING_COMPENSATION: Scaled = Scaled::from_raw(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GeometryError {
    MissingEffect { effect_index: u32 },
    PositionOverflow,
}

pub(crate) fn add(left: Scaled, right: Scaled) -> Result<Scaled, GeometryError> {
    left.checked_add(right)
        .ok_or(GeometryError::PositionOverflow)
}

fn scaled(value: i64) -> Result<Scaled, GeometryError> {
    i32::try_from(value)
        .map(Scaled::from_raw)
        .map_err(|_| GeometryError::PositionOverflow)
}

/// Canonical artifact preorder ordinals used by every positioned sink.
pub(crate) struct NodeOrdinals(BTreeMap<usize, u32>);

impl NodeOrdinals {
    pub(crate) fn new(root: &PageNode) -> Self {
        let mut ordinals = BTreeMap::new();
        let mut next = 0_u32;
        for event in ArtifactNodeCursor::new(root) {
            if let ArtifactNodeEvent::Node { node, .. } = event {
                ordinals.insert(node as *const PageNode as usize, next);
                next = next
                    .checked_add(1)
                    .expect("validated artifact node count fits u32");
            }
        }
        Self(ordinals)
    }

    pub(crate) fn get(&self, node: &PageNode) -> u32 {
        self.0[&(node as *const PageNode as usize)]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LeaderMode {
    Aligned,
    Centered,
    Expanded,
}

impl LeaderMode {
    pub(crate) fn from_glue(kind: GlueKind) -> Option<Self> {
        match kind {
            GlueKind::Leaders => Some(Self::Aligned),
            GlueKind::Cleaders => Some(Self::Centered),
            GlueKind::Xleaders => Some(Self::Expanded),
            GlueKind::Normal
            | GlueKind::BaselineSkip
            | GlueKind::LineSkip
            | GlueKind::LeftSkip
            | GlueKind::RightSkip
            | GlueKind::ParFillSkip => None,
        }
    }
}

pub(crate) fn leader_start(
    kind: LeaderMode,
    cur: Scaled,
    origin: Scaled,
    available: Scaled,
    size: Scaled,
) -> Result<(Scaled, Scaled), GeometryError> {
    debug_assert!(available.raw() > 0);
    debug_assert!(size.raw() > 0);
    match kind {
        LeaderMode::Aligned => {
            let diff = i64::from(cur.raw()) - i64::from(origin.raw());
            let quotient = diff / i64::from(size.raw());
            let mut start = scaled(i64::from(origin.raw()) + i64::from(size.raw()) * quotient)?;
            if start.raw() < cur.raw() {
                start = add(start, size)?;
            }
            Ok((start, Scaled::from_raw(0)))
        }
        LeaderMode::Centered => Ok((
            add(cur, Scaled::from_raw(available.raw() % size.raw() / 2))?,
            Scaled::from_raw(0),
        )),
        LeaderMode::Expanded => {
            let quotient = i64::from(available.raw() / size.raw());
            let remainder = i64::from(available.raw() % size.raw());
            let extra = remainder / (quotient + 1);
            Ok((
                add(cur, scaled((remainder - (quotient - 1) * extra) / 2)?)?,
                scaled(extra)?,
            ))
        }
    }
}

pub(crate) fn predict_snap_correction(
    following: &[PageNode],
    effects: &[PageEffect],
    this_box: &BoxNode,
    mut current: Scaled,
    mut reference: (Scaled, Scaled),
    mut cur_g: Scaled,
    mut cur_glue: Scaled,
) -> Result<Option<Scaled>, GeometryError> {
    for child in following {
        match child {
            PageNode::HList(node) | PageNode::VList(node) => {
                current = add(current, add(node.height, node.depth)?)?;
            }
            PageNode::Rule { height, depth, .. } => {
                current = add(
                    current,
                    add(
                        height.unwrap_or(Scaled::from_raw(0)),
                        depth.unwrap_or(Scaled::from_raw(0)),
                    )?,
                )?;
            }
            PageNode::Glue { spec, .. } => {
                let width = adjusted_glue_width(
                    *spec,
                    this_box.glue_sign,
                    this_box.glue_order,
                    this_box.glue_set,
                    &mut cur_glue,
                    &mut cur_g,
                )
                .map_err(|_| GeometryError::PositionOverflow)?;
                current = add(current, width)?;
            }
            PageNode::Kern { amount, .. } | PageNode::MarginKern { amount, .. } => {
                current = add(current, *amount)?;
            }
            PageNode::WhatsitAnchor { effect_index } => {
                let effect =
                    effects
                        .get(*effect_index as usize)
                        .ok_or(GeometryError::MissingEffect {
                            effect_index: *effect_index,
                        })?;
                match effect {
                    PageEffect::PdfSnapRefPoint => reference.1 = current,
                    PageEffect::PdfSnapY { spec } => {
                        return Ok(crate::snapping::correction(current, reference.1, *spec));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(None)
}
