use super::*;
use crate::align::support::alignment_mode;
use crate::{AlignmentKind, ModeNest, install_unexpandable_primitives};
use tex_lex::{InputStack, MemoryInput};
use tex_state::GroupKind;
use tex_state::token::{Catcode, Token};

#[test]
fn noalign_close_resumes_row_lookahead_in_both_modes() {
    for (kind, source) in [
        (AlignmentKind::HAlign, r"{\count0=7}x"),
        (AlignmentKind::VAlign, "{\\count0=7}x"),
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        install_unexpandable_primitives(&mut stores);
        stores.set_count(0, 3);
        stores.enter_group_with_kind(GroupKind::Align);
        let mut nest = ModeNest::new();
        nest.push(alignment_mode(kind)).expect("test mode push");
        let alignment_mode = nest.current_mode();
        let mut input = InputStack::new(MemoryInput::new(source));
        input.begin_alignment();
        let mut execution = crate::ExecutionContext::new("texput");

        execute_noalign(
            nest.depth() - 1,
            &mut nest,
            &mut input,
            &mut stores,
            &mut execution,
        )
        .expect("noalign group closes");

        assert_eq!(stores.innermost_group_kind(), Some(GroupKind::Align));
        assert_eq!(stores.count(0), 3, "local noalign assignments restore");
        assert_eq!(nest.current_mode(), alignment_mode);
        let next = tex_expand::get_x_token_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut execution,
        )
        .expect("row lookahead resumes")
        .expect("following row token remains");
        assert_eq!(
            tex_expand::semantic_token(next),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }
        );
    }
}
