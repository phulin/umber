use super::*;

use std::sync::Arc;

use tex_command::{
    CommandObservation, CommandObserver, EffectRecord as ObservedEffect, InputReason,
    InputTransition, ObservedToken, RegisteredSourceKind, SourceRegistration,
};
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

fn register_input(control: &mut CommandReplayControl, name: &str, bytes: &[u8]) {
    control.capabilities_mut().register_input(
        name,
        SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(bytes)),
    );
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

fn observed(source: &[u8]) -> (Universe, CommandReplayControl, Vec<CommandObservation>) {
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
    (stores, control, observer.0)
}

fn observed_effect_records(source: &[u8]) -> Vec<ObservedEffect> {
    observed(source)
        .2
        .into_iter()
        .filter_map(|observation| match observation {
            CommandObservation::Effect(effect) => Some(effect),
            _ => None,
        })
        .collect()
}

fn observed_effects(source: &[u8]) -> Vec<(String, String)> {
    observed_effect_records(source)
        .into_iter()
        .filter(|effect| matches!(effect.kind, "open" | "close" | "shipout"))
        .map(|effect| (effect.kind.into(), effect.detail))
        .collect()
}

fn observed_text_tokens(text: &str) -> Vec<ObservedToken> {
    text.chars()
        .map(|character| ObservedToken::Character {
            character,
            catcode: if character.is_ascii_alphabetic() {
                tex_state::token::Catcode::Letter
            } else {
                tex_state::token::Catcode::Other
            },
        })
        .collect()
}

#[test]
fn deferred_stream_effect_cursor_preserves_exact_records_and_write_order() {
    // TeX82 §§1374--1375 run each `out_what` synchronously: opening the
    // stream precedes each following write expansion, and closing it follows.
    for (source, expected_timeline, expected_writes) in [
        (
            br"\shipout\hbox{\openout1=trace.out\closeout1}".as_slice(),
            vec!["open", "close", "shipout"],
            vec![],
        ),
        (
            br"\shipout\hbox{\openout1=trace.out\write1{one}\write1{two}\closeout1}".as_slice(),
            vec!["open", "write", "write", "close", "shipout"],
            vec![
                ObservedEffect {
                    kind: "write",
                    detail: "stream:1\0".into(),
                    source: None,
                    tokens: Some(observed_text_tokens("one")),
                },
                ObservedEffect {
                    kind: "write",
                    detail: "stream:1\0".into(),
                    source: None,
                    tokens: Some(observed_text_tokens("two")),
                },
            ],
        ),
    ] {
        let (_, _, observations) = observed(source);
        let timeline = observations
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Effect(effect)
                    if matches!(effect.kind, "open" | "close" | "shipout") =>
                {
                    Some(effect.kind)
                }
                CommandObservation::Input(input)
                    if input.transition == InputTransition::Push
                        && input.reason == InputReason::Write =>
                {
                    Some("write")
                }
                _ => None,
            });
        assert_eq!(timeline.collect::<Vec<_>>(), expected_timeline);
        assert_eq!(
            observations
                .iter()
                .filter_map(|observation| match observation {
                    CommandObservation::Effect(effect) if effect.kind == "write" => {
                        Some(effect.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
            expected_writes,
            "deferred writes publish exactly once with their expanded payloads"
        );
        assert_eq!(
            observations
                .into_iter()
                .filter_map(|observation| match observation {
                    CommandObservation::Effect(effect)
                        if matches!(effect.kind, "open" | "close" | "shipout") =>
                    {
                        Some(effect)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [
                ObservedEffect {
                    kind: "open",
                    detail: "stream:1\0trace.out".into(),
                    source: None,
                    tokens: None,
                },
                ObservedEffect {
                    kind: "close",
                    detail: "stream:1\0".into(),
                    source: None,
                    tokens: None,
                },
                ObservedEffect {
                    kind: "shipout",
                    detail: "dvi\0".to_owned() + "1",
                    source: None,
                    tokens: None,
                },
            ]
        );
    }
}

#[test]
fn immediate_stream_effects_keep_exact_write_tokens_without_shipout_receipt() {
    assert_eq!(
        observed_effect_records(
            br"\immediate\openout1=trace.out\immediate\write1{Ready!}\immediate\closeout1"
        ),
        [
            ObservedEffect {
                kind: "open",
                detail: "stream:1\0trace.out".into(),
                source: None,
                tokens: None,
            },
            ObservedEffect {
                kind: "write",
                detail: "stream:1\0".into(),
                source: None,
                tokens: Some(observed_text_tokens("Ready!")),
            },
            ObservedEffect {
                kind: "close",
                detail: "stream:1\0".into(),
                source: None,
                tokens: None,
            },
        ]
    );
}

#[test]
fn deferred_stream_write_observation_has_the_full_expanded_payload_exactly_once() {
    // TeX82 §§1369--1372 finish the `write_out` token-list episode before
    // §1375 prints the result. The semantic effect therefore owns that exact
    // expanded list and is published once, between the surrounding stream
    // lifecycle effects.
    let effects = observed_effect_records(
        br"\shipout\hbox{\openout1=trace.out\write1{\noexpand\endgroup!\noexpand\fi}\closeout1}",
    );
    assert_eq!(
        effects,
        [
            ObservedEffect {
                kind: "open",
                detail: "stream:1\0trace.out".into(),
                source: None,
                tokens: None,
            },
            ObservedEffect {
                kind: "write",
                detail: "stream:1\0".into(),
                source: None,
                tokens: Some(vec![
                    ObservedToken::ControlSequence("endgroup".into()),
                    ObservedToken::Character {
                        character: '!',
                        catcode: tex_state::token::Catcode::Other,
                    },
                    ObservedToken::ControlSequence("fi".into()),
                ]),
            },
            ObservedEffect {
                kind: "close",
                detail: "stream:1\0".into(),
                source: None,
                tokens: None,
            },
            ObservedEffect {
                kind: "shipout",
                detail: "dvi\0".to_owned() + "1",
                source: None,
                tokens: None,
            },
        ]
    );
    assert_eq!(
        effects
            .iter()
            .filter(|effect| effect.kind == "write")
            .count(),
        1
    );
}

#[test]
fn prepared_page_receipt_retains_only_the_unobserved_effect_suffix() {
    let (_, mut control, observations) =
        observed(br"\shipout\hbox{\openout1=trace.out\write1{one}\closeout1}");
    let pages = control.take_prepared_dvi_pages();
    assert_eq!(pages.len(), 1);
    assert!(matches!(
        pages[0].committed_effects.as_ref(),
        [
            EffectRecord::StreamWrite {
                sink: PrintSink::Stream(write_slot),
                text,
            },
            EffectRecord::StreamClose { slot },
        ] if *write_slot == StreamSlot::new(1)
            && text == "one\n"
            && *slot == StreamSlot::new(1)
    ));
    assert_eq!(
        pages[0].committed_effects.len(),
        2,
        "the pre-write open was consumed by the cursor; finalization retains the write/close suffix"
    );
    assert_eq!(
        observations
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Effect(effect)
                    if matches!(effect.kind, "open" | "close" | "shipout") =>
                {
                    Some((effect.kind, effect.detail.as_str(), effect.tokens.as_ref()))
                }
                _ => None,
            })
            .collect::<Vec<_>>(),
        [
            ("open", "stream:1\0trace.out", None),
            ("close", "stream:1\0", None),
            ("shipout", "dvi\u{0}1", None),
        ],
        "open, close, and shipout each publish once"
    );
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
fn out_what_retries_unavailable_openout_with_a_replacement_name() {
    let mut stores = Universe::new();
    stores.world_mut().deny_memory_output("blocked.tex");
    stores
        .world_mut()
        .push_memory_terminal_line("recovered")
        .expect("terminal replacement");
    let ordinary = state_box(
        &mut stores,
        &[Node::Whatsit(Whatsit::OpenOut {
            slot: StreamSlot::new(2),
            path: "blocked".to_owned(),
        })],
        false,
    );
    let artifact = crate::assignments::test_stage_shipout_artifact(ordinary, &mut stores)
        .expect("unavailable openout retries");
    assert!(matches!(
        artifact.effects.as_slice(),
        [PageEffect::OpenOut { stream: 2, path }] if path == "recovered.tex"
    ));
    let diagnostic = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(diagnostic.contains("I can't write on file `blocked.tex'"));
    assert!(matches!(
        stores.world().effect_records().last(),
        Some(EffectRecord::StreamOpen { slot, target })
            if *slot == StreamSlot::new(2)
                && target.path() == std::path::Path::new("recovered.tex")
    ));
}

#[test]
fn out_what_retry_prints_captured_tex_context_before_prompt() {
    let source = br"\setbox0=\hbox{\openout2=blocked }\shipout\copy0\end";
    for interaction in [
        tex_state::InteractionMode::Scroll,
        tex_state::InteractionMode::ErrorStop,
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_interaction_mode(interaction);
        stores.world_mut().deny_memory_output("blocked.tex");
        stores
            .world_mut()
            .push_memory_terminal_line("recovered")
            .expect("terminal replacement");
        let mut control = CommandReplayControl::tex82_initex(&mut stores);
        register_source(&mut control, source);
        run_to_end(&mut control, &mut stores);
        let terminal = String::from_utf8(
            stores
                .world()
                .memory_terminal_output()
                .unwrap_or_default()
                .to_vec(),
        )
        .expect("terminal is utf-8");
        assert!(
            terminal.contains(
                "I can't write on file `blocked.tex'.\n<to be read again> \n                   \\end \nl.1 ...\\hbox{\\openout2=blocked }\\shipout\\copy0\\end\n                                                  \nPlease type another output file name: "
            ),
            "{terminal:?}"
        );
    }
}

#[test]
fn out_what_retry_show_context_includes_nested_token_level_and_obeys_context_limit() {
    let padding = "abcdefghijklmnopqrstuvwxyz".repeat(4);
    let source = format!(
        "\\errorcontextlines=1 \\def\\outer#1{{\\setbox0=\\hbox{{#1}}\\shipout\\copy0}}\
         \\outer{{\\openout2=blocked {padding}}}\\end"
    );
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
    stores.world_mut().deny_memory_output("blocked.tex");
    stores
        .world_mut()
        .push_memory_terminal_line("recovered")
        .expect("terminal replacement");
    let mut control = CommandReplayControl::tex82_initex(&mut stores);
    register_source(&mut control, source.as_bytes());
    run_to_end(&mut control, &mut stores);
    let terminal = String::from_utf8(
        stores
            .world()
            .memory_terminal_output()
            .expect("terminal output")
            .to_vec(),
    )
    .expect("terminal output is utf-8");
    assert!(terminal.contains("<to be read again> "), "{terminal:?}");
    assert!(terminal.contains("mnopqrstuvwxyz}\\end"), "{terminal:?}");
    assert!(terminal.contains("..."), "{terminal:?}");
    assert!(
        terminal.contains("l.1 ..."),
        "§82 cropping must preserve the source-location label: {terminal:?}"
    );
    assert!(
        terminal
            .find("<to be read again> ")
            .expect("backed-up context")
            < terminal
                .find("Please type another output file name")
                .expect("replacement prompt"),
        "{terminal:?}"
    );
}

#[test]
fn out_what_retry_show_context_traverses_nested_sources_with_limit() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
    stores.world_mut().deny_memory_output("blocked.tex");
    stores
        .world_mut()
        .push_memory_terminal_line("recovered")
        .expect("terminal replacement");
    let mut control = CommandReplayControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\errorcontextlines=0 \input middle ROOT-CONTEXT\end",
    );
    register_input(&mut control, "middle.tex", br"\input leaf MIDDLE-CONTEXT");
    register_input(
        &mut control,
        "leaf.tex",
        br"\setbox0=\hbox{\openout2=blocked }\shipout\copy0 LEAF-CONTEXT",
    );
    run_to_end(&mut control, &mut stores);

    let terminal = String::from_utf8(
        stores
            .world()
            .memory_terminal_output()
            .expect("terminal output")
            .to_vec(),
    )
    .expect("terminal output is utf-8");
    let leaf = terminal
        .find("LEAF-CONTEXT")
        .expect("current nested source");
    let omitted = terminal[leaf..].find("\n...").expect("omission marker") + leaf;
    let root = terminal.find("ROOT-CONTEXT").expect("bottom root source");
    assert!(leaf < omitted && omitted < root, "{terminal:?}");
    assert!(!terminal.contains("MIDDLE-CONTEXT"), "{terminal:?}");
    assert!(
        root < terminal
            .find("Please type another output file name")
            .expect("replacement prompt"),
        "{terminal:?}"
    );
}

#[test]
fn out_what_retries_terminal_names_with_tex_buffer_rules() {
    let mut stores = Universe::new();
    for path in ["blocked.tex", ".tex", "again.tex"] {
        stores.world_mut().deny_memory_output(path);
    }
    for line in ["", "again ignored", "\"final name\" ignored"] {
        stores
            .world_mut()
            .push_memory_terminal_line(line)
            .expect("terminal replacement");
    }
    let ordinary = state_box(
        &mut stores,
        &[Node::Whatsit(Whatsit::OpenOut {
            slot: StreamSlot::new(2),
            path: "blocked".to_owned(),
        })],
        false,
    );
    let artifact = crate::assignments::test_stage_shipout_artifact(ordinary, &mut stores)
        .expect("repeated output retry succeeds");
    assert!(matches!(
        artifact.effects.as_slice(),
        [PageEffect::OpenOut { path, .. }] if path == "final name.tex"
    ));
}

#[test]
fn out_what_noninteractive_failure_is_fatal_without_reading_terminal() {
    for interaction in [
        tex_state::InteractionMode::Batch,
        tex_state::InteractionMode::Nonstop,
    ] {
        let mut stores = Universe::new();
        stores.set_interaction_mode(interaction);
        stores.world_mut().deny_memory_output("blocked.tex");
        stores
            .world_mut()
            .push_memory_terminal_line("must-remain")
            .expect("terminal line");
        let ordinary = state_box(
            &mut stores,
            &[Node::Whatsit(Whatsit::OpenOut {
                slot: StreamSlot::new(2),
                path: "blocked".to_owned(),
            })],
            false,
        );
        assert!(matches!(
            crate::assignments::test_stage_shipout_artifact(ordinary, &mut stores),
            Err(ExecError::Fatal(tex_command::FatalError::EmergencyStop {
                help: "job aborted, file error in nonstop mode"
            }))
        ));
        assert_eq!(
            stores
                .world_mut()
                .read_terminal_line()
                .expect("terminal remains readable")
                .as_deref(),
            Some("must-remain")
        );
        let terminal = String::from_utf8(
            stores
                .world()
                .memory_terminal_output()
                .unwrap_or_default()
                .to_vec(),
        )
        .expect("terminal output is utf-8");
        assert!(!terminal.contains("Please type another output file name"));
        assert!(!terminal.contains(": "));
    }
}

#[test]
fn commit_time_open_retry_uses_captured_interaction_and_terminal_context() {
    let mut stores = Universe::new();
    stores.begin_retained_session().expect("retained session");
    stores.world_mut().open_out(StreamSlot::new(2), ".");
    stores
        .world_mut()
        .push_memory_terminal_line("\"replacement name\" ignored")
        .expect("terminal replacement");
    stores
        .world_mut()
        .retarget_output_backend(&tex_state::World::real())
        .expect("real output backend");
    let error = stores
        .export_retained_effects()
        .expect_err("open is unavailable");
    let failed = error
        .stream_open_unavailable()
        .expect("typed failed target")
        .to_owned();

    let replacement = crate::retry_unavailable_stream_open(&mut stores, &failed)
        .expect("captured §530 context supplies replacement");
    stores
        .world_mut()
        .retarget_pending_stream_open(&failed, replacement)
        .expect("exact failed effect retargets");

    assert!(matches!(
        stores.world().effect_records().first(),
        Some(EffectRecord::StreamOpen { target, .. })
            if target.path() == std::path::Path::new("replacement name.tex")
    ));
}

#[test]
fn commit_time_open_retry_is_fatal_without_consuming_input_in_nonstop_modes() {
    for interaction in [
        tex_state::InteractionMode::Batch,
        tex_state::InteractionMode::Nonstop,
    ] {
        let mut stores = Universe::new();
        stores.set_interaction_mode(interaction);
        stores.begin_retained_session().expect("retained session");
        stores.world_mut().open_out(StreamSlot::new(2), ".");
        stores
            .world_mut()
            .push_memory_terminal_line("must-remain")
            .expect("terminal line");
        stores
            .world_mut()
            .retarget_output_backend(&tex_state::World::real())
            .expect("real output backend");
        let error = stores
            .export_retained_effects()
            .expect_err("open is unavailable");
        let failed = error
            .stream_open_unavailable()
            .expect("typed failed target")
            .to_owned();

        assert!(matches!(
            crate::retry_unavailable_stream_open(&mut stores, &failed),
            Err(ExecError::Fatal(tex_command::FatalError::EmergencyStop {
                help: "job aborted, file error in nonstop mode"
            }))
        ));
        assert_eq!(
            stores
                .world_mut()
                .read_terminal_line()
                .expect("terminal remains readable")
                .as_deref(),
            Some("must-remain")
        );
        let terminal = String::from_utf8(
            stores
                .world()
                .memory_terminal_output()
                .unwrap_or_default()
                .to_vec(),
        )
        .expect("terminal output is utf-8");
        assert!(!terminal.contains("Please type another output file name"));
        assert!(!terminal.contains(": "));
    }
}

#[test]
fn out_what_closes_existing_slot_before_failed_replacement() {
    let mut stores = Universe::new();
    let slot = StreamSlot::new(2);
    stores.world_mut().open_out(slot, "existing.tex");
    stores.world_mut().deny_memory_output("blocked.tex");
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let ordinary = state_box(
        &mut stores,
        &[Node::Whatsit(Whatsit::OpenOut {
            slot,
            path: "blocked".to_owned(),
        })],
        false,
    );
    assert!(crate::assignments::test_stage_shipout_artifact(ordinary, &mut stores).is_err());
    assert!(matches!(
        stores.world().effect_records().get(1),
        Some(EffectRecord::StreamClose { slot: closed }) if *closed == slot
    ));
}

#[test]
fn out_what_preserves_leader_stream_suppression_before_open_retry() {
    let mut stores = Universe::new();
    stores.world_mut().deny_memory_output("suppressed.tex");
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
    assert!(
        stores.world().effect_records().is_empty(),
        "leader-contained stream whatsits must not probe, prompt, or open"
    );
}
