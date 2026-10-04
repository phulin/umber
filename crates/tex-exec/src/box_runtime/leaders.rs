//! Source-free leader payload and contribution runtime.

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::glue::GlueSpec;
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

#[allow(clippy::too_many_arguments)] // Leader contribution keeps mode, glue, fuel, and diagnostics independent.
pub(crate) fn append_leader_contribution<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    kind: GlueKind,
    payload: LeaderPayload,
    spec: GlueSpec,
    origin: tex_state::node::GlueSpecOrigin,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    append_node_to_current_list(
        nest,
        stores,
        diagnostic_effects,
        Node::Glue {
            origin,
            spec,
            kind,
            leader: Some(payload),
        },
        fuel,
    )?;
    Ok(())
}
