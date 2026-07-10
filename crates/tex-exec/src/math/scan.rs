use tex_expand::{
    DriverExpandNext, ExpandError, ExpansionHooks, ReadRecorder,
    get_x_token_with_recorder_and_hooks, scan_dimen,
};
use tex_lex::{InputSource, InputStack};
use tex_state::math::{
    FractionThickness, LimitType, MathChoice, MathField, MathFraction, MathNoad, NoadClass,
    NoadKind,
};
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::Node;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{GroupKind, Universe};
use tex_typeset::PackSpec;

use crate::assignments;
use crate::executor::sync_engine_state;
use crate::mode::IncompleteFraction;
use crate::packing_params::vpack;
use crate::{DispatchAction, ExecError, Mode, ModeNest};

use super::{dispatch_math_token_with_recorder, support::report_math_error};

mod chars;
#[cfg(test)]
mod tests;

pub(crate) use chars::{
    append_math_char_code, append_mathcode_char, append_noad, attach_script, math_char_from_code,
    math_char_from_mathcode, redispatch_active_char,
};

pub(super) fn scan_math_field<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<MathField, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let token =
        next_non_space_x(input, stores, recorder, hooks)?.ok_or(ExecError::MissingToken {
            context: "math field",
        })?;
    match token {
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        } => scan_math_field_group_after_open(nest, input, stores, recorder, hooks),
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => {
            redispatch_active_char(input, stores, ch);
            scan_math_field(nest, input, stores, recorder, hooks)
        }
        Token::Char { ch, .. } => {
            let value = stores.mathcode(ch);
            if value == 0x8000 {
                redispatch_active_char(input, stores, ch);
                scan_math_field(nest, input, stores, recorder, hooks)
            } else {
                let (_, math_char) = math_char_from_mathcode(ch, value, stores)?;
                Ok(MathField::MathChar(math_char))
            }
        }
        Token::Cs(_) if assignments::has_catcode_meaning(stores, token, Catcode::BeginGroup) => {
            scan_math_field_group_after_open(nest, input, stores, recorder, hooks)
        }
        Token::Cs(symbol) => match stores.meaning(symbol) {
            Meaning::CharGiven(ch) => {
                let (_, math_char) = math_char_from_mathcode(ch, stores.mathcode(ch), stores)?;
                Ok(MathField::MathChar(math_char))
            }
            Meaning::MathCharGiven(value) => Ok(MathField::MathChar(math_char_from_code(
                u32::from(value),
                stores,
            )?)),
            _ => {
                let mut temp = ModeNest::new();
                temp.push(nest.current_mode());
                dispatch_math_token_with_recorder(
                    &mut temp,
                    TracedTokenWord::pack(Token::Cs(symbol), OriginId::UNKNOWN),
                    input,
                    stores,
                    recorder,
                    hooks,
                )?;
                let id = finish_current_math_list(&mut temp, stores);
                Ok(MathField::SubMlist(id))
            }
        },
        Token::Param(_) | Token::Frozen(_) => Err(ExecError::MissingToken {
            context: "math field",
        }),
    }
}

pub(super) fn scan_math_group_after_open<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<tex_state::ids::NodeListId, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    stores.enter_group_with_kind(GroupKind::Simple);
    nest.push(Mode::Math);
    loop {
        sync_engine_state::<S, _>(hooks, nest, stores);
        let token = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?.ok_or(
            ExecError::MissingToken {
                context: "math group closing brace",
            },
        )?;
        let semantic = tex_expand::semantic_token(token);
        if assignments::has_catcode_meaning(stores, semantic, Catcode::EndGroup) {
            crate::leave_group_with_origin(input, stores, GroupKind::Simple, token.origin())?;
            let list = finish_current_math_list(nest, stores);
            let _ = nest.pop()?;
            return Ok(list);
        }
        match dispatch_math_token_with_recorder(nest, token, input, stores, recorder, hooks)? {
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

pub(super) fn scan_math_field_group_after_open<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<MathField, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let list = scan_math_group_after_open(nest, input, stores, recorder, hooks)?;
    Ok(simplify_math_group_field(stores, list))
}

fn simplify_math_group_field(stores: &Universe, list: tex_state::ids::NodeListId) -> MathField {
    // TeX.web removes braces around a single unscripted Ord atom by copying
    // its nucleus into the field that the group was scanned to fill.
    if let [Node::MathNoad(noad)] = stores.nodes(list)
        && matches!(noad.kind, NoadKind::Normal(NoadClass::Ord))
        && matches!(noad.subscript, MathField::Empty)
        && matches!(noad.superscript, MathField::Empty)
    {
        noad.nucleus.clone()
    } else {
        MathField::SubMlist(list)
    }
}

pub(super) fn start_left_group<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let delimiter = scan_delimiter_token(input, stores, recorder, hooks)?;
    nest.push(Mode::Math);
    nest.current_list_mut().push(Node::MathNoad(MathNoad::new(
        NoadKind::LeftDelimiter { delimiter },
        MathField::Empty,
    )));
    sync_engine_state::<S, _>(hooks, nest, stores);
    Ok(())
}

pub(super) fn finish_left_group<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let delimiter = scan_delimiter_token(input, stores, recorder, hooks)?;
    if !current_list_is_left_group(nest) {
        report_math_error(stores, "Extra \\right");
        return Ok(());
    }
    close_left_group(nest, stores, delimiter)?;
    sync_engine_state::<S, _>(hooks, nest, stores);
    Ok(())
}

pub(super) fn close_missing_left_group(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<bool, ExecError> {
    if !current_list_is_left_group(nest) {
        return Ok(false);
    }
    report_math_error(stores, "Missing \\right. inserted");
    close_left_group(nest, stores, 0)?;
    Ok(true)
}

fn current_list_is_left_group(nest: &ModeNest) -> bool {
    matches!(
        nest.current_list().nodes().first(),
        Some(Node::MathNoad(MathNoad {
            kind: NoadKind::LeftDelimiter { .. },
            ..
        }))
    )
}

fn close_left_group(
    nest: &mut ModeNest,
    stores: &mut Universe,
    right_delimiter: u32,
) -> Result<(), ExecError> {
    nest.current_list_mut().push(Node::MathNoad(MathNoad::new(
        NoadKind::RightDelimiter {
            delimiter: right_delimiter,
        },
        MathField::Empty,
    )));
    let content = finish_current_math_list(nest, stores);
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
        vec![Node::FractionNoad(MathFraction {
            numerator: incomplete.numerator,
            denominator,
            thickness: incomplete.thickness,
            left_delimiter: incomplete.left_delimiter,
            right_delimiter: incomplete.right_delimiter,
        })]
    } else {
        nodes
    };
    stores.freeze_node_list(&nodes)
}

pub(super) fn start_fraction<S, R, H>(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    if nest.current_list().incomplete_fraction().is_some() {
        report_math_error(stores, "Ambiguous; you need another { and }");
        return Ok(());
    }
    let (left_delimiter, right_delimiter) = match primitive {
        UnexpandablePrimitive::OverWithDelims
        | UnexpandablePrimitive::AtopWithDelims
        | UnexpandablePrimitive::AboveWithDelims => (
            Some(scan_delimiter_token(input, stores, recorder, hooks)?),
            Some(scan_delimiter_token(input, stores, recorder, hooks)?),
        ),
        _ => (None, None),
    };
    let thickness = match primitive {
        UnexpandablePrimitive::Atop | UnexpandablePrimitive::AtopWithDelims => {
            FractionThickness::Explicit(Scaled::from_raw(0))
        }
        UnexpandablePrimitive::Above | UnexpandablePrimitive::AboveWithDelims => {
            FractionThickness::Explicit(assignments::scan_scaled(input, stores, hooks, context)?)
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

pub(super) fn append_math_choice<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let display = scan_required_math_group(nest, input, stores, recorder, hooks, "\\mathchoice")?;
    let text = scan_required_math_group(nest, input, stores, recorder, hooks, "\\mathchoice")?;
    let script = scan_required_math_group(nest, input, stores, recorder, hooks, "\\mathchoice")?;
    let script_script =
        scan_required_math_group(nest, input, stores, recorder, hooks, "\\mathchoice")?;
    nest.current_list_mut().push(Node::MathChoice(MathChoice {
        display,
        text,
        script,
        script_script,
    }));
    Ok(())
}

fn scan_required_math_group<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    context: &'static str,
) -> Result<tex_state::ids::NodeListId, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let opener = next_non_space_x(input, stores, recorder, hooks)?
        .ok_or(ExecError::MissingToken { context })?;
    if !assignments::has_catcode_meaning(stores, opener, Catcode::BeginGroup) {
        return Err(ExecError::MissingToken { context });
    }
    scan_math_group_after_open(nest, input, stores, recorder, hooks)
}

pub(super) fn scan_vcenter_field<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<MathField, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let opener =
        assignments::next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
            context: "\\vcenter",
        })?;
    if !assignments::is_begin_group(opener) {
        return Err(ExecError::MissingToken {
            context: "\\vcenter",
        });
    }
    stores.enter_group_with_kind(GroupKind::Simple);
    let mut inner = ModeNest::new();
    inner.push(Mode::InternalVertical);
    assignments::scan_box_group(&mut inner, input, stores, hooks)?;
    let level = inner.pop()?;
    let children = stores.freeze_node_list(level.list().nodes());
    let vbox = Node::VList(
        vpack(
            stores,
            children,
            PackSpec::Natural,
            crate::packing_params::vpack_params(stores),
        )
        .node,
    );
    let boxed = stores.freeze_node_list(&[vbox]);
    crate::leave_group(input, stores, GroupKind::Simple)?;
    Ok(MathField::SubBox(boxed))
}

pub(super) fn scan_math_char_code<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<u32, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let value = assignments::scan_i32(input, stores, hooks, context)?;
    if !(0..=32_767).contains(&value) {
        return Err(ExecError::InvalidCode {
            context: "\\mathchar",
            value,
        });
    }
    Ok(value as u32)
}

pub(super) fn scan_delimiter_code<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<u32, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let value = assignments::scan_i32(input, stores, hooks, context)?;
    if !(0..=0x07ff_ffff).contains(&value) {
        return Err(ExecError::InvalidCode {
            context: "\\delimiter",
            value,
        });
    }
    Ok(value as u32)
}

fn scan_delimiter_token<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<u32, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    loop {
        let Some(traced) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
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
                let code = if ch == '.' { 0 } else { stores.delcode(ch) };
                if code >= 0 {
                    return Ok(code as u32);
                }
            }
            Token::Cs(symbol) => {
                let meaning = stores.meaning(symbol);
                recorder.record_meaning(symbol, meaning);
                match meaning {
                    Meaning::Relax => continue,
                    Meaning::MathCharGiven(value) => return Ok(u32::from(value)),
                    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Delimiter) => {
                        return scan_delimiter_code(input, stores, hooks, traced);
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

pub(super) fn scan_mu_dimen<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<Scaled, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = tex_expand::NoopRecorder;
    let scanned = scan_dimen::scan_dimen_with_expander_and_hooks(
        input,
        stores,
        &mut recorder,
        hooks,
        &mut DriverExpandNext,
        scan_dimen::ScanDimenOptions::STANDARD.requiring_mu_unit(),
        context,
    )
    .map_err(ExpandError::from)?;
    Ok(scanned.value())
}

fn next_non_space_x<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Option<Token>, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
            .map(tex_expand::semantic_token)
        else {
            return Ok(None);
        };
        if !assignments::is_space(token) {
            return Ok(Some(token));
        }
    }
}
