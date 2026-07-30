use tex_state::math::{LimitType, MathStyle, NoadClass, NoadKind};
use tex_state::meaning::UnexpandablePrimitive;

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

/// tex.web §1064's `off_save` help, shared by every group it can repair.
///
/// `off_save` prints one `help5` regardless of which terminator it inserted,
/// so `Missing }`, `Missing \endgroup` and `Missing \right.` all carry it.
pub(super) const OFF_SAVE_HELP: [&str; 5] = [
    "I've inserted something that you may have forgotten.",
    "(See the <inserted text> above.)",
    "With luck, this will get me unwedged. But if you",
    "really didn't forget anything, try typing `2' now; then",
    "my insertion and my current dilemma will both disappear.",
];
