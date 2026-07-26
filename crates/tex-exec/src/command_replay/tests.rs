use std::sync::Arc;

use tex_command::{
    AlignmentCellTemplates, AlignmentRequest, CommandDeliveryBoundary, CommandObservation,
    CommandObserver, FontResource, InputReason, InputTransition, ObservedToken, RecoveryKind,
    RegisteredSourceKind, SourceRegistration, TracedTokenList,
};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::ids::TokenListId;
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::node::Node;
use tex_state::scaled::Scaled;
use tex_state::{EffectRecord, InputOpenState, StreamSlot, Universe};

use super::*;

fn install_input(universe: &mut Universe) {
    let input = universe.intern("input").symbol();
    universe.set_meaning(
        input,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Input),
    );
}

fn register_source(control: &mut CommandReplayControl, bytes: &[u8]) {
    let source = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("source registers");
    control
        .command_mut()
        .open_registered_source(source)
        .expect("source opens");
}

fn run_to_end(control: &mut CanonicalMainControl, universe: &mut Universe) {
    loop {
        match control.step(universe).expect("canonical program executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

/// Like `run_to_end`, but for the small set of `ExecError`s (such as
/// `CannotDeleteFromCurrentPage`) that abort the run rather than being
/// printed as an ordinary `Universe` diagnostic and continuing.
fn run_to_error(control: &mut CanonicalMainControl, universe: &mut Universe) -> ExecError {
    loop {
        match control.step(universe) {
            Ok(MainControlStep::End | MainControlStep::EndOfInput) => {
                panic!("expected the run to abort with an error before completion")
            }
            Ok(MainControlStep::Continue) => {}
            Err(err) => return err,
        }
    }
}

fn terminal_text(universe: &Universe) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink:
                    tex_state::PrintSink::Terminal
                    | tex_state::PrintSink::TerminalAndLog
                    | tex_state::PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn register_cmr10_font(control: &mut CanonicalMainControl, universe: &mut Universe) {
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    universe
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("font fixture installs");
    let metrics = tex_state::InputReadState::read_input_file(
        &mut universe.input_open_context(),
        std::path::Path::new("cmr10.tfm"),
    )
    .expect("font fixture reads");
    control.capabilities_mut().register_font(
        "cmr10.tfm",
        FontResource::Tfm {
            metrics,
            opentype: None,
        },
    );
}

#[test]
fn canonical_character_definitions_scan_scope_and_recovery() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\chardef\local=65 \mathchardef\math=4660
           {\chardef\local=66 \global\mathchardef\math=9029}
           \globaldefs=1 {\chardef\forced=67}
           \globaldefs=-1 {\global\chardef\suppressed=68}
           \globaldefs=0 \chardef\self=69\self
           \chardef\badchar=256 \mathchardef\badmath=32768 \end",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(
        universe.meaning(universe.symbol("local").expect("local character")),
        Meaning::CharGiven('A')
    );
    assert_eq!(
        universe.meaning(universe.symbol("math").expect("math character")),
        Meaning::MathCharGiven(9029)
    );
    assert_eq!(
        universe.meaning(universe.symbol("badchar").expect("bad character")),
        Meaning::CharGiven('\0')
    );
    assert_eq!(
        universe.meaning(universe.symbol("badmath").expect("bad math character")),
        Meaning::MathCharGiven(0)
    );
    assert_eq!(
        universe.meaning(universe.symbol("forced").expect("globaldefs character")),
        Meaning::CharGiven('C')
    );
    assert_eq!(
        universe.meaning(universe.symbol("suppressed").expect("suppressed character")),
        Meaning::Undefined
    );
    assert_eq!(
        universe.meaning(universe.symbol("self").expect("self-referential character")),
        Meaning::CharGiven('E')
    );
    let output = terminal_text(&universe);
    assert!(output.contains("Bad character code (256)"));
    assert!(output.contains("Bad mathchar (32768)"));
}

#[test]
fn canonical_pdf_navigation_scans_rules_actions_and_deferred_markers() {
    let mut universe = Universe::new();
    universe.set_int_param(IntParam::PDF_OUTPUT, 1);
    for (name, primitive) in [
        (
            "pdfannot",
            tex_state::meaning::UnexpandablePrimitive::PdfAnnot,
        ),
        (
            "pdfdest",
            tex_state::meaning::UnexpandablePrimitive::PdfDest,
        ),
        (
            "pdfstartlink",
            tex_state::meaning::UnexpandablePrimitive::PdfStartLink,
        ),
        (
            "pdfendlink",
            tex_state::meaning::UnexpandablePrimitive::PdfEndLink,
        ),
        (
            "pdfthread",
            tex_state::meaning::UnexpandablePrimitive::PdfThread,
        ),
        (
            "pdfstartthread",
            tex_state::meaning::UnexpandablePrimitive::PdfStartThread,
        ),
        (
            "pdfendthread",
            tex_state::meaning::UnexpandablePrimitive::PdfEndThread,
        ),
    ] {
        let symbol = universe.intern(name).symbol();
        universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Horizontal);
    register_source(
        &mut control,
        br"\pdfannot width 2pt height 3pt { /Subtype /Text }\pdfdest name {target} fitr depth 4pt\pdfstartlink width 5pt attr { /Border [0 0 0] } goto name {target}\pdfendlink\pdfthread depth 3pt width 10pt height 4pt attr { /I << /Title (custom) >> } name {chapter}\pdfstartthread height 7pt name {running}\pdfendthread\pdfstartlink thread name {reserved}\pdfendlink",
    );
    run_to_end(&mut control, &mut universe);

    let nodes = control.modes.current_list().nodes();
    assert!(
        matches!(
            nodes[0],
            Node::Whatsit(tex_state::node::Whatsit::PdfAnnotation { .. })
        ),
        "nodes: {nodes:#?}"
    );
    assert!(matches!(
        nodes[1],
        Node::Whatsit(tex_state::node::Whatsit::PdfDestination(_))
    ));
    assert!(matches!(
        nodes[2],
        Node::Whatsit(tex_state::node::Whatsit::PdfLinkStart { .. })
    ));
    assert!(matches!(
        nodes[3],
        Node::Whatsit(tex_state::node::Whatsit::PdfLinkEnd { .. })
    ));
    let Node::Whatsit(tex_state::node::Whatsit::PdfThread(thread)) = &nodes[4] else {
        panic!("expected one-shot thread marker: {nodes:#?}");
    };
    assert_eq!(thread.dimensions.width, Some(Scaled::from_raw(10 * 65_536)));
    assert_eq!(thread.dimensions.height, Some(Scaled::from_raw(4 * 65_536)));
    assert_eq!(thread.dimensions.depth, Some(Scaled::from_raw(3 * 65_536)));
    assert!(!thread.running);
    assert_ne!(thread.attributes, TokenListId::EMPTY);
    let Node::Whatsit(tex_state::node::Whatsit::PdfThread(thread)) = &nodes[5] else {
        panic!("expected running thread marker: {nodes:#?}");
    };
    assert_eq!(thread.dimensions.width, None);
    assert_eq!(thread.dimensions.height, Some(Scaled::from_raw(7 * 65_536)));
    assert_eq!(thread.dimensions.depth, None);
    assert!(thread.running);
    assert!(matches!(
        nodes[6],
        Node::Whatsit(tex_state::node::Whatsit::PdfEndThread)
    ));
    assert!(matches!(
        nodes[7],
        Node::Whatsit(tex_state::node::Whatsit::PdfLinkStart { .. })
    ));
    assert!(matches!(
        nodes[8],
        Node::Whatsit(tex_state::node::Whatsit::PdfLinkEnd { .. })
    ));
    assert!(universe.pdf_threads().iter().any(|thread| {
        thread.identity() == &tex_state::PdfDestinationIdentity::Name(b"reserved".to_vec())
            && thread.beads().is_empty()
    }));
    assert!(
        universe.open_pdf_links().is_empty(),
        "canonical scanning defers link nesting to final traversal"
    );
}

#[test]
fn canonical_pdf_graphics_objects_and_forms_cross_only_typed_requests() {
    let mut universe = Universe::new();
    universe.set_int_param(IntParam::PDF_OUTPUT, 1);
    for (name, primitive) in [
        (
            "pdfliteral",
            tex_state::meaning::UnexpandablePrimitive::PdfLiteral,
        ),
        (
            "pdfsetmatrix",
            tex_state::meaning::UnexpandablePrimitive::PdfSetMatrix,
        ),
        (
            "pdfsave",
            tex_state::meaning::UnexpandablePrimitive::PdfSave,
        ),
        (
            "pdfrestore",
            tex_state::meaning::UnexpandablePrimitive::PdfRestore,
        ),
        (
            "pdfcolorstack",
            tex_state::meaning::UnexpandablePrimitive::PdfColorStack,
        ),
        (
            "pdfsavepos",
            tex_state::meaning::UnexpandablePrimitive::PdfSavePos,
        ),
        (
            "pdfobj",
            tex_state::meaning::UnexpandablePrimitive::PdfObject,
        ),
        (
            "pdfrefobj",
            tex_state::meaning::UnexpandablePrimitive::PdfReferenceObject,
        ),
        (
            "pdfxform",
            tex_state::meaning::UnexpandablePrimitive::PdfXForm,
        ),
        (
            "pdfrefxform",
            tex_state::meaning::UnexpandablePrimitive::PdfRefXForm,
        ),
        (
            "pdfinfo",
            tex_state::meaning::UnexpandablePrimitive::PdfInfo,
        ),
        (
            "pdfcatalog",
            tex_state::meaning::UnexpandablePrimitive::PdfCatalog,
        ),
    ] {
        let symbol = universe.intern(name).symbol();
        universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Horizontal);
    register_source(
        &mut control,
        br"\pdfliteral direct{q}\pdfliteral shipout page{Q}\pdfsetmatrix{1 0 0 1}\pdfsave\pdfrestore\pdfcolorstack0 current\pdfsavepos\setbox0=\hbox{}\pdfobj{raw}\pdfrefobj1\pdfxform0\pdfrefxform2\pdfinfo{/Producer(test)}\pdfcatalog{/PageMode/UseNone}openaction thread name{catalog-thread}",
    );
    run_to_end(&mut control, &mut universe);

    let nodes = control.modes.current_list().nodes();
    assert!(matches!(
        nodes[0],
        Node::Whatsit(tex_state::node::Whatsit::PdfLiteral { .. })
    ));
    assert!(matches!(
        nodes[1],
        Node::Whatsit(tex_state::node::Whatsit::DeferredPdfLiteral { .. })
    ));
    assert!(matches!(
        nodes[2],
        Node::Whatsit(tex_state::node::Whatsit::PdfSetMatrix { .. })
    ));
    assert!(matches!(
        nodes[3],
        Node::Whatsit(tex_state::node::Whatsit::PdfSave)
    ));
    assert!(matches!(
        nodes[4],
        Node::Whatsit(tex_state::node::Whatsit::PdfRestore)
    ));
    assert!(matches!(
        nodes[5],
        Node::Whatsit(tex_state::node::Whatsit::PdfColorStack { .. })
    ));
    assert!(matches!(
        nodes[6],
        Node::Whatsit(tex_state::node::Whatsit::PdfSavePos)
    ));
    assert!(matches!(
        nodes[7],
        Node::Whatsit(tex_state::node::Whatsit::PdfReferenceObject { object: 1 })
    ));
    assert!(matches!(
        nodes[8],
        Node::Whatsit(tex_state::node::Whatsit::PdfRefXForm { object: 2, .. })
    ));
    assert_eq!(universe.pdf_raw_objects().len(), 1);
    assert!(
        !universe.pdf_raw_objects()[0].is_referenced(),
        "pdfrefobj remains a deferred shipout handoff"
    );
    assert_eq!(
        universe.pdf_form(2).expect("form exists").resource(),
        1,
        "form resource names use their independent source-order identity"
    );
    assert_eq!(
        universe
            .pdf_document_fragments(tex_state::PdfDocumentFragmentKind::Info)
            .count(),
        1
    );
    assert!(universe.pdf_catalog_open_action().is_some());
    assert!(universe.pdf_threads().iter().any(|thread| {
        thread.identity() == &tex_state::PdfDestinationIdentity::Name(b"catalog-thread".to_vec())
            && thread.beads().is_empty()
    }));
}

#[test]
fn canonical_math_replay_finalizes_fields_and_delimiter_groups_before_parent_source() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Math);
    register_source(&mut control, br"\mathop{a}^b\left( c d \right)\over e\end");
    run_to_end(&mut control, &mut universe);

    let content = take_finished_canonical_math_list(&mut control.modes, &mut universe)
        .expect("math material freezes");
    let nodes = universe.nodes(content);
    assert_eq!(nodes.len(), 1, "generalized fraction completes at list end");
    let tex_state::node_arena::NodeRef::FractionNoad(fraction) = nodes.first().expect("fraction")
    else {
        panic!("expected generalized fraction");
    };
    let numerator = universe.nodes(fraction.numerator);
    assert!(numerator.len() >= 2);
    let tex_state::node_arena::NodeRef::MathNoad(operator) = numerator.first().expect("operator")
    else {
        panic!("operator noad");
    };
    assert!(matches!(
        operator.kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Op)
    ));
    assert!(!matches!(
        operator.superscript,
        tex_state::math::MathField::Empty
    ));
    let inner = numerator
        .iter()
        .find_map(|node| match node {
            tex_state::node_arena::NodeRef::MathNoad(noad)
                if matches!(
                    noad.kind,
                    tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Inner)
                ) =>
            {
                Some(noad)
            }
            _ => None,
        })
        .expect("left/right inner noad");
    let tex_state::math::MathField::SubMlist(inner_list) = inner.nucleus else {
        panic!("left/right inner noad");
    };
    assert!(universe.nodes(inner_list).iter().any(|node| matches!(node, tex_state::node_arena::NodeRef::MathNoad(noad) if matches!(noad.kind, tex_state::math::NoadKind::RightDelimiter { .. }))));
}

#[test]
fn canonical_math_field_kern_lookahead_does_not_leak_past_field_boundary() {
    // TeX82's `scan_dimen` (§455) reads one token past a unit like `pt` to
    // check for an optional trailing space, backing it up when absent
    // (§448's `scan_keyword`/optional-space handling). When that lookahead
    // happens to land on the very last token of a command-owned math field
    // episode -- `{\kern1pt}` as a braced nucleus, say -- retiring the
    // stored replay level is discovered deep inside `scan_dimension`, not at
    // `execute_math_episode`'s own top-level command fetch. TeX82's
    // `get_x_token` deliberately "retains TeX82's uninterrupted behavior by
    // consuming this boundary internally" here (docs/tex_command_core.md
    // §2.1's description of scalar lookahead crossing a stored episode), so
    // `execute_math_episode` must detect the field's end by polling
    // `CommandState::replay_episode_is_active` rather than by waiting for
    // its own `ScannedStep::ReplayCompleted` event, which this exact
    // lookahead never produces. Before that fix this source would loop past
    // the field into `Z` and beyond, eventually hitting an EOF-driven
    // `ExecError::MissingToken`; the fix keeps `Z` a sibling ordinary math
    // character noad instead of folding it into the kern field's sub-mlist.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Math);
    register_source(&mut control, br"\mathord{\kern1pt}Z\end");
    run_to_end(&mut control, &mut universe);

    let content = take_finished_canonical_math_list(&mut control.modes, &mut universe)
        .expect("math material freezes");
    let nodes = universe.nodes(content);
    assert_eq!(
        nodes.len(),
        2,
        "the kern field and the following Z must be separate noads"
    );
    assert!(
        matches!(
            nodes.first(),
            Some(tex_state::node_arena::NodeRef::MathNoad(
                tex_state::math::MathNoad {
                    nucleus: tex_state::math::MathField::SubMlist(_),
                    ..
                }
            ))
        ),
        "the braced kern field freezes as its own sub-mlist nucleus"
    );
    assert!(
        matches!(
            nodes.get(1),
            Some(tex_state::node_arena::NodeRef::MathNoad(
                tex_state::math::MathNoad {
                    nucleus: tex_state::math::MathField::MathChar(tex_state::math::MathChar {
                        character: 'Z',
                        ..
                    }),
                    ..
                }
            ))
        ),
        "Z must land as ordinary trailing math content, not inside the kern field"
    );
}

#[test]
fn canonical_math_replay_observer_does_not_change_frozen_mlist() {
    let source = br"\mathord{a}_b^c\mskip2mu\mkern3mu\over d\end";
    let mut plain_universe = Universe::new();
    let mut plain = CanonicalMainControl::tex82_initex(&mut plain_universe);
    plain.modes.push(Mode::Math);
    register_source(&mut plain, source);
    run_to_end(&mut plain, &mut plain_universe);
    let plain_list = take_finished_canonical_math_list(&mut plain.modes, &mut plain_universe)
        .expect("plain mlist freezes");

    let mut observed_universe = Universe::new();
    let mut observed = CanonicalMainControl::tex82_initex(&mut observed_universe);
    observed.modes.push(Mode::Math);
    register_source(&mut observed, source);
    let mut observations = ObservationRecorder::default();
    loop {
        match observed
            .step_with_observer(&mut observed_universe, &mut observations)
            .expect("observed canonical math executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    let observed_list =
        take_finished_canonical_math_list(&mut observed.modes, &mut observed_universe)
            .expect("observed mlist freezes");
    let plain_nodes = plain_universe.nodes(plain_list);
    let observed_nodes = observed_universe.nodes(observed_list);
    assert_eq!(plain_nodes.len(), observed_nodes.len());
    assert!(matches!(
        plain_nodes.first(),
        Some(tex_state::node_arena::NodeRef::FractionNoad(_))
    ));
    assert!(matches!(
        observed_nodes.first(),
        Some(tex_state::node_arena::NodeRef::FractionNoad(_))
    ));
    assert_eq!(
        plain_universe.world().effect_records(),
        observed_universe.world().effect_records()
    );
}

#[test]
fn canonical_math_family_assignment_and_fam_select_variable_mathcode_family() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    control.modes.push(Mode::Math);
    register_source(
        &mut control,
        br#"\font\f=cmr10 \textfont2\f \mathcode`a="7161 \fam2 a\end"#,
    );
    run_to_end(&mut control, &mut universe);
    let f = universe.intern("f");
    let font = match universe.meaning(f) {
        Meaning::Font(font) => font,
        meaning => panic!("font definition missing: {meaning:?}"),
    };
    assert_eq!(
        universe.math_family_font(tex_state::math::MathFontSize::Text, 2),
        font
    );
    let nodes = control.modes.current_list().nodes();
    let tex_state::node::Node::MathNoad(noad) = nodes.last().expect("variable-family noad") else {
        panic!("math noad");
    };
    let tex_state::math::MathField::MathChar(character) = noad.nucleus else {
        panic!("math character field");
    };
    assert_eq!(character.family, 2);
}

#[test]
fn canonical_font_definition_scans_size_and_respects_local_global_scope() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\begingroup\font\local=cmr10 at 11pt\global\font\global=cmr10 scaled 1200\endgroup\end",
    );

    run_to_end(&mut control, &mut universe);

    let local = universe.intern("local");
    let global = universe.intern("global");
    assert!(matches!(universe.meaning(local), Meaning::Undefined));
    assert!(matches!(universe.meaning(global), Meaning::Font(_)));
}

#[test]
fn canonical_missing_font_rolls_back_then_retries_once_with_registered_resource() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\font\f=cmr10\message{ok}\end");
    let mut observations = ObservationRecorder::default();

    assert!(matches!(
        control
            .advance_with_observer(&mut universe, &mut observations)
            .expect("missing font suspends"),
        CanonicalStepResult::Suspended(CanonicalResourceNeed::Font { request })
            if request.name == "cmr10"
    ));
    let f = universe.intern("f");
    assert!(matches!(universe.meaning(f), Meaning::Undefined));
    assert!(
        observations.0.is_empty(),
        "suspended command leaked observations"
    );

    register_cmr10_font(&mut control, &mut universe);
    assert!(matches!(
        control
            .advance_with_observer(&mut universe, &mut observations)
            .expect("fresh retry installs font"),
        CanonicalStepResult::Progress(ReplayStep::Continue)
    ));
    assert!(matches!(universe.meaning(f), Meaning::Font(_)));
    run_to_end(&mut control, &mut universe);
    assert_eq!(terminal_text(&universe), "ok");
}

#[test]
fn canonical_unavailable_font_recovers_to_nullfont() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control
        .capabilities_mut()
        .register_font("missing.tfm", FontResource::Unavailable);
    register_source(&mut control, br"\font\missing=missing\end");

    run_to_end(&mut control, &mut universe);

    let missing = universe.intern("missing");
    assert_eq!(
        universe.meaning(missing),
        Meaning::Font(tex_state::font::NULL_FONT)
    );
}

#[test]
fn canonical_openin_read_and_closein_use_registered_immutable_input() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(&b"hello"[..]),
        ),
    );
    register_source(
        &mut control,
        br"\openin1=child.tex \read1 to \line \closein1\end",
    );

    run_to_end(&mut control, &mut universe);

    let line = universe.intern("line");
    let replacement = universe
        .macro_meaning(line)
        .expect("read target is defined")
        .replacement_text();
    assert_eq!(
        universe.tokens(replacement).first(),
        Some(&tex_state::token::Token::Char {
            ch: 'h',
            cat: tex_state::token::Catcode::Letter
        })
    );
    assert!(universe.world().input_stream_eof(StreamSlot::new(1)));
}

#[test]
fn canonical_read_collects_balanced_multiline_text_and_recovers_file_eof() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(&b"{one\ntwo"[..]),
        ),
    );
    register_source(&mut control, br"\openin1=child.tex \read1 to \line\end");

    run_to_end(&mut control, &mut universe);

    let line = universe.intern("line");
    let replacement = universe
        .macro_meaning(line)
        .expect("read target is defined")
        .replacement_text();
    let text: String = universe
        .tokens(replacement)
        .iter()
        .filter_map(|token| match token {
            tex_state::token::Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert!(text.contains("one"));
    assert!(text.contains("two"));
    assert!(text.ends_with('}'));
    assert!(terminal_text(&universe).contains("File ended within \\read"));
}

#[test]
fn canonical_terminal_read_prompts_once_and_collects_until_balanced() {
    let mut universe = Universe::new();
    universe.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
    universe
        .world_mut()
        .push_memory_terminal_line("{first")
        .expect("terminal line");
    universe
        .world_mut()
        .push_memory_terminal_line("second}")
        .expect("terminal line");
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\read1 to \line\end");

    run_to_end(&mut control, &mut universe);

    let output = terminal_text(&universe);
    assert_eq!(output.matches("\\line=").count(), 1, "{output:?}");
    let line = universe.intern("line");
    let replacement = universe
        .macro_meaning(line)
        .expect("read target")
        .replacement_text();
    assert!(
        universe
            .tokens(replacement)
            .iter()
            .any(|token| { matches!(token, tex_state::token::Token::Char { ch: 's', .. }) })
    );
}

#[test]
fn canonical_read_closes_partial_text_at_an_outer_token() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(&b"{x\\stop"[..]),
        ),
    );
    register_source(
        &mut control,
        br"\outer\def\stop{}\openin1=child.tex \read1 to \line\end",
    );

    run_to_end(&mut control, &mut universe);

    let line = universe.intern("line");
    let replacement = universe
        .macro_meaning(line)
        .expect("read target")
        .replacement_text();
    assert!(matches!(
        universe.tokens(replacement).last(),
        Some(tex_state::token::Token::Char {
            cat: tex_state::token::Catcode::EndGroup,
            ..
        })
    ));
}

#[test]
fn canonical_openin_missing_resource_rolls_back_and_retries_fresh() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\openin1=child.tex\read1to\line\end");

    assert!(matches!(
        control.advance(&mut universe).expect("openin suspends"),
        CanonicalStepResult::Suspended(CanonicalResourceNeed::Input { name }) if name == "child.tex"
    ));
    assert!(universe.world().input_stream_eof(StreamSlot::new(1)));

    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(&b"retry"[..]),
        ),
    );
    assert!(matches!(
        control.advance(&mut universe).expect("fresh openin retry"),
        CanonicalStepResult::Progress(ReplayStep::Continue)
    ));
    run_to_end(&mut control, &mut universe);
    let line = universe.intern("line");
    assert!(universe.macro_meaning(line).is_some());
}

#[test]
fn canonical_begingroup_uses_semisimple_local_and_global_restoration() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\begingroup\count0=1\global\count1=2\endgroup\count2=3\end",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.count(0), 0);
    assert_eq!(universe.count(1), 2);
    assert_eq!(universe.count(2), 3);
}

#[test]
fn canonical_definition_recovery_keeps_target_and_parameter_tokens_command_owned() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\def A{}\def\f#2{#2}\count7=9\end");

    run_to_end(&mut control, &mut universe);

    let inaccessible = universe.intern("inaccessible");
    assert!(universe.macro_meaning(inaccessible).is_some());
    assert_eq!(universe.count(7), 9);
    let output = terminal_text(&universe);
    assert!(output.contains("Missing control sequence inserted"));
    assert!(output.contains("Illegal parameter number in definition"));
}

#[test]
fn canonical_macro_prefix_mismatch_surfaces_its_own_command_error() {
    // `\foo` requires the literal text `XY` before its (parameterless) body,
    // but the invocation supplies `AB`. `CommandError::MacroPrefixMismatch`
    // is not recovered anywhere in `tex-command`'s expansion loop (unlike
    // `ParagraphInMacroArgument`/`OuterInMacroArgument`, which are absorbed
    // in `processor/expand.rs`), so it must reach `tex-exec` as its own
    // named `ExecError::Command` variant rather than collapsing into a
    // generic `ExecError::MissingToken`.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\def\foo XY{Z}\foo AB");

    let error = loop {
        match control.step(&mut universe) {
            Ok(MainControlStep::End | MainControlStep::EndOfInput) => {
                panic!("expected the mismatched macro invocation to fail")
            }
            Ok(MainControlStep::Continue) => {}
            Err(error) => break error,
        }
    };

    assert!(
        matches!(error, ExecError::Command(CommandError::MacroPrefixMismatch)),
        "expected ExecError::Command(CommandError::MacroPrefixMismatch), got {error:?}"
    );
    assert_eq!(
        error.to_string(),
        "macro invocation does not match its definition"
    );
}

#[test]
fn canonical_grouping_reports_and_recovers_extra_closers() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"}\endgroup\begingroup}\count7=9\end");

    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.count(7), 9);
    let output = terminal_text(&universe);
    assert!(output.contains("Too many }'s"));
    assert!(output.contains("Extra \\endgroup"));
    assert!(output.contains("Extra }, or forgotten \\endgroup"));
}

#[test]
fn replay_uses_typed_scanners_for_definitions_assignments_and_termination() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(
        &mut control,
        br"\def\id#1{#1}\count12=\id{7}\global\def\g{z}\end",
    );

    assert_eq!(
        control.step(&mut universe).expect("definition"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(12), 7);
    assert_eq!(
        control.step(&mut universe).expect("global definition"),
        ReplayStep::Continue
    );
    let id = universe.intern("id").symbol();
    let g = universe.intern("g").symbol();
    assert!(universe.macro_meaning(id).is_some());
    assert!(universe.macro_meaning(g).is_some());
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
    assert_eq!(
        control.step(&mut universe).expect("eof"),
        ReplayStep::EndOfInput
    );
}

#[test]
fn production_driver_replays_paragraph_backup_and_typed_assignment() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, b"a\\count3=9\\end");

    // TeX82 §1095 starts the paragraph only after the command processor has
    // backed up the triggering character.  The next step redelivers it.
    assert_eq!(
        control.step(&mut universe).expect("paragraph start"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("replayed character"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("typed count scan"),
        MainControlStep::Continue
    );
    assert_eq!(universe.count(3), 9);
}

#[test]
fn production_driver_schedules_everypar_before_replayed_first_character() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\everypar{\count7=41}a\par\end");

    assert_eq!(
        control.step(&mut universe).expect("everypar assignment"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("paragraph start"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("backup replay boundary"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("everypar replay"),
        MainControlStep::Continue
    );
    assert_eq!(
        universe.count(7),
        41,
        "everypar runs before the backup replay"
    );
    assert_eq!(
        control.step(&mut universe).expect("first character replay"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("paragraph end"),
        MainControlStep::Continue
    );
}

#[test]
fn production_driver_applies_typed_parshape_indent_and_horizontal_nodes() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\parshape=1 2pt 20pt\noindent\char65\kern2pt\hskip3pt\hfil",
    );

    for context in ["parshape", "noindent", "char", "kern", "hskip", "hfil"] {
        assert_eq!(
            control.step(&mut universe).expect(context),
            MainControlStep::Continue
        );
    }

    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(universe.paragraph_shape().len(), 1);
    assert_eq!(
        universe.paragraph_shape()[0].indent,
        Scaled::from_raw(2 * Scaled::UNITY)
    );
    assert_eq!(
        universe.paragraph_shape()[0].width,
        Scaled::from_raw(20 * Scaled::UNITY)
    );
    assert!(
        control
            .modes
            .current_list()
            .nodes()
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Kern { .. }))
    );
    assert!(
        control
            .modes
            .current_list()
            .nodes()
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Glue { .. }))
    );
}

#[test]
fn canonical_paragraph_page_builder_is_observer_neutral() {
    #[derive(Debug, Eq, PartialEq)]
    struct MemoParity {
        lookups: u64,
        hits: u64,
        misses: u64,
        inserts: u64,
        page_lookups: u64,
        page_hits: u64,
        page_inserts: u64,
        page_contributions_skipped: u64,
        page_key_misses: u64,
        page_validation_failures: u64,
    }

    fn run(observed: bool) -> (u64, Vec<EffectRecord>, MemoParity, Mode) {
        let mut universe = Universe::with_world(tex_state::World::memory());
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        universe.enable_pure_memo(tex_state::PureMemoConfig::default());
        universe.enable_page_memo();
        register_source(
            &mut control,
            br"\vsize=1pt\noindent paragraph text\ifhmode\count7=1\else\count7=2\fi\par\hrule height 10pt\vfill\eject\end",
        );
        let mut observations = ObservationRecorder::default();

        loop {
            let step = if observed {
                control.step_with_observer(&mut universe, &mut observations)
            } else {
                control.step(&mut universe)
            }
            .expect("canonical paragraph/page-builder program");
            if matches!(step, ReplayStep::End | ReplayStep::EndOfInput) {
                break;
            }
        }
        assert_eq!(universe.count(7), 1, "conditional sees horizontal mode");
        let memo = universe.pure_memo_stats();
        (
            universe.snapshot().state_hash(),
            universe.world().effect_records().to_vec(),
            MemoParity {
                lookups: memo.lookups,
                hits: memo.hits,
                misses: memo.misses,
                inserts: memo.inserts,
                page_lookups: memo.page_lookups,
                page_hits: memo.page_hits,
                page_inserts: memo.page_inserts,
                page_contributions_skipped: memo.page_contributions_skipped,
                page_key_misses: memo.page.key_misses,
                page_validation_failures: memo.page.validation_failures,
            },
            control.current_mode(),
        )
    }

    let cold = run(false);
    let observed = run(true);
    assert_eq!(observed, cold);
}

#[test]
fn production_driver_executes_discretionary_parts_in_isolated_hmode_episodes() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\noindent\discretionary{\count3=7\kern1pt}{\kern2pt}{\kern3pt}\count4=9",
    );

    assert_eq!(
        control.step(&mut universe).expect("paragraph start"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("discretionary episodes"),
        MainControlStep::Continue
    );
    assert_eq!(universe.count(3), 0, "disc group assignments stay local");
    let Some(tex_state::node::Node::Disc {
        kind: tex_state::node::DiscKind::Discretionary,
        pre,
        post,
        replace,
    }) = control.modes.current_list().nodes().last()
    else {
        panic!("canonical replay appended a discretionary node");
    };
    for (part, expected) in [
        (pre, Scaled::from_raw(Scaled::UNITY)),
        (post, Scaled::from_raw(2 * Scaled::UNITY)),
        (replace, Scaled::from_raw(3 * Scaled::UNITY)),
    ] {
        assert!(matches!(
            universe.nodes(*part).first(),
            Some(tex_state::node_arena::NodeRef::Kern { amount, .. }) if amount == expected
        ));
    }
    assert_eq!(
        control.step(&mut universe).expect("following command"),
        MainControlStep::Continue
    );
    assert_eq!(universe.count(4), 9);
}

#[test]
fn production_driver_enters_and_packs_hbox_without_legacy_dispatch() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{A}");

    for context in [
        "setbox",
        "hbox opening",
        "body brace",
        "replayed brace",
        "hbox character",
        "hbox close",
    ] {
        assert_eq!(
            control.step(&mut universe).expect(context),
            MainControlStep::Continue
        );
    }
    let box_nodes = universe
        .box_reg(0)
        .map(|id| universe.nodes(id))
        .expect("canonical hbox was assigned");
    assert!(matches!(
        box_nodes.first(),
        Some(tex_state::node_arena::NodeRef::HList(_))
    ));
}

#[test]
fn production_driver_hands_box_math_and_alignment_to_typed_control() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\vbox{}$$\halign{#\cr a\cr}\end");

    assert_eq!(
        control.step(&mut universe).expect("setbox scan"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("vbox opening"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("vbox body opening"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("replayed vbox brace"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("vbox completion"),
        MainControlStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("paragraph start before math"),
        MainControlStep::Continue
    );
    assert!(matches!(control.current_mode(), crate::Mode::Horizontal));
    assert_eq!(
        control.step(&mut universe).expect("display math entry"),
        MainControlStep::Continue
    );
    assert!(matches!(control.current_mode(), crate::Mode::DisplayMath));
    assert_eq!(
        control.step(&mut universe).expect("alignment begin"),
        MainControlStep::Continue
    );
    assert!(control.active_alignment().is_some());
}

#[test]
fn production_driver_indent_in_display_math_appends_an_ord_sub_box() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, b"$$\\indent");

    assert_eq!(
        control.step(&mut universe).expect("paragraph start"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("display math entry"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("display-math indent"),
        MainControlStep::Continue
    );

    assert_eq!(control.current_mode(), crate::Mode::DisplayMath);
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::MathNoad(MathNoad {
            kind: NoadKind::Normal(NoadClass::Ord),
            nucleus: MathField::SubBox(_),
            ..
        })]
    ));
}

#[test]
fn production_driver_char_num_in_display_math_appends_a_math_char_noad() {
    // TeX82 §1154's `mmode+char_num` scans the character number and calls
    // `set_math_char` (§1155) exactly like an ordinary math-mode letter or
    // other character: it must append a math-char noad, not reach the
    // horizontal-mode character path.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, b"$$\\char43 ");

    assert_eq!(
        control.step(&mut universe).expect("paragraph start"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("display math entry"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("display-math char_num"),
        MainControlStep::Continue
    );

    assert_eq!(control.current_mode(), crate::Mode::DisplayMath);
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::MathNoad(MathNoad {
            kind: NoadKind::Normal(NoadClass::Ord),
            nucleus: MathField::MathChar(_),
            ..
        })]
    ));
}

#[test]
fn production_driver_hrule_in_math_mode_inserts_missing_dollar_and_replays_hrule() {
    // TeX82 §1046 lists `mmode+hrule` among the "math-only cases in
    // non-math modes, or vice versa"; §1047's `insert_dollar_sign` closes
    // math mode with an inserted `$` and replays `\hrule` in the resulting
    // mode instead of reaching the generic unimplemented-typesetting error.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, b"a$\\hrule\\par");

    // TeX82 §1090's `vmode+letter` backs the letter up and starts the
    // paragraph; the letter itself is appended on the next delivery.
    assert_eq!(
        control
            .step(&mut universe)
            .expect("letter starts paragraph"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control
            .step(&mut universe)
            .expect("backed-up letter appends"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);

    assert_eq!(
        control.step(&mut universe).expect("math shift enters math"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Math);

    assert_eq!(
        control
            .step(&mut universe)
            .expect("hrule recovers a missing math shift"),
        MainControlStep::Continue
    );
    assert!(terminal_text(&universe).contains("Missing $ inserted"));
    // The recovery only rewrites pending input; the mode transition happens
    // once the inserted `$` is itself replayed as the next command.
    assert_eq!(control.current_mode(), crate::Mode::Math);

    assert_eq!(
        control.step(&mut universe).expect("inserted $ closes math"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);

    assert_eq!(
        control
            .step(&mut universe)
            .expect("replayed hrule ends the paragraph and contributes a rule"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Vertical);
    // §1056's `vmode+hrule` resets `prev_depth` to the ignored-depth
    // sentinel; observing that confirms the replayed `\hrule` reached the
    // ordinary vertical rule path rather than being silently dropped.
    assert_eq!(
        control.modes.current_list().prev_depth(),
        Some(crate::mode::ignored_depth(&universe))
    );
}

#[test]
fn show_reads_its_target_raw_without_starting_macro_matching() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\def\shown#1{#1}\show\shown\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("definition"),
        ReplayStep::Continue
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("show"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [
            CommandObservation::Command(show_raw),
            CommandObservation::Command(show_expanded),
            CommandObservation::Command(target_raw),
        ] if show_raw.command == "xray"
            && show_raw.boundary == CommandDeliveryBoundary::Raw
            && show_expanded.command == "xray"
            && show_expanded.boundary == CommandDeliveryBoundary::Expanded
            && target_raw.command == "call"
            && target_raw.boundary == CommandDeliveryBoundary::Raw
    ));
}

#[test]
fn canonical_initex_replay_scans_and_applies_integer_parameters() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\year=2026\month=7\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("year assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.int_param(IntParam::YEAR), 2026);
    assert_eq!(
        control.step(&mut universe).expect("month assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.int_param(IntParam::MONTH), 7);
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);

    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Scanner(scanner),
            CommandObservation::Mutation(mutation)]
            if scanner.kind == "integer"
                && scanner.value == "2026"
                && mutation.target == "parameter"
                && mutation.value == "integer_parameter:23=2026"
    ));
}

#[test]
fn replay_executes_immediate_stream_extensions_and_replays_other_lookahead() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\immediate\openout2=trace \immediate\write2{ready}\immediate\closeout2\immediate\catcode`A=12\end",
    );

    for _ in 0..5 {
        assert_eq!(
            control.step(&mut universe).expect("immediate replay"),
            ReplayStep::Continue
        );
    }
    assert_eq!(universe.catcode('A'), tex_state::token::Catcode::Other);
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
    assert!(matches!(
        universe.world().effect_records(),
        [
            EffectRecord::StreamOpen { slot, target },
            EffectRecord::StreamWrite { sink: tex_state::PrintSink::Stream(write_slot), text },
            EffectRecord::StreamClose { slot: close_slot },
        ] if *slot == StreamSlot::new(2)
            && target.path() == std::path::Path::new("trace.tex")
            && *write_slot == StreamSlot::new(2)
            && text == "ready\n"
            && *close_slot == StreamSlot::new(2)
    ));
}

#[test]
fn replay_appends_an_unexpanded_deferred_write_whatsit() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    control.modes.push(Mode::RestrictedHorizontal);
    register_source(&mut control, br"\write2{during}");

    assert_eq!(
        control.step(&mut universe).expect("deferred write"),
        ReplayStep::Continue
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(tex_state::node::Whatsit::DeferredWrite { sink, .. })]
            if *sink == tex_state::PrintSink::Stream(StreamSlot::new(2))
    ));
}

#[test]
fn canonical_initex_replay_scans_tabskip_before_alignment_preamble() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\tabskip = 2pt\halign&\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("tabskip assignment"),
        ReplayStep::Continue
    );
    let tabskip = universe.glue(universe.glue_param(GlueParam::TAB_SKIP));
    assert_eq!(tabskip.width, Scaled::from_raw(2 * Scaled::UNITY));
    assert!(matches!(
        observations.0.as_slice(),
        [.., CommandObservation::Scanner(scanner), CommandObservation::Mutation(mutation)]
            if scanner.kind == "glue"
                && mutation.target == "parameter"
                && mutation.key.as_deref() == Some("glue_parameter:11")
                && mutation.value.starts_with("glue:width=131072")
    ));
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("alignment"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Command(raw),
            CommandObservation::Command(expanded),
            CommandObservation::Alignment(alignment)]
            if raw.command == "halign"
                && expanded.command == "halign"
                && alignment.transition == "begin"
                && alignment.align_state == -1_000_000
    ));
    let alignment = control
        .active_alignment()
        .expect("alignment begins after tabskip");
    control
        .apply_alignment_request(AlignmentRequest::Preamble(alignment))
        .expect("preamble lifecycle remains available");
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("scanner-backed alignment token retires before the next delivery"),
        ReplayStep::Continue
    );
    let alignment_begin = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Alignment(alignment) if alignment.transition == "begin")
        })
        .expect("replayed hAlign publishes its typed begin transition");
    let backup_retirement = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Input(input)
                if input.transition == InputTransition::Retire && input.reason == InputReason::Backup)
        })
        .expect("exhausted hAlign backup retires on the following delivery");
    assert!(alignment_begin < backup_retirement);
}

#[test]
fn omit_cell_sets_body_state_without_backing_up_or_installing_a_u_template() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{u#v\cr {a}\cr \omit b\cr}\end");
    let mut observations = ObservationRecorder::default();

    for _ in 0..20 {
        if control
            .step_with_observer(&mut universe, &mut observations)
            .is_err()
        {
            break;
        }
        if observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                ] if raw.command == "omit"
                    && expanded.command == "omit"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 0
                    && state_change.previous_align_state == Some(1_000_000)
            )
        }) {
            break;
        }
    }

    assert!(
        observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                ] if raw.command == "omit"
                    && expanded.command == "omit"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 0
                    && state_change.previous_align_state == Some(1_000_000)
            )
        }),
        "omit must transition directly from the lookahead sentinel to the cell body: {:?}",
        observations.0
    );
    assert!(
        !observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(command),
                    CommandObservation::Input(input),
                    ..
                ] if command.command == "omit"
                    && input.transition == InputTransition::Backup
            )
        }),
        "TeX82 init_col never backs up its omit lookahead: {:?}",
        observations.0
    );

    for _ in 0..20 {
        if control
            .step_with_observer(&mut universe, &mut observations)
            .is_err()
        {
            break;
        }
    }
    assert!(
        observations.0.iter().any(|event| {
            matches!(event, CommandObservation::Alignment(template)
                if template.transition == "omit_template_push")
        }),
        "omit must install TeX82's omit_template, not the selected v-template: {:?}",
        observations.0
    );
}

#[test]
fn noalign_uses_command_owned_brace_scan_without_a_generic_backup() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\halign{#\cr {a}\cr\noalign{\relax}{b}\cr}\end",
    );
    let mut observations = ObservationRecorder::default();

    for _ in 0..40 {
        if control
            .step_with_observer(&mut universe, &mut observations)
            .is_err()
        {
            break;
        }
        if observations.0.windows(4).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                    CommandObservation::Command(brace),
                ] if raw.command == "no_align"
                    && raw.command_operand == Some(0)
                    && expanded.command == "no_align"
                    && expanded.command_operand == Some(0)
                    && state_change.transition == "begin_group"
                    && state_change.previous_align_state == Some(1_000_000)
                    && state_change.align_state == 1_000_001
                    && brace.spelling == ObservedToken::Character {
                        character: '{',
                        catcode: Catcode::BeginGroup,
                    }
            )
        }) {
            break;
        }
    }

    let noalign = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw && command.command == "no_align")
        })
        .expect("raw TeX82 no_align delivery");
    let brace = observations
        .0
        .iter()
        .skip(noalign + 1)
        .position(|event| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw
                && command.spelling == ObservedToken::Character {
                    character: '{', catcode: Catcode::BeginGroup
                })
        })
        .map(|offset| noalign + 1 + offset)
        .expect("command-owned noalign opening brace");
    assert!(
        observations.0[noalign..=brace]
            .iter()
            .any(|event| matches!(event,
                CommandObservation::Alignment(state_change)
                    if state_change.transition == "begin_group"
                        && state_change.previous_align_state == Some(1_000_000)
                        && state_change.align_state == 1_000_001
            ))
    );
    assert!(
        !observations.0[noalign..=brace]
            .iter()
            .any(|event| matches!(event,
                CommandObservation::Input(input) if input.transition == InputTransition::Backup
            ))
    );
}

#[test]
fn alignment_preamble_opener_uses_command_owned_backup_before_source_resumes() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{ U#\cr{\end");

    assert_eq!(
        control.step(&mut universe).expect("alignment begins"),
        ReplayStep::Continue
    );
    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("preamble opening is backed up"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [..,
                CommandObservation::Alignment(state_change),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Alignment(correction),
            ]
                if state_change.transition == "begin_group"
                    && state_change.align_state == -999_999
                    && state_change.previous_align_state == Some(-1_000_000)
                    && matches!(raw.spelling, ObservedToken::Character { character: '{', .. })
                    && matches!(expanded.spelling, ObservedToken::Character { character: '{', .. })
                    && backup.transition == InputTransition::Backup
                    && backup.reason == InputReason::Backup
                    && recovery.kind == RecoveryKind::Backup
                    && correction.transition == "backup_correction"
                    && correction.align_state == -1_000_000
                    && correction.previous_align_state == Some(-999_999)
        ),
        "unexpected observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("replayed preamble opener is backed up again"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Alignment(state_change),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(retirement),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Alignment(correction),
            ]
                if state_change.transition == "begin_group"
                    && state_change.align_state == -999_999
                    && state_change.previous_align_state == Some(-1_000_000)
                    && matches!(raw.spelling, ObservedToken::Character { character: '{', .. })
                    && matches!(expanded.spelling, ObservedToken::Character { character: '{', .. })
                    && retirement.transition == InputTransition::Retire
                    && retirement.reason == InputReason::Backup
                    && backup.transition == InputTransition::Backup
                    && backup.reason == InputReason::Backup
                    && recovery.kind == RecoveryKind::Backup
                    && correction.transition == "backup_correction"
                    && correction.align_state == -1_000_000
                    && correction.previous_align_state == Some(-999_999)
        ),
        "unexpected observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("replayed brace enters the live preamble scanner"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Alignment(state_change),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::ScannerStatus(status),
                CommandObservation::Alignment(preamble_start),
                CommandObservation::Input(retirement),
                CommandObservation::Command(space),
                CommandObservation::Command(template),
                CommandObservation::Command(parameter),
                CommandObservation::Command(terminator),
                CommandObservation::Alignment(preamble_finish),
                CommandObservation::ScannerStatus(finished),
            ]
                if state_change.transition == "begin_group"
                    && state_change.align_state == -999_999
                    && state_change.previous_align_state == Some(-1_000_000)
                    && matches!(raw.spelling, ObservedToken::Character { character: '{', .. })
                    && matches!(expanded.spelling, ObservedToken::Character { character: '{', .. })
                    && status.from.starts_with("Normal")
                    && status.to.starts_with("Aligning")
                    && preamble_start.transition == "preamble_start"
                    && preamble_start.align_state == -1_000_000
                    && retirement.transition == InputTransition::Retire
                    && retirement.reason == InputReason::Backup
                    && matches!(space.spelling, ObservedToken::Character { character: ' ', .. })
                    && matches!(template.spelling, ObservedToken::Character { character: 'U', .. })
                    && matches!(parameter.spelling, ObservedToken::Character { character: '#', .. })
                    && matches!(terminator.spelling, ObservedToken::ControlSequence(ref name) if name == "cr")
                    && finished.from.starts_with("Aligning")
                    && finished.to.starts_with("Normal")
                    && preamble_finish.transition == "preamble_finish"
        ),
        "unexpected observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("first cell opener is backed up before the u-template"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Alignment(peek),
                CommandObservation::Alignment(state_change),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Alignment(correction),
                CommandObservation::Input(template),
                CommandObservation::Alignment(template_alignment),
            ]
                if peek.transition == "state_change"
                    && peek.align_state == 1_000_000
                    && peek.previous_align_state.is_none()
                    && state_change.transition == "begin_group"
                    && state_change.align_state == 1_000_001
                    && state_change.previous_align_state == Some(1_000_000)
                    && matches!(raw.spelling, ObservedToken::Character { character: '{', .. })
                    && matches!(expanded.spelling, ObservedToken::Character { character: '{', .. })
                    && backup.transition == InputTransition::Backup
                    && backup.reason == InputReason::Backup
                    && recovery.kind == RecoveryKind::Backup
                    && correction.transition == "backup_correction"
                    && correction.align_state == 1_000_000
                    && correction.previous_align_state == Some(1_000_001)
                    && template.transition == InputTransition::Push
                    && template.reason == InputReason::AlignmentUTemplate
                    && template_alignment.transition == "u_template_push"
                    && template_alignment.align_state == 1_000_000
        ),
        "unexpected observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("u-template delivers its final token"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("backed-up u-template token replays"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("u-template retires before the cell body resumes"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Input(input),
                    CommandObservation::Alignment(retirement),
                    CommandObservation::Alignment(body),
                ] if input.transition == InputTransition::Retire
                    && input.reason == InputReason::AlignmentUTemplate
                    && retirement.transition == "u_template_retire"
                    && retirement.align_state == 1_000_000
                    && body.transition == "state_change"
                    && body.align_state == 0
                    && body.previous_align_state == Some(1_000_000)
            )
        }),
        "unexpected observations: {:?}",
        observations.0
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("inserted paragraph ends the outer horizontal list"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("end retries in vertical mode"),
        ReplayStep::End
    );
}

#[test]
fn empty_ordinary_u_template_pushes_and_retires_before_the_cell_opener_replays() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    // The empty list before `#` is an ordinary u-template, not `\omit`.
    // `init_col` backs up any ordinary first-cell command, not just `{`.
    // This mirrors the nested `\halign{#\cr\vrule...}` trace case.
    register_source(&mut control, br"\halign{#\cr\vrule\end");
    let mut observations = ObservationRecorder::default();

    for phase in [
        "alignment begin",
        "preamble opener backup",
        "preamble opener replay",
        "preamble scan and cell setup",
        "cell opener backup and empty template installation",
    ] {
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect(phase),
            ReplayStep::Continue
        );
    }

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("empty template retires before the backed-up opener"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.windows(5).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Input(push),
                    CommandObservation::Alignment(template_push),
                    CommandObservation::Input(retire),
                    CommandObservation::Alignment(template_retire),
                    CommandObservation::Alignment(body),
                ] if push.transition == InputTransition::Push
                    && push.reason == InputReason::AlignmentUTemplate
                    && template_push.transition == "u_template_push"
                    && template_push.align_state == 1_000_000
                    && retire.transition == InputTransition::Retire
                    && retire.reason == InputReason::AlignmentUTemplate
                    && template_retire.transition == "u_template_retire"
                    && template_retire.align_state == 1_000_000
                    && body.transition == "state_change"
                    && body.align_state == 0
                    && body.previous_align_state == Some(1_000_000)
            )
        }),
        "empty ordinary u-template must retain the TeX82 lifecycle: {:?}",
        observations.0
    );
}

#[test]
fn periodic_preamble_replays_its_u_template_before_retirement() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    // TeX82 §760 treats `&&` as the start of the periodic preamble suffix,
    // not as an empty second column. The following cell must therefore see
    // `\hskip` from that u-template before `end_token_list` retires it.
    register_source(&mut control, br"\halign{#&&\hskip1pt#\cr\relax&\relax\end");
    let mut observations = ObservationRecorder::default();

    for _ in 0..16 {
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("periodic preamble replay");
        if observations.0.windows(6).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Input(push),
                    CommandObservation::Alignment(template_push),
                    CommandObservation::Command(raw_hskip),
                    CommandObservation::Command(expanded_hskip),
                    CommandObservation::Command(raw_numeric),
                    CommandObservation::Command(expanded_numeric),
                ] if push.transition == InputTransition::Push
                    && push.reason == InputReason::AlignmentUTemplate
                    && template_push.transition == "u_template_push"
                    && raw_hskip.boundary == CommandDeliveryBoundary::Raw
                    && raw_hskip.command == "hskip"
                    && expanded_hskip.boundary == CommandDeliveryBoundary::Expanded
                    && expanded_hskip.command == "hskip"
                    && matches!(raw_numeric.spelling, ObservedToken::Character { character: '1', .. })
                    && matches!(expanded_numeric.spelling, ObservedToken::Character { character: '1', .. })
            )
        }) && observations.0.windows(6).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw_hskip),
                    CommandObservation::Command(expanded_hskip),
                    CommandObservation::Command(raw_numeric),
                    CommandObservation::Command(expanded_numeric),
                    CommandObservation::Input(backup),
                    CommandObservation::Recovery(recovery),
                ] if raw_hskip.boundary == CommandDeliveryBoundary::Raw
                    && raw_hskip.command == "hskip"
                    && expanded_hskip.boundary == CommandDeliveryBoundary::Expanded
                    && expanded_hskip.command == "hskip"
                    && matches!(raw_numeric.spelling, ObservedToken::Character { character: '1', .. })
                    && matches!(expanded_numeric.spelling, ObservedToken::Character { character: '1', .. })
                    && backup.transition == InputTransition::Backup
                    && backup.reason == InputReason::Backup
                    && recovery.kind == RecoveryKind::Backup
                    && matches!(recovery.tokens.as_slice(), [ObservedToken::Character { character: '1', .. }])
            )
        }) {
            return;
        }
    }
    panic!(
        "periodic u-template must deliver hskip before retirement: {:?}",
        observations.0
    );
}

#[test]
fn completed_rule_spec_restarts_active_cell_through_typed_delimiter_delivery() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{#\cr{\vrule width1pt}&\end");
    let mut observations = ObservationRecorder::default();

    for phase in [
        "alignment begin",
        "preamble opener backup",
        "preamble opener replay",
        "preamble scan and first cell",
        "cell opener and template installation",
        "replayed cell opener",
    ] {
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect(phase),
            ReplayStep::Continue
        );
        observations.0.clear();
    }

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("rule specification"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.iter().any(|observation| {
            matches!(
                observation,
                CommandObservation::Scanner(scanner)
                    if scanner.kind == "dimension" && scanner.value == "65536"
            )
        }),
        "unexpected rule observations: {:?}",
        observations.0
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("cell-body closing brace"),
        ReplayStep::Continue
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("backed-up tab reaches the alignment delivery boundary"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.windows(4).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Alignment(delimiter),
                    CommandObservation::Input(template_input),
                    CommandObservation::Alignment(template),
                    CommandObservation::Alignment(state_change),
                ] if delimiter.transition == "delimiter"
                    && delimiter.align_state == 0
                    && delimiter.delimiter == Some("tab")
                    && template_input.transition == InputTransition::Push
                    && template_input.reason == InputReason::AlignmentVTemplate
                    && template.transition == "v_template_push"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 1_000_000
            )
        }),
        "unexpected observations: {:?}",
        observations.0
    );
}

#[test]
fn canonical_initex_replay_scans_token_register_assignments_through_command_core() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\toks0={TOKEN LIST}\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("token-register assignment"),
        ReplayStep::Continue
    );
    assert_eq!(replay_text(universe.tokens(universe.toks(0))), "TOKEN LIST");
    assert!(observations.0.windows(2).any(|pair| {
        matches!(
            &pair[0],
            CommandObservation::Input(input)
                if input.transition == InputTransition::Backup && input.reason == InputReason::Backup
        ) && matches!(
            &pair[1],
            CommandObservation::Recovery(recovery) if recovery.kind == RecoveryKind::Backup
        )
    }));
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::ScannerStatus(status),
            CommandObservation::TokenList(tokens),
            CommandObservation::Mutation(mutation)]
            if status.to.starts_with("Normal")
                && tokens.transition == "complete"
                && tokens.purpose == "scan_toks"
                && mutation.target == "register"
                && mutation.key.as_deref() == Some("toks:0")
                && mutation.value == "tokens"
                && !mutation.global
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_toksdef_projects_its_committed_named_register_meaning() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\toksdef\tokens=256\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("toksdef"),
        ReplayStep::Continue
    );
    assert_eq!(
        universe.meaning(universe.symbol("tokens").expect("toksdef target")),
        Meaning::ToksRegister(0)
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Mutation(mutation))
            if mutation.target == "meaning"
                && mutation.key.as_deref() == Some("tokens")
                && mutation.value == "assign_toks"
                && !mutation.global
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_copies_direct_token_register_rhs() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\toks0={zero}\toks20={twenty}\toks1=\toks20\toks2=\toks256\end",
    );
    let mut observations = ObservationRecorder::default();

    for _ in 0..4 {
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect("token-register assignment"),
            ReplayStep::Continue
        );
    }

    assert_eq!(replay_text(universe.tokens(universe.toks(1))), "twenty");
    assert_eq!(replay_text(universe.tokens(universe.toks(2))), "zero");
    assert!(observations.0.windows(2).any(|pair| {
        matches!(
            pair,
            [
                CommandObservation::Scanner(scanner),
                CommandObservation::Mutation(mutation),
            ] if scanner.kind == "integer"
                && scanner.value == "20"
                && mutation.key.as_deref() == Some("toks:1")
        )
    }));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_scans_setbox_then_hands_vbox_to_executor() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox10=\vbox{}\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("setbox prefix"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("vbox handoff"),
        ReplayStep::Continue
    );
    assert!(matches!(
        universe.group_kinds().next_back(),
        Some(tex_state::GroupKind::VBox)
    ));
    assert_eq!(
        control.step(&mut universe).expect("replay opener"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("enter body"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("package vbox"),
        ReplayStep::Continue
    );
    assert!(
        universe.box_reg(10).is_some(),
        "vbox is assigned at group exit"
    );

    assert!(observations.0.windows(2).any(|pair| {
        matches!(
            &pair,
            [CommandObservation::Input(input), CommandObservation::Recovery(recovery)]
                if input.transition == InputTransition::Backup
                    && input.reason == InputReason::Backup
                    && recovery.kind == RecoveryKind::Backup
        )
    }));
    assert!(observations.0.iter().any(|event| {
        matches!(event, CommandObservation::Command(command)
            if command.command == "make_box" && command.command_operand == Some(5))
    }));
}

#[test]
fn canonical_initex_replay_scans_box_register_before_stomach_consumes_it() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox10=\vbox{}\box10\end");

    for _ in 0..5 {
        assert_eq!(
            control.step(&mut universe).expect("setbox construction"),
            ReplayStep::Continue
        );
    }
    assert!(universe.box_reg(10).is_some(), "setbox completed");

    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("box register scan"),
        ReplayStep::Continue
    );
    assert!(universe.box_reg(10).is_none(), "box consumes its register");

    let make_box = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw
                && command.command == "make_box"
                && command.command_operand == Some(0))
        })
        .expect("raw make_box(box_code) identity");
    let first_digit = observations
        .0
        .iter()
        .enumerate()
        .skip(make_box + 1)
        .find_map(|(index, event)| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw
                && command.spelling == ObservedToken::Character {
                    character: '1', catcode: tex_state::token::Catcode::Other,
                })
            .then_some(index)
        })
        .expect("command-owned scan_int delivers the first register digit raw");
    assert!(
        !observations.0[make_box + 1..first_digit]
            .iter()
            .any(|event| matches!(event, CommandObservation::Input(input)
                if input.transition == InputTransition::Backup)),
        "the register digit is not an executor-created backup replay: {:?}",
        observations.0
    );
    let second_digit = observations
        .0
        .iter()
        .enumerate()
        .skip(first_digit + 1)
        .find_map(|(index, event)| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw
                && command.spelling == ObservedToken::Character {
                    character: '0', catcode: tex_state::token::Catcode::Other,
                })
            .then_some(index)
        })
        .expect("second register digit remains raw command input");
    let terminator_backup = observations
        .0
        .iter()
        .enumerate()
        .skip(second_digit + 1)
        .find_map(|(index, event)| {
            matches!(event, CommandObservation::Input(input)
            if input.transition == InputTransition::Backup)
            .then_some(index)
        })
        .expect("scan_int backs up the following box terminator after both digits");
    assert!(
        second_digit < terminator_backup,
        "integer terminator backup follows the completed register operand"
    );
}

#[test]
fn shipout_box_completion_precedes_its_terminator_backup_retirement() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox10=\vbox{}\setbox11=\vbox{}\shipout\vbox{\box10\box11}\end",
    );
    let mut observations = ObservationRecorder::default();

    for _ in 0..32 {
        if matches!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect("canonical replay"),
            ReplayStep::End | ReplayStep::EndOfInput
        ) {
            break;
        }
    }

    let shipout = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Effect(effect)
                if effect.kind == "shipout" && effect.detail == "dvi\0".to_owned() + "1")
        })
        .expect("completed vbox publishes DVI page one");
    let retirement = observations
        .0
        .iter()
        .enumerate()
        .skip(shipout + 1)
        .find_map(|(index, event)| {
            matches!(event, CommandObservation::Input(input)
                if input.transition == InputTransition::Retire && input.reason == InputReason::Backup)
            .then_some(index)
        })
        .expect("box-register terminator backup retires on the next raw fetch");
    assert!(
        shipout < retirement,
        "TeX82 box_end ships out before scan_int's terminator backup retires: {:?}",
        observations.0
    );
}

#[test]
fn canonical_initex_replay_observes_committed_message_effects() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\message{READY}\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("message"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Effect(effect))
            if effect.kind == "message" && effect.detail == "READY"
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn end_in_outer_horizontal_mode_replays_paragraph_before_retrying_stop() {
    // TeX82 §1095 (`hmode+stop` / `head_for_vmode`) first backs up \end,
    // then backs up inserted \par. The command processor owns both input
    // transitions; applying the delivered paragraph is the executor's typed
    // mode transition before \end is reconsidered in vertical mode.
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, b"x\\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("paragraph starts"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("replayed paragraph character"),
        ReplayStep::Continue
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("stop is deferred to vertical mode"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Input(previous_retirement),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(end_backup),
                CommandObservation::Recovery(end_recovery),
                CommandObservation::Input(par_backup),
                CommandObservation::Recovery(par_recovery),
            ] if previous_retirement.transition == InputTransition::Retire
                && previous_retirement.reason == InputReason::Backup
                && raw.command == "stop"
                && expanded.command == "stop"
                && end_backup.transition == InputTransition::Backup
                && end_recovery.kind == RecoveryKind::Backup
                && par_backup.transition == InputTransition::Backup
                && par_recovery.kind == RecoveryKind::Backup
                && matches!(par_recovery.tokens.as_slice(), [ObservedToken::ControlSequence(name)] if name == "par")
        ),
        "unexpected stop recovery observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("inserted paragraph"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Vertical);
    assert!(matches!(
        observations.0.as_slice(),
        [CommandObservation::Command(raw), CommandObservation::Command(expanded)]
            if raw.command == "par_end" && expanded.command == "par_end"
    ));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("stop retries in vertical mode"),
        ReplayStep::End
    );
    assert!(matches!(
        observations.0.as_slice(),
        [
            CommandObservation::Input(retirement),
            CommandObservation::Command(raw),
            CommandObservation::Command(expanded),
        ] if retirement.transition == InputTransition::Retire
            && retirement.reason == InputReason::Recovery
            && raw.command == "stop"
            && expanded.command == "stop"
    ));
}

#[test]
fn final_stop_retires_its_backup_before_starting_output_input() {
    // TeX82 §46 (`its_all_over`) starts \output only after the §1095
    // redelivery's exhausted \end backup has retired and a new final-stop
    // backup is in place.
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\output={O}x\end");
    let mut observations = ObservationRecorder::default();

    for expected in [
        "output assignment",
        "paragraph start",
        "paragraph character",
    ] {
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect(expected),
            ReplayStep::Continue
        );
        observations.0.clear();
    }

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("horizontal stop recovery"),
        ReplayStep::Continue
    );
    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("inserted paragraph"),
        ReplayStep::Continue
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("output hand-off"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Input(recovery_retirement),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(retirement),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Input(output),
            ] if recovery_retirement.transition == InputTransition::Retire
                && recovery_retirement.reason == InputReason::Recovery
                && retirement.transition == InputTransition::Retire
                && retirement.reason == InputReason::Backup
                && raw.command == "stop"
                && expanded.command == "stop"
                && backup.transition == InputTransition::Backup
                && recovery.kind == RecoveryKind::Backup
                && output.transition == InputTransition::Push
                && output.reason == InputReason::TokenList
        ),
        "unexpected output hand-off observations: {:?}",
        observations.0
    );

    for expected in [
        "output opening brace",
        "output paragraph start",
        "output paragraph character",
        "output closing brace",
    ] {
        observations.0.clear();
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect(expected),
            ReplayStep::Continue
        );
    }

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("final stop retries below the completed output routine"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Input(output_retirement),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(backup_retirement),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Input(output),
            ] if output_retirement.transition == InputTransition::Retire
                && output_retirement.reason == InputReason::TokenList
                && raw.command == "stop"
                && expanded.command == "stop"
                && backup_retirement.transition == InputTransition::Retire
                && backup_retirement.reason == InputReason::Backup
                && backup.transition == InputTransition::Backup
                && recovery.kind == RecoveryKind::Backup
                && output.transition == InputTransition::Push
                && output.reason == InputReason::TokenList
        ),
        "completed output routine must restore vertical final-stop retry: {:?}",
        observations.0
    );
}

#[test]
fn output_group_waits_for_nested_box_body_before_closing() {
    // TeX82 §1016 starts the braced \output list inside an already-open
    // output_group. §1077's nested box body must consume its own right brace
    // before §1026 can tear down that enclosing output group. This is the
    // reduced Plain `\line{\vbox to8.5\p@{}}` shape.
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\output={\global\output={}\shipout\vbox{\hbox{\vbox to8.5pt{}}}}\topskip=0pt\vsize=1pt\setbox0=\hbox{}\ht0=2pt\copy0\penalty-10000\end",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.world().artifact_commits().len(), 2);
    assert!(universe.box_reg(255).is_none());
}

#[test]
fn simple_group_ancestor_does_not_close_nested_output_box() {
    // TeX82 §1068 dispatches `}` from the live `cur_group`.  Plain's `\big`
    // has this shape: its simple group stays open while the nested `\vbox`
    // body closes.  The closer must package the vbox, not unsave that
    // ancestor simple group.
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\output={{\global\output={}\shipout\vbox{\hbox{\vbox to8.5pt{}}}}}\topskip=0pt\vsize=1pt\setbox0=\hbox{}\ht0=2pt\copy0\penalty-10000\end",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.world().artifact_commits().len(), 2);
    assert!(universe.box_reg(255).is_none());
}

#[test]
fn hrule_contributes_to_outer_page_before_final_shipout() {
    // TeX82 §1095 puts a vertical-mode \hrule directly on the current page;
    // it must not remain stranded on the mode-nest list until job cleanup.
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\hrule height1pt width2pt");

    assert_eq!(
        control.step(&mut universe).expect("hrule executes"),
        ReplayStep::Continue
    );
    assert!(matches!(
        universe.current_page_nodes().as_slice(),
        [.., Node::Rule { width, height, depth }]
            if *width == Some(Scaled::from_raw(2 * Scaled::UNITY))
                && *height == Some(Scaled::from_raw(Scaled::UNITY))
                && *depth == Some(Scaled::from_raw(0))
    ));
}

#[test]
fn dead_output_cycle_forces_shipout_after_final_stop_backup() {
    // TeX82 §§46 and 1005: after `maxdeadcycles` completed output routines
    // that leave `\box255` unused, `its_all_over` backs up the final stop
    // before `fire_up` forces the page through `ship_out`.
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\maxdeadcycles=1\output={X}\end");
    let mut observations = ObservationRecorder::default();

    for _ in 0..32 {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("output-cycle replay")
        {
            ReplayStep::Continue => {}
            ReplayStep::End | ReplayStep::EndOfInput => break,
        }
    }

    let shipout = observations
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Effect(effect)
                    if effect.kind == "shipout" && effect.detail == "dvi\0".to_owned() + "1"
            )
        })
        .expect("forced output shipout effect");
    assert!(matches!(
        observations.0.get(shipout.checked_sub(1).expect("effect has predecessor")),
        Some(CommandObservation::Recovery(recovery)) if recovery.kind == RecoveryKind::Backup
    ));
    assert_eq!(universe.world().artifact_commits().len(), 1);
}

#[test]
fn off_save_reports_before_replaying_its_inserted_closer() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"{\endgroup}");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("opening group"),
        ReplayStep::Continue
    );
    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("off_save recovery"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [
            CommandObservation::Command(_),
            CommandObservation::Command(_),
            CommandObservation::Diagnostic(diagnostic),
            CommandObservation::Input(backup),
            CommandObservation::Recovery(recovery),
            CommandObservation::Input(inserted),
            CommandObservation::Recovery(inserted_recovery),
        ] if diagnostic.diagnostic == "off_save_replay"
            && backup.transition == InputTransition::Backup
            && recovery.kind == RecoveryKind::Backup
            && inserted.transition == InputTransition::Recovery
            && inserted_recovery.kind == RecoveryKind::InsertedToken
    ));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("inserted closer"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [CommandObservation::Command(raw), CommandObservation::Command(expanded)]
                if matches!(raw.spelling, ObservedToken::Character { character: '}', catcode: Catcode::EndGroup })
                    && matches!(expanded.spelling, ObservedToken::Character { character: '}', catcode: Catcode::EndGroup })
        ),
        "unexpected inserted-closer observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("bottom-level replay drop"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [
            CommandObservation::Input(retirement),
            CommandObservation::Command(_),
            CommandObservation::Command(_),
            CommandObservation::Diagnostic(diagnostic),
        ] if retirement.transition == InputTransition::Retire
            && retirement.reason == InputReason::Recovery
            && diagnostic.diagnostic == "off_save_bottom_drop"
    ));
}

#[test]
fn canonical_initex_replay_scans_and_applies_code_table_assignments() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\catcode`@=11 \lccode`Z=`z \end");

    assert_eq!(
        control.step(&mut universe).expect("catcode assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.catcode('@'), tex_state::token::Catcode::Letter);
    assert_eq!(
        control.step(&mut universe).expect("lccode assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.lccode('Z'), u32::from('z'));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_scans_raw_let_operands_and_commits_the_meaning() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\let\alias = \begingroup\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("let assignment"),
        ReplayStep::Continue
    );
    let alias = universe.symbol("alias").expect("let target is interned");
    assert_eq!(
        universe.meaning(alias),
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::BeginGroup)
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Mutation(mutation))
            if mutation.target == "meaning"
                && mutation.key.as_deref() == Some("alias")
                && mutation.value == "begin_group"
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_keeps_macro_target_and_expanded_body_in_command_core() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\def\source{expanded}\long\outer\edef\target{\source}\end",
    );

    assert_eq!(
        control.step(&mut universe).expect("source definition"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("expanded definition"),
        ReplayStep::Continue
    );
    let target = universe.symbol("target").expect("macro target is interned");
    let meaning = universe.macro_meaning(target).expect("target is a macro");
    assert!(
        meaning
            .flags()
            .contains(tex_state::meaning::MeaningFlags::LONG)
    );
    assert!(
        meaning
            .flags()
            .contains(tex_state::meaning::MeaningFlags::OUTER)
    );
    assert_eq!(
        replay_text(universe.tokens(meaning.replacement_text())),
        "expanded"
    );
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_case_shift_replays_raw_text_with_categories_and_order_intact() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\uccode`!=`Z \lccode`?=`y
           \uppercase{\def\up{!\relax}}
           \lowercase{\def\down{?\relax}}
           \uppercase{\def\zero{@}}\end",
    );

    for _ in 0..2 {
        assert_eq!(
            control.step(&mut universe).expect("case setup"),
            ReplayStep::Continue
        );
    }

    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("uppercase scans its balanced text"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::TokenList(record))
            if record.transition == "complete" && record.purpose == "scan_toks"
    ));
    assert_eq!(
        control
            .step(&mut universe)
            .expect("uppercase replays definition"),
        ReplayStep::Continue
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("lowercase follows the retired uppercase replay"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [CommandObservation::Input(retirement), ..]
            if retirement.transition == InputTransition::Retire
                && retirement.reason == InputReason::TokenList
    ));
    assert_eq!(
        control
            .step(&mut universe)
            .expect("lowercase replays definition"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("zero-code uppercase scans"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("zero-code definition replays"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("zero-code replay retires before the next command"),
        ReplayStep::Continue
    );

    let relax = universe
        .symbol("relax")
        .expect("primitive is interned")
        .symbol();
    let up = universe
        .macro_meaning(universe.symbol("up").expect("uppercase macro"))
        .expect("uppercase definition persists");
    assert_eq!(
        universe.tokens(up.replacement_text()),
        &[
            Token::Char {
                ch: 'Z',
                cat: Catcode::Other,
            },
            Token::Cs(relax),
        ]
    );
    let down = universe
        .macro_meaning(universe.symbol("down").expect("lowercase macro"))
        .expect("lowercase definition persists");
    assert_eq!(
        universe.tokens(down.replacement_text()),
        &[
            Token::Char {
                ch: 'y',
                cat: Catcode::Other,
            },
            Token::Cs(relax),
        ]
    );
    while matches!(
        control.step(&mut universe).expect("finish case replay"),
        ReplayStep::Continue
    ) {}
    let zero = universe
        .macro_meaning(universe.symbol("zero").expect("zero-code macro"))
        .unwrap_or_else(|| {
            panic!(
                "zero-code definition persists: {}",
                terminal_text(&universe)
            )
        });
    assert_eq!(
        universe.tokens(zero.replacement_text()),
        &[Token::Char {
            ch: '@',
            cat: Catcode::Other,
        }]
    );
}

#[test]
fn case_shift_substitution_preserves_active_categories_and_origins() {
    let mut universe = Universe::new();
    let origin = universe.bootstrap_origin();
    let control = universe.intern("control").symbol();
    let tokens = universe.finish_traced_token_list(&[
        tex_state::token::TracedTokenWord::pack(
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            origin,
        ),
        tex_state::token::TracedTokenWord::pack(
            Token::Char {
                ch: 'b',
                cat: Catcode::Active,
            },
            origin,
        ),
        tex_state::token::TracedTokenWord::pack(
            Token::Char {
                ch: '@',
                cat: Catcode::Other,
            },
            origin,
        ),
        tex_state::token::TracedTokenWord::pack(Token::Cs(control), origin),
        tex_state::token::TracedTokenWord::pack(Token::param(1), origin),
    ]);

    let shifted = case_shift_tokens(tokens, true, &mut universe);
    assert_eq!(
        universe.tokens(shifted.token_list()),
        &[
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'B',
                cat: Catcode::Active,
            },
            Token::Char {
                ch: '@',
                cat: Catcode::Other,
            },
            Token::Cs(control),
            Token::param(1),
        ]
    );
    assert_eq!(
        universe.origin_list(shifted.origin_list()),
        universe.origin_list(tokens.origin_list())
    );
}

#[test]
fn canonical_initex_replays_afterassignment_before_fifo_aftergroup_tokens() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\def\first{\global\count0=1}\def\second{\global\count0=2}\def\assigned{\global\count2=\count1}{\aftergroup\first\aftergroup\second\afterassignment\assigned\count1=7}\end",
    );

    loop {
        if matches!(
            control
                .step(&mut universe)
                .expect("canonical after-token replay"),
            ReplayStep::End
        ) {
            break;
        }
    }
    assert_eq!(
        universe.count(0),
        2,
        "aftergroup tokens replay FIFO after restoration"
    );
    assert_eq!(
        universe.count(2),
        7,
        "afterassignment observes the committed assignment"
    );
    assert_eq!(
        universe.count(1),
        0,
        "the local assignment restores before group exit"
    );
}

#[test]
fn canonical_initex_replay_futurelet_preserves_lookahead_order_after_assignment() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\futurelet\next\first x\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("futurelet assignment"),
        ReplayStep::Continue
    );
    let next = universe
        .symbol("next")
        .expect("futurelet target is interned");
    assert_eq!(
        universe.meaning(next),
        Meaning::CharToken {
            ch: 'x',
            cat: tex_state::token::Catcode::Letter,
        }
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Mutation(mutation))
            if mutation.target == "meaning" && mutation.key.as_deref() == Some("next")
    ));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("first lookahead replay"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [.., CommandObservation::Command(delivery)]
            if matches!(delivery.spelling, ObservedToken::ControlSequence(ref name) if name == "first")
    ));
    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("second lookahead replay"),
        ReplayStep::Continue
    );
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Command(delivery)
                if matches!(delivery.spelling, ObservedToken::Character { character: 'x', .. })
        )
    }));
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control
            .step(&mut universe)
            .expect("paragraph character replay"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("end is deferred through paragraph recovery"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("inserted paragraph"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("retried end"),
        ReplayStep::End
    );
}

#[test]
fn canonical_initex_replay_scans_and_applies_dimension_and_glue_registers() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\dimen0=1.5pt\skip0=2pt plus 3fil minus 4pt\end",
    );
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("dimension assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.dimen(0).raw(), 98_304);
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Scanner(scanner),
            CommandObservation::Mutation(mutation)]
            if scanner.kind == "dimension"
                && scanner.value == "98304"
                && mutation.target == "register"
                && mutation.key.as_deref() == Some("dimen:0")
                && mutation.value == "scaled:98304"
    ));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("glue assignment"),
        ReplayStep::Continue
    );
    let glue = universe.glue(universe.skip(0));
    assert_eq!(glue.width.raw(), 131_072);
    assert_eq!(glue.stretch.raw(), 196_608);
    assert_eq!(glue.stretch_order, tex_state::glue::Order::Fil);
    assert_eq!(glue.shrink.raw(), 262_144);
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Scanner(scanner),
            CommandObservation::Mutation(mutation)]
            if scanner.kind == "glue"
                && mutation.target == "register"
                && mutation.key.as_deref() == Some("skip:0")
                && mutation.value.starts_with("glue:width=131072;")
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_scans_complete_rule_specs_through_command_control() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\vrule width1pt height2pt depth0pt\hrule width3pt height4pt depth1pt\end",
    );
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("vertical rule"),
        ReplayStep::Continue
    );
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Scanner(scanner)
                if scanner.kind == "dimension" && scanner.value == "65536"
        )
    }));
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Command(delivery)
                if matches!(delivery.spelling, ObservedToken::Character { character: 'w', .. })
        )
    }));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("horizontal rule"),
        ReplayStep::Continue
    );
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Scanner(scanner)
                if scanner.kind == "dimension" && scanner.value == "196608"
        )
    }));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[derive(Default)]
struct ObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for ObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

#[test]
fn replay_expands_registered_input_without_executor_source_consumption() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    install_input(&mut universe);
    let mut control = CommandReplayControl::default();
    control.capabilities_mut().register_input(
        "child",
        SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"\\count3=9"[..]),
        ),
    );
    register_source(&mut control, br"\input child\count4=8\end");

    assert_eq!(
        control.step(&mut universe).expect("nested assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(3), 9);
    assert_eq!(
        control.step(&mut universe).expect("parent assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(4), 8);
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn replay_command_snapshot_restores_typed_scanner_input_deterministically() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"\count12=7\end");
    let snapshot = control.command_mut().snapshot();

    assert_eq!(
        control.step(&mut universe).expect("first assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(12), 7);

    control
        .command_mut()
        .rollback(snapshot)
        .expect("command snapshot restores scanner-owned input");
    let mut replayed_universe = Universe::new();
    crate::install_unexpandable_primitives(&mut replayed_universe);
    assert_eq!(
        control
            .step(&mut replayed_universe)
            .expect("replayed assignment"),
        ReplayStep::Continue
    );
    assert_eq!(replayed_universe.count(12), 7);
    assert_eq!(
        control.step(&mut replayed_universe).expect("end"),
        ReplayStep::End
    );
}

#[test]
fn replay_dispatches_modes_effects_and_typed_alignment_lifecycle() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"a$ $\par\message{ok}\halign&\end");

    assert_eq!(
        control.step(&mut universe).expect("character"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("backed-up character"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("math start"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Math);
    assert_eq!(
        control.step(&mut universe).expect("math space"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("math end"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("paragraph"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Vertical);
    assert_eq!(
        control.step(&mut universe).expect("message"),
        ReplayStep::Continue
    );
    assert!(matches!(
        universe.world().effect_records(),
        [tex_state::EffectRecord::StreamWrite { text, .. }] if text == "ok"
    ));
    assert_eq!(
        control.step(&mut universe).expect("alignment"),
        ReplayStep::Continue
    );
    let alignment = control
        .active_alignment()
        .expect("typed alignment identity");
    control
        .apply_alignment_request(AlignmentRequest::Preamble(alignment))
        .expect("preamble lifecycle");
    control
        .apply_alignment_request(AlignmentRequest::BeginCell {
            alignment,
            templates: AlignmentCellTemplates {
                u_template: None,
                v_template: TracedTokenList::synthetic(universe.intern_token_list(&[])),
            },
        })
        .expect("cell lifecycle");
    control
        .apply_alignment_request(AlignmentRequest::InstallCellTemplate(alignment))
        .expect("cell template lifecycle");
    assert_eq!(
        control
            .alignment_step(alignment, &mut universe)
            .expect("command processor intercepts the cell delimiter"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("expanded end-v finishes the cell through typed command control"),
        ReplayStep::Continue
    );
    control
        .apply_alignment_request(AlignmentRequest::Finish(alignment))
        .expect("alignment lifecycle finishes through command core");
    assert_eq!(control.active_alignment(), None);
    assert_eq!(
        control
            .step(&mut universe)
            .expect("saved delimiter does not re-enter ordinary delivery"),
        ReplayStep::End
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("input exhausted after end"),
        ReplayStep::EndOfInput
    );
}

#[test]
fn command_owned_endv_finishes_cell_and_publishes_retirement_in_canonical_order() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"\halign&\end");

    assert_eq!(
        control.step(&mut universe).expect("alignment"),
        ReplayStep::Continue
    );
    let alignment = control.active_alignment().expect("active alignment");
    for request in [
        AlignmentRequest::Preamble(alignment),
        AlignmentRequest::BeginCell {
            alignment,
            templates: AlignmentCellTemplates {
                u_template: None,
                v_template: TracedTokenList::synthetic(universe.intern_token_list(&[])),
            },
        },
        AlignmentRequest::InstallCellTemplate(alignment),
    ] {
        control
            .apply_alignment_request(request)
            .expect("cell lifecycle setup");
    }
    assert_eq!(
        control.step(&mut universe).expect("intercepted delimiter"),
        ReplayStep::Continue
    );

    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("command-owned end-v"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.windows(5).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                    CommandObservation::Input(retirement),
                    CommandObservation::Alignment(template_retire),
                ] if raw.command == "end_template"
                    && expanded.command == "endv"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 1_000_000
                    && retirement.transition == InputTransition::Retire
                    && retirement.reason == InputReason::AlignmentVTemplate
                    && template_retire.transition == "v_template_retire"
                    && template_retire.align_state == 1_000_000
            )
        }),
        "unexpected observations: {:?}",
        observations.0
    );

    control
        .apply_alignment_request(AlignmentRequest::Finish(alignment))
        .expect("alignment lifecycle finishes through command core");
    assert_eq!(
        control
            .step(&mut universe)
            .expect("saved delimiter does not re-enter ordinary delivery"),
        ReplayStep::End
    );
}

#[test]
fn nested_alignment_begin_suspends_the_outer_replay_context() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    let outer = AlignmentIdentity::new(1);
    control
        .command
        .apply_alignment_request(AlignmentRequest::Begin(outer))
        .expect("outer alignment begins");
    control.active_alignment = Some(ActiveReplayAlignment {
        identity: outer,
        kind: AlignmentKind::HAlign,
        columns: Vec::new(),
        repeat_start: None,
        column: 0,
        preamble_opening_pending: false,
        preamble_opening_replay_pending: false,
        preamble_start_pending: false,
        cell_opening_pending: false,
        next_cell_opening_pending: false,
        align_peek_pending: false,
        align_peek_after_noalign: false,
        noalign_depth: None,
        captured_rows: Vec::new(),
        tabskip: universe.glue_param(GlueParam::TAB_SKIP),
        cell_span: 1,
        row_open: false,
        cell_open: false,
    });
    control.next_alignment_identity = 2;

    apply_scanned_step(
        ScannedStep::BeginAlignment { vertical: false },
        &mut universe,
        &mut control.modes,
        &mut control.next_alignment_identity,
        &mut control.active_alignment,
        &mut control.command,
        &mut control.boxes,
        &mut control.prepared_dvi_pages,
    )
    .expect("nested alignment begins through typed suspension");

    assert_eq!(control.boxes.suspended_alignments.len(), 1);
    let inner = control
        .active_alignment()
        .expect("inner alignment is active");
    assert_ne!(inner, outer);
    apply_scanned_step(
        ScannedStep::AlignmentFinish { alignment: inner },
        &mut universe,
        &mut control.modes,
        &mut control.next_alignment_identity,
        &mut control.active_alignment,
        &mut control.command,
        &mut control.boxes,
        &mut control.prepared_dvi_pages,
    )
    .expect("right-brace align_peek finish resumes the outer context");
    assert_eq!(control.active_alignment(), Some(outer));
    assert_eq!(control.boxes.suspended_alignments.len(), 0);
}

#[test]
fn canonical_alignment_captures_completed_cell_material_before_fin_align() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{#\cr\kern1pt\cr\kern2pt\cr}\end");

    for _ in 0..32 {
        control
            .step(&mut universe)
            .expect("canonical alignment structural step");
        let Some(active) = control.active_alignment.as_ref() else {
            continue;
        };
        if active
            .captured_rows
            .first()
            .is_some_and(|row| !row.is_empty())
        {
            assert!(
                !universe.nodes(active.captured_rows[0][0]).is_empty(),
                "the first completed cell is frozen before final alignment packaging"
            );
            assert_eq!(control.current_mode(), Mode::InternalVertical);
            return;
        }
    }
    panic!("canonical alignment did not capture its first completed row");
}

#[test]
fn canonical_alignment_finalizes_rows_into_the_enclosing_list() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{#&#\cr\kern1pt&\kern2pt\cr}");

    run_to_end(&mut control, &mut universe);

    // TeX82 §800's `fin_align` pops the alignment's nest level and returns
    // to whatever mode enclosed it (§812's "Insert the current list into
    // its environment"); a top-level `\halign` therefore leaves outer
    // vertical mode exactly as `new_graf` never ran. The finished row
    // becomes a page contribution rather than a `current_list` node because
    // outer-vertical material is routed through the page builder (TeX82
    // §994's `build_page`), not retained on the enclosing nest level.
    assert_eq!(control.current_mode(), Mode::Vertical);
    let nodes = universe.current_page_nodes();
    assert!(
        nodes.iter().any(|node| matches!(node, Node::HList(_))),
        "nodes: {nodes:?}; terminal: {}",
        terminal_text(&universe)
    );
}

#[test]
fn scanner_backed_endv_retires_before_an_omit_template() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    // `scan_rule_spec` reads the alignment delimiter while scanning the
    // omitted cell's rule. Its scalar retry backs up the effective `endv`,
    // reproducing TeX82 §772's exhausted backup above `omit_template`.
    register_source(
        &mut control,
        br"\halign{#&#\cr\omit\vrule width3pt&\relax\cr}\end",
    );
    let mut observations = ObservationRecorder::default();

    for _ in 0..48 {
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("scanner-backed end-v completion");
        if observations.0.windows(6).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                    CommandObservation::Input(backup_retirement),
                    CommandObservation::Input(template_retirement),
                    CommandObservation::Alignment(template),
                ] if raw.command == "endv"
                    && expanded.command == "endv"
                    && state_change.transition == "state_change"
                    && backup_retirement.transition == InputTransition::Retire
                    && backup_retirement.reason == InputReason::Backup
                    && template_retirement.transition == InputTransition::Retire
                    && template_retirement.reason == InputReason::AlignmentVTemplate
                    && template.transition == "omit_template_retire"
            )
        }) {
            return;
        }
    }
    panic!(
        "scanner-backed end-v must retire backup then omit-template: {:?}",
        observations.0
    );
}

#[test]
fn paragraph_start_backs_up_the_triggering_macro_parameter_before_replay() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"\def\pair#1#2{#2#1}\pair AB\end");

    assert_eq!(
        control.step(&mut universe).expect("definition"),
        ReplayStep::Continue
    );

    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("paragraph start"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert!(observations.0.windows(2).any(|pair| {
        matches!(
            &pair[0],
            CommandObservation::Input(input)
                if input.transition == InputTransition::Backup && input.reason == InputReason::Backup
        ) && matches!(
            &pair[1],
            CommandObservation::Recovery(recovery)
                if recovery.kind == RecoveryKind::Backup
                    && matches!(
                        recovery.tokens.as_slice(),
                        [ObservedToken::Character { character: 'B', .. }]
                    )
        )
    }));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("backed-up character replay"),
        ReplayStep::Continue
    );
    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("following macro parameter"),
        ReplayStep::Continue
    );
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Input(input)
                if input.transition == InputTransition::Retire && input.reason == InputReason::Backup
        )
    }));
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Command(delivery)
                if matches!(
                    delivery.spelling,
                    ObservedToken::Character { character: 'A', .. }
                )
        )
    }));
}

#[test]
fn missing_canonical_input_rolls_back_the_whole_step_and_retries_fresh() {
    let child = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(&b"\\count3=9"[..]),
    );

    let mut failed_universe = Universe::new();
    crate::install_unexpandable_primitives(&mut failed_universe);
    install_input(&mut failed_universe);
    let mut failed = CommandReplayControl::default();
    register_source(&mut failed, br"\input child\end");
    let mut failed_observations = ObservationRecorder::default();

    assert!(matches!(
        failed
            .advance_with_observer(&mut failed_universe, &mut failed_observations)
            .expect("missing input suspends"),
        CanonicalStepResult::Suspended(CanonicalResourceNeed::Input { name }) if name == "child"
    ));
    assert_eq!(failed_universe.count(3), 0);
    assert_eq!(failed.current_mode(), Mode::Vertical);
    assert!(
        failed_observations.0.is_empty(),
        "failed delivery leaked observation"
    );

    failed
        .capabilities_mut()
        .register_input("child", child.clone());
    assert_eq!(
        failed
            .advance_with_observer(&mut failed_universe, &mut failed_observations)
            .expect("retry succeeds"),
        CanonicalStepResult::Progress(ReplayStep::Continue)
    );

    let mut fresh_universe = Universe::new();
    crate::install_unexpandable_primitives(&mut fresh_universe);
    install_input(&mut fresh_universe);
    let mut fresh = CommandReplayControl::default();
    register_source(&mut fresh, br"\input child\end");
    fresh.capabilities_mut().register_input("child", child);
    let mut fresh_observations = ObservationRecorder::default();
    assert_eq!(
        fresh
            .advance_with_observer(&mut fresh_universe, &mut fresh_observations)
            .expect("fresh input succeeds"),
        CanonicalStepResult::Progress(ReplayStep::Continue)
    );

    assert_eq!(failed_universe.count(3), fresh_universe.count(3));
    assert_eq!(failed.current_mode(), fresh.current_mode());
    assert_eq!(failed_observations.0, fresh_observations.0);
}

#[test]
fn canonical_assignments_apply_prefix_globaldefs_registers_and_arithmetic() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"{\count0=9}\globaldefs=1{\count1=4}\globaldefs=-1{\global\count2=5}\globaldefs=0 \count3=7\advance\count3 by 5\multiply\count3 3\divide\count3 by 2 \dimen4=2pt\advance\dimen4 by 1pt \hsize=4pt\advance\hsize by 2pt \skip5=1pt\advance\skip5 by 2pt \muskip6=1mu\advance\muskip6 by 2mu \end",
    );
    loop {
        if matches!(
            control.step(&mut universe).expect("canonical assignment"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }
    assert_eq!(universe.count(0), 0, "local count restores");
    assert_eq!(universe.count(1), 4, "positive globaldefs forces global");
    assert_eq!(
        universe.count(2),
        0,
        "negative globaldefs suppresses global prefix"
    );
    assert_eq!(universe.count(3), 18);
    assert_eq!(universe.dimen(4), Scaled::from_raw(3 * Scaled::UNITY));
    assert_eq!(
        universe.dimen_param(DimenParam::H_SIZE),
        Scaled::from_raw(6 * Scaled::UNITY)
    );
    assert_eq!(
        universe.glue(universe.skip(5)).width,
        Scaled::from_raw(3 * Scaled::UNITY)
    );
    assert_eq!(
        universe.glue(universe.muskip(6)).width,
        Scaled::from_raw(3 * Scaled::UNITY)
    );
}

#[test]
fn canonical_box_construction_scans_specs_hooks_and_scopes_targets() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\everyhbox{\global\advance\count6 by1}\everyvbox{\global\advance\count7 by1}\begingroup\setbox0=\hbox to 10pt{}\global\setbox1=\vbox to 12pt{}\global\setbox2=\vtop spread 2pt{}\global\setbox3=\hbox{}\endgroup\end",
    );

    loop {
        match control.advance(&mut universe).expect("fresh hook retry") {
            CanonicalStepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => {
                break;
            }
            CanonicalStepResult::Progress(MainControlStep::Continue) => {}
            CanonicalStepResult::Suspended(need) => {
                panic!("registered hook remained suspended: {need:?}")
            }
        }
    }

    assert!(universe.box_reg(0).is_none(), "local setbox restores");
    assert_eq!((universe.count(6), universe.count(7)), (2, 2));
    let vbox = universe
        .box_reg(1)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("global vbox survives");
    let Node::VList(vbox) = vbox else {
        panic!("setbox1 contains a vbox");
    };
    assert_eq!(
        vbox.height + vbox.depth,
        Scaled::from_raw(12 * Scaled::UNITY)
    );
    let vtop = universe
        .box_reg(2)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("global vtop survives");
    assert!(matches!(vtop, Node::VList(_)), "setbox2 contains a vtop");
    let natural = universe
        .box_reg(3)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("global natural hbox survives");
    let Node::HList(natural) = natural else {
        panic!("setbox3 contains an hbox");
    };
    assert_eq!(natural.width, Scaled::from_raw(0));

    let mut packed = Universe::new();
    let mut packed_control = CanonicalMainControl::tex82_initex(&mut packed);
    register_source(&mut packed_control, br"\setbox0=\hbox to 10pt{}\end");
    run_to_end(&mut packed_control, &mut packed);
    let hbox = packed
        .box_reg(0)
        .and_then(|id| packed.nodes(id).first().map(|node| node.to_owned()))
        .expect("hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    assert_eq!(hbox.width, Scaled::from_raw(10 * Scaled::UNITY));
}

#[test]
fn canonical_box_groups_nest_recover_and_preserve_everybox_provenance() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\hbox{}}");
    loop {
        if matches!(
            control
                .step(&mut universe)
                .expect("nested and recovered box program executes"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }

    let outer = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(outer) = outer else {
        panic!("nested result is an hbox");
    };
    assert!(
        matches!(universe.nodes(outer.children).first(), Some(node) if matches!(node.to_owned(), Node::HList(_)))
    );

    let mut provenance = Universe::new();
    let mut provenance_control = CanonicalMainControl::tex82_initex(&mut provenance);
    register_source(
        &mut provenance_control,
        br"\everyhbox{\relax}\setbox0=\hbox{}",
    );
    let mut hook_observations = ObservationRecorder::default();
    loop {
        if matches!(
            provenance_control
                .step_with_observer(&mut provenance, &mut hook_observations)
                .expect("everyhbox program executes"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }
    let hook_origin = hook_observations
        .0
        .iter()
        .find_map(|event| match event {
            CommandObservation::Command(command) if command.command == "relax" => {
                Some(command.provenance.origin)
            }
            _ => None,
        })
        .expect("everyhbox relax is delivered");
    assert_ne!(hook_origin, tex_state::token::OriginId::UNKNOWN);

    let mut recovered = Universe::new();
    let mut recovery_control = CanonicalMainControl::tex82_initex(&mut recovered);
    register_source(&mut recovery_control, br"\setbox1=\hbox to 1pt\relax}");
    run_to_end(&mut recovery_control, &mut recovered);
    assert!(
        recovered.box_reg(1).is_some(),
        "missing brace recovers as a box group"
    );
}

#[test]
fn canonical_box_lifecycle_unboxing_and_leaders_do_not_reopen_input() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\hbox{A}\setbox1=\hbox{\unhcopy0}\setbox2=\hbox{\unhbox0}\setbox3=\hbox{\leaders\hbox{}\hskip10pt}\setbox4=\hbox{\cleaders\hrule\hskip10pt}\setbox5=\hbox{\xleaders\copy3\hskip10pt}\end",
    );
    run_to_end(&mut control, &mut universe);

    assert!(
        universe.box_reg(0).is_none(),
        "unhbox consumes its register"
    );
    assert!(
        universe.box_reg(1).is_some(),
        "unhcopy retains source children"
    );
    for index in [3, 4, 5] {
        let node = universe
            .box_reg(index)
            .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
            .expect("leader result box");
        let Node::HList(box_node) = node else {
            panic!("leader result is hbox")
        };
        assert!(
            matches!(universe.nodes(box_node.children).first(), Some(node) if matches!(node.to_owned(), Node::Glue { leader: Some(_), .. }))
        );
    }
}

#[test]
fn canonical_insert_builds_typed_ins_node_in_current_list() {
    // TeX82 §968/§1096: `\insert<class>{...}` packages its body like a
    // natural `\vbox` and appends the resulting `ins_node` to whatever list
    // was open, not a side channel -- so it shows up as a plain child of the
    // enclosing `\vbox` here, carrying its own separate `content` list.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\vbox{\insert3{\hrule height5pt}}\end",
    );
    run_to_end(&mut control, &mut universe);

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children);
    assert_eq!(children.len(), 1, "the ins_node is the vbox's only child");
    let Node::Ins {
        class,
        size,
        content,
        ..
    } = children.first().expect("checked len").to_owned()
    else {
        panic!("vbox child is an ins_node");
    };
    assert_eq!(class, 3);
    assert_eq!(size, Scaled::from_raw(5 * Scaled::UNITY));
    let inner = universe.nodes(content);
    assert_eq!(inner.len(), 1, "insertion content is the lone rule");
    assert!(matches!(
        inner.first().expect("checked len").to_owned(),
        Node::Rule {
            height: Some(height),
            depth: Some(depth),
            ..
        } if height == Scaled::from_raw(5 * Scaled::UNITY) && depth == Scaled::from_raw(0)
    ));
}

#[test]
fn canonical_insert_recovers_reserved_and_out_of_range_class_numbers() {
    // TeX82 §968: `scan_eight_bit_int`'s own 0..=255 clamp ("Bad register
    // code") and the additional `\insert255` rejection ("box 255 is
    // special") both recover as class 0 rather than aborting the run.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\vbox{\insert255{}\insert1000{}}\end",
    );
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("You can't \\insert255"),
        "reserved class 255 is diagnosed: {text}"
    );
    assert!(
        text.contains("Bad register code (1000)"),
        "out-of-range class is diagnosed: {text}"
    );

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children);
    assert_eq!(children.len(), 2);
    for child in children {
        let Node::Ins { class, .. } = child.to_owned() else {
            panic!("vbox child is an ins_node");
        };
        assert_eq!(class, 0, "both recoveries fall back to class 0");
    }
}

#[test]
fn canonical_insert_at_outer_vertical_reaches_the_page_builder() {
    // TeX82 §968: `if nest_ptr=0 then build_page` -- an `\insert` delivered
    // directly in outer vertical mode must hand its ins_node to the page
    // builder immediately, exercising §§980--987's insertion-class
    // accounting (`tex-exec::page_builder`) rather than merely constructing
    // the typed node.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\dimen0=100pt \insert0{\hrule height5pt}");
    run_to_end(&mut control, &mut universe);

    let insertions = universe.page_insertions();
    assert_eq!(insertions.len(), 1);
    assert_eq!(insertions[0].class(), 0);
    assert_eq!(insertions[0].height(), Scaled::from_raw(5 * Scaled::UNITY));
    assert_eq!(
        insertions[0].status(),
        tex_state::page::PageInsertionStatus::Inserting
    );
}

/// Extracts the shift of each `HList`/`VList` child of `children`, skipping
/// interleaved glue (e.g. `\baselineskip` inserted between vertical-list
/// boxes) so tests can assert on box shifts alone.
fn box_shifts(universe: &Universe, children: tex_state::ids::NodeListId) -> Vec<Scaled> {
    universe
        .nodes(children)
        .into_iter()
        .filter_map(|node| match node.to_owned() {
            Node::HList(box_node) | Node::VList(box_node) => Some(box_node.shift),
            _ => None,
        })
        .collect()
}

#[test]
fn canonical_raise_lower_apply_signed_shift_in_horizontal_mode() {
    // TeX82 §1073: `hmode+vmove` is legal (`\raise`/`\lower`'s own mode
    // family), and `t:=cur_chr; scan_normal_dimen; if t=0 then
    // scan_box(cur_val) else scan_box(-cur_val)` -- `\lower` (chr_code 0)
    // keeps the scanned dimension, `\raise` (chr_code 1) negates it.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\hbox{\lower3pt\hbox{}\raise2pt\hbox{}}",
    );
    run_to_end(&mut control, &mut universe);

    let outer = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(outer) = outer else {
        panic!("setbox0 contains an hbox");
    };
    assert_eq!(
        box_shifts(&universe, outer.children),
        vec![
            Scaled::from_raw(3 * Scaled::UNITY),
            Scaled::from_raw(-2 * Scaled::UNITY),
        ],
        "\\lower keeps its sign, \\raise negates it"
    );
}

#[test]
fn canonical_moveleft_moveright_apply_signed_shift_in_vertical_mode() {
    // TeX82 §1073: `vmode+hmove` is legal (`\moveleft`/`\moveright`'s own
    // mode family). `\moveright` (chr_code 0) keeps the scanned dimension,
    // `\moveleft` (chr_code 1) negates it -- the opposite pairing from
    // `\raise`/`\lower` even though the sign rule is the same shape.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\vbox{\moveright3pt\hbox{}\moveleft2pt\hbox{}}",
    );
    run_to_end(&mut control, &mut universe);

    let outer = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(outer) = outer else {
        panic!("setbox0 contains a vbox");
    };
    assert_eq!(
        box_shifts(&universe, outer.children),
        vec![
            Scaled::from_raw(3 * Scaled::UNITY),
            Scaled::from_raw(-2 * Scaled::UNITY),
        ],
        "\\moveright keeps its sign, \\moveleft negates it"
    );
}

#[test]
fn canonical_box_shift_illegal_mode_reports_and_never_scans_a_dimension() {
    // TeX82 §1073's "Forbidden cases" list `hmode+hmove` (`\moveleft`/
    // `\moveright` used outside vertical mode) alongside `vmode+vmove` and
    // `mmode+hmove`. `report_illegal_case` fires immediately and
    // `scan_normal_dimen` is never called, so the following "2pt" is left
    // as perfectly ordinary character tokens -- not consumed as an
    // operand -- and `\hbox{}` after them is a plain, unshifted box. A real
    // font is selected first so those characters actually reach the list
    // instead of being dropped as "Missing character" under `\nullfont`.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10 \setbox0=\hbox{\f \moveleft2pt\hbox{}}",
    );
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("You can't use `\\moveleft' in restricted horizontal mode"),
        "illegal box-shift mode is diagnosed: {text}"
    );

    let outer = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(outer) = outer else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(outer.children);
    let chars: String = children
        .iter()
        .filter_map(|node| match node.to_owned() {
            Node::Char { ch, .. } => Some(ch),
            _ => None,
        })
        .collect();
    assert_eq!(
        chars, "2pt",
        "the un-scanned dimension became ordinary characters: {children:?}"
    );
    assert_eq!(
        box_shifts(&universe, outer.children),
        vec![Scaled::from_raw(0)],
        "the trailing \\hbox{{}} is an ordinary, unshifted box"
    );
}

#[test]
fn canonical_box_shift_applies_to_box_register_and_last_box() {
    // TeX82 §1076's `scan_box` accepts any `make_box` command, not just
    // `\hbox`/`\vbox`/`\vtop`: `\box`/`\copy` and `\lastbox` resolve to a
    // node immediately rather than opening a group, and the shift must
    // still apply to that immediate result. (A box-shift's own box can
    // never itself be a `\setbox` target -- `scan_box` requires
    // `cur_cmd=make_box`, which `\raise`/`\lower`/`\moveleft`/`\moveright`
    // never are -- so both results are observed as the last thing appended
    // to an enclosing box body instead.)
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox1=\hbox{}\setbox0=\hbox{\raise5pt\box1}\setbox3=\hbox{\hbox{}\lower4pt\lastbox}",
    );
    run_to_end(&mut control, &mut universe);

    let register_shift = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(register_shift) = register_shift else {
        panic!("setbox0 contains an hbox");
    };
    assert_eq!(
        box_shifts(&universe, register_shift.children),
        vec![Scaled::from_raw(-5 * Scaled::UNITY)],
        "\\raise5pt\\box1 shifts the register's box by -5pt"
    );

    let last_box_shift = universe
        .box_reg(3)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(last_box_shift) = last_box_shift else {
        panic!("setbox3 contains an hbox");
    };
    // `\lastbox` removes the inner `\hbox{}` from this same body, then the
    // shifted result is re-appended to it -- so exactly one child remains.
    assert_eq!(
        box_shifts(&universe, last_box_shift.children),
        vec![Scaled::from_raw(4 * Scaled::UNITY)],
        "\\lower4pt\\lastbox shifts the removed box by +4pt and reappends it"
    );
}

#[test]
fn canonical_box_shift_missing_box_operand_recovers_and_replays_the_command() {
    // TeX82 §1076's `scan_box` "A <box> was supposed to be here" recovery:
    // a non-`make_box` command is backed up and replayed normally, rather
    // than being consumed as (or silently dropping) the shift's operand.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\raise2pt\kern1pt}");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("A <box> was supposed to be here"),
        "the missing box operand is diagnosed: {text}"
    );

    let outer = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(outer) = outer else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(outer.children);
    assert!(
        children
            .iter()
            .any(|node| matches!(node.to_owned(), Node::Kern { amount, .. } if amount == Scaled::from_raw(Scaled::UNITY))),
        "the backed-up \\kern1pt was replayed normally: {children:?}"
    );
}

#[test]
fn canonical_assignments_cover_code_tables_and_reject_macro_prefixes() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br#"\catcode`@=11\lccode`A=`a\uccode`a=`A\sfcode`A=1000\mathcode`x="7131\delcode`(=123\end"#,
    );
    loop {
        if matches!(
            control.step(&mut universe).expect("code table assignment"),
            MainControlStep::End
        ) {
            break;
        }
    }
    assert_eq!(universe.catcode('@'), tex_state::token::Catcode::Letter);
    assert_eq!(universe.lccode('A'), 'a' as u32);
    assert_eq!(universe.uccode('a'), 'A' as u32);
    assert_eq!(universe.sfcode('A'), 1000);
    assert_eq!(universe.mathcode('x'), 0x7131);
    assert_eq!(universe.delcode('('), 123);

    let mut invalid_universe = Universe::new();
    let mut invalid = CanonicalMainControl::tex82_initex(&mut invalid_universe);
    register_source(&mut invalid, br"\long\count0=1");
    assert!(matches!(
        invalid.step(&mut invalid_universe),
        Err(ExecError::PrefixWithNonDefinition { .. })
    ));
}

#[test]
fn canonical_vsplit_scans_operands_before_replaying_destructive_repack() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\vbox{\hrule height 10pt\vskip10pt\hrule height 10pt}\setbox1=\vsplit0 to 10pt\end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        if matches!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect("canonical vsplit executes"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }

    let split = universe.box_reg(1).expect("split result is assigned");
    let remainder = universe.box_reg(0).expect("remainder is repacked");
    assert_ne!(
        split, remainder,
        "vsplit does not alias its destructive remainder"
    );
}

#[test]
fn canonical_display_diagnostics_keep_show_raw_and_scan_other_operands() {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\def\shown{expanded}\show\shown\count0=17\showthe\count0\setbox0=\hbox{}\showbox0\end",
    );
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("> \\shown=macro:->expanded."), "{text}");
    assert!(text.contains("> 17."), "{text}");
    assert!(text.contains("> \\box0="), "{text}");
    assert!(
        !text.contains("Undefined control sequence \\expanded"),
        "raw show must not expand its operand: {text}"
    );
}

#[test]
fn canonical_showthe_and_the_preserve_font_identifier_tokens() {
    let mut universe = Universe::new();
    let nullfont = universe.intern("nullfont");
    universe.set_font_identifier_symbol(tex_state::font::NULL_FONT, nullfont);
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\showthe\font\showthe\nullfont\showthe\textfont0\message{\the\font}\end",
    );
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert_eq!(text.matches("> \\nullfont.").count(), 3, "{text}");
    assert!(text.contains("\\nullfont"), "{text}");
}

#[test]
fn canonical_errmessage_uses_the_world_terminal_and_log_boundary() {
    // TeX82's `issue_message` sends \errmessage through `print_err`/`error`,
    // then main control resumes normally.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\errmessage{canonical diagnostic}\message{recovered}\end",
    );
    run_to_end(&mut control, &mut universe);

    let error_effects = universe
        .world()
        .effect_records()
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                EffectRecord::StreamWrite {
                    sink: tex_state::PrintSink::TerminalAndLog,
                    text,
                } if text == "\n! canonical diagnostic.\n"
            )
        })
        .count();
    assert_eq!(error_effects, 1, "diagnostic must be a World output effect");

    let text = terminal_text(&universe);
    assert!(text.contains("! canonical diagnostic."), "{text}");
    assert!(text.contains("recovered"), "{text}");
}

#[test]
fn canonical_delete_last_removes_only_matching_tail_node() {
    // TeX82 §1105's `delete_last`: `\unpenalty` after a kern tail leaves the
    // list untouched (no error, no removal), while `\unkern` correctly
    // matches and removes it.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    // `\hbox`, not `\vbox`: `\kern` in vertical mode is covered separately by
    // `canonical_kern_in_vertical_mode_does_not_start_a_paragraph` below, so
    // this test exercises only `delete_last`'s ordinary (`tail<>head`)
    // branch.
    register_source(
        &mut control,
        br"\setbox0=\hbox{\kern1pt\unpenalty\kern2pt\unkern}",
    );
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert_eq!(
        children,
        vec![Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Explicit,
        }],
        "\\unpenalty left the kern alone; \\unkern removed the second one: {children:?}"
    );
}

#[test]
fn canonical_kern_in_vertical_mode_does_not_start_a_paragraph() {
    // TeX82 §1057's `any_mode(kern): append_kern` (§1061): `\kern` has no
    // mode-specific dispatch entry at all, unlike `\hskip` (§1090's
    // `head_for_vmode`, genuinely `vmode+hskip`-listed). A bare `\kern` in
    // (internal) vertical mode must append directly to the current list, not
    // silently trigger `new_graf` and swallow the kern into a bogus empty
    // paragraph -- regression test for umber2-johp.85.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\vbox{\kern5pt\hrule height1pt}");
    run_to_end(&mut control, &mut universe);

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children).to_vec();
    assert_eq!(
        children,
        vec![
            Node::Kern {
                amount: Scaled::from_raw(5 * Scaled::UNITY),
                kind: KernKind::Explicit,
            },
            Node::Rule {
                width: None,
                height: Some(Scaled::from_raw(Scaled::UNITY)),
                depth: Some(Scaled::from_raw(0)),
            },
        ],
        "the kern must land directly on the vlist, not be dropped by a bogus \
         empty paragraph: {children:?}"
    );
}

#[test]
fn canonical_delete_last_outer_vertical_apologizes_only_when_last_page_item_is_glue() {
    // TeX82 §1105: `(mode=vmode)and(tail=head)` never structurally removes
    // anything -- `\unpenalty`/`\unkern` always apologize, but `\unskip`
    // apologizes only when the page builder's own `last_glue` memo (§996)
    // shows the most recently placed page item really was glue. This is a
    // regression test for a real bug: the outer-vertical empty-list branch
    // used to ignore that memo entirely and always succeed for `\unskip`.
    let mut universe = Universe::new();
    let glue_spec = universe.intern_glue(GlueSpec::ZERO);
    universe.update_page_last_from_node(&Node::Glue {
        spec: glue_spec,
        kind: tex_state::node::GlueKind::Normal,
        leader: None,
    });
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\unskip");
    let err = run_to_error(&mut control, &mut universe);
    assert_eq!(
        err.to_string(),
        "You can't use `\\unskip' in vertical mode."
    );

    let mut universe = Universe::new();
    universe.update_page_last_from_node(&Node::Penalty(0));
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\unskip\end");
    run_to_end(&mut control, &mut universe);
    assert!(
        !terminal_text(&universe).contains("You can't"),
        "\\unskip is silent when the last page item was not glue: {}",
        terminal_text(&universe)
    );
}

#[test]
fn canonical_italic_correction_appends_kern_even_when_zero() {
    // TeX82 §1113's `append_italic_correction` appends the kern
    // unconditionally when the tail is a character node, even when the
    // correction happens to be exactly zero -- there is no width guard in
    // tex.web. This is a regression test: the ported logic used to skip the
    // append whenever the metric was zero.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(&mut control, br"\font\f=cmr10 \setbox0=\hbox{\f A\/}");
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert!(
        matches!(
            children.last(),
            Some(Node::Kern {
                amount,
                kind: KernKind::Explicit,
            }) if *amount == Scaled::from_raw(0)
        ),
        "a zero-width explicit kern still lands after `A`: {children:?}"
    );
}

#[test]
fn canonical_italic_correction_uses_the_font_metric_amount() {
    // Same TeX82 §1113 path as above, but `f` has a real (nonzero) italic
    // correction in cmr10 -- the classic textbook example.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(&mut control, br"\font\f=cmr10 \setbox0=\hbox{\f f\/}");
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert!(
        matches!(
            children.last(),
            Some(Node::Kern {
                amount,
                kind: KernKind::Explicit,
            }) if amount.raw() > 0
        ),
        "`f`'s nonzero italic correction lands as an explicit kern: {children:?}"
    );
}

#[test]
fn canonical_italic_correction_in_math_mode_appends_a_font_kind_zero_kern() {
    // TeX82 §1112's `mmode+ital_corr: tail_append(new_kern(0))` never
    // overrides `new_kern`'s default `normal` subtype the way hmode's
    // italic-correction kern (or an explicit `\kern`) does, so it must not
    // become a legal kern-then-glue line-break point. `KernKind::Font` is
    // Umber's non-breakpoint kern kind (see `linebreak::mod`'s
    // `KernKind::Explicit | KernKind::Mu` break-legality check).
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{$\/$}");
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert!(
        children.iter().any(|node| matches!(
            node,
            Node::Kern {
                amount,
                kind: KernKind::Font,
            } if *amount == Scaled::from_raw(0)
        )),
        "math-mode \\/ appends a zero kern with the non-breakpoint `Font` kind: {children:?}"
    );
}

#[test]
fn canonical_italic_correction_in_vertical_mode_reports_illegal_case() {
    // TeX82 §1111's "Forbidden cases": `vmode+ital_corr` never starts a
    // paragraph the way most other hmode-triggering commands do; it is a
    // plain `report_illegal_case` diagnostic, and the following text is
    // therefore never scanned as an operand.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\/\end");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("You can't use `\\/' in vertical mode."),
        "{text}"
    );
}

#[test]
fn canonical_ignorespaces_skips_spaces_before_the_next_command() {
    // TeX82 §1045's `any_mode(ignore_spaces)`: repeated `get_x_token` skips
    // spaces, then the first non-space command is reprocessed in place --
    // it must not itself become an interword-glue-triggering space.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10 \setbox0=\hbox{\f \ignorespaces   A}",
    );
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert!(
        matches!(children.as_slice(), [Node::Char { ch: 'A', .. }]),
        "the skipped spaces left no glue behind: {children:?}"
    );
}

#[test]
fn canonical_noboundary_in_vertical_mode_starts_a_paragraph() {
    // TeX82 §1090's `vmode+no_boundary` groups with `vmode+ex_space` and
    // friends: `back_input; new_graf(true)`.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\vbox{\noboundary}");
    run_to_end(&mut control, &mut universe);

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children).to_vec();
    assert!(
        matches!(children.first(), Some(Node::HList(_))),
        "\\noboundary started a paragraph, indenting the (empty) first line: {children:?}"
    );
}

#[test]
fn canonical_noboundary_in_math_mode_is_a_no_op() {
    // TeX82 §1045's `mmode+no_boundary: do_nothing`.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"$\noboundary$\end");
    run_to_end(&mut control, &mut universe);

    assert!(
        !terminal_text(&universe).contains('!'),
        "no diagnostics expected: {}",
        terminal_text(&universe)
    );
}

#[test]
fn canonical_nonscript_appends_a_zero_glue_in_math_mode() {
    // TeX82 §1171's `mmode+non_script: tail_append(new_glue(zero_glue));
    // subtype(tail):=cond_math_glue`.
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{$\nonscript\kern1pt$}");
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert!(
        children.iter().any(|node| matches!(
            node,
            Node::Glue {
                kind: tex_state::node::GlueKind::NonScript,
                ..
            }
        )),
        "the nonscript glue reaches the finished hlist: {children:?}"
    );
}

#[test]
fn canonical_nonscript_outside_math_mode_inserts_missing_dollar_sign() {
    // TeX82 §1046's `non_math(non_script)`: `insert_dollar_sign` recovers by
    // opening math mode and reconsidering `\nonscript` there, exactly like
    // the existing `\vskip`-in-math-mode recovery this reuses
    // (`recover_missing_math_shift` is generic over the offending command).
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\nonscript\kern1pt$}");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("Missing $ inserted."), "{text}");
}
