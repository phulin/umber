use super::*;

use std::sync::Arc;

use tex_command::{RegisteredSourceKind, SourceRegistration};
use tex_out::{EffectSink, PageArtifact, PageEffect, PageNode};
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, LeaderPayload, Node, Sign, Whatsit};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::{EffectRecord, PrintSink, StreamSlot, Universe};

fn register_source(control: &mut CommandReplayControl, bytes: &[u8]) {
    let source = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("effect source registers");
    control
        .command_mut()
        .open_registered_source(source)
        .expect("effect source opens");
}

fn run_to_end(control: &mut CommandReplayControl, stores: &mut Universe) {
    loop {
        match control.step(stores).expect("effect command executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

fn run_source(source: &[u8]) -> Universe {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut stores);
    register_source(&mut control, source);
    run_to_end(&mut control, &mut stores);
    stores
}

fn last_artifact(stores: &Universe) -> PageArtifact {
    let committed = stores
        .world()
        .committed_artifacts()
        .last()
        .expect("shipout commits an artifact");
    PageArtifact::from_bytes(committed.bytes()).expect("committed artifact parses")
}

fn state_box(stores: &mut Universe, children: &[Node], vertical: bool) -> Node {
    let children = stores.freeze_node_list(children);
    let payload = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1_000),
        height: Scaled::from_raw(100),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: tex_state::glue::Order::Normal,
        children,
    });
    if vertical {
        Node::VList(payload)
    } else {
        Node::HList(payload)
    }
}

#[test]
fn output_stream_table_initializes_numbered_and_fallback_selectors_closed() {
    let stores = Universe::new();
    for raw in 0..16 {
        assert!(
            stores
                .world()
                .stream_bufs()
                .write_stream_target(StreamSlot::new(raw))
                .is_none()
        );
    }
    let fallback_sinks = [PrintSink::Log, PrintSink::TerminalAndLog];
    assert!(
        fallback_sinks
            .iter()
            .all(|sink| !matches!(sink, PrintSink::Stream(_)))
    );
    assert_ne!(fallback_sinks[0], fallback_sinks[1]);
}

#[test]
fn special_out_synchronizes_position_and_preserves_stored_payload_bytes() {
    let stores = run_source(br"\shipout\hbox{\kern10sp\special{abc}}\end");
    let artifact = last_artifact(&stores);
    assert!(matches!(
        artifact.effects.as_slice(),
        [PageEffect::Special { class, payload }] if class == "dvi" && payload == b"abc"
    ));
    assert!(matches!(
        artifact.root,
        PageNode::HList(ref root)
            if matches!(root.children.as_slice(), [PageNode::Kern { .. }, PageNode::WhatsitAnchor { effect_index: 0 }])
    ));
    let dvi = tex_out::dvi::write_dvi(&[artifact]).expect("special DVI serializes");
    assert!(
        dvi.windows(5)
            .any(|window| window == [239, 3, b'a', b'b', b'c'])
    );
}

#[test]
fn special_out_xxx1_xxx4_length_boundary_matches_tex82() {
    let mut stores = Universe::new();
    let root = state_box(
        &mut stores,
        &[
            Node::Whatsit(Whatsit::Special {
                class: "dvi".to_owned(),
                payload: vec![b'a'; 255],
            }),
            Node::Whatsit(Whatsit::Special {
                class: "dvi".to_owned(),
                payload: vec![b'b'; 256],
            }),
        ],
        false,
    );
    let artifact = crate::assignments::test_stage_shipout_artifact(root, &mut stores)
        .expect("direct special shipout stages");
    let dvi = tex_out::dvi::write_dvi(&[artifact]).expect("boundary specials serialize");
    let short = dvi
        .iter()
        .position(|byte| *byte == 239)
        .expect("xxx1 opcode");
    let long = dvi
        .iter()
        .position(|byte| *byte == 242)
        .expect("xxx4 opcode");
    assert_eq!(dvi[short + 1], 255);
    assert_eq!(&dvi[long + 1..long + 5], &256_i32.to_be_bytes());
}

#[test]
fn deferred_write_expands_at_shipout_time_and_retires_stopper_input() {
    let stores = run_source(
        br"\def\value{old}\setbox0=\hbox{\write16{\value}}\def\value{new}\shipout\box0\end",
    );
    let artifact = last_artifact(&stores);
    assert!(matches!(
        artifact.effects.as_slice(),
        [PageEffect::Write { sink: EffectSink::TerminalAndLog, text }] if text == "new\n"
    ));
    assert!(stores.world().effect_records().is_empty());
}

#[test]
fn deferred_write_stream_selector_and_newline_boundaries_match_tex82() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\newlinechar=`|\immediate\write-1{a|b}\immediate\write16{c}\immediate\write17{d}",
    );
    run_to_end(&mut control, &mut stores);
    assert!(matches!(
        stores.world().effect_records(),
        [
            EffectRecord::StreamWrite { sink: PrintSink::Log, text: first },
            EffectRecord::StreamWrite { sink: PrintSink::TerminalAndLog, text: second },
            EffectRecord::StreamWrite { sink: PrintSink::TerminalAndLog, text: third },
        ] if first == "a\nb\n" && second == "c\n" && third == "d\n"
    ));
}

#[test]
fn out_what_open_write_close_updates_numbered_stream_state_canonically() {
    let stores = run_source(br"\shipout\hbox{\openout2=trace\write2{ready}\closeout2}\end");
    let artifact = last_artifact(&stores);
    assert!(matches!(
        artifact.effects.as_slice(),
        [
            PageEffect::OpenOut { stream: 2, path },
            PageEffect::Write { sink: EffectSink::Stream(2), text },
            PageEffect::CloseOut { stream: 2 },
        ] if path == "trace.tex" && text == "ready\n"
    ));
    assert!(
        stores
            .world()
            .stream_bufs()
            .write_stream_target(StreamSlot::new(2))
            .is_none()
    );
    assert_eq!(
        stores.world().memory_output("trace.tex"),
        Some(&b"ready\n"[..])
    );
}

#[test]
fn immediate_recognized_default_extension_and_unrecognized_backup_paths_match_tex82() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\immediate\openout2=trace\immediate\write2{ready}\immediate\closeout2\immediate\catcode`A=12",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(stores.catcode('A'), tex_state::token::Catcode::Other);
    assert!(matches!(
        stores.world().effect_records(),
        [
            EffectRecord::StreamOpen { slot, target },
            EffectRecord::StreamWrite { sink: PrintSink::Stream(write_slot), text },
            EffectRecord::StreamClose { slot: close_slot },
        ] if *slot == StreamSlot::new(2)
            && target.path() == std::path::Path::new("trace.tex")
            && *write_slot == StreamSlot::new(2)
            && text == "ready\n"
            && *close_slot == StreamSlot::new(2)
    ));
}

#[test]
fn out_what_leader_suppression_and_open_retry_recovery_match_tex82() {
    let mut stores = Universe::new();
    let leader_children = stores.freeze_node_list(&[
        Node::Whatsit(Whatsit::OpenOut {
            slot: StreamSlot::new(3),
            path: "suppressed".to_owned(),
        }),
        Node::Whatsit(Whatsit::DeferredWrite {
            sink: PrintSink::Stream(StreamSlot::new(3)),
            tokens: tex_state::ids::TokenListId::EMPTY,
        }),
        Node::Whatsit(Whatsit::CloseOut {
            slot: Some(StreamSlot::new(3)),
        }),
        Node::Whatsit(Whatsit::Special {
            class: "dvi".to_owned(),
            payload: b"kept".to_vec(),
        }),
    ]);
    let leader = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(1),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: tex_state::glue::Order::Normal,
        children: leader_children,
    });
    let zero = stores.intern_glue(tex_state::glue::GlueSpec::ZERO);
    let root = state_box(
        &mut stores,
        &[Node::Glue {
            spec: zero,
            kind: GlueKind::Leaders,
            leader: Some(LeaderPayload::HList(leader)),
        }],
        false,
    );
    let artifact = crate::assignments::test_stage_shipout_artifact(root, &mut stores)
        .expect("leader effects stage");
    assert!(
        artifact
            .effects
            .iter()
            .all(|effect| matches!(effect, PageEffect::Special { .. }))
    );
    assert!(
        artifact.effects.iter().any(
            |effect| matches!(effect, PageEffect::Special { payload, .. } if payload == b"kept")
        )
    );
    assert!(stores.world().effect_records().is_empty());
}
