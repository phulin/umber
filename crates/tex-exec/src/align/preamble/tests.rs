use super::*;
use crate::{Mode, install_unexpandable_primitives};
use tex_lex::{InputStack, MemoryInput};
use tex_state::scaled::Scaled;
use tex_state::token::OriginId;
use tex_state::{EffectRecord, PrintSink};

fn context_token() -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch: '&',
            cat: Catcode::AlignmentTab,
        },
        OriginId::UNKNOWN,
    )
}

fn scan(primitive: UnexpandablePrimitive, source: &str) -> (Universe, AlignState) {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(source));
    input.begin_alignment();
    let mut execution = crate::ExecutionContext::new("texput");
    let state = scan_preamble(
        primitive,
        context_token(),
        &mut input,
        &mut stores,
        &mut execution,
    )
    .expect("alignment preamble should scan");
    (stores, state)
}

fn character(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

fn terminal_text(stores: &Universe) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|record| match record {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn alignment_scan_spec_consumes_brace_after_keyword_backups() {
    for (source, expected) in [
        ("{#\\cr}", AlignmentPackSpec::Natural),
        (
            "to 12pt{#\\cr}",
            AlignmentPackSpec::Exactly(Scaled::from_raw(12 * Scaled::UNITY)),
        ),
        (
            "spread 2pt{#\\cr}",
            AlignmentPackSpec::Spread(Scaled::from_raw(2 * Scaled::UNITY)),
        ),
    ] {
        let (stores, state) = scan(UnexpandablePrimitive::HAlign, source);
        assert_eq!(state.pack_spec(), expected, "source {source:?}");
        assert_eq!(state.columns().len(), 1, "source {source:?}");
        assert_eq!(
            stores.tokens(state.columns()[0].u_template),
            &[],
            "the source opener must not leak into the u-template"
        );
        assert_eq!(
            stores.tokens(state.columns()[0].v_template),
            &[stores.frozen_end_template_token()]
        );
    }
}

#[test]
fn alignment_initial_mode_matrix() {
    use crate::align::support::{alignment_mode, cell_mode, row_mode};

    assert_eq!(
        alignment_kind(UnexpandablePrimitive::HAlign).expect("alignment test precondition"),
        AlignmentKind::HAlign
    );
    assert_eq!(
        alignment_kind(UnexpandablePrimitive::VAlign).expect("alignment test precondition"),
        AlignmentKind::VAlign
    );
    assert_eq!(
        alignment_mode(AlignmentKind::HAlign),
        Mode::InternalVertical
    );
    assert_eq!(row_mode(AlignmentKind::HAlign), Mode::RestrictedHorizontal);
    assert_eq!(cell_mode(AlignmentKind::HAlign), Mode::RestrictedHorizontal);
    assert_eq!(
        alignment_mode(AlignmentKind::VAlign),
        Mode::RestrictedHorizontal
    );
    assert_eq!(row_mode(AlignmentKind::VAlign), Mode::InternalVertical);
    assert_eq!(cell_mode(AlignmentKind::VAlign), Mode::InternalVertical);
}

#[test]
fn preamble_tabskip_span_expansion_and_delimiter_identity() {
    let (stores, state) = scan(
        UnexpandablePrimitive::HAlign,
        "{\\span x#y&\\tabskip=3pt z#w\\crcr}",
    );

    assert_eq!(state.columns().len(), 2);
    assert_eq!(state.tabskips().len(), 3);
    assert_eq!(stores.glue(state.tabskips()[0]).width.raw(), 0);
    assert_eq!(stores.glue(state.tabskips()[1]).width.raw(), 0);
    assert_eq!(
        stores.glue(state.tabskips()[2]).width.raw(),
        3 * Scaled::UNITY
    );
    assert_eq!(state.default_tabskip(), state.tabskips()[2]);
    assert_eq!(
        stores.tokens(state.columns()[0].u_template),
        &[character('x', Catcode::Letter)]
    );
    assert_eq!(
        stores.tokens(state.columns()[0].v_template),
        &[
            character('y', Catcode::Letter),
            stores.frozen_end_template_token(),
        ]
    );
    assert_eq!(
        stores.tokens(state.columns()[1].u_template),
        &[character('z', Catcode::Letter)]
    );
    assert_eq!(
        stores.tokens(state.columns()[1].v_template),
        &[
            character('w', Catcode::Letter),
            stores.frozen_end_template_token(),
        ]
    );
}

#[test]
fn scan_u_v_templates_empty_nested_and_malformed() {
    let (empty_stores, empty) = scan(UnexpandablePrimitive::HAlign, "{#\\cr}");
    assert_eq!(empty_stores.tokens(empty.columns()[0].u_template), &[]);
    assert_eq!(
        empty_stores.tokens(empty.columns()[0].v_template),
        &[empty_stores.frozen_end_template_token()]
    );

    let (stores, state) = scan(UnexpandablePrimitive::HAlign, "{  a{&}#v##w\\cr}");
    assert_eq!(
        stores.tokens(state.columns()[0].u_template),
        &[
            character('a', Catcode::Letter),
            character('{', Catcode::BeginGroup),
            character('&', Catcode::AlignmentTab),
            character('}', Catcode::EndGroup),
        ],
        "leading spaces are discarded but a nested tab remains template material"
    );
    assert_eq!(
        stores.tokens(state.columns()[0].v_template),
        &[
            character('v', Catcode::Letter),
            character('w', Catcode::Letter),
            stores.frozen_end_template_token(),
        ],
        "a second parameter token is diagnosed and ignored"
    );
    assert!(terminal_text(&stores).contains("Only one # is allowed per tab"));
}
