use std::sync::Arc;

use tex_state::Universe;

use super::{NativeBatchBarrier, NativeBatchNode, NativeBatchProgram};
use crate::{CharacterCode, CommandProfile};

fn compile(source: &[u8], calls: usize) -> Result<NativeBatchProgram, NativeBatchBarrier> {
    let stores = Universe::new_with_plain_catcodes();
    NativeBatchProgram::compile(
        Arc::<[u8]>::from(source),
        CommandProfile::TEX82,
        stores.endlinechar(),
        |code: CharacterCode| {
            let byte = code.to_byte().expect("exact-byte profile");
            stores.catcode(char::from(byte))
        },
        calls,
    )
}

#[test]
fn canonical_lexer_feeds_grouped_assignment_macro_and_output_episode() {
    let source = br"\count0=0\count1=0\count2=0\def\e#1{\advance\count0by#1\global\advance\count1by#1\ifnum#1<5\global\advance\count2by1\else\global\advance\count2by2\fi A\kern#1sp}\shipout\hbox{\e{1}\e{2}\e{3}\e{4}\e{5}\e{6}\e{7}\e{8}}\end";
    let outcome = compile(source, 8)
        .expect("supported program admits")
        .execute()
        .expect("admitted program executes");

    assert_eq!(outcome.counts, [0, 36, 12]);
    assert_eq!(outcome.calls, 8);
    assert_eq!(outcome.nodes.len(), 16);
    assert!(matches!(outcome.nodes[0], NativeBatchNode::Character(b'A')));
    assert!(matches!(outcome.nodes[1], NativeBatchNode::Kern(1)));
}

#[test]
fn unsupported_control_sequence_stops_before_execution() {
    let error = compile(br"\count0=1\message{observable}\end", 0)
        .expect_err("observable command is outside the episode");
    assert_eq!(
        error,
        NativeBatchBarrier::UnsupportedControlSequence("message".to_owned())
    );
}

#[test]
fn material_after_end_is_an_explicit_admission_barrier() {
    let error = compile(br"\end\relax", 0).expect_err("post-end material is refused");
    assert_eq!(error, NativeBatchBarrier::MaterialAfterEnd);
}
