use tex_expand::{DriverExpansionMode, ExpandError, get_x_token_with_context, scan_dimen};
use tex_lex::InputStack;
use tex_state::math::{
    FractionThickness, LimitType, MathChoice, MathField, MathFraction, MathNoad, NoadClass,
    NoadKind,
};
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::Node;
use tex_state::provenance::InsertedOriginKind;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{ExpansionState, GroupKind, Universe};

use crate::assignments;
use crate::executor::sync_engine_state;
use crate::mode::IncompleteFraction;
use crate::packing_params::vpack;
use crate::{DispatchAction, ExecError, Mode, ModeNest};

use super::{dispatch_math_token_with_context, support::report_math_error};

mod chars;
#[cfg(test)]
mod tests;

pub(crate) use chars::{
    append_math_char_code, append_mathcode_char, append_noad, attach_script, math_char_from_code,
    math_char_from_mathcode, redispatch_active_char,
};

pub(super) fn scan_math_field(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<MathField, ExecError> {
    loop {
        let traced =
            next_non_space_traced_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
                context: "math field",
            })?;
        let token = tex_expand::semantic_token(traced);
        if let Token::Cs(symbol) = token
            && stores.meaning(symbol) == Meaning::Relax
        {
            continue;
        }
        return match token {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => scan_math_field_group_after_open(nest, input, stores, execution),
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => {
                redispatch_active_char(input, stores, ch);
                scan_math_field(nest, input, stores, execution)
            }
            Token::Char { ch, .. } => {
                execution.record_paragraph_mathcode(ch);
                let value = stores.mathcode(ch);
                if value == 0x8000 {
                    redispatch_active_char(input, stores, ch);
                    scan_math_field(nest, input, stores, execution)
                } else {
                    let (_, math_char) =
                        math_char_from_mathcode(ch, value, stores, traced.origin())?;
                    Ok(MathField::MathChar(math_char))
                }
            }
            Token::Cs(_)
                if assignments::has_catcode_meaning(stores, token, Catcode::BeginGroup) =>
            {
                scan_math_field_group_after_open(nest, input, stores, execution)
            }
            Token::Cs(symbol) => match stores.meaning(symbol) {
                Meaning::CharGiven(ch) => {
                    execution.record_paragraph_mathcode(ch);
                    let (_, math_char) =
                        math_char_from_mathcode(ch, stores.mathcode(ch), stores, traced.origin())?;
                    Ok(MathField::MathChar(math_char))
                }
                Meaning::MathCharGiven(value) => Ok(MathField::MathChar(math_char_from_code(
                    u32::from(value),
                    stores,
                    traced.origin(),
                )?)),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
                    let context =
                        TracedTokenWord::pack(Token::Cs(symbol.symbol()), OriginId::UNKNOWN);
                    let value = assignments::scan_i32(input, stores, execution, context)?;
                    let ch = u8::try_from(value).map(char::from).map_err(|_| {
                        ExecError::InvalidCode {
                            context: "\\char",
                            value,
                        }
                    })?;
                    execution.record_paragraph_mathcode(ch);
                    let (_, math_char) =
                        math_char_from_mathcode(ch, stores.mathcode(ch), stores, traced.origin())?;
                    Ok(MathField::MathChar(math_char))
                }
                Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Leaders
                    | UnexpandablePrimitive::CLeaders
                    | UnexpandablePrimitive::XLeaders,
                ) => {
                    stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    "\n! Missing { inserted.\nA left brace was mandatory here, so I've put one in.\n",
                );
                    let opener = Token::Char {
                        ch: '{',
                        cat: Catcode::BeginGroup,
                    };
                    let origin = stores.inserted_origin(
                        InsertedOriginKind::ErrorRecovery,
                        opener,
                        traced.origin(),
                    );
                    input.back_input_alignment_token(traced);
                    crate::insert_traced_tokens(
                        input,
                        stores,
                        [TracedTokenWord::pack(opener, origin), traced],
                    );
                    scan_math_field(nest, input, stores, execution)
                }
                _ => {
                    let mut temp = ModeNest::new();
                    temp.push(nest.current_mode());
                    dispatch_math_token_with_context(&mut temp, traced, input, stores, execution)?;
                    let id = finish_current_math_list(&mut temp, stores);
                    Ok(MathField::SubMlist(id))
                }
            },
            Token::Param(_) | Token::Frozen(_) => Err(ExecError::MissingToken {
                context: "math field",
            }),
        };
    }
}

pub(super) fn scan_math_group_after_open(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<tex_state::ids::NodeListId, ExecError> {
    let mut transaction = crate::transaction::ExecutionTransaction::begin(nest, stores);
    let result = {
        let (nest, stores) = transaction.parts();
        scan_math_group_after_open_inner(nest, input, stores, execution)
    };
    if result.is_ok() {
        transaction.commit();
    }
    result
}

fn scan_math_group_after_open_inner(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<tex_state::ids::NodeListId, ExecError> {
    stores.enter_group_with_kind(GroupKind::Math);
    nest.push(Mode::Math);
    loop {
        sync_engine_state(execution, nest, stores);
        let token = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        .ok_or(ExecError::MissingToken {
            context: "math group closing brace",
        })?;
        let semantic = tex_expand::semantic_token(token);
        if assignments::has_catcode_meaning(stores, semantic, Catcode::EndGroup) {
            crate::leave_group_with_origin(input, stores, GroupKind::Math, token.origin())?;
            let list = finish_current_math_list(nest, stores);
            let _ = nest.pop()?;
            return Ok(list);
        }
        match dispatch_math_token_with_context(nest, token, input, stores, execution)? {
            DispatchAction::Continue | DispatchAction::Shipout(_) => {}
            DispatchAction::End => {
                return Err(ExecError::MissingToken {
                    context: "math group closing brace",
                });
            }
            DispatchAction::NotConsumed => {
                return Err(ExecError::UnimplementedTypesetting {
                    mode: nest.current_mode(),
                    token: semantic,
                    origin: token.origin(),
                    operation: "math group",
                });
            }
        }
    }
}

pub(super) fn scan_math_field_group_after_open(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<MathField, ExecError> {
    let list = scan_math_group_after_open(nest, input, stores, execution)?;
    Ok(simplify_math_group_field(stores, list))
}

pub(super) fn scan_math_atom_group_after_open(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<MathNoad, ExecError> {
    let list = scan_math_group_after_open(nest, input, stores, execution)?;
    let nodes = stores.nodes(list);
    if nodes.len() == 1
        && let Some(tex_state::node_arena::NodeRef::MathNoad(noad)) = nodes.first()
        && matches!(noad.kind, NoadKind::Accent { .. })
    {
        // TeX.web §1196 replaces the placeholder Ord noad by a sole accent
        // noad when braces supplied that Ord's nucleus. Keeping the wrapper
        // would add an observable hbox/push around the accent.
        return Ok(noad.clone());
    }
    Ok(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        simplify_math_group_field(stores, list),
    ))
}

fn simplify_math_group_field(stores: &Universe, list: tex_state::ids::NodeListId) -> MathField {
    // TeX.web removes braces around a single unscripted Ord atom by copying
    // its nucleus into the field that the group was scanned to fill.
    let nodes = stores.nodes(list);
    if nodes.len() == 1
        && let Some(tex_state::node_arena::NodeRef::MathNoad(noad)) = nodes.first()
        && matches!(noad.kind, NoadKind::Normal(NoadClass::Ord))
        && matches!(noad.subscript, MathField::Empty)
        && matches!(noad.superscript, MathField::Empty)
    {
        noad.nucleus
    } else {
        MathField::SubMlist(list)
    }
}

pub(super) fn start_left_group(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let delimiter = scan_delimiter_token(input, stores, execution)?;
    nest.push(Mode::Math);
    nest.current_list_mut().push(Node::MathNoad(MathNoad::new(
        NoadKind::LeftDelimiter { delimiter },
        MathField::Empty,
    )));
    sync_engine_state(execution, nest, stores);
    Ok(())
}

pub(super) fn finish_left_group(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let delimiter = scan_delimiter_token(input, stores, execution)?;
    if !current_list_is_left_group(nest, stores) {
        report_math_error(stores, "Extra \\right");
        return Ok(());
    }
    close_left_group(nest, stores, delimiter)?;
    sync_engine_state(execution, nest, stores);
    Ok(())
}

pub(super) fn append_middle_delimiter(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let delimiter = scan_delimiter_token(input, stores, execution)?;
    if !current_list_is_left_group(nest, stores) {
        report_math_error(stores, "Extra \\middle");
        return Ok(());
    }
    nest.current_list_mut().push(Node::MathNoad(MathNoad::new(
        NoadKind::MiddleDelimiter { delimiter },
        MathField::Empty,
    )));
    Ok(())
}

pub(super) fn close_missing_left_group(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<bool, ExecError> {
    if !current_list_is_left_group(nest, stores) {
        return Ok(false);
    }
    report_math_error(stores, "Missing \\right. inserted");
    close_left_group(nest, stores, 0)?;
    Ok(true)
}

fn current_list_is_left_group(nest: &ModeNest, stores: &Universe) -> bool {
    let is_left = |node: Option<tex_state::node_arena::NodeRef<'_>>| {
        matches!(
            node,
            Some(tex_state::node_arena::NodeRef::MathNoad(MathNoad {
                kind: NoadKind::LeftDelimiter { .. },
                ..
            }))
        )
    };
    if matches!(
        nest.current_list().nodes().first(),
        Some(Node::MathNoad(MathNoad {
            kind: NoadKind::LeftDelimiter { .. },
            ..
        }))
    ) {
        return true;
    }
    nest.current_list()
        .incomplete_fraction()
        .is_some_and(|fraction| is_left(stores.nodes(fraction.numerator).first()))
}

fn close_left_group(
    nest: &mut ModeNest,
    stores: &mut Universe,
    right_delimiter: u32,
) -> Result<(), ExecError> {
    // TeX completes an outstanding generalized fraction before appending the
    // matching \right noad. Otherwise the right delimiter incorrectly becomes
    // part of the fraction denominator.
    let content = finish_current_math_list(nest, stores);
    let mut nodes: Vec<_> = stores
        .nodes(content)
        .into_iter()
        .map(|node| node.to_owned())
        .collect();
    nodes.push(Node::MathNoad(MathNoad::new(
        NoadKind::RightDelimiter {
            delimiter: right_delimiter,
        },
        MathField::Empty,
    )));
    let content = stores.freeze_node_list(&nodes);
    let _ = nest.pop()?;
    append_noad(
        nest,
        NoadKind::Normal(NoadClass::Inner),
        MathField::SubMlist(content),
    );
    Ok(())
}

pub(super) fn finish_current_math_list(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> tex_state::ids::NodeListId {
    let (nodes, incomplete) = {
        let list = nest.current_list_mut();
        (list.take_nodes(), list.take_incomplete_fraction())
    };
    let nodes = if let Some(incomplete) = incomplete {
        let denominator = stores.freeze_node_list(&nodes);
        let mut numerator_nodes: Vec<_> = stores
            .nodes(incomplete.numerator)
            .into_iter()
            .map(|node| node.to_owned())
            .collect();
        let leading_left = matches!(numerator_nodes.first(), Some(Node::MathNoad(noad)) if matches!(noad.kind, NoadKind::LeftDelimiter { .. }))
            .then(|| numerator_nodes.remove(0));
        let numerator = if leading_left.is_some() {
            stores.freeze_node_list(&numerator_nodes)
        } else {
            incomplete.numerator
        };
        let fraction = Node::FractionNoad(MathFraction {
            numerator,
            denominator,
            thickness: incomplete.thickness,
            left_delimiter: incomplete.left_delimiter,
            right_delimiter: incomplete.right_delimiter,
        });
        leading_left.into_iter().chain([fraction]).collect()
    } else {
        nodes
    };
    stores.freeze_node_list(&nodes)
}

pub(super) fn start_fraction(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    if nest.current_list().incomplete_fraction().is_some() {
        report_math_error(stores, "Ambiguous; you need another { and }");
        return Ok(());
    }
    let (left_delimiter, right_delimiter) = match primitive {
        UnexpandablePrimitive::OverWithDelims
        | UnexpandablePrimitive::AtopWithDelims
        | UnexpandablePrimitive::AboveWithDelims => (
            Some(scan_delimiter_token(input, stores, execution)?),
            Some(scan_delimiter_token(input, stores, execution)?),
        ),
        _ => (None, None),
    };
    let thickness = match primitive {
        UnexpandablePrimitive::Atop | UnexpandablePrimitive::AtopWithDelims => {
            FractionThickness::Explicit(Scaled::from_raw(0))
        }
        UnexpandablePrimitive::Above | UnexpandablePrimitive::AboveWithDelims => {
            FractionThickness::Explicit(assignments::scan_scaled(
                input, stores, execution, context,
            )?)
        }
        _ => FractionThickness::Default,
    };
    let numerator_nodes = nest.current_list_mut().take_nodes();
    let numerator = stores.freeze_node_list(&numerator_nodes);
    nest.current_list_mut()
        .set_incomplete_fraction(IncompleteFraction {
            numerator,
            thickness,
            left_delimiter,
            right_delimiter,
        });
    Ok(())
}

pub(super) fn apply_limit_switch(
    nest: &mut ModeNest,
    stores: &mut Universe,
    primitive: UnexpandablePrimitive,
) {
    let limit_type = match primitive {
        UnexpandablePrimitive::Limits => LimitType::Limits,
        UnexpandablePrimitive::NoLimits => LimitType::NoLimits,
        UnexpandablePrimitive::DisplayLimits => LimitType::DisplayLimits,
        _ => unreachable!("caller restricts limit primitive"),
    };
    let Some(Node::MathNoad(noad)) = nest.current_list_mut().last_node_mut() else {
        report_math_error(stores, "Limit controls must follow a math operator");
        return;
    };
    match noad.kind {
        NoadKind::Operator(_) | NoadKind::Normal(NoadClass::Op) => {
            noad.kind = NoadKind::Operator(limit_type);
        }
        _ => report_math_error(stores, "Limit controls must follow a math operator"),
    }
}

pub(super) fn append_math_choice(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let display = scan_required_math_group(nest, input, stores, execution, "\\mathchoice")?;
    let text = scan_required_math_group(nest, input, stores, execution, "\\mathchoice")?;
    let script = scan_required_math_group(nest, input, stores, execution, "\\mathchoice")?;
    let script_script = scan_required_math_group(nest, input, stores, execution, "\\mathchoice")?;
    nest.current_list_mut().push(Node::MathChoice(MathChoice {
        display,
        text,
        script,
        script_script,
    }));
    Ok(())
}

fn scan_required_math_group(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: &'static str,
) -> Result<tex_state::ids::NodeListId, ExecError> {
    let Some(opener) = assignments::next_non_space_traced_x(input, stores, execution)? else {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            &format!("\n! Missing {{ inserted while scanning {context}.\n"),
        );
        return Ok(stores.freeze_node_list(&[]));
    };
    if !assignments::has_catcode_meaning(
        stores,
        tex_expand::semantic_token(opener),
        Catcode::BeginGroup,
    ) {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            &format!("\n! Missing {{ inserted while scanning {context}.\n"),
        );
        crate::push_traced_tokens(input, stores, [opener]);
        return Ok(stores.freeze_node_list(&[]));
    }
    scan_math_group_after_open(nest, input, stores, execution)
}

pub(super) fn scan_vcenter_field(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<MathField, ExecError> {
    let spec = assignments::scan_pack_spec(input, stores, execution, context)?;
    let opener = assignments::next_non_space_x(input, stores, execution)?.ok_or(
        ExecError::MissingToken {
            context: "\\vcenter",
        },
    )?;
    if !assignments::has_catcode_meaning(stores, opener, Catcode::BeginGroup) {
        return Err(ExecError::MissingToken {
            context: "\\vcenter",
        });
    }
    let mut inner = ModeNest::new();
    let mut transaction = crate::transaction::ExecutionTransaction::begin(&mut inner, stores);
    let (inner, stores) = transaction.parts();
    stores.enter_group_with_kind(GroupKind::VCenter);
    let box_group_depth = stores.execution_group_depth();
    inner.push(Mode::InternalVertical);
    assignments::scan_box_group(inner, input, stores, execution, box_group_depth)?;
    let level = inner.pop()?;
    let children = stores.freeze_node_list(level.list().nodes());
    let vbox = Node::VList(
        vpack(
            stores,
            children,
            spec,
            crate::packing_params::vpack_params(stores),
        )
        .node,
    );
    let boxed = stores.freeze_node_list(&[vbox]);
    crate::leave_group(input, stores, GroupKind::VCenter)?;
    transaction.commit();
    Ok(MathField::SubBox(boxed))
}

pub(super) fn scan_math_char_code(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<u32, ExecError> {
    let value = assignments::scan_i32(input, stores, execution, context)?;
    if !(0..=32_767).contains(&value) {
        return Err(ExecError::InvalidCode {
            context: "\\mathchar",
            value,
        });
    }
    Ok(value as u32)
}

pub(super) fn scan_delimiter_code(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<u32, ExecError> {
    let value = assignments::scan_i32(input, stores, execution, context)?;
    if !(0..=0x07ff_ffff).contains(&value) {
        return Err(ExecError::InvalidCode {
            context: "\\delimiter",
            value,
        });
    }
    Ok(value as u32)
}

fn scan_delimiter_token(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<u32, ExecError> {
    loop {
        let Some(traced) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        else {
            report_math_error(stores, "Missing delimiter (. inserted)");
            return Ok(0);
        };
        let token = tex_expand::semantic_token(traced);
        match token {
            Token::Char {
                cat: Catcode::Space,
                ..
            } => continue,
            Token::Char {
                ch,
                cat: Catcode::Letter | Catcode::Other,
            } => {
                let code = if ch == '.' {
                    0
                } else {
                    execution.record_paragraph_delcode(ch);
                    stores.delcode(ch)
                };
                if code >= 0 {
                    return Ok(code as u32);
                }
            }
            Token::Cs(symbol) => {
                let meaning = stores.meaning(symbol);
                execution.record_meaning(symbol, meaning);
                match meaning {
                    Meaning::Relax => continue,
                    Meaning::MathCharGiven(value) => return Ok(u32::from(value)),
                    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Delimiter) => {
                        return scan_delimiter_code(input, stores, execution, traced);
                    }
                    _ => {}
                }
            }
            Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => {}
        }

        crate::push_traced_tokens(input, stores, [traced]);
        report_math_error(stores, "Missing delimiter (. inserted)");
        return Ok(0);
    }
}

pub(super) fn scan_mu_dimen(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<Scaled, ExecError> {
    let scanned = scan_dimen::scan_dimen_with_mode_and_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        &mut DriverExpansionMode,
        scan_dimen::ScanDimenOptions::STANDARD.requiring_mu_unit(),
        context,
    )
    .map_err(ExpandError::from)?;
    Ok(scanned.value())
}

fn next_non_space_traced_x(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExecError> {
    loop {
        let Some(traced) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        else {
            return Ok(None);
        };
        if !assignments::is_space(tex_expand::semantic_token(traced)) {
            return Ok(Some(traced));
        }
    }
}
