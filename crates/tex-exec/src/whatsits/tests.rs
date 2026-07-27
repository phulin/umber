use super::*;

use std::sync::Arc;

use tex_command::{RegisteredSourceKind, SourceRegistration};
use tex_state::env::banks::IntParam;
use tex_state::node::{Node, Whatsit};
use tex_state::page::{EJECT_PENALTY, PageContents, PageDimension};
use tex_state::scaled::Scaled;
use tex_state::{PrintSink, StreamSlot, Universe};

fn register_source(control: &mut CommandReplayControl, bytes: &[u8]) {
    let source = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("whatsit source registers");
    control
        .command_mut()
        .open_registered_source(source)
        .expect("whatsit source opens");
}

fn run_to_end(control: &mut CommandReplayControl, stores: &mut Universe) {
    loop {
        match control.step(stores).expect("whatsit command executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

#[test]
fn base_whatsit_scanners_construct_each_canonical_subtype() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\value{expanded}\openout2=trace\write-1{\value}\closeout2\special{\value}",
    );
    for _ in 0..5 {
        assert_eq!(
            control.step(&mut stores).expect("construct whatsit"),
            MainControlStep::Continue
        );
    }

    let nodes = control.modes.current_list().nodes();
    assert!(matches!(
        nodes,
        [
            Node::Whatsit(Whatsit::OpenOut { slot, path }),
            Node::Whatsit(Whatsit::DeferredWrite { sink: PrintSink::Log, .. }),
            Node::Whatsit(Whatsit::CloseOut { slot: close }),
            Node::Whatsit(Whatsit::Special { class, payload }),
        ] if slot == &StreamSlot::new(2)
            && path == "trace"
            && close == &StreamSlot::new(2)
            && class == "dvi"
            && payload == b"expanded"
    ));
}

#[test]
fn base_whatsit_copy_free_and_zero_dimension_ownership_match_tex82() {
    let mut stores = Universe::new();
    let tokens = stores.intern_token_list(&[]);
    let originals = vec![
        Node::Whatsit(Whatsit::OpenOut {
            slot: StreamSlot::new(1),
            path: "copy".to_owned(),
        }),
        Node::Whatsit(Whatsit::DeferredWrite {
            sink: PrintSink::TerminalAndLog,
            tokens,
        }),
        Node::Whatsit(Whatsit::Special {
            class: "dvi".to_owned(),
            payload: b"copy".to_vec(),
        }),
    ];
    let copies = originals.clone();
    assert_eq!(copies, originals);
    for node in copies {
        stores.append_page_contribution(node);
    }
    crate::page_builder::build_page(&mut stores).expect("contentless page builds");
    assert_eq!(stores.page_contents(), PageContents::Empty);
    assert_eq!(
        stores.page_dimension(PageDimension::Total),
        Scaled::from_raw(0)
    );
    assert_eq!(stores.current_page_len(), 3);
}

#[test]
fn write_whatsit_stream_clamping_and_malformed_scan_recovery_match_tex82() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut stores);
    control.modes.push(Mode::RestrictedHorizontal);
    register_source(&mut control, br"\write-99{x}\write99{y}");
    run_to_end(&mut control, &mut stores);
    let whatsits = control
        .modes
        .current_list()
        .nodes()
        .iter()
        .filter(|node| matches!(node, Node::Whatsit(_)))
        .collect::<Vec<_>>();
    assert!(matches!(
        whatsits.as_slice(),
        [
            Node::Whatsit(Whatsit::DeferredWrite {
                sink: PrintSink::Log,
                ..
            }),
            Node::Whatsit(Whatsit::DeferredWrite {
                sink: PrintSink::TerminalAndLog,
                ..
            }),
        ]
    ));
}

#[test]
fn whatsit_consumers_each_apply_only_their_primary_visit_action() {
    let mut stores = Universe::new();
    let passive = Node::Whatsit(Whatsit::Special {
        class: "dvi".to_owned(),
        payload: b"passive".to_vec(),
    });
    let language = Node::Whatsit(Whatsit::Language {
        language: 7,
        left_hyphen_min: 2,
        right_hyphen_min: 3,
    });
    assert_eq!(
        crate::assignments::test_language_context(&[passive.clone(), language.clone(), passive]),
        (7, 2, 3)
    );
    stores.append_page_contribution(language);
    crate::page_builder::build_page(&mut stores).expect("language whatsit visits page builder");
    assert_eq!(stores.current_page_len(), 1);
    assert!(stores.world().effect_records().is_empty());
}

#[test]
fn contentless_whatsits_preserve_page_and_vertical_break_boundaries() {
    let mut stores = Universe::new();
    let whatsit = Node::Whatsit(Whatsit::Special {
        class: "dvi".to_owned(),
        payload: Vec::new(),
    });
    stores.append_page_contribution(whatsit.clone());
    crate::page_builder::build_page(&mut stores).expect("contentless whatsit enters page");
    assert_eq!(stores.page_contents(), PageContents::Empty);
    assert_eq!(
        stores.page_dimension(PageDimension::Goal),
        Scaled::MAX_DIMEN
    );

    let split = tex_typeset::vert_break(
        &stores,
        &[whatsit, Node::Penalty(EJECT_PENALTY)],
        Scaled::from_raw(10),
        Scaled::from_raw(0),
    )
    .expect("vertical break skips whatsit dimensions");
    assert_eq!(split.break_index, Some(1));
    assert_eq!(split.best_height_plus_depth, Scaled::from_raw(0));
}

#[test]
fn fix_language_and_setlanguage_append_with_captured_hyphen_minima() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::LANGUAGE, 7);
    stores.set_int_param(IntParam::LEFT_HYPHEN_MIN, 0);
    stores.set_int_param(IntParam::RIGHT_HYPHEN_MIN, 90);
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal);

    crate::assignments::test_fix_hyphen_language(&mut nest, &mut stores, Mode::Horizontal);
    crate::assignments::test_fix_hyphen_language(&mut nest, &mut stores, Mode::Horizontal);

    assert!(matches!(
        nest.current_list().nodes(),
        [Node::Whatsit(Whatsit::Language {
            language: 7,
            left_hyphen_min: 1,
            right_hyphen_min: 63,
        })]
    ));
}
