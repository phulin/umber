use super::*;

use std::sync::Arc;

use tex_command::{CommandObservation, CommandObserver, RegisteredSourceKind, SourceRegistration};
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

#[test]
fn ninth_parameter_recovery_preserves_the_mismatched_delimiter() {
    let stores = run_source(
        br"\catcode`U=6 \long\def\lo#1#2U3#4#5#6#7#8#8#99#{\relax}\lo\par\par\par P\par\par\par\par\par\par89{}\count0=37\end",
    );
    assert_eq!(stores.count(0), 37, "{:?}", stores.world().effect_records());
}

#[derive(Default)]
struct ObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for ObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn observed_effects(source: &[u8]) -> Vec<(String, String)> {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut stores);
    let mut observer = ObservationRecorder::default();
    register_source(&mut control, source);
    loop {
        match control
            .step_with_observer(&mut stores, &mut observer)
            .expect("observed effect command executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    observer
        .0
        .into_iter()
        .filter_map(|observation| match observation {
            CommandObservation::Effect(effect)
                if matches!(effect.kind, "open" | "close" | "shipout") =>
            {
                Some((effect.kind.into(), effect.detail))
            }
            _ => None,
        })
        .collect()
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
fn output_stream_final_cleanup_closes_only_live_numbered_files() {
    let source = br"\immediate\openout15=last
           \immediate\openout2=already-closed
           \immediate\closeout2
           \immediate\openout0=first
           \immediate\write-1{log fallback}
           \immediate\write16{terminal fallback}
           \end";
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut stores);
    let mut observer = ObservationRecorder::default();
    register_source(&mut control, source);
    loop {
        match control
            .step_with_observer(&mut stores, &mut observer)
            .expect("observed cleanup command executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert!(matches!(
        stores.world().effect_records(),
        [
            EffectRecord::StreamOpen { slot: last, target: last_target },
            EffectRecord::StreamOpen { slot: closed, target: closed_target },
            EffectRecord::StreamClose { slot: explicitly_closed },
            EffectRecord::StreamOpen { slot: first, target: first_target },
            EffectRecord::StreamWrite { sink: PrintSink::Log, text: log },
            EffectRecord::StreamWrite {
                sink: PrintSink::TerminalAndLog,
                text: terminal,
            },
            EffectRecord::StreamClose { slot: cleanup_first },
            EffectRecord::StreamClose { slot: cleanup_last },
        ] if *last == StreamSlot::new(15)
            && last_target.path() == std::path::Path::new("last.tex")
            && *closed == StreamSlot::new(2)
            && closed_target.path() == std::path::Path::new("already-closed.tex")
            && *explicitly_closed == StreamSlot::new(2)
            && *first == StreamSlot::new(0)
            && first_target.path() == std::path::Path::new("first.tex")
            && log == "log fallback\n"
            && terminal == "terminal fallback\n"
            && *cleanup_first == StreamSlot::new(0)
            && *cleanup_last == StreamSlot::new(15)
    ));
    let observed_effects: Vec<_> = observer
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Effect(effect) => Some((effect.kind, effect.detail.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        &observed_effects[observed_effects.len() - 3..],
        [
            ("close", "stream:0\0"),
            ("close", "stream:15\0"),
            ("terminate", "engine\0"),
        ],
        "§1378 cleanup closes precede the terminal engine observation"
    );
    for raw in 0..tex_state::world::STREAM_SLOT_COUNT as u8 {
        assert!(
            stores
                .world()
                .stream_bufs()
                .write_stream_target(StreamSlot::new(raw))
                .is_none(),
            "numbered stream {raw} survived final cleanup"
        );
    }
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
fn deferred_write_unbalanced_recovery_stops_at_endwrite() {
    let stores = run_source(
        br"\def\missingright{\iftrue{\else}\fi}\shipout\hbox{\write16{before\missingright after}}\count0=37\end",
    );
    let terminal = std::str::from_utf8(
        stores
            .world()
            .memory_terminal_output()
            .expect("memory terminal"),
    )
    .expect("terminal is UTF-8");
    assert!(
        !stores.world().committed_artifacts().is_empty(),
        "{terminal:?}"
    );
    let artifact = last_artifact(&stores);
    assert!(matches!(
        artifact.effects.as_slice(),
        [PageEffect::Write { text, .. }] if text.contains("before")
    ));
    assert!(
        terminal.contains("Unbalanced write command"),
        "{terminal:?}"
    );
    assert_eq!(stores.count(0), 37, "following source input must survive");
}

#[test]
fn deferred_write_balanced_and_nested_groups_do_not_trigger_recovery() {
    for source in [
        br"\shipout\hbox{\write16{balanced}}\end".as_slice(),
        br"\def\nested{{inner}}\shipout\hbox{\write16{outer\nested tail}}\end".as_slice(),
    ] {
        let stores = run_source(source);
        let terminal = stores.world().memory_terminal_output().unwrap_or_default();
        assert!(
            !terminal
                .windows(b"Unbalanced write command".len())
                .any(|window| window == b"Unbalanced write command")
        );
        assert!(matches!(
            last_artifact(&stores).effects.as_slice(),
            [PageEffect::Write { text, .. }] if text.ends_with('\n')
        ));
    }
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
fn immediate_and_deferred_stream_effects_share_commit_order_and_identity() {
    let immediate = observed_effects(br"\immediate\openout2=trace\immediate\closeout2");
    assert_eq!(
        immediate,
        [
            ("open".into(), "stream:2\0trace.tex".into()),
            ("close".into(), "stream:2\0".into()),
        ]
    );

    let deferred = observed_effects(br"\shipout\hbox{\openout2=trace\closeout2}");
    assert_eq!(
        deferred,
        [
            ("open".into(), "stream:2\0trace.tex".into()),
            ("close".into(), "stream:2\0".into()),
            ("shipout".into(), "dvi\0".to_owned() + "1"),
        ]
    );
}

#[test]
fn closed_and_normalized_output_selectors_publish_no_close_effect() {
    assert!(observed_effects(br"\immediate\closeout2").is_empty());
    assert!(observed_effects(br"\immediate\closeout16\immediate\closeout17").is_empty());
    assert_eq!(
        observed_effects(br"\shipout\hbox{\closeout2\closeout16\closeout17}"),
        [("shipout".into(), "dvi\0".to_owned() + "1")]
    );
}

#[test]
fn each_real_stream_transition_is_observed_exactly_once() {
    assert_eq!(
        observed_effects(
            br"\immediate\openout3=first\immediate\closeout3\immediate\closeout3\shipout\hbox{\openout3=second\closeout3\closeout3}",
        ),
        [
            ("open".into(), "stream:3\0first.tex".into()),
            ("close".into(), "stream:3\0".into()),
            ("open".into(), "stream:3\0second.tex".into()),
            ("close".into(), "stream:3\0".into()),
            ("shipout".into(), "dvi\0".to_owned() + "1"),
        ]
    );
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
