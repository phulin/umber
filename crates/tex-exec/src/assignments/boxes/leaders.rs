use tex_expand::ExpansionHooks;
use tex_lex::{InputSource, InputStack};
use tex_state::Universe;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::GlueId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::{GlueKind, LeaderPayload, Node};
use tex_state::token::{Token, TracedTokenWord};

use crate::{ExecError, Mode};

use super::super::{
    infinite_glue, next_non_space_traced_x, push_tokens, push_traced_tokens, scan_glue_id,
    scan_register_index, scan_rule_node,
};
use super::packaging::{first_box_node, kind_for_primitive, scan_box_node};
use super::vsplit::scan_vsplit_node;

pub(super) fn scan_leader_payload<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<LeaderPayload, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let traced = next_non_space_traced_x(input, stores, hooks)?
        .ok_or(ExecError::MissingLeaderPayload { context })?;
    let token = tex_expand::semantic_token(traced);
    let Token::Cs(symbol) = token else {
        push_traced_tokens(input, stores, [traced]);
        return Err(ExecError::MissingLeaderPayload { context: traced });
    };
    match stores.meaning(symbol) {
        Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::HBox)
        | Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::VBox)
        | Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::VTop) => {
            let node = scan_box_node(kind_for_primitive(primitive)?, input, stores, hooks, traced)?;
            leader_payload_from_node(node, traced)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
        | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy) => {
            let index = scan_register_index(input, stores, hooks, traced)?;
            let id = if matches!(
                stores.meaning(symbol),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)
            ) {
                stores.take_box_reg_same_level(index)
            } else {
                stores.box_reg(index)
            };
            if matches!(
                stores.meaning(symbol),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy)
            ) && let Some(id) = id
            {
                stores.pin_survivor(id);
            }
            first_box_node(stores, id)
                .ok_or(ExecError::MissingLeaderPayload { context: traced })
                .and_then(|node| leader_payload_from_node(node, traced))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSplit) => {
            scan_vsplit_node(input, stores, hooks, traced)?
                .ok_or(ExecError::MissingLeaderPayload { context: traced })
                .and_then(|node| leader_payload_from_node(node, traced))
        }
        Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::HRule)
        | Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::VRule) => {
            leader_payload_from_node(
                scan_rule_node(input, stores, hooks, primitive, traced)?,
                traced,
            )
        }
        _ => {
            push_tokens(input, stores, [token]);
            Err(ExecError::MissingLeaderPayload { context: traced })
        }
    }
}

pub(super) fn scan_leader_glue<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    mode: Mode,
    context: TracedTokenWord,
) -> Result<GlueId, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let traced = next_non_space_traced_x(input, stores, hooks)?
        .ok_or(ExecError::LeadersNotFollowedByProperGlue { context })?;
    let token = tex_expand::semantic_token(traced);
    let Token::Cs(symbol) = token else {
        push_tokens(input, stores, [token]);
        return Err(ExecError::LeadersNotFollowedByProperGlue { context: traced });
    };
    let meaning = stores.meaning(symbol);
    match (mode, meaning) {
        (
            Mode::Horizontal | Mode::RestrictedHorizontal | Mode::Math | Mode::DisplayMath,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSkip),
        )
        | (
            Mode::Vertical | Mode::InternalVertical,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSkip),
        ) => scan_glue_id(input, stores, hooks, false, traced),
        (
            Mode::Horizontal | Mode::RestrictedHorizontal | Mode::Math | Mode::DisplayMath,
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::HFil
                | UnexpandablePrimitive::HFill
                | UnexpandablePrimitive::HSs
                | UnexpandablePrimitive::HFilNeg),
            ),
        )
        | (
            Mode::Vertical | Mode::InternalVertical,
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::VFil
                | UnexpandablePrimitive::VFill
                | UnexpandablePrimitive::VSs
                | UnexpandablePrimitive::VFilNeg),
            ),
        ) => Ok(stores.intern_glue(infinite_glue_for_skip_primitive(primitive))),
        _ => {
            push_traced_tokens(input, stores, [traced]);
            Err(ExecError::LeadersNotFollowedByProperGlue { context: traced })
        }
    }
}

fn leader_payload_from_node(
    node: Node,
    context: TracedTokenWord,
) -> Result<LeaderPayload, ExecError> {
    match node {
        Node::HList(box_node) => Ok(LeaderPayload::HList(box_node)),
        Node::VList(box_node) => Ok(LeaderPayload::VList(box_node)),
        Node::Rule {
            width,
            height,
            depth,
        } => Ok(LeaderPayload::Rule {
            width,
            height,
            depth,
        }),
        _ => Err(ExecError::MissingLeaderPayload { context }),
    }
}

pub(super) fn leader_glue_kind(primitive: UnexpandablePrimitive) -> GlueKind {
    match primitive {
        UnexpandablePrimitive::Leaders => GlueKind::Leaders,
        UnexpandablePrimitive::CLeaders => GlueKind::Cleaders,
        UnexpandablePrimitive::XLeaders => GlueKind::Xleaders,
        _ => unreachable!("caller restricts leader primitives"),
    }
}

fn infinite_glue_for_skip_primitive(primitive: UnexpandablePrimitive) -> GlueSpec {
    match primitive {
        UnexpandablePrimitive::HFil | UnexpandablePrimitive::VFil => {
            infinite_glue(Order::Fil, false, false)
        }
        UnexpandablePrimitive::HFill | UnexpandablePrimitive::VFill => {
            infinite_glue(Order::Fill, false, false)
        }
        UnexpandablePrimitive::HSs | UnexpandablePrimitive::VSs => {
            infinite_glue(Order::Fil, false, true)
        }
        UnexpandablePrimitive::HFilNeg | UnexpandablePrimitive::VFilNeg => {
            infinite_glue(Order::Fil, true, false)
        }
        _ => unreachable!("caller restricts fill glue primitives"),
    }
}
