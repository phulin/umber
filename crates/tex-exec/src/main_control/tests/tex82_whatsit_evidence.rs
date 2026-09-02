use super::*;

use tex_out::{EffectSink, PageArtifact, PageEffect, PageNode};
use tex_state::node::{BoxNode, BoxNodeFields, Sign, Whatsit};
use tex_state::scaled::GlueSetRatio;

struct EmptyArtifactSourceResolver;

impl crate::output_provenance::ArtifactSourceResolver for EmptyArtifactSourceResolver {
    fn detach_artifact_source(
        &self,
        _origin: tex_state::token::OriginId,
    ) -> Option<tex_state::world::ArtifactSourceRecipe> {
        None
    }
}

fn with_observed_run<R>(
    source: &[u8],
    test: impl for<'id> FnOnce(
        &mut Universe<tex_state::GenerationBrand<'id>>,
        &MainControl<tex_state::GenerationBrand<'id>>,
        Vec<CommandObservation>,
    ) -> R,
) -> R {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut control = MainControl::tex82_initex(universe);
        let mut observations = ObservationRecorder::default();
        register_source(&mut control, source);
        run_to_end_observed(&mut control, universe, &mut observations);
        test(universe, &control, observations.0)
    })
}

fn last_artifact<G>(universe: &Universe<G>) -> PageArtifact {
    let committed = universe
        .world()
        .committed_artifacts()
        .last()
        .expect("shipout commits an artifact");
    PageArtifact::from_bytes(committed.bytes()).expect("committed artifact parses")
}

fn state_box<G>(
    context: &mut tex_state::CommandContext<'_, G>,
    children: &[Node],
    vertical: bool,
) -> Node {
    let children = context.publish_page_nodes(children.to_vec());
    let boxed = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1_000),
        height: Scaled::from_raw(100),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: tex_state::glue::Order::Normal,
        children,
    });
    if vertical {
        Node::VList(boxed)
    } else {
        Node::HList(boxed)
    }
}

fn base_whatsits<G>(context: &mut tex_state::CommandContext<'_, G>) -> Vec<Node> {
    let write = context
        .allocate_node_token_list(&[tex_state::token::TokenWord::pack(Token::Char {
            ch: 'w',
            cat: Catcode::Letter,
        })])
        .expect("test node token list");
    vec![
        Node::Whatsit(Whatsit::OpenOut {
            slot: tex_state::StreamSlot::new(0),
            path: "matrix.tex".into(),
        }),
        Node::Whatsit(Whatsit::DeferredWrite {
            sink: PrintSink::Stream(tex_state::StreamSlot::new(0)),
            tokens: write,
        }),
        Node::Whatsit(Whatsit::CloseOut {
            slot: Some(tex_state::StreamSlot::new(0)),
        }),
        Node::Whatsit(Whatsit::CloseOut { slot: None }),
        Node::Whatsit(Whatsit::Special {
            class: "dvi".into(),
            payload: b"special".to_vec(),
        }),
        Node::Whatsit(Whatsit::Language {
            language: 7,
            left_hyphen_min: 2,
            right_hyphen_min: 3,
        }),
    ]
}

#[test]
fn output_stream_lifecycle_projects_all_numbered_and_fallback_states() {
    // TeX82 §§1342--1343, 1378: 0..15 are paired file/open entries, while
    // normalized 16 and 17 are permanently closed print fallbacks. Cleanup
    // closes the sparse live set in slot order and repeated closes are inert.
    crate::test_harness::with_nonstop_universe(|initial| {
        for raw in 0..tex_state::world::STREAM_SLOT_COUNT as u8 {
            assert!(
                initial
                    .world()
                    .stream_bufs()
                    .write_stream_target(tex_state::StreamSlot::new(raw))
                    .is_none(),
                "numbered stream {raw} initializes closed"
            );
        }
    });
    assert!(!matches!(PrintSink::TerminalAndLog, PrintSink::Stream(_)));
    assert!(!matches!(PrintSink::Log, PrintSink::Stream(_)));

    let source = br"\immediate\openout15=last
        \immediate\openout2=already-closed
        \immediate\closeout2\immediate\closeout2
        \immediate\openout0=first
        \immediate\write16{terminal fallback}
        \immediate\write-1{log fallback}
        \immediate\closeout16\immediate\closeout17\end";
    with_observed_run(source, |universe, _, observations| {
        assert!(matches!(
            universe.world().effect_records(),
        [
            tex_state::EffectRecord::StreamOpen { slot: last, target: last_target },
            tex_state::EffectRecord::StreamOpen { slot: closed, target: closed_target },
            tex_state::EffectRecord::StreamClose { slot: explicitly_closed },
            tex_state::EffectRecord::StreamOpen { slot: first, target: first_target },
            tex_state::EffectRecord::StreamWrite { sink: PrintSink::TerminalAndLog, text: terminal },
            tex_state::EffectRecord::StreamWrite { sink: PrintSink::Log, text: log },
            tex_state::EffectRecord::StreamClose { slot: cleanup_first },
            tex_state::EffectRecord::StreamClose { slot: cleanup_last },
        ] if *last == tex_state::StreamSlot::new(15)
            && last_target.path() == std::path::Path::new("last.tex")
            && *closed == tex_state::StreamSlot::new(2)
            && closed_target.path() == std::path::Path::new("already-closed.tex")
            && *explicitly_closed == tex_state::StreamSlot::new(2)
            && *first == tex_state::StreamSlot::new(0)
            && first_target.path() == std::path::Path::new("first.tex")
            && terminal == "terminal fallback\n"
            && log == "log fallback\n"
            && *cleanup_first == tex_state::StreamSlot::new(0)
            && *cleanup_last == tex_state::StreamSlot::new(15)
        ));
        let lifecycle: Vec<_> = observations
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Effect(effect)
                    if matches!(
                        effect.kind,
                        ObservationEffectKind::Open
                            | ObservationEffectKind::Close
                            | ObservationEffectKind::Terminate
                    ) =>
                {
                    Some((effect.kind, effect.channel.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            lifecycle,
            [
                (ObservationEffectKind::Open, "stream:15"),
                (ObservationEffectKind::Open, "stream:2"),
                (ObservationEffectKind::Close, "stream:2"),
                (ObservationEffectKind::Open, "stream:0"),
                (ObservationEffectKind::Close, "stream:0"),
                (ObservationEffectKind::Close, "stream:15"),
                (ObservationEffectKind::Terminate, "engine"),
            ]
        );
        for raw in 0..tex_state::world::STREAM_SLOT_COUNT as u8 {
            assert!(
                universe
                    .world()
                    .stream_bufs()
                    .write_stream_target(tex_state::StreamSlot::new(raw))
                    .is_none(),
                "cleanup closes numbered stream {raw}"
            );
        }
    });

    let mut all_source = String::from("\\nonstopmode");
    for raw in 0..tex_state::world::STREAM_SLOT_COUNT as u8 {
        all_source.push_str(&format!("\\immediate\\openout{raw}=slot-{raw} "));
    }
    for raw in (1..tex_state::world::STREAM_SLOT_COUNT as u8).step_by(2) {
        all_source.push_str(&format!("\\immediate\\closeout{raw}"));
    }
    all_source.push_str("\\end");
    with_observed_run(all_source.as_bytes(), |universe, _, _| {
        let transitions: Vec<_> = universe
            .world()
            .effect_records()
            .iter()
            .filter_map(|effect| match effect {
                tex_state::EffectRecord::StreamOpen { slot, target } => Some((
                    "open",
                    slot.raw(),
                    target.path().to_string_lossy().into_owned(),
                )),
                tex_state::EffectRecord::StreamClose { slot } => {
                    Some(("close", slot.raw(), String::new()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(transitions.len(), 32);
        for raw in 0..tex_state::world::STREAM_SLOT_COUNT as u8 {
            assert_eq!(
                transitions[usize::from(raw)],
                ("open", raw, format!("slot-{raw}.tex"))
            );
        }
        let explicit_closes: Vec<_> = transitions[16..24]
            .iter()
            .map(|transition| transition.1)
            .collect();
        let cleanup_closes: Vec<_> = transitions[24..]
            .iter()
            .map(|transition| transition.1)
            .collect();
        assert_eq!(explicit_closes, [1, 3, 5, 7, 9, 11, 13, 15]);
        assert_eq!(cleanup_closes, [0, 2, 4, 6, 8, 10, 12, 14]);
    });
}

#[test]
fn extension_dispatch_executes_every_selector_in_every_tex82_mode() {
    // TeX82 §§1344, 1347 route all six extension selectors through
    // do_extension independent of the current mode. The subtype branches,
    // rather than a command-identity assertion, prove that dispatch: four
    // ordered whatsit effects, immediate lookahead backup, and setlanguage's
    // legal-horizontal versus illegal-mode behavior all execute.
    const BODY: &str = "\\openout0=mode \\write16{w}\\closeout0\\special{s}\\immediate\\relax\\setlanguage\\relax\\global\\advance\\count0 by1";
    let cases = [
        (
            "outer vertical",
            format!("\\nonstopmode{BODY}\\hrule\\end"),
            false,
        ),
        (
            "internal vertical",
            format!("\\nonstopmode\\setbox0=\\vbox{{{BODY}\\hrule}}\\shipout\\box0\\end"),
            false,
        ),
        (
            "outer horizontal",
            format!("\\nonstopmode\\noindent{BODY}x\\par\\end"),
            true,
        ),
        (
            "restricted horizontal",
            format!("\\nonstopmode\\setbox0=\\hbox{{{BODY}x}}\\shipout\\box0\\end"),
            true,
        ),
        (
            "inline math",
            format!("\\nonstopmode\\setbox0=\\hbox{{$ {BODY} $}}\\shipout\\box0\\end"),
            false,
        ),
        (
            "display math",
            format!("\\nonstopmode$$ {BODY} $$\\end"),
            false,
        ),
    ];

    for (mode, source, setlanguage_is_legal) in cases {
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut control = MainControl::tex82_initex(universe);
            let mut recorder = ObservationRecorder::default();
            register_source(&mut control, source.as_bytes());
            while crate::test_harness::with_admitted(universe, |context| {
                context.count(0).expect("count register")
            }) == 0
            {
                assert_eq!(
                    control
                        .step_with_observer(universe, &mut recorder)
                        .expect("mode matrix executes"),
                    MainControlStep::Continue,
                    "{mode}: all extensions execute before input ends"
                );
            }
            let live_nodes = if mode == "outer vertical" {
                crate::test_harness::with_admitted(universe, |context| {
                    context.page_contributions().iter().cloned().collect()
                })
            } else {
                mode_vec(&control, universe)
            };
            let live_whatsits: Vec<_> = live_nodes
                .iter()
                .filter(|node| matches!(node, Node::Whatsit(_)))
                .cloned()
                .collect();
            assert!(
                matches!(
                    live_whatsits.as_slice(),
                    [
                        Node::Whatsit(Whatsit::OpenOut { slot, path }),
                        Node::Whatsit(Whatsit::DeferredWrite { sink: PrintSink::TerminalAndLog, .. }),
                        Node::Whatsit(Whatsit::CloseOut { slot: Some(close) }),
                        Node::Whatsit(Whatsit::Special { class, payload }),
                        ..
                    ] if *slot == tex_state::StreamSlot::new(0)
                        && path == "mode"
                        && *close == tex_state::StreamSlot::new(0)
                        && class == "dvi"
                        && payload == b"s"
                ),
                "{mode}: typed subtype state: {live_nodes:#?}"
            );
            assert_eq!(
                live_whatsits
                    .iter()
                    .filter(|node| matches!(
                        node,
                        Node::Whatsit(Whatsit::Language { language: 0, .. })
                    ))
                    .count(),
                usize::from(setlanguage_is_legal),
                "{mode}: setlanguage appends only in horizontal mode"
            );
            let selected: Vec<_> = recorder
                .0
                .iter()
                .filter_map(|observation| match observation {
                    CommandObservation::Command(command)
                        if command.boundary == CommandDeliveryBoundary::Raw
                            && command.command == "extension"
                            && command.provenance.source_range.is_some() =>
                    {
                        command.command_operand
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                selected,
                [0, 1, 2, 3, 4, 5],
                "{mode}: each selector reaches MainControl exactly once before output replay"
            );
            run_to_end_observed(&mut control, universe, &mut recorder);
            assert_eq!(
                crate::test_harness::with_admitted(universe, |context| {
                    context.count(0).expect("count register")
                }),
                1,
                "{mode}: immediate backed up relax"
            );
            let effects: Vec<_> = universe
                .world()
                .committed_artifacts()
                .iter()
                .flat_map(|record| {
                    PageArtifact::from_bytes(record.bytes())
                        .expect("mode artifact parses")
                        .effects
                        .to_vec()
                })
                .filter(|effect| !matches!(effect, PageEffect::Write { text, .. } if text != "w\n"))
                .collect();
            if !mode.contains("math") {
                assert_eq!(
                    effects,
                    [
                        PageEffect::OpenOut {
                            stream: 0,
                            path: "mode.tex".into(),
                        },
                        PageEffect::Write {
                            sink: EffectSink::TerminalAndLog,
                            text: "w\n".into(),
                        },
                        PageEffect::CloseOut { stream: 0 },
                        PageEffect::Special {
                            class: "dvi".into(),
                            payload: b"s".to_vec(),
                        },
                    ],
                    "{mode}: subtype branches retain ordered effects: {:?}",
                    terminal_text(universe)
                );
            }
            assert_eq!(
                terminal_text(universe).contains("You can't use `\\setlanguage'"),
                !setlanguage_is_legal,
                "{mode}: legality is decided inside setlanguage's branch"
            );
        });
    }
}

#[test]
fn base_whatsit_construction_projects_fields_display_size_and_ownership() {
    // TeX82 §§1349--1361: every base subtype is complete before append;
    // writes retain unexpanded tokens, specials retain scan-time expansion,
    // copies share immutable payload ownership, and base whatsits are null
    // dimensional material in both packing directions.
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut control = MainControl::tex82_initex(universe);
        register_source(
        &mut control,
        br"\nonstopmode\showboxbreadth=100\showboxdepth=100
           \def\payload{early}
           \setbox0=\hbox{\openout15=owned\write-1{\payload}\write16{x}\write17{y}\write99{z}\closeout0\closeout16\special{\payload}\setlanguage7}
           \def\payload{late}\showbox0\setbox1=\copy0
           \setbox2=\vbox{\hbox{\copy0}}\setbox0=\hbox{}\end",
        );
        run_to_end(&mut control, universe);

        let nodes = box_child_nodes(universe, 1);
        let payload_matches = crate::test_harness::with_admitted(universe, |context| {
            let payload_symbol = context.symbol("payload").expect("payload");
            matches!(
                nodes.as_slice(),
                [
                    Node::Whatsit(Whatsit::OpenOut { slot, path }),
                    Node::Whatsit(Whatsit::DeferredWrite { sink: PrintSink::Log, tokens }),
                    Node::Whatsit(Whatsit::DeferredWrite { sink: PrintSink::TerminalAndLog, .. }),
                    Node::Whatsit(Whatsit::DeferredWrite { sink: PrintSink::TerminalAndLog, .. }),
                    Node::Whatsit(Whatsit::DeferredWrite { sink: PrintSink::TerminalAndLog, .. }),
                    Node::Whatsit(Whatsit::CloseOut { slot: Some(close) }),
                    Node::Whatsit(Whatsit::CloseOut { slot: None }),
                    Node::Whatsit(Whatsit::Special { class, payload }),
                    Node::Whatsit(Whatsit::Language { language: 7, .. }),
                ] if *slot == tex_state::StreamSlot::new(15)
                    && path == "owned"
                    && *close == tex_state::StreamSlot::new(0)
                    && class == "dvi"
                    && payload == b"early"
                    && context.node_token_words(*tokens) == Some(&[
                        tex_state::token::TokenWord::pack(Token::Cs(payload_symbol))
                    ])
            )
        });
        assert!(payload_matches, "constructed nodes: {nodes:#?}");
        assert!(box_child_nodes(universe, 0).is_empty());
        crate::test_harness::with_admitted(universe, |context| {
            assert!(
                context.copy_box_to_page(2).is_some(),
                "nested copy survives original replacement"
            );
            for (register, vertical) in [(1, false), (2, true)] {
                let list = context.copy_box_to_page(register).expect("box exists");
                let root = context
                    .page_node_list(list)
                    .expect("box list belongs to the page arena")
                    .nodes()
                    .first()
                    .expect("box has a root");
                let boxed = match (vertical, root) {
                    (false, tex_state::NodeView::HList(boxed))
                    | (true, tex_state::NodeView::VList(boxed)) => boxed,
                    (_, other) => {
                        panic!("box {register} has the expected orientation: {other:?}")
                    }
                };
                assert_eq!(
                    (boxed.width.raw(), boxed.height.raw(), boxed.depth.raw()),
                    (0, 0, 0),
                    "base whatsits have zero dimensions in register {register}"
                );
            }
        });
        let shown = terminal_text(universe);
        for row in [
            "\\openout15=owned",
            "\\write-{\\payload }",
            "\\write*{x}",
            "\\closeout0",
            "\\closeout*",
            "\\special{early}",
            "\\setlanguage7 (hyphenmin",
        ] {
            assert!(
                shown.contains(row),
                "missing display row {row:?}: {shown:?}"
            );
        }
    });

    crate::test_harness::with_nonstop_plain_universe(|malformed| {
        let mut malformed_control = MainControl::tex82_initex(malformed);
        register_source(
            &mut malformed_control,
            br"\setbox0=\hbox{\write16}}\count0=41\end",
        );
        run_to_end(&mut malformed_control, malformed);
        assert_eq!(
            crate::test_harness::with_admitted(malformed, |context| {
                context.count(0).expect("count register")
            }),
            41,
            "scanner recovery preserves following input"
        );
        assert!(terminal_text(malformed).contains("Missing { inserted"));
        assert!(matches!(
            box_child_nodes(malformed, 0).as_slice(),
            [Node::Whatsit(Whatsit::DeferredWrite { .. })]
        ));
    });
}

#[test]
fn base_whatsits_are_passive_for_page_and_vertical_break_visits() {
    // TeX82 §§1364--1365: all base subtypes pass through page construction
    // without freezing an empty page or performing effects, and vert_break
    // neither measures them nor chooses them as breakpoints.
    crate::test_harness::with_nonstop_universe(|universe| {
        crate::test_harness::with_admitted(universe, |context| {
            let whatsits = base_whatsits(context);
            for node in &whatsits {
                context.append_page_contribution(node.clone());
            }
            crate::page_builder::build_page_without_error_context(context)
                .expect("contentless whatsit page visits");
            assert_eq!(
                context.page_contents(),
                tex_state::page::PageContents::Empty
            );
            assert_eq!(
                context.current_page_nodes().cloned().collect::<Vec<_>>(),
                whatsits
            );

            let bare = vec![Node::Penalty(tex_state::page::EJECT_PENALTY)];
            let mut decorated = whatsits.clone();
            decorated.extend(bare.clone());
            let typeset = crate::typeset_context::TypesetContext::new(context);
            let bare_break = tex_typeset::vert_break(
                &typeset,
                tex_state::node_arena::NodeCursor::owned(&bare),
                Scaled::from_raw(0),
                Scaled::from_raw(0),
            )
            .expect("bare vertical break");
            let decorated_break = tex_typeset::vert_break(
                &typeset,
                tex_state::node_arena::NodeCursor::owned(&decorated),
                Scaled::from_raw(0),
                Scaled::from_raw(0),
            )
            .expect("decorated vertical break");
            assert_eq!(decorated_break.break_index, Some(whatsits.len()));
            assert_eq!(
                decorated_break.best_height_plus_depth,
                bare_break.best_height_plus_depth
            );
            assert_eq!(&decorated[..whatsits.len()], whatsits);
        });
        assert!(universe.world().effect_records().is_empty());
    });
}

#[test]
fn hlist_and_vlist_visit_each_base_whatsit_once_in_position() {
    // TeX82 §§1366--1367: hlist_out and vlist_out reach the same base
    // payloads once, in list position. Language and fallback close nodes are
    // passive; open/write/numbered-close/special retain ordered ownership.
    for vertical in [false, true] {
        use crate::shipout::direct::{BaseWhatsitVisit, BaseWhatsitVisitKind};

        crate::test_harness::with_nonstop_plain_universe(|trace_universe| {
            let trace_root = crate::test_harness::with_admitted(trace_universe, |context| {
                let trace_nodes = base_whatsits(context);
                state_box(context, &trace_nodes, vertical)
            });
            let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut write = |_: &mut Universe<_>,
                             _: &mut tex_state::diagnostic::DiagnosticEffects,
                             _: PrintSink,
                             _: tex_state::ShipoutTokenSource<_>| {
                Ok(crate::shipout::ExpandedWrite::transactional_encoded(
                    "w\n".into(),
                    b"w\n".to_vec(),
                ))
            };
            let mut unexpected_replay =
                |_: &mut Universe<_>,
                 _: &mut tex_state::diagnostic::DiagnosticEffects,
                 _: crate::shipout::ReplayTextKind,
                 _: tex_state::ShipoutTokenSource<_>| {
                    panic!("the typed visit trace does not replay text")
                };
            let staged = crate::shipout::direct::stage_shipout(
                crate::shipout::direct::ShipoutRoot::Page(trace_root),
                crate::shipout::ShipoutOrigin {
                    output_open_context: String::new(),
                    announce_openout: false,
                },
                0,
                trace_universe,
                &mut diagnostic_effects,
                &EmptyArtifactSourceResolver,
                tex_state::ProvenanceDemand::DIAGNOSTICS,
                0,
                true,
                &mut write,
                &mut unexpected_replay,
            )
            .expect("base-whatsit visit trace stages");
            assert_eq!(
                staged.base_whatsit_visits,
                [
                    BaseWhatsitVisit {
                        in_hlist: !vertical,
                        position: 0,
                        kind: BaseWhatsitVisitKind::OpenOut
                    },
                    BaseWhatsitVisit {
                        in_hlist: !vertical,
                        position: 1,
                        kind: BaseWhatsitVisitKind::DeferredWrite
                    },
                    BaseWhatsitVisit {
                        in_hlist: !vertical,
                        position: 2,
                        kind: BaseWhatsitVisitKind::NumberedCloseOut
                    },
                    BaseWhatsitVisit {
                        in_hlist: !vertical,
                        position: 3,
                        kind: BaseWhatsitVisitKind::FallbackCloseOut
                    },
                    BaseWhatsitVisit {
                        in_hlist: !vertical,
                        position: 4,
                        kind: BaseWhatsitVisitKind::Special
                    },
                    BaseWhatsitVisit {
                        in_hlist: !vertical,
                        position: 5,
                        kind: BaseWhatsitVisitKind::Language
                    },
                ],
                "both effectful and passive base whatsits are visited exactly once"
            );
        });

        crate::test_harness::with_nonstop_plain_universe(|universe| {
            crate::test_harness::with_admitted(universe, |context| {
                let nodes = base_whatsits(context);
                let root = state_box(context, &nodes, vertical);
                let register = context.publish_page_nodes(vec![root]);
                context
                    .assign_page_box(0, Some(register), tex_state::AssignmentScope::Local)
                    .expect("shipout test box assignment");
            });
            let mut control = MainControl::tex82_initex(universe);
            register_source(&mut control, br"\shipout\box0\end");
            run_to_end(&mut control, universe);
            let artifact = last_artifact(universe);
            assert_eq!(
                artifact.effects,
                [
                    PageEffect::OpenOut {
                        stream: 0,
                        path: "matrix.tex".into(),
                    },
                    PageEffect::Write {
                        sink: EffectSink::Stream(0),
                        text: "w\n".into(),
                    },
                    PageEffect::CloseOut { stream: 0 },
                    PageEffect::Special {
                        class: "dvi".into(),
                        payload: b"special".to_vec(),
                    },
                ]
            );
            let children = match &artifact.root {
                PageNode::HList(boxed) | PageNode::VList(boxed) => &boxed.children,
                other => panic!("root remains a list: {other:?}"),
            };
            let anchors: Vec<_> = children
                .iter()
                .enumerate()
                .filter_map(|(position, node)| match node {
                    PageNode::WhatsitAnchor { effect_index } => Some((position, *effect_index)),
                    _ => None,
                })
                .collect();
            assert_eq!(anchors, [(0, 0), (1, 1), (2, 2), (3, 3)]);
        });
    }
}

#[test]
fn deferred_write_projects_stopper_selector_mode_stream_and_recovery_matrix() {
    // TeX82 §§1369--1372: each deferred replay owns and retires its synthetic
    // write input, expands at traversal, restores the surrounding command
    // mode, applies the live newlinechar and selector, and preserves source
    // input after balanced, nested, and unbalanced token lists.
    let source = br"\nonstopmode\newlinechar=`|
        \immediate\openout0=numbered
        \def\same{stable}\immediate\write0{\same|line}
        \setbox0=\hbox{\write0{\same|line}\write16{terminal}\write-1{log}\write17{log17}}
        \shipout\copy0\immediate\closeout0
        \def\same{changed}\shipout\box0
        \def\nested{{inner}}\shipout\hbox{\write16{outer\nested tail}}
        \def\extra{\iffalse{\else}\fi}
        \shipout\hbox{\write16{before\extra after}}
        \message{selector-restored}\setbox9=\hbox{mode-restored}
        \global\count0=73\end";
    with_observed_run(source, |universe, control, observations| {
        assert_eq!(
            crate::test_harness::with_admitted(universe, |context| {
                context.count(0).expect("count register")
            }),
            73,
            "following source survives write recovery: {:?}",
            terminal_text(universe)
        );
        assert_eq!(control.current_mode(), Mode::Vertical);
        assert_eq!(
            universe.world().memory_output("numbered.tex"),
            Some(&b"stable\nline\nstable\nline\n"[..])
        );

        let artifacts: Vec<_> = universe
            .world()
            .committed_artifacts()
            .iter()
            .map(|record| PageArtifact::from_bytes(record.bytes()).expect("artifact parses"))
            .collect();
        let writes: Vec<_> = artifacts
            .iter()
            .flat_map(|artifact| artifact.effects.iter())
            .filter_map(|effect| match effect {
                PageEffect::Write { sink, text } => Some((*sink, text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            writes,
            [
                (EffectSink::TerminalAndLog, "\n".into()),
                (EffectSink::Stream(0), "stable\nline\n".into()),
                (EffectSink::Stream(0), "stable\nline\n".into()),
                (EffectSink::TerminalAndLog, "terminal\n".into()),
                (EffectSink::Log, "log\n".into()),
                (EffectSink::TerminalAndLog, "log17\n".into()),
                (EffectSink::TerminalAndLog, "changed\nline\n".into()),
                (EffectSink::TerminalAndLog, "terminal\n".into()),
                (EffectSink::Log, "log\n".into()),
                (EffectSink::TerminalAndLog, "log17\n".into()),
                (EffectSink::TerminalAndLog, "outer{inner}tail\n".into()),
                (EffectSink::TerminalAndLog, "before\n".into()),
            ],
            "copied first shipout uses stable meanings; taken second box routes its closed stream away from the file"
        );
        let write_inputs: Vec<_> = observations
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Input(input) if input.reason == InputReason::Write => {
                    Some(input.transition)
                }
                _ => None,
            })
            .collect();
        assert!(!write_inputs.is_empty());
        assert_eq!(
            write_inputs
                .iter()
                .filter(|event| **event == InputTransition::Push)
                .count(),
            write_inputs
                .iter()
                .filter(|event| **event == InputTransition::Retire)
                .count(),
            "every synthetic write input, including its stopper, retires"
        );
        let stopper_installs: Vec<_> = observations
            .windows(2)
            .enumerate()
            .filter_map(|(index, pair)| match pair {
                [
                    CommandObservation::Input(input),
                    CommandObservation::Recovery(recovery),
                ] if input.reason == InputReason::Recovery
                    && input.transition == InputTransition::Recovery
                    && recovery.kind == RecoveryKind::InsertedToken
                    && matches!(
                        recovery.tokens.as_slice(),
                        [ObservedToken::Character {
                            character: '}',
                            catcode: Catcode::EndGroup
                        }]
                    ) =>
                {
                    Some((index, input.level))
                }
                _ => None,
            })
            .collect();
        assert!(!stopper_installs.is_empty());
        for (install_index, level) in &stopper_installs {
            let next_same_level = observations[install_index + 2..].iter().find_map(
                |observation| match observation {
                    CommandObservation::Input(input)
                        if input.reason == InputReason::Recovery && input.level == *level =>
                    {
                        Some(input.transition)
                    }
                    _ => None,
                },
            );
            assert_eq!(
                next_same_level,
                Some(InputTransition::Retire),
                "ReplayTrace::Inserted stopper install at level {level} has an exact-ID retirement"
            );
        }
        assert_eq!(
            stopper_installs.len(),
            write_inputs
                .iter()
                .filter(|transition| **transition == InputTransition::Push)
                .count(),
            "every stored write replay has one inserted-stopper install/retire pair"
        );
        assert!(terminal_text(universe).contains("Unbalanced write command"));
        assert_eq!(
            terminal_text(universe).matches("selector-restored").count(),
            1
        );
        assert!(
            crate::test_harness::with_admitted(universe, |context| {
                context.copy_box_to_page(9).is_some()
            }),
            "following box scan proves mode restoration"
        );
    });

    let trace = with_observed_run(
        br"\nonstopmode\tracingcommands=2\tracingonline=1
           \shipout\hbox{\write16{\romannumeral0\relax}}\message{restored}\end",
        |traced, _, _| terminal_text(traced),
    );
    let no_mode = trace
        .find("{no mode: \\romannumeral}")
        .unwrap_or_else(|| panic!("deferred expansion observes write-time no mode: {trace}"));
    let restored = no_mode
        + trace[no_mode..]
            .find("{vertical mode: \\message}")
            .unwrap_or_else(|| {
                panic!("the command after write replay observes restored vmode: {trace}")
            });
    assert!(
        restored > no_mode,
        "mode restoration follows the no-mode replay boundary"
    );

    let immediate_text = with_observed_run(
        br"\def\same{stable}\newlinechar=`|\immediate\openout0=parity\immediate\write0{\same|line}\immediate\closeout0\end",
        |immediate, _, _| {
            immediate
                .world()
                .effect_records()
                .iter()
                .find_map(|effect| match effect {
                    tex_state::EffectRecord::StreamWrite { text, .. }
                        if text.contains("stable") =>
                    {
                        Some(text.clone())
                    }
                    _ => None,
                })
                .expect("immediate write effect")
        },
    );
    let deferred_text = with_observed_run(
        br"\def\same{stable}\newlinechar=`|\shipout\hbox{\openout0=parity\write0{\same|line}\closeout0}\end",
        |deferred, _, _| {
            last_artifact(deferred)
                .effects
                .iter()
                .find_map(|effect| match effect {
                    PageEffect::Write { text, .. } if text.contains("stable") => {
                        Some(text.clone())
                    }
                    _ => None,
                })
                .expect("deferred write effect")
        },
    );
    assert_eq!(
        immediate_text, deferred_text,
        "unchanged meanings and newlinechar give immediate/deferred byte parity"
    );
}

#[test]
fn exact_byte_profile_serializes_immediate_and_deferred_write_characters_once() {
    // TeX82 §§58, 262, and 1370: exact-byte input tokens remain internal
    // character codes through token_show; print_char applies xchr exactly
    // once when the open write_file selector commits them.
    let mut source = br"\immediate\openout0=bytes
        \immediate\write0{Magalh"
        .to_vec();
    source.extend_from_slice(&[0xc3, 0xa3]);
    source.extend_from_slice(
        br"es}
        \shipout\hbox{\write0{Magalh",
    );
    source.extend_from_slice(&[0xc3, 0xa3]);
    source.extend_from_slice(
        br"es}}
        \immediate\closeout0\end",
    );

    with_observed_run(&source, |universe, _, observations| {
        assert_eq!(
            universe.world().memory_output("bytes.tex"),
            Some(&b"Magalh\xc3\xa3es\nMagalh\xc3\xa3es\n"[..])
        );
        let byte_pairs = observations
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Effect(effect)
                    if effect.kind == ObservationEffectKind::Write =>
                {
                    match &effect.value {
                        ObservationValue::Tokens(tokens) => Some(tokens.as_slice()),
                        _ => None,
                    }
                }
                _ => None,
            })
            .filter(|tokens| {
                tokens.windows(2).any(|pair| {
                    matches!(
                        pair,
                        [
                            ObservedToken::Character {
                                character: 'Ã', ..
                            },
                            ObservedToken::Character {
                                character: '£', ..
                            }
                        ]
                    )
                })
            })
            .count();
        assert_eq!(
            byte_pairs, 2,
            "immediate and deferred expansion retain exact byte-domain token codes"
        );
    });
}

#[test]
fn deferred_write_publishes_trace_and_diagnostic_before_payload() {
    // TeX82 §§1370/418: `write_out` expands its token list with `mode=0`,
    // so the macro trace and improper `\spacefactor` diagnostic are emitted
    // before `token_show(def_ref)` writes the recovered zero payload.
    let transcript = with_observed_run(
        br"\nonstopmode\tracingonline=1\tracingmacros=2
           x\write16{\the\spacefactor}\par\vfill\penalty-10000\end",
        |universe, _, _| terminal_text(universe),
    );
    let trace = transcript
        .find("\\write->\\the \\spacefactor")
        .unwrap_or_else(|| panic!("deferred-write trace is visible: {transcript:?}"));
    let diagnostic = transcript
        .find("Improper \\spacefactor")
        .unwrap_or_else(|| panic!("deferred-write diagnostic is visible: {transcript:?}"));
    let payload = transcript
        .rfind("\n0\n")
        .unwrap_or_else(|| panic!("deferred-write payload is visible: {transcript:?}"));

    assert!(trace < diagnostic, "{transcript:?}");
    assert!(diagnostic < payload, "{transcript:?}");
}
