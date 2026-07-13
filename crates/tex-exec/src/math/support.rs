use tex_state::math::{LimitType, MathStyle, NoadClass, NoadKind};
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::{PrintSink, Universe};

pub(super) fn noad_kind_for_constructor(primitive: UnexpandablePrimitive) -> NoadKind {
    match primitive {
        UnexpandablePrimitive::MathOrd => NoadKind::Normal(NoadClass::Ord),
        UnexpandablePrimitive::MathOp => NoadKind::Operator(LimitType::DisplayLimits),
        UnexpandablePrimitive::MathBin => NoadKind::Normal(NoadClass::Bin),
        UnexpandablePrimitive::MathRel => NoadKind::Normal(NoadClass::Rel),
        UnexpandablePrimitive::MathOpen => NoadKind::Normal(NoadClass::Open),
        UnexpandablePrimitive::MathClose => NoadKind::Normal(NoadClass::Close),
        UnexpandablePrimitive::MathPunct => NoadKind::Normal(NoadClass::Punct),
        UnexpandablePrimitive::MathInner => NoadKind::Normal(NoadClass::Inner),
        _ => unreachable!("caller restricts constructor primitive"),
    }
}

pub(super) fn style_for_primitive(primitive: UnexpandablePrimitive) -> MathStyle {
    match primitive {
        UnexpandablePrimitive::DisplayStyle => MathStyle::Display,
        UnexpandablePrimitive::TextStyle => MathStyle::Text,
        UnexpandablePrimitive::ScriptStyle => MathStyle::Script,
        UnexpandablePrimitive::ScriptScriptStyle => MathStyle::ScriptScript,
        _ => unreachable!("caller restricts style primitive"),
    }
}

pub(super) fn report_math_error(stores: &mut Universe, text: &str) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, &format!("\n! {text}.\n"));
}
