use super::*;
use crate::executor::NoopExecHooks;
use tex_expand::{NoopRecorder, ReadRecorder};
use tex_lex::MemoryInput;
use tex_state::interner::Symbol;
use tex_state::provenance::SyntheticOriginKind;
use tex_state::token::TracedTokenWord;

#[test]
fn non_character_accent_lookahead_replays_the_original_traced_token() {
    let mut stores = Universe::new();
    crate::install_unexpandable_primitives(&mut stores);
    let origin = stores.synthetic_origin(SyntheticOriginKind::Test);
    let closing_group = TracedTokenWord::pack(
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
        origin,
    );
    let mut input = InputStack::new(MemoryInput::new(""));
    push_traced_tokens(&mut input, &mut stores, [closing_group]);

    let base = scan_accent_base(
        &mut ModeNest::new(),
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut NoopExecHooks,
    )
    .expect("accent lookahead should recover");

    assert_eq!(base, None);
    let summary = input.summary();
    let mut resumed = InputStack::<MemoryInput>::from_summary(&summary, |_, _, _| {
        Ok::<_, core::convert::Infallible>(MemoryInput::new(""))
    })
    .expect("pushed-back token should be checkpoint-resumable");
    let replayed = resumed
        .next_traced_token(&mut stores)
        .expect("read replayed token")
        .expect("closing group should be backed up");
    assert_eq!(replayed, closing_group);
}

#[derive(Default)]
struct CountingRecorder(usize);

impl ReadRecorder for CountingRecorder {
    fn record_meaning(&mut self, _symbol: Symbol, _meaning: Meaning) {
        self.0 += 1;
    }
}

#[test]
fn accent_lookahead_runs_assignments_and_accepts_char_num() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\count0=7 \\char65"));
    let mut recorder = CountingRecorder::default();

    let base = scan_accent_base(
        &mut ModeNest::new(),
        &mut input,
        &mut stores,
        &mut recorder,
        &mut NoopExecHooks,
    )
    .expect("accent base should scan");

    assert_eq!(base, Some(b'A'));
    assert_eq!(stores.count(0), 7);
    assert!(recorder.0 >= 2, "lookahead meanings should be recorded");
}
