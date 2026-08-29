//! Source-free leader payload and contribution runtime.

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::glue::GlueSpec;
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

pub(crate) fn take_register_payload<G>(
    stores: &mut CommandContext<'_, G>,
    index: u16,
    copy: bool,
) -> Option<LeaderPayload> {
    let owner = if copy {
        stores.copy_box_to_page(index)
    } else {
        stores.take_box_to_page(index)
    };
    owner
        .and_then(|owner| {
            stores
                .page_node_list(owner)
                .ok()?
                .get(0)
                .map(|node| node.to_owned_with(|id| id))
        })
        .and_then(payload_from_node)
}

#[allow(clippy::too_many_arguments)] // Leader contribution keeps mode, glue, fuel, and diagnostics independent.
pub(crate) fn append_leader_contribution<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    kind: GlueKind,
    payload: LeaderPayload,
    spec: GlueSpec,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    append_node_to_current_list(
        nest,
        stores,
        diagnostic_effects,
        Node::Glue {
            spec,
            kind,
            leader: Some(payload),
        },
        fuel,
    )?;
    Ok(())
}
