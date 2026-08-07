//! Source-free leader payload and contribution runtime.

use tex_state::Universe;
use tex_state::ids::GlueId;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{GlueKind, LeaderPayload, Node};

use super::append_node_to_current_list;
use crate::{ExecError, ModeNest};

pub(crate) fn payload_from_node(node: Node) -> Option<LeaderPayload> {
    match node {
        Node::HList(node) => Some(LeaderPayload::HList(node)),
        Node::VList(node) => Some(LeaderPayload::VList(node)),
        Node::Rule {
            width,
            height,
            depth,
        } => Some(LeaderPayload::Rule {
            width,
            height,
            depth,
        }),
        _ => None,
    }
}

pub(crate) fn leader_glue_kind(primitive: UnexpandablePrimitive) -> GlueKind {
    match primitive {
        UnexpandablePrimitive::Leaders => GlueKind::Leaders,
        UnexpandablePrimitive::CLeaders => GlueKind::Cleaders,
        UnexpandablePrimitive::XLeaders => GlueKind::Xleaders,
        _ => unreachable!("caller restricts leader primitives"),
    }
}

pub(crate) fn take_register_payload(
    stores: &mut Universe,
    index: u16,
    copy: bool,
) -> Option<LeaderPayload> {
    let id = if copy {
        stores.box_reg(index)
    } else {
        stores.take_box_reg_same_level(index)
    };
    if copy && let Some(id) = id {
        stores.pin_survivor(id);
    }
    id.and_then(|id| stores.nodes(id).first().map(|node| node.to_owned()))
        .and_then(payload_from_node)
}

pub(crate) fn append_leader_contribution(
    nest: &mut ModeNest,
    stores: &mut Universe,
    kind: GlueKind,
    payload: LeaderPayload,
    spec: GlueId,
    fuel: &mut tex_command::CommandFuel,
    error_context: &str,
) -> Result<(), ExecError> {
    append_node_to_current_list(
        nest,
        stores,
        Node::Glue {
            spec,
            kind,
            leader: Some(payload),
        },
        fuel,
    )?;
    crate::vertical::build_page_if_outer_vertical_with_error_context(nest, stores, error_context)?;
    Ok(())
}
