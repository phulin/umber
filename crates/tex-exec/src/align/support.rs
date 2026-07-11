use tex_state::Universe;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token};

use crate::mode::{AlignState, AlignmentKind};
use crate::{ExecError, Mode, ModeNest};

pub(super) fn align_state(nest: &ModeNest, align_level: usize) -> Result<&AlignState, ExecError> {
    nest.list(align_level)
        .and_then(crate::mode::ModeList::align_state)
        .ok_or(ExecError::MissingToken {
            context: "alignment state",
        })
}

pub(super) fn align_state_mut(
    nest: &mut ModeNest,
    align_level: usize,
) -> Result<&mut AlignState, ExecError> {
    nest.list_mut(align_level)
        .and_then(crate::mode::ModeList::align_state_mut)
        .ok_or(ExecError::MissingToken {
            context: "alignment state",
        })
}

pub(super) fn set_align_brace_depth(nest: &mut ModeNest, align_level: usize, value: i32) {
    if let Some(state) = nest
        .list_mut(align_level)
        .and_then(crate::mode::ModeList::align_state_mut)
    {
        state.set_brace_depth(value);
    }
}

pub(super) fn align_kind(nest: &ModeNest, align_level: usize) -> Result<AlignmentKind, ExecError> {
    Ok(align_state(nest, align_level)?.kind())
}

pub(super) fn alignment_mode(kind: AlignmentKind) -> Mode {
    match kind {
        AlignmentKind::HAlign => Mode::InternalVertical,
        AlignmentKind::VAlign => Mode::RestrictedHorizontal,
    }
}

pub(super) fn row_mode(kind: AlignmentKind) -> Mode {
    match kind {
        AlignmentKind::HAlign => Mode::RestrictedHorizontal,
        AlignmentKind::VAlign => Mode::InternalVertical,
    }
}

pub(super) fn cell_mode(kind: AlignmentKind) -> Mode {
    row_mode(kind)
}

pub(super) fn is_alignment_tab(stores: &Universe, token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::AlignmentTab,
            ..
        }
    ) || matches!(
        token_meaning(stores, token),
        Some(Meaning::CharToken {
            cat: Catcode::AlignmentTab,
            ..
        })
    )
}

pub(super) fn is_begin_group(stores: &Universe, token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    ) || matches!(
        token_meaning(stores, token),
        Some(Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        })
    )
}

pub(super) fn is_end_group(stores: &Universe, token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    ) || matches!(
        token_meaning(stores, token),
        Some(Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        })
    )
}

pub(super) fn is_noalign(stores: &Universe, token: Token) -> bool {
    primitive_token(stores, token) == Some(UnexpandablePrimitive::NoAlign)
}

pub(super) fn is_omit(stores: &Universe, token: Token) -> bool {
    primitive_token(stores, token) == Some(UnexpandablePrimitive::Omit)
}

pub(super) fn is_cr(stores: &Universe, token: Token) -> bool {
    matches!(
        primitive_token(stores, token),
        Some(UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr)
    )
}

pub(super) fn is_crcr(stores: &Universe, token: Token) -> bool {
    primitive_token(stores, token) == Some(UnexpandablePrimitive::CrCr)
}

pub(super) fn is_span(stores: &Universe, token: Token) -> bool {
    primitive_token(stores, token) == Some(UnexpandablePrimitive::Span)
}

fn primitive_token(stores: &Universe, token: Token) -> Option<UnexpandablePrimitive> {
    match token_meaning(stores, token) {
        Some(Meaning::UnexpandablePrimitive(primitive)) => Some(primitive),
        _ => None,
    }
}

fn token_meaning(stores: &Universe, token: Token) -> Option<Meaning> {
    let Token::Cs(symbol) = token else {
        return None;
    };
    Some(stores.meaning(symbol))
}
