use super::{InputFrameSummary, InputSummary, LexerState, SourceFrameSummary, SourceId};
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};

fn source_frame(text: &str, pending: Vec<TracedTokenWord>) -> SourceFrameSummary {
    SourceFrameSummary::new(
        0,
        text.len(),
        1,
        1,
        LexerState::MidLine,
        text.to_owned(),
        0,
        pending,
        false,
    )
}

#[test]
fn input_summary_clone_preserves_all_replay_payloads() {
    let pending = vec![TracedTokenWord::pack(
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
        OriginId::UNKNOWN,
    )];
    let active = source_frame(&"a".repeat(256 * 1024), pending.clone());
    let last = source_frame(&"b".repeat(128 * 1024), pending);
    let summary = InputSummary::new(
        vec![InputFrameSummary::Source {
            source_id: SourceId::new(0),
            input_record: None,
            source: active,
        }],
        Some(SourceId::new(1)),
        Some(last),
    );

    let cloned = summary.clone();

    assert_eq!(summary.semantic_root, cloned.semantic_root);
    let (
        InputFrameSummary::Source { source: left, .. },
        InputFrameSummary::Source { source: right, .. },
    ) = (&summary.frames()[0], &cloned.frames()[0])
    else {
        panic!("expected source frames");
    };
    assert_eq!(left.normalized_line, right.normalized_line);
    assert_eq!(left.pending, right.pending);

    let left = summary.last_source_frame().expect("last source");
    let right = cloned.last_source_frame().expect("last source");
    assert_eq!(left.normalized_line, right.normalized_line);
    assert_eq!(left.pending, right.pending);
}
