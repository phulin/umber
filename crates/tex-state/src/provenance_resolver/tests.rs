use crate::ids::OriginListId;
use crate::input::{InputFrameSummary, InputSummary, MacroArguments, TokenListReplayKind};
use crate::macro_store::MacroMeaning;
use crate::meaning::MeaningFlags;
use crate::provenance::{DiagnosticSite, RelatedLocation, RelatedLocationRole, SourceOrigin};
use crate::source_map::SourceDescriptor;
use crate::token::{OriginId, Token};
use crate::{ProvenanceResolver, Universe, World};
use std::sync::Arc;

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

#[test]
fn resolver_derives_utf8_crlf_and_missing_final_newline_coordinates_from_world_backing() {
    let mut stores = stores_with_input("utf8.tex", "α\r\nbéta".as_bytes());
    let record = stores
        .world()
        .input_records()
        .first()
        .expect("source-map operation succeeds");
    let record_id = crate::InputRecordId::new(0);
    stores
        .register_source(
            crate::SourceId::new(9),
            SourceDescriptor::world(record_id, record.len() as u64),
        )
        .expect("source-map operation succeeds");
    // Deliberately-wrong legacy line/column proves rendering uses the source map.
    let origin = stores.source_origin(crate::SourceId::new(9), 5, 99, 99);

    let rendered = ProvenanceResolver::new(&stores).render_diagnostic("boom", Some(origin));

    assert!(rendered.contains("utf8.tex:2:2"));
    assert!(rendered.contains("  2 | béta"));
    assert!(rendered.contains("    |  ^"));
}

#[test]
fn generated_and_empty_sources_remain_renderable_without_an_input_frame() {
    let mut stores = Universe::new();
    stores
        .register_source(
            crate::SourceId::new(0),
            SourceDescriptor::generated(Arc::from(&b"memory line"[..])),
        )
        .expect("source-map operation succeeds");
    stores
        .register_source(
            crate::SourceId::new(1),
            SourceDescriptor::generated(Arc::from(&b""[..])),
        )
        .expect("source-map operation succeeds");
    let text = stores.source_origin(crate::SourceId::new(0), 7, 40, 40);
    let empty_anchor = stores.source_origin(crate::SourceId::new(1), 0, 1, 0);

    let text_rendered = ProvenanceResolver::new(&stores).render_diagnostic("text", Some(text));
    let empty_rendered =
        ProvenanceResolver::new(&stores).render_diagnostic("empty", Some(empty_anchor));
    assert!(text_rendered.contains("<source 0>:1:8"));
    assert!(text_rendered.contains("memory line"));
    assert!(empty_rendered.contains("<source 1>:1:1"));
}

#[test]
fn missing_source_byte_degrades_without_a_secondary_failure() {
    let mut stores = Universe::new();
    stores
        .register_source(
            crate::SourceId::new(0),
            SourceDescriptor::generated(Arc::from(&b"x"[..])),
        )
        .expect("source-map operation succeeds");
    let origin = stores.source_origin(crate::SourceId::new(0), 99, 7, 3);

    let rendered = ProvenanceResolver::new(&stores).render_diagnostic("boom", Some(origin));

    assert!(rendered.contains("<source 0>:7:4"));
}

#[test]
fn exact_ranges_use_unicode_cells_tab_stops_zero_width_and_multiline_edges() {
    let bytes = "\tα中x\nlast".as_bytes();
    let mut stores = Universe::new();
    stores
        .register_source(
            crate::SourceId::new(0),
            SourceDescriptor::generated(Arc::from(bytes)),
        )
        .expect("source registration");

    let wide = stores.source_range_origin(crate::SourceId::new(0), 1, 6);
    let zero = stores.source_range_origin(crate::SourceId::new(0), 6, 6);
    let multiline = stores.source_range_origin(crate::SourceId::new(0), 3, bytes.len() as u64);

    let wide = ProvenanceResolver::new(&stores).render_diagnostic("wide", Some(wide));
    assert!(wide.contains("<source 0>:1:9"), "{wide}");
    assert!(wide.contains("^^^"), "{wide}");

    let zero = ProvenanceResolver::new(&stores).render_diagnostic("zero", Some(zero));
    assert!(zero.lines().any(|line| line.ends_with('^')), "{zero}");

    let multiline = ProvenanceResolver::new(&stores).render_diagnostic("multi", Some(multiline));
    assert!(multiline.contains("  1 | \tα中x"), "{multiline}");
    assert!(multiline.contains("  2 | last"), "{multiline}");
}

#[test]
fn captured_site_renders_related_locations_and_trace_after_frames_are_gone() {
    let mut stores = stores_with_input("main.tex", b"call definition\n");
    let invocation = stores.source_origin(crate::SourceId::new(0), 0, 1, 0);
    let definition_origin = stores.source_origin(crate::SourceId::new(0), 5, 1, 5);
    let parameter_text = stores.intern_token_list(&[]);
    let replacement_text = stores.intern_token_list(&[]);
    let definition = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::from_bits(0),
        parameter_text,
        replacement_text,
    ));
    let macro_origin = stores.macro_invocation_origin(definition, invocation, definition_origin);
    let site = DiagnosticSite::new(
        Some(invocation),
        [RelatedLocation::new(
            RelatedLocationRole::Definition,
            definition_origin,
        )],
        [macro_origin],
    );
    stores.set_input_summary(InputSummary::new(vec![], None, None));

    let rendered = ProvenanceResolver::new(&stores).render_diagnostic_site("boom", &site);
    assert!(rendered.contains("defined here"), "{rendered}");
    assert!(rendered.contains("expansion trace:"), "{rendered}");
    assert!(rendered.contains("invoked at main.tex:1:1"), "{rendered}");
}
