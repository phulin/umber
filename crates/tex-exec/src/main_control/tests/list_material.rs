//! Box, list, and material ownership with typed node projections.

use super::*;

#[test]
fn private_box_construction_retains_only_committed_lists() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\setbox0=\hbox{\kern1pt}");

        run_to_end(&mut control, stores);

        let boxed = stores
            .copy_box_to_page(0)
            .expect("completed box is committed");
        let children = match stores
            .page_node_list(boxed)
            .expect("copied box belongs to the page arena")
            .nodes()
            .first()
        {
            Some(tex_state::NodeView::HList(node)) => stores
                .page_node_list(node.children)
                .expect("hbox children belong to the page arena"),
            other => panic!("expected committed hbox, got {other:?}"),
        };
        assert!(matches!(
            children.nodes().first(),
            Some(tex_state::NodeView::Kern { .. })
        ));
    });
}
#[test]
fn box_save_stack_projection_distinguishes_scan_spec_callers() {
    // TeX82 §§645/1083's ordinary boxes preserve box_context plus the
    // packing pair. Section 1167's vcenter omits the context, and §1099's
    // insertion opens its group without calling scan_spec at all.
    for kind in [
        ReplayBoxKind::HBox,
        ReplayBoxKind::VBox,
        ReplayBoxKind::VTop,
    ] {
        assert_eq!(kind.save_stack_spec_words(), 3);
    }
    assert_eq!(ReplayBoxKind::VCenter.save_stack_spec_words(), 2);
    assert_eq!(ReplayBoxKind::Insert(7, false).save_stack_spec_words(), 0);
}
#[test]
fn named_output_token_list_trace_uses_live_escape_character() {
    // TeX82 §§63/323: `begin_token_list(output_routine,output_text)` names
    // `output` through `print_esc`, so an out-of-range escape character emits
    // no prefix.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        b"\\tracingmacros=2\\tracingonline=1\\maxdeadcycles=1\\output={\\dimen0=1pt}\\escapechar=256\\topskip=0pt\\setbox0=\\vbox to1pt{}\\copy0\\penalty-10000\\end",
    );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        assert!(terminal.contains("output->{dimen 0=1pt}"), "{terminal:?}");
        assert!(!terminal.contains("\\output->"), "{terminal:?}");
    });
}
#[test]
fn vsplit_kernel_separates_result_remainder_and_split_marks() {
    // TeX82 §§977--979: the chosen prefix becomes a separately packed box,
    // the source register is replaced by its pruned remainder, and the split
    // marks describe only the extracted prefix.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\setbox0=\vbox{\mark{first}\hrule height10pt\penalty-10000\mark{second}\hrule height10pt}
           \setbox1=\vsplit0 to10pt\end",
    );

        run_to_end(&mut control, stores);

        let split = stores
            .copy_box_to_page(1)
            .expect("split prefix is assigned");
        let remainder = stores
            .copy_box_to_page(0)
            .expect("split remainder replaces source");
        assert_ne!(
            split, remainder,
            "prefix and remainder have distinct ownership"
        );
        assert!(matches!(
            first_published_node(stores, split),
            Some(Node::VList(_))
        ));
        assert!(matches!(
            first_published_node(stores, remainder),
            Some(Node::VList(_))
        ));
        for mark in [PageMark::SplitFirst, PageMark::SplitBot] {
            let tokens = admitted!(stores, |context| {
                let key = context.page_mark(mark);
                context
                    .node_token_words(key)
                    .expect("live page mark")
                    .to_vec()
            });
            assert_eq!(
                tokens,
                [
                    Token::Char {
                        ch: 'f',
                        cat: Catcode::Letter,
                    },
                    Token::Char {
                        ch: 'i',
                        cat: Catcode::Letter,
                    },
                    Token::Char {
                        ch: 'r',
                        cat: Catcode::Letter,
                    },
                    Token::Char {
                        ch: 's',
                        cat: Catcode::Letter,
                    },
                    Token::Char {
                        ch: 't',
                        cat: Catcode::Letter,
                    }
                ]
                .map(tex_state::token::TokenWord::pack)
            );
        }
    });
}
#[test]
fn direct_material_appends_typed_nodes_in_source_order() {
    // TeX82 §§1055--1061: each completed typed operand is appended exactly
    // once and preserves its distinct node kind and numeric value.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\hbox{\kern1pt\hskip2pt\vrule width3pt height4pt depth5pt}\end",
        );

        run_to_end(&mut control, stores);

        let nodes = box_child_nodes(stores, 0);
        let [
            Node::Kern { amount, kind },
            Node::Glue { spec, .. },
            Node::Rule {
                width,
                height,
                depth,
            },
        ] = nodes.as_slice()
        else {
            panic!("direct material remains in source order: {nodes:?}");
        };
        assert_eq!(*kind, tex_state::node::KernKind::Explicit);
        assert_eq!(amount.raw(), Scaled::UNITY);
        assert_eq!(spec.width.raw(), 2 * Scaled::UNITY);
        assert_eq!(width.map(Scaled::raw), Some(3 * Scaled::UNITY));
        assert_eq!(height.map(Scaled::raw), Some(4 * Scaled::UNITY));
        assert_eq!(depth.map(Scaled::raw), Some(5 * Scaled::UNITY));
    });
}
#[test]
fn destructive_unbox_transfers_nested_structural_children() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\hbox{\hbox{\kern1pt}}\setbox1=\vbox{\vbox{\kern2pt}}",
        );
        run_to_end(&mut control, stores);

        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox2=\hbox{\unhbox0}\setbox3=\vbox{\unvbox1}",
        );
        run_to_end(&mut control, stores);

        assert!(stores.copy_box_to_page(0).is_none());
        assert!(stores.copy_box_to_page(1).is_none());
        assert!(stores.copy_box_to_page(2).is_some());
        assert!(stores.copy_box_to_page(3).is_some());
    });
}
#[test]
fn unvbox_splices_vertical_nodes_without_inserting_baseline_glue() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\vsize=1000pt \setbox0=\vbox{\hrule\hbox{}}\unvbox0",
        );
        run_to_end(&mut control, stores);

        assert!(
            !admitted!(stores, |context| context
                .current_page_nodes()
                .cloned()
                .collect::<Vec<_>>())
            .iter()
            .any(|node| matches!(
                node,
                tex_state::node::Node::Glue {
                    kind: tex_state::node::GlueKind::BaselineSkip,
                    ..
                }
            ))
        );
    });
}
#[test]
fn vbox_sets_overfull_badness_when_the_box_cannot_shrink() {
    // TeX82 §§668/674 initializes and updates `last_badness` during
    // vertical packing; §§422--424 exposes the resulting value as `\badness`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\vbox to10pt{\hrule height20pt}\count0=\badness",
        );
        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            tex_typeset::OVERFULL_BADNESS
        );
    });
}
#[test]
fn etex_lastnodetype_reads_each_live_mode_tail_without_mutation() {
    // e-TeX 2.6 `etex.ch` [26.424]: `find_effective_tail` returns -1 for an
    // empty list, otherwise the e-TRIP node code of the real current tail.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\xdef\outerempty{\the\lastnodetype}
          \hbox{\xdef\hempty{\the\lastnodetype}}
          \hbox{\vrule\xdef\hrule{\the\lastnodetype}}
          \hbox{\kern1pt\xdef\hkern{\the\lastnodetype}}
          \vbox{\hbox{}\xdef\vboxnode{\the\lastnodetype}}
          $\mathord{1}\xdef\mathnode{\the\lastnodetype}$
          \end",
        );

        run_to_end(&mut control, stores);

        for (name, expected) in [
            ("outerempty", "-1"),
            ("hempty", "-1"),
            ("hrule", "3"),
            ("hkern", "12"),
            ("vboxnode", "1"),
            ("mathnode", "15"),
        ] {
            assert!(stores.intern(name).is_ok(), "missing probe macro {name}");
            assert_eq!(macro_character_text(stores, name), expected, "{name}");
        }
    });
}
#[test]
fn etex_lastnodetype_covers_every_node_code() {
    // e-TeX 2.6 `etex.ch` block 99 maps the complete 0..=15 node-type
    // interval.  Each enquiry is made while its node is still the live tail;
    // the alignment row is observed from `\noalign`, where it is an unset
    // node until `fin_align` resolves it.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\font\f=cmr10 \f
          \hbox{x\xdef\nzero{\the\lastnodetype}}
          \hbox{\hbox{}\xdef\none{\the\lastnodetype}}
          \hbox{\vbox{}\xdef\ntwo{\the\lastnodetype}}
          \hbox{\vrule\xdef\nthree{\the\lastnodetype}}
          \vbox{\insert0{}\xdef\nfour{\the\lastnodetype}}
          \vbox{\mark{}\xdef\nfive{\the\lastnodetype}}
          \hbox{\vadjust{}\xdef\nsix{\the\lastnodetype}}
          \hbox{\discretionary{}{}{}\xdef\neight{\the\lastnodetype}}
          \hbox{\special{}\xdef\nnine{\the\lastnodetype}}
          \hbox{\hskip1pt\xdef\neleven{\the\lastnodetype}}
          \hbox{\kern1pt\xdef\ntwelve{\the\lastnodetype}}
          \hbox{\penalty1\xdef\nthirteen{\the\lastnodetype}}
          \vbox{\halign{#\cr x\cr\noalign{\xdef\nfourteen{\the\lastnodetype}}}}
          \end",
        );

        run_to_end(&mut control, stores);

        for (name, expected) in [
            ("nzero", "0"),
            ("none", "1"),
            ("ntwo", "2"),
            ("nthree", "3"),
            ("nfour", "4"),
            ("nfive", "5"),
            ("nsix", "6"),
            ("neight", "8"),
            ("nnine", "9"),
            ("neleven", "11"),
            ("ntwelve", "12"),
            ("nthirteen", "13"),
            ("nfourteen", "14"),
        ] {
            assert!(stores.intern(name).is_ok(), "missing probe macro {name}");
            assert_eq!(macro_character_text(stores, name), expected, "{name}");
        }
    });
}
#[test]
fn observed_box_packaging_commits_geometry_at_the_operation_boundary() {
    // TeX82 §§649--668 and §§668--676 commit the finished hpack/vpack
    // dimensions. The observer sees those transitions only after each
    // enclosing command operation commits; no Universe-owned queue is
    // involved.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\setbox0=\hbox{}\setbox1=\vbox{}\end");
        let mut observations = GeometryObservationRecorder::default();
        loop {
            match control
                .advance_with_observer(stores, &mut observations)
                .expect("box packaging executes")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }
        let geometry = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Geometry(record) => Some(record),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            geometry.as_slice(),
            [
                GeometryRecord::Hpack {
                    width_sp: 0,
                    height_sp: 0,
                    depth_sp: 0,
                    ..
                },
                GeometryRecord::Vpack {
                    width_sp: 0,
                    height_sp: 0,
                    depth_sp: 0,
                    ..
                }
            ]
        ));
    });
}
#[test]
fn etex_sparse_copy_keeps_a_nested_constructed_source_box() {
    // TeX82 §§1079--1081 make `\copy` a non-destructive read. e-TeX 2.6
    // [47.1077] extends the same operation to sparse box registers.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode
           \setbox32101=\hbox{\global\setbox32102=\vbox{\setbox32103=\vtop{}}}
           \showbox32101
           \setbox32103=\copy32101 \end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if observation_name(&record.key) == Some("box:32103") =>
                {
                    observation_name(&record.value)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(mutations, ["occupied", "occupied"]);
        assert!(stores.copy_box_to_page(32101).is_some());
        assert!(stores.copy_box_to_page(32103).is_some());
    });
}
#[test]
fn end_inside_unterminated_box_reaches_outer_cleanup() {
    // TeX82 §§1064--1065/1095/1054: the stop is backed up behind an inserted
    // right brace, the recovered hbox is appended to the outer vertical list,
    // and the same stop then ejects that residual page exactly once. Use the
    // standard nonstop test host so §82 tests the recovery instead of ending
    // at an exhausted interactive terminal while asking for error advice.
    // Shipout remains output-ledger evidence and does not create a restart
    // checkpoint or an extra TeX command.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\hbox{A\end");

        let mut terminal_step = None;
        let mut artifact_counts = Vec::new();
        for step_index in 1..=16 {
            let step = control
                .advance(stores)
                .expect("unterminated-box recovery executes");
            artifact_counts.push(stores.world().committed_artifacts().len());
            assert!(
                artifact_counts.last() <= Some(&1),
                "end-job recovery must not repeat shipout"
            );
            if matches!(
                step,
                StepResult::Progress(MainControlStep::End)
                    | StepResult::Progress(MainControlStep::EndOfInput)
            ) {
                terminal_step = Some((step_index, step));
                break;
            }
        }

        assert_eq!(
            terminal_step,
            Some((6, StepResult::Progress(MainControlStep::End)))
        );
        assert_eq!(artifact_counts, [0, 0, 0, 0, 1, 1]);
        assert_eq!(stores.world().committed_artifacts().len(), 1);
        assert!(
            admitted!(stores, |context| context
                .current_page_nodes()
                .cloned()
                .collect::<Vec<_>>())
            .is_empty()
        );
        assert!(admitted!(stores, |context| context
            .page_contributions()
            .is_empty()));
        assert_eq!(
            admitted!(stores, |context| context.execution_group_depth()),
            0
        );
        assert_eq!(control.current_mode(), Mode::Vertical);
        assert!(control.fatal_error().is_none());
        let terminal = terminal_text(stores);
        assert_eq!(terminal.matches("! Missing } inserted.").count(), 1);
        assert!(!terminal.contains("That makes 100 errors"), "{terminal}");
    });
}
