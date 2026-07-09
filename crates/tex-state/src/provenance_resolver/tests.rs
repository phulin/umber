use crate::ids::OriginListId;
use crate::input::{InputFrameSummary, InputSummary, MacroArguments, TokenListReplayKind};
use crate::macro_store::MacroMeaning;
use crate::meaning::MeaningFlags;
use crate::provenance::SourceOrigin;
use crate::token::{OriginId, Token};
use crate::{ProvenanceResolver, Universe, World};

#[test]
fn resolver_renders_source_line_and_caret() {
    let mut stores = stores_with_input("main.tex", b"alpha\nbeta\n");
    let origin = stores.source_origin(crate::SourceId::new(0), 6, 2, 1);

    let rendered = ProvenanceResolver::new(&stores).render_diagnostic("boom", Some(origin));

    assert!(rendered.contains("boom\n"));
    assert!(rendered.contains("main.tex:2:2"));
    assert!(rendered.contains("  2 | beta"));
    assert!(rendered.contains("    |  ^"));
}

#[test]
fn resolver_treats_rolled_back_origin_as_unknown() {
    let mut stores = stores_with_input("main.tex", b"alpha\n");
    let snapshot = stores.snapshot();
    let stale = stores.source_origin(crate::SourceId::new(0), 0, 1, 0);
    stores.rollback(&snapshot);

    let rendered = ProvenanceResolver::new(&stores).render_diagnostic("boom", Some(stale));

    assert!(rendered.contains("unknown origin"));
}

#[test]
fn resolver_renders_bounded_live_macro_trace() {
    let mut stores = stores_with_input("main.tex", b"\\def\\a{\\endgroup}\\a\n");
    let definition_origin = stores.source_origin(crate::SourceId::new(0), 0, 1, 0);
    let invocation_origin = stores.source_origin(crate::SourceId::new(0), 18, 1, 18);
    let parameter_text = stores.intern_token_list(&[]);
    let endgroup = stores.intern("endgroup");
    let replacement_text = stores.intern_token_list(&[Token::Cs(endgroup)]);
    let definition = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::from_bits(0),
        parameter_text,
        replacement_text,
    ));
    let macro_origin =
        stores.macro_invocation_origin(definition, invocation_origin, definition_origin);
    stores.set_input_summary(InputSummary::new(
        vec![InputFrameSummary::TokenList {
            token_list: replacement_text,
            origin_list: OriginListId::EMPTY,
            replay_kind: TokenListReplayKind::MacroBody,
            index: 0,
            macro_arguments: MacroArguments::new(),
            macro_invocation: macro_origin,
        }],
        None,
        None,
    ));

    let rendered = ProvenanceResolver::with_trace_depth(&stores, 1)
        .render_diagnostic("boom", Some(OriginId::UNKNOWN));

    assert!(rendered.contains("expansion trace:"));
    assert!(rendered.contains("invoked at main.tex:1:19"));
    assert!(rendered.contains("defined at main.tex:1:1"));
}

fn stores_with_input(path: &str, bytes: &[u8]) -> Universe {
    let mut world = World::memory();
    world
        .set_memory_file(path, bytes.to_vec())
        .expect("memory world accepts files");
    let mut stores = Universe::with_world(world);
    stores
        .world_mut()
        .read_file(path)
        .expect("memory input can be read");
    stores
}

#[test]
fn source_origin_accessors_are_covered_for_resolver_inputs() {
    let source = SourceOrigin::new(crate::SourceId::new(7), 3, 2, 1);
    assert_eq!(source.source(), crate::SourceId::new(7));
    assert_eq!(source.byte_offset(), 3);
    assert_eq!(source.line(), 2);
    assert_eq!(source.column(), 1);
}
