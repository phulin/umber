//! Assignment scope, mutation ordering, restoration, and register state under execution.

use super::*;

#[test]
fn repeated_setbox_regions_preserve_durable_aliases_and_publish_pages() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut source = br"\setbox0=\hbox{\kern1pt}\setbox1=\copy0".to_vec();
        for _ in 0..128 {
            source.extend_from_slice(br"\setbox0=\hbox{\kern2pt}");
        }
        source.extend_from_slice(br"\shipout\copy1\shipout\box0\end");

        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, &source);
        run_to_end(&mut control, stores);

        assert_eq!(stores.world().committed_artifacts().len(), 2);
        let lifecycle = stores.page_region_counters();
        assert_eq!(
            lifecycle.page_to_durable_nodes_copied, 0,
            "ordinary setbox construction transfers its closure"
        );
        assert_eq!(
            lifecycle.history_preservation_nodes_copied, 0,
            "a live command operation uses a rollbackable transfer loan"
        );
        assert!(
            lifecycle.tex_copy_nodes_copied > 0,
            "explicit TeX copy remains the one deep-copy seam"
        );
        let alias = stores
            .copy_box_to_page(1)
            .expect("overwriting box 0 preserves the copied durable alias");
        let alias = stores
            .page_node_list(alias)
            .expect("alias publishes back into the current page arena");
        assert!(matches!(
            alias.nodes().first(),
            Some(tex_state::NodeView::HList(_))
        ));
    });
}
#[test]
fn tracked_advance_records_command_and_execution_reads_after_commit() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\count0=17");

        let tracked = control
            .advance_with_tracked_region(stores)
            .expect("tracked operation executes");

        assert_eq!(
            tracked.step,
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            17,
            "the TeX operation committed first"
        );
        let record = tracked
            .region
            .expect("committed operation finishes its region")
            .expect("ordinary assignment is supported");
        assert!(record.observations().iter().any(|observation| {
            observation.key == DependencyKey::Engine(DependencyEngineField::Mode)
        }));
        assert!(record.observations().iter().any(|observation| {
            observation.key == DependencyKey::Engine(DependencyEngineField::GroupType)
        }));
        assert!(!admitted!(stores, |context| context.tracked_region_is_active()));
    });
}
#[test]
fn tracked_group_exit_fails_closed_at_the_journal_timeline_barrier() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\begingroup\endgroup");

        let entered = control
            .advance_with_tracked_region(stores)
            .expect("group entry executes");
        assert!(
            matches!(entered.region, Some(Ok(_))),
            "group entry result: {:?}",
            entered.region
        );
        let exited = control
            .advance_with_tracked_region(stores)
            .expect("group exit executes");

        assert!(matches!(
            exited.region,
            Some(Err(DependencyRegionError::Unsupported(
                TrackedRegionBarrier::EnvironmentTimelineChange,
            )))
        ));
        assert!(!admitted!(stores, |context| context.tracked_region_is_active()));
    });
}
#[test]
fn outer_level_aftergroup_consumes_its_token_without_saving_or_replaying_it() {
    // TeX82 §280: `save_for_after` always consumes the following token, but
    // appends an `insert_token` save word only above `level_one`. The outer-
    // level case is shared unchanged by e-TeX 2.6 and pdfTeX 1.40.29.
    for profile in [
        CommandProfile::TEX82,
        CommandProfile::ETEX26,
        CommandProfile::PDFTEX14029,
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = match profile {
                CommandProfile::TEX82 => MainControl::tex82_initex(stores),
                CommandProfile::ETEX26 => etex_initex(stores),
                CommandProfile::PDFTEX14029 => pdftex_initex(stores),
                _ => unreachable!("test enumerates the three canonical profiles"),
            };
            register_source(
                &mut control,
                br"\def\unexpected{\global\count0=1}\aftergroup\unexpected\end",
            );
            let mut observations = ObservationRecorder::default();

            run_to_end_observed(&mut control, stores, &mut observations);

            assert_eq!(stores.count(0).expect("count register"), 0, "{profile:?}");
            assert_eq!(
                observations
                    .0
                    .iter()
                    .filter(|event| matches!(
                        event,
                        CommandObservation::Input(record)
                            if record.transition == InputTransition::Backup
                                && record.reason == InputReason::Backup
                    ))
                    .count(),
                0,
                "{profile:?}"
            );
        });
    }
}
#[test]
fn accent_assignment_dispatches_backed_up_font_without_redelivery() {
    // TeX82 §§1123--1124 and 1270: the accent-code scan backs up its
    // non-space terminator, then `do_assignments` executes that already
    // expanded current command in place. Its backup level retires before the
    // following base character is fetched; the assignment must not synthesize
    // a second expanded delivery for the same command.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br#"\font\f=cmr10 \font\accentfont=cmr10 \f\accent"7F\accentfont o\end"#,
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let deliveries: Vec<_> = observations
            .0
            .iter()
            .enumerate()
            .filter_map(|(index, observation)| match observation {
                CommandObservation::Command(record)
                    if record.spelling == ObservedToken::ControlSequence("accentfont".into())
                        && record.command == "set_font" =>
                {
                    Some((index, record.boundary))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            deliveries
                .iter()
                .map(|(_, boundary)| *boundary)
                .collect::<Vec<_>>(),
            [
                CommandDeliveryBoundary::Raw,
                CommandDeliveryBoundary::Expanded,
                CommandDeliveryBoundary::Raw,
                CommandDeliveryBoundary::Expanded,
            ]
        );
        let backup = observations.0[deliveries[1].0 + 1..deliveries[2].0]
            .iter()
            .any(|observation| {
                matches!(
                    observation,
                    CommandObservation::Input(record)
                        if record.reason == InputReason::Backup
                            && record.transition == InputTransition::Backup
                )
            });
        assert!(backup, "the integer terminator is backed up before §1270");
        let retirement = observations.0[deliveries[3].0 + 1..]
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::Input(record)
                        if record.reason == InputReason::Backup
                            && record.transition == InputTransition::Retire
                )
            });
        assert_eq!(retirement, Some(0), "the backup retires next");
    });
}
#[test]
fn consuming_current_group_box_preserves_original_void_restore() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingrestores=1\\tracingonline=1{\\setbox2=\\hbox to2pt{}\\setbox3=\\box2}\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{restoring \\box3=void}\n{restoring \\box2=void}\n"
        );
        assert!(stores.copy_box_to_page(2).is_none());
    });
}
#[test]
fn etex_sparse_box_restore_reports_value_before_first_local_assignment() {
    // e-TeX [47.1077] sends box registers above 255 through [53a]'s
    // `sa_def_box`; repeated local assignments save only the original value,
    // and `sa_restore` displays that value when the group ends.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
        &mut control,
        b"\\tracingrestores=1\\tracingonline=1{\\setbox32106=\\vbox{}\\setbox32106=\\hbox{X}}\\end",
    );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{restoring \\box32106=void}\n"
        );
    });
}
#[test]
fn etex_identical_sparse_pointer_assignments_do_not_create_restore_entries() {
    // e-TeX 2.6 [53a] `sa_def` reports an identical pointer as
    // `reassigning`, destroys the scanned reference, and never calls
    // `sa_save`. The sparse mutation remains observable, but §283 therefore
    // has no register entry to restore before the ordinary parameter entry.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
        &mut control,
        br"\tracingrestores=1\tracingonline=1{\tracingassigns=1\muskip2000=0mu\toks2000={}}\end",
    );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            concat!(
                "{into \\tracingassigns=1}\n",
                "{reassigning \\muskip2000=0.0mu}\n",
                "{reassigning \\toks2000=}\n",
                "{restoring \\tracingassigns=0}\n",
            )
        );
    });
}
#[test]
fn etex_everyeof_assignment_is_visible_to_scantokens_during_edef() {
    // e-TeX 2.6 etex.ch §24.362 inserts a non-null \everyeof token list
    // before retiring the pseudo-file, including while \edef is defining.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\everyeof={\noexpand}\edef\x{\scantokens{\begingroup}\endgroup}\end",
        );
        let mut observations = ObservationRecorder::default();

        run_to_end_observed(&mut control, stores, &mut observations);

        assert!(
            admitted!(stores, |context| context
                .token_parameter(tex_state::env::banks::TokParam::EVERY_EOF)
                .expect("everyeof parameter"))
            .is_some(),
            "the source assignment must remain present"
        );
        assert!(observations.0.iter().any(|event| matches!(
            event,
            CommandObservation::Input(record)
                if record.transition == InputTransition::Push
                    && record.reason == InputReason::EveryEof
        )));
    });
}
#[test]
fn etex_scantokens_warns_for_box_group_before_following_conditional() {
    // e-TeX 2.6 [23.328]: each closer warns immediately before its own
    // `unsave`/conditional pop. The two lines of one scantokens source must
    // therefore report the hbox group before the enclosing ifcase.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\let\egroup=}\tracingonline=1\tracingnesting=1
           \setbox0=\hbox{\ifcase0
           \scantokens{\egroup^^J\fi}
           \end",
        );

        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        let group = output
            .find("Warning: end of hbox group")
            .unwrap_or_else(|| panic!("box group warning is rendered: {output:?}"));
        let condition = output
            .find("Warning: end of \\ifcase")
            .unwrap_or_else(|| panic!("conditional warning is rendered: {output:?}"));
        assert!(group < condition, "{output:?}");
    });
}
#[test]
fn meaning_mutation_value_projects_protected_macro_storage_marker() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let definition = stores
            .allocate_definition(&[], &[])
            .expect("empty protected macro definition");

        let value = admitted!(stores, |context| meaning_mutation_value(
            tex_state::ResolvedMeaning::Macro {
                definition,
                flags: MeaningFlags::PROTECTED,
            },
            context,
        ));

        assert_eq!(
            value,
            ObservationValue::Tokens(vec![
                tex_command::ObservedToken::Character {
                    character: '\u{1}',
                    catcode: Catcode::Comment,
                },
                tex_command::ObservedToken::MacroEndMatch,
            ])
        );
    });
}
#[test]
fn protected_macro_marker_observation_precedes_meaning_mutation() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(&mut control, br"\protected\def\p{X}\end");
        let mut observations = ObservationRecorder::default();

        run_to_end_observed(&mut control, stores, &mut observations);

        let marker = observations
            .0
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::TokenList(TokenListRecord {
                        transition: "complete",
                        purpose: "protected_macro",
                        tokens,
                    }) if tokens == &[
                        ObservedToken::MacroEndMatch,
                        ObservedToken::Character {
                            character: 'X',
                            catcode: Catcode::Letter,
                        },
                    ]
                )
            })
            .expect("protected marker transition is observed");
        let mutation = observations
            .0
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::Mutation(MutationRecord {
                        target: MutationTarget::Meaning,
                        key: ObservationValue::Name(name),
                        ..
                    }) if name == "p"
                )
            })
            .expect("protected definition mutation is observed");
        assert_eq!(marker + 1, mutation, "{:?}", observations.0);
    });
}
#[test]
fn etex_optimized_aftergroup_links_tokens_onto_one_backup_level() {
    // TeX82 §§282/326 create one `backed_up` level per saved token. e-TeX
    // 2.6 etex.ch [15.282] instead applies `back_input` only once, then links
    // the remaining tokens onto that level. The TeX82 run is the negative
    // control for the same bounded source microfixture.
    for (profile, expected_backups) in [(CommandProfile::TEX82, 3), (CommandProfile::ETEX26, 1)] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = if profile == CommandProfile::ETEX26 {
                etex_initex(stores)
            } else {
                MainControl::tex82_initex(stores)
            };
            register_source(
                &mut control,
                br"{\aftergroup\relax\aftergroup\relax\aftergroup\relax}\end",
            );
            let mut observations = ObservationRecorder::default();

            run_to_end_observed(&mut control, stores, &mut observations);

            let backups = observations
                .0
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        CommandObservation::Input(record)
                            if record.transition == InputTransition::Backup
                                && record.reason == InputReason::Backup
                    )
                })
                .count();
            let relax_deliveries = observations
                .0
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        CommandObservation::Command(command)
                        if command.boundary == tex_command::CommandDeliveryBoundary::Raw
                            && command.spelling
                                == tex_command::ObservedToken::ControlSequence("relax".into())
                    )
                })
                .count();
            assert_eq!(backups, expected_backups, "profile {profile:?}");
            assert_eq!(relax_deliveries, 6, "profile {profile:?}");
        });
    }
}
#[test]
fn discretionary_parts_execute_live_in_disc_group_without_duplicate_delivery() {
    // TeX82 §§1117/1120: each part returns to main control in restricted
    // horizontal mode under disc_group (e-TeX group code 10). Two macro
    // layers and a conditional make any fixed body-prefetch scheme invalid;
    // the literal `\kern` is the nonmacro negative control for duplicate
    // delivery.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        stores.install_primitive_meaning(
            "currentgrouptype",
            Meaning::InternalInteger(tex_state::meaning::InternalInteger::CurrentGroupType),
        );
        control.set_fuel_limit(10_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\def\layera{\layerb}
          \def\layerb{\ifnum\currentgrouptype=10
            \global\count0=10
          \else
            \global\count0=-1
          \fi}
          \discretionary{\layera\kern1pt}{}{}",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            10,
            "body expansion saw disc_group; terminal={}",
            terminal_text(stores)
        );
        let current_nodes = mode_vec(&control, stores);
        let disc = current_nodes
            .iter()
            .find_map(|node| match node {
                Node::Disc {
                    pre, post, replace, ..
                } => Some((*pre, *post, *replace)),
                _ => None,
            })
            .expect("completed discretionary node");
        assert_eq!(
            page_vec(stores, disc.0)
                .iter()
                .filter(|node| matches!(
                    node,
                    Node::Kern {
                        amount,
                        ..
                    } if *amount == Scaled::from_raw(Scaled::UNITY)
                ))
                .count(),
            1,
            "unexpandable body command executes exactly once"
        );
        assert!(disc.1.is_empty());
        assert!(disc.2.is_empty());
        assert_eq!(
            admitted!(stores, |context| context.innermost_group_kind()),
            None
        );
    });
}
#[test]
fn nested_discretionary_preserves_aftergroup_before_rejecting_the_outer_part() {
    // TeX82 §§282/1120–1121: unsave inserts aftergroup material before
    // build_discretionary scans the next part's left brace. Make that token
    // itself the opener; the literal brace that follows must therefore be an
    // ordinary nested group inside the second part. The inner discretionary
    // simultaneously proves that ActiveDiscretionary is a proper stack, then
    // §1121 rejects it as a forbidden node in the outer discretionary list.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(10_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\let\opener={\noindent
          \discretionary{
            \discretionary{\kern1pt}{}{}
            \aftergroup\opener
          }{\kern2pt}}{\kern3pt}",
        );

        run_to_end(&mut control, stores);

        let current_nodes = mode_vec(&control, stores);
        let [
            Node::Disc {
                pre, post, replace, ..
            },
            ..,
        ] = current_nodes.as_slice()
        else {
            panic!(
                "the forbidden nested discretionary is pruned from the retained outer discretionary: {:?}",
                current_nodes
            );
        };
        assert!(
            pre.is_empty(),
            "the forbidden nested discretionary and its suffix were pruned"
        );
        assert!(matches!(
            page_vec(stores, *post).as_slice(),
            [Node::Kern { .. }]
        ));
        assert!(matches!(
            page_vec(stores, *replace).as_slice(),
            [Node::Kern { .. }]
        ));
        assert!(terminal_text(stores).contains("Improper discretionary list"));
        assert!(
            !terminal_text(stores).contains("Missing { inserted"),
            "aftergroup token supplied the next part opener"
        );
        assert!(control.active_discretionaries.is_empty());
        assert_eq!(
            admitted!(stores, |context| context.innermost_group_kind()),
            None
        );
    });
}
#[test]
fn discretionary_nest_overflow_leaves_group_and_active_stack_untouched() {
    // TeX82 §216 rejects a semantic-nest push before saving any new level.
    // Fatal overflow is committed rather than rolled back, so the
    // discretionary opener must not install disc_group or its executor frame
    // until that bounded push has succeeded.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\noindent\discretionary{}{}{}");
        assert_eq!(
            control.advance(stores).expect("paragraph starts"),
            StepResult::Progress(MainControlStep::Continue)
        );
        while control.modes.depth() < 41 {
            control
                .modes
                .push(Mode::RestrictedHorizontal)
                .expect("fill the TeX82 semantic nest");
        }

        assert_eq!(
            control.advance(stores).expect("fatal overflow succumbs"),
            StepResult::Progress(MainControlStep::End)
        );
        assert_eq!(control.modes.depth(), 41);
        assert_eq!(
            admitted!(stores, |context| context.innermost_group_kind()),
            None
        );
        assert!(control.active_discretionaries.is_empty());
    });
}
#[test]
fn vtop_resets_inherited_parshape_before_display_line_measurement() {
    // TeX82 §§1051--1052 run `normal_paragraph` after opening a `\vtop`.
    // The display therefore uses the box-local 100pt hsize, not the inherited
    // 12pt second `\parshape` line. The empty display's centered reference
    // point therefore extends the vtop's exact natural width to 50pt.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(10_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\nonstopmode
          \hsize=100pt
          \parshape=2 1pt 11pt 2pt 12pt
          \setbox0=\vtop{\noindent$$\kern5pt$$}
          \end",
        );

        run_to_end(&mut control, stores);

        let root = stores
            .copy_box_to_page(0)
            .expect("vtop is assigned to box 0");
        let Some(Node::VList(boxed)) = first_published_node(stores, root) else {
            panic!("box 0 holds a vlist");
        };
        assert_eq!(boxed.width.raw(), 3_276_800);
    });
}
#[test]
fn tex82_display_parameters_are_local_to_the_math_shift_group() {
    // TeX82 §§1145/1194/283: display parameters are defined after
    // `push_math(math_shift_group)` and restored in reverse assignment order.
    // e-TeX's `\predisplaydirection` extension is absent in TeX82 mode.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingrestores=1\tracingonline=1\noindent $$x$$\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        let display_indent = terminal
            .find("{restoring \\displayindent=0.0pt}")
            .expect("display indent restore");
        let display_width = terminal
            .find("{restoring \\displaywidth=0.0pt}")
            .expect("display width restore");
        let pre_display_size = terminal
            .find("{restoring \\predisplaysize=0.0pt}")
            .expect("pre-display size restore");
        let family = terminal
            .find("{restoring \\fam=0}")
            .expect("display family restore");
        assert!(display_indent < display_width);
        assert!(display_width < pre_display_size);
        assert!(pre_display_size < family);
        assert!(!terminal.contains("predisplaydirection"));
    });
}
#[test]
fn let_recognizes_only_raw_other_equals() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\source{X}\let\otherequals==\let\rawtest\otherequals\source\end",
        );
        run_to_end(&mut control, stores);

        let raw_test = stores.intern("rawtest").expect("name").symbol();
        assert_eq!(
            stores.meaning(raw_test).expect("meaning"),
            tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken {
                ch: '=',
                cat: Catcode::Other,
            })
        );
    });
}
#[test]
fn hot_definition_group_and_catcode_apply_is_observation_neutral() {
    // TeX82 §§1211--1234: attaching the detached command observer must not
    // select another semantic implementation. This source exercises every
    // measured direct-apply family, local restoration, explicit and forced
    // global scope, expanded definitions, and future-let replay.
    const SOURCE: &[u8] = br"\def\a#1{A#1}
        \edef\b{\a B}
        \long\gdef\c#1{C#1}
        \xdef\d{D}
        \begingroup
          \def\local{inside}
          \let\alias=\a
          \futurelet\peek\relax\relax
          \catcode64=11
        \endgroup
        \globaldefs=-1 \global\def\forcedlocal{gone}
        \globaldefs=1 {\def\forcedglobal{kept}}
        \globaldefs=0
        \end";

    with_etex(SOURCE, |unobserved| {
        let unobserved_terminal = terminal_text(unobserved);
        let unobserved_log = pending_sink_text(unobserved, false);
        crate::test_harness::with_nonstop_plain_universe(|observed| {
            tex_command::install_tex82_expandable_primitives(observed);
            tex_command::install_etex_expandable_primitives(observed);
            crate::install_unexpandable_primitives(observed);
            crate::install_etex_unexpandable_primitives(observed);
            let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
            register_source(&mut control, SOURCE);
            let mut observations = ObservationRecorder::default();
            run_to_end_observed(&mut control, observed, &mut observations);

            assert_eq!(terminal_text(observed), unobserved_terminal);
            assert_eq!(pending_sink_text(observed, false), unobserved_log);
            assert_eq!(observed.catcode('@'), Catcode::Other);
            assert!(admitted!(observed, |context| {
                context.symbol("local").is_none_or(|symbol| {
                    context.meaning(symbol) == ResolvedMeaning::Static(Meaning::Undefined)
                })
            }));
            assert_eq!(macro_character_text(observed, "forcedglobal"), "kept");
            assert!(observations.0.iter().any(|observation| matches!(
                observation,
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Meaning
                        && observation_tokens(&record.value).is_some()
            )));
        });
    });
}
#[test]
fn local_definition_region_survives_active_body_and_global_let_escape() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\result{bad}
                \begingroup
                  \def\source{promoted}
                  \global\let\escaped=\source
                  \def\cross{\endgroup\gdef\result{continued}}
                  \cross
                \end",
        );
        run_to_end(&mut control, stores);

        assert_eq!(macro_character_text(stores, "escaped"), "promoted");
        assert_eq!(macro_character_text(stores, "result"), "continued");
        assert!(admitted!(stores, |context| {
            let source = context.intern_control_sequence("source");
            let cross = context.intern_control_sequence("cross");
            context.meaning(source) == ResolvedMeaning::Static(Meaning::Undefined)
                && context.meaning(cross) == ResolvedMeaning::Static(Meaning::Undefined)
        }));
    });
}
#[cfg(feature = "profiling")]
#[test]
fn synchronous_assignment_mode_and_list_families_preserve_semantics_without_frames() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\count0=7\advance\count0 by5\begingroup\count0=2\endgroup\setbox0=\hbox{\kern1pt}\end",
        );
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 12);
        assert!(stores.copy_box_to_page(0).is_some());
        assert_eq!(control.modes.current_mode(), Mode::Vertical);
    });
}
#[test]
fn prefixed_definition_scanner_executes_its_exact_substantive_command() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(&mut control, PREFIXED_DEFINITION_RESOURCE_SOURCE);
        register_named_file_size_probe(&mut control, "first", b"ABCD");
        register_named_file_size_probe(&mut control, "second", b"AB");
        run_to_end(&mut control, stores);

        admitted!(stores, |context| {
            let result = context.intern_control_sequence("result4");
            let ResolvedMeaning::Macro { definition, flags } = context.meaning(result) else {
                panic!("resumed prefixed definition is installed")
            };
            assert!(flags.contains(MeaningFlags::LONG));
            assert!(flags.contains(MeaningFlags::OUTER));
            assert!(flags.contains(MeaningFlags::PROTECTED));
            let definition = context.definition(definition);
            let replacement = definition.replacement_text();
            assert_eq!(replacement.len(), 1);
            assert_eq!(
                replacement
                    .get(0)
                    .expect("replacement word")
                    .semantic_token(),
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                }
            );
        });
    });
}
#[test]
fn ordinary_assignment_opens_no_aggregate_savepoint() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\count0=11");

        assert_eq!(
            control.advance(stores).expect("assignment commits"),
            StepResult::Progress(ReplayStep::Continue)
        );
        assert_eq!(stores.count(0).expect("count register"), 11);
        assert_eq!(control.advance_telemetry().attempts, 1);
        assert_eq!(control.advance_telemetry().commits, 1);
        assert_eq!(control.advance_telemetry().maximum_live_savepoints, 0);
    });
}
#[test]
fn group_entry_local_restore_and_exit_open_no_aggregate_savepoint() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"{\count0=11}");

        control.advance(stores).expect("group enters");
        assert_eq!(
            admitted!(stores, |context| context.execution_group_depth()),
            1
        );
        control.advance(stores).expect("local assignment commits");
        assert_eq!(stores.count(0).expect("count register"), 11);
        control.advance(stores).expect("group exits");
        assert_eq!(
            admitted!(stores, |context| context.execution_group_depth()),
            0
        );
        assert_eq!(stores.count(0).expect("count register"), 0);
        assert_eq!(control.advance_telemetry().maximum_live_savepoints, 0);
    });
}
#[test]
fn copy_preserves_every_recursive_node_payload_and_source_register() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let graph = recursive_test_box(stores);
        stores.assign_page_box_local(0, graph);
        let source = stores.copy_box_to_page(0).expect("promoted source graph");

        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\setbox1=\copy0");
        run_to_end(&mut control, stores);
        let source_after_copy = stores.copy_box_to_page(0).expect("copy retains its source");
        assert_eq!(
            recursive_node_signature(stores, &source_after_copy),
            recursive_node_signature(stores, &source)
        );

        let copied = stores.copy_box_to_page(1).expect("copied register");
        let expected = recursive_node_signature(stores, &copied);
        assert_eq!(
            recursive_node_signature(stores, &source),
            expected,
            "copy retains the exact recursive structure"
        );
        let copied_nodes = page_vec(stores, copied);
        let [Node::HList(root)] = copied_nodes.as_slice() else {
            panic!("fixture root should be an hbox")
        };
        let children = page_vec(stores, root.children);
        assert_eq!(children.len(), 13, "every payload remains in child order");
        assert!(
            matches!(&children[1], Node::Glue { spec, leader: Some(_), .. } if spec.width.raw() == 301)
        );
        assert!(
            matches!(&children[3], Node::Mark { tokens, .. } if admitted!(stores, |context| context.node_token_words(*tokens).expect("live mark").to_vec()) == [
                tex_state::token::TokenWord::pack(Token::Char { ch: 'm', cat: Catcode::Letter }),
                tex_state::token::TokenWord::pack(Token::Char { ch: '!', cat: Catcode::Other }),
            ])
        );

        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\setbox2=\box0");
        run_to_end(&mut control, stores);
        assert!(
            stores.copy_box_to_page(0).is_none(),
            "box consumes its source"
        );
        let surviving_copy = stores
            .copy_box_to_page(1)
            .expect("copy survives source release");
        assert_eq!(recursive_node_signature(stores, &surviving_copy), expected);
        let consumed = stores.copy_box_to_page(2).expect("consumed destination");
        assert_eq!(
            recursive_node_signature(stores, &consumed),
            expected,
            "consumption preserves graph"
        );
    });
}
#[test]
fn grouped_copy_keeps_structural_children() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"{\setbox0\hbox{X}\copy0}");
        run_to_end(&mut control, stores);

        assert_eq!(stores.copy_box_to_page(0), None);
    });
}
#[test]
fn incompatible_unbox_commands_preserve_registers_and_replay_state() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\vbox{\hbox{}}\setbox1=\hbox{\kern1pt}",
        );
        run_to_end(&mut control, stores);
        let vbox_root = stores
            .copy_box_to_page(0)
            .expect("vbox register is nonvoid");
        let hbox_root = stores
            .copy_box_to_page(1)
            .expect("hbox register is nonvoid");
        let vbox = recursive_node_signature(stores, &vbox_root);
        let hbox = recursive_node_signature(stores, &hbox_root);
        let source = "\\unhbox0\\par\\unhcopy0\\par\\unvbox1\\unvcopy1";

        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, source.as_bytes());
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("incompatible unbox source checkpoints");
        run_to_end(&mut control, stores);
        let current_vbox = stores.copy_box_to_page(0).expect("vbox remains nonvoid");
        let current_hbox = stores.copy_box_to_page(1).expect("hbox remains nonvoid");
        assert_eq!(recursive_node_signature(stores, &current_vbox), vbox);
        assert_eq!(recursive_node_signature(stores, &current_hbox), hbox);
        let first_output = terminal_text(stores);

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("incompatible unbox source restores");
        run_to_end(&mut control, stores);
        let replayed_vbox = stores.copy_box_to_page(0).expect("vbox remains nonvoid");
        let replayed_hbox = stores.copy_box_to_page(1).expect("hbox remains nonvoid");
        assert_eq!(recursive_node_signature(stores, &replayed_vbox), vbox);
        assert_eq!(recursive_node_signature(stores, &replayed_hbox), hbox);
        assert_eq!(terminal_text(stores), first_output);
    });
}
#[test]
fn badness_reads_most_recent_pack_and_is_not_assignable() {
    // TeX82 §§422--424 reads `\badness` from `last_badness`; §§644/660
    // initializes and updates that same cell during horizontal packing.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let initial = admitted!(stores, |context| (
            context.int_param(IntParam::LAST_BADNESS),
            context
                .internal_integer(tex_state::meaning::InternalInteger::Badness)
                .expect("badness is state-owned"),
        ));
        assert_eq!(initial, (0, 0), "badness is zero before the first pack");

        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"{\setbox0=\hbox to 10pt{\hskip0pt plus1pt}}\count0=\badness\edef\x{\the\badness}",
        );
        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            tex_typeset::INF_BAD
        );
        let packed = admitted!(stores, |context| (
            context.int_param(IntParam::LAST_BADNESS),
            context
                .internal_integer(tex_state::meaning::InternalInteger::Badness)
                .expect("badness is state-owned"),
        ));
        assert_eq!(packed, (tex_typeset::INF_BAD, tex_typeset::INF_BAD));
        let rendered: String = macro_semantic_tokens(stores, "x")
            .into_iter()
            .filter_map(|token| match token {
                Token::Char { ch, .. } => Some(ch),
                _ => None,
            })
            .collect();
        assert_eq!(rendered, "10000");

        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\badness=0");
        run_to_end(&mut control, stores);
        assert!(terminal_text(stores).contains("You can't use `\\badness'"));
    });
}
#[test]
fn etex_identical_local_integer_parameter_reassignment_is_not_a_mutation() {
    // e-TeX §275: `eq_word_define` returns immediately when extended mode
    // locally assigns the value already present. The negative controls pin
    // that a changed local value and an identical global value still commit.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\endlinechar=13 \endlinechar=12 \global\endlinechar=12 \end",
        );
        let mut observations = ObservationRecorder::default();
        loop {
            match control
                .advance_with_observer(stores, &mut observations)
                .expect("e-TeX integer-parameter reassignments execute")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }

        let mutations: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Parameter =>
                {
                    Some((&record.key, &record.value, record.global))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            mutations,
            [
                (
                    &ObservationValue::Name("integer_parameter:48".into()),
                    &ObservationValue::Integer(12),
                    false,
                ),
                (
                    &ObservationValue::Name("integer_parameter:48".into()),
                    &ObservationValue::Integer(12),
                    true,
                ),
            ]
        );
    });
}
#[test]
fn etex_sparse_word_reassignment_retains_its_observed_boundary() {
    // e-TeX 2.6 [49.1236-1237] routes sparse count and dimen words through
    // `sa_w_def`, not §§277-278's dense `eq_word_define`. The canonical
    // oracle observes the sparse assignment boundary even when its value is
    // the default; dense identical assignments retain their shortcut.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"{\count0=0 \dimen0=0pt \count300=0 \dimen301=0pt}\end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register =>
                {
                    Some((&record.key, &record.value))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            mutations,
            [
                (
                    &ObservationValue::Name("count:300".into()),
                    &ObservationValue::Integer(0),
                ),
                (
                    &ObservationValue::Name("dimen:301".into()),
                    &ObservationValue::Scaled(0),
                ),
            ]
        );
        assert_eq!(stores.count(300).expect("count register"), 0);
        assert_eq!(
            admitted!(stores, |context| context.dimen(301)),
            Scaled::from_raw(0)
        );
    });
}
#[test]
fn etex_sparse_register_reads_keep_the_extended_index_after_group_exit() {
    // e-TeX 2.6 [26.427] scans an internal word-register selector with
    // `scan_register_num`. Keep the real sparse value and the independently
    // chosen register-zero sentinel distinct so an eight-bit recovery cannot
    // masquerade as a state-restoration failure.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\begingroup\tracingrestores=1\count20=5\count2000=5\endgroup
           \begingroup{\tracingassigns=1\count2000=0}\count2001=5
           \ifnum\count2000=0 \global\count0=17\fi\endgroup\end",
        );
        run_to_end(&mut control, stores);

        assert_eq!(
            stores.int_param(IntParam::ETEX_EXTENDED_MODE),
            1,
            "extended register domain must survive grouping"
        );
        assert_eq!(
            stores.count(2000).expect("count register"),
            0,
            "sparse state must restore to zero"
        );
        assert_eq!(stores.count(0).expect("count register"), 17);
    });
}
#[test]
fn etex_toks_assignment_and_rhs_keep_sparse_register_indices() {
    // e-TeX 2.6 [49.1226--1227] uses `scan_register_num` for both the direct
    // token-register assignment target and a direct token-register RHS.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(&mut control, br"\toks2000={a b c} \toks2001=\toks2000 \end");
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register =>
                {
                    Some((
                        observation_name(&record.key),
                        observation_tokens(&record.value),
                    ))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(mutations.len(), 2);
        assert_eq!(mutations[0].0, Some("toks:2000"));
        assert_eq!(mutations[1].0, Some("toks:2001"));
        let (copied, source, zero) = admitted!(stores, |context| {
            let token_register = |index| {
                context
                    .token_register(index)
                    .expect("token register")
                    .map(|tokens| context.token_list(tokens).iter().collect::<Vec<_>>())
                    .unwrap_or_default()
            };
            (
                token_register(2_001),
                token_register(2_000),
                token_register(0),
            )
        });
        assert_eq!(copied, source);
        assert!(!copied.is_empty());
        assert!(zero.is_empty());
    });
}
#[test]
fn etex_dense_token_list_reassignments_use_eq_define_shortcut() {
    // e-TeX 2.6 [19.277] returns from `eq_define` when both the command and
    // token-list pointer are unchanged. This covers both dense `\toks`
    // registers and token-list parameters; [49.1226]'s sparse `sa_def` path
    // retains its independently observed assignment boundary.
    let source = br"{\toks20={} \everypar={} \toks300={}
                      \global\toks20={} \global\everypar={}}\end";
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(&mut control, source);
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register
                        || record.target == MutationTarget::Parameter =>
                {
                    Some((record.target, observation_name(&record.key), record.global))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            mutations,
            [
                (MutationTarget::Register, Some("toks:300"), false),
                (MutationTarget::Register, Some("toks:20"), true),
                (MutationTarget::Parameter, Some("token_parameter:1"), true),
            ]
        );

        crate::test_harness::with_nonstop_plain_universe(|tex82| {
            let mut tex82_control = MainControl::tex82_initex(tex82);
            register_source(&mut tex82_control, br"\toks20={} \everypar={} \end");
            let mut tex82_observations = ObservationRecorder::default();
            run_to_end_observed(&mut tex82_control, tex82, &mut tex82_observations);
            assert_eq!(
                tex82_observations
                    .0
                    .iter()
                    .filter(|observation| matches!(observation, CommandObservation::Mutation(_)))
                    .count(),
                2,
                "TeX82 does not have e-TeX's identical-definition shortcut"
            );
        });
    });
}
#[test]
fn braced_token_parameter_assignment_normalizes_empty_to_null_and_restores_scope() {
    // TeX82 §1226 maps a braced scan with `link(def_ref)=null` to
    // `undefined_cs,null`, while a nonempty scan installs `call,def_ref`.
    // Sections 275--283 then restore the exact outer pointer at group exit.
    crate::test_harness::with_nonstop_plain_universe(|empty_stores| {
        let mut empty_control = MainControl::tex82_initex(empty_stores);
        register_source(&mut empty_control, br"\everypar={}\end");
        assert_eq!(
            empty_control
                .advance(empty_stores)
                .expect("empty assignment executes"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(
            admitted!(empty_stores, |context| context
                .token_parameter(TokParam::EVERY_PAR)
                .expect("everypar parameter")),
            None,
            "a braced empty assignment must store TeX's null pointer"
        );

        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(&mut control, br"\everypar={A}{\everypar={}}\end");
            assert_eq!(
                control
                    .advance(stores)
                    .expect("nonempty assignment executes"),
                StepResult::Progress(MainControlStep::Continue)
            );
            let outer = admitted!(stores, |context| context
                .token_parameter(TokParam::EVERY_PAR)
                .expect("everypar parameter"))
            .expect("nonempty assignment stores a pointer");
            assert_eq!(
                admitted!(stores, |context| context
                    .token_list(outer.clone())
                    .iter()
                    .collect::<Vec<_>>()),
                [tex_state::token::TokenWord::pack(Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                })]
            );
            assert_eq!(
                control.advance(stores).expect("group opens"),
                StepResult::Progress(MainControlStep::Continue)
            );
            assert_eq!(
                control
                    .advance(stores)
                    .expect("scoped empty assignment executes"),
                StepResult::Progress(MainControlStep::Continue)
            );
            assert_eq!(
                admitted!(stores, |context| context
                    .token_parameter(TokParam::EVERY_PAR)
                    .expect("everypar parameter")),
                None
            );
            assert_eq!(
                control.advance(stores).expect("group closes"),
                StepResult::Progress(MainControlStep::Continue)
            );
            assert_eq!(
                admitted!(stores, |context| context
                    .token_parameter(TokParam::EVERY_PAR)
                    .expect("everypar parameter")),
                Some(outer)
            );
        });
    });
}
#[test]
fn etex_sparse_setbox_observes_delayed_and_immediate_commits() {
    // TeX82 §§1077/1085 commits a constructed box only after its box group is
    // unsaved. e-TeX 2.6 [47.1077] sends targets above 255 through [53a]'s
    // `sa_def_box`, so those delayed writes (and immediate void operands) are
    // sparse mutation boundaries; the dense `eq_define` target stays silent.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"{\setbox20=\hbox{} \setbox300=\hbox{}
             \global\setbox301=\vbox{} \setbox302=\box0}\end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register =>
                {
                    Some((
                        observation_name(&record.key),
                        observation_name(&record.value),
                        record.global,
                    ))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            mutations,
            [
                (Some("box:300"), Some("occupied"), false),
                (Some("box:301"), Some("occupied"), true),
                (Some("box:302"), Some("void"), false),
            ]
        );
        assert!(stores.copy_box_to_page(20).is_none());
        assert!(stores.copy_box_to_page(300).is_none());
        assert!(stores.copy_box_to_page(301).is_some());
        assert!(stores.copy_box_to_page(302).is_none());
    });
}
#[test]
fn etex_sparse_box_dimension_assignment_is_visible_to_internal_scans() {
    // e-TeX 2.6 [49.1247] widens `alter_box_dimen` with
    // `scan_register_num`; [26.420] uses the same sparse fetch when `\ht`
    // is subsequently scanned as an internal dimension.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\setbox32101=\hbox{} \ht32101=2pt
           \ifdim\ht32101=2pt \count0=1\fi \end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            admitted!(stores, |context| context
                .box_dimension(32101, tex_state::BoxDimension::Height)),
            Some(Scaled::from_raw(2 * Scaled::UNITY))
        );
        assert_eq!(stores.count(0).expect("count register"), 1);
    });
}
#[test]
fn etex_identical_local_code_reassignment_is_a_save_stack_noop() {
    // e-TeX §275 applies the `eq_word_define` reassignment shortcut to every
    // fullword eqtb location, including the code tables. The nested identical
    // assignment must not create a save-stack entry that can roll back over
    // the later global assignment.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(&mut control, br"{\lccode`A=`a \global\lccode`A=`z}\end");
        let mut observations = ObservationRecorder::default();
        loop {
            match control
                .advance_with_observer(stores, &mut observations)
                .expect("e-TeX code-table reassignments execute")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }

        assert_eq!(
            admitted!(stores, |context| context.lccode('A')),
            u32::from('z')
        );
        let mutations: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::CodeTable =>
                {
                    Some((&record.key, &record.value, record.global))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            mutations,
            [(
                &ObservationValue::Name("lccode:65".into()),
                &ObservationValue::Integer(122),
                true,
            )]
        );
    });
}
#[test]
fn code_table_invalid_values_recover_before_scoped_commit() {
    // TeX82 §1230 substitutes zero after consuming the bad value. Both the
    // fused catcode path and the cold lccode path must publish that local
    // assignment, restore it at group exit, then accept a global write.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\aftercat{\message{AFTER-CATCODE}}
                \def\afterlc{\message{AFTER-LCCODE}}
                \catcode63=12 \lccode65=65
                {\afterassignment\aftercat\catcode63=16
                 \afterassignment\afterlc\lccode65=256
                 \ifnum\catcode63=0 \global\count0=1\fi
                 \ifnum\lccode65=0 \global\count1=1\fi}
                \ifnum\catcode63=12 \global\count2=1\fi
                \ifnum\lccode65=65 \global\count3=1\fi
                \global\catcode63=13 \global\lccode65=122 \end",
        );
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("catcode recovery flag"), 1);
        assert_eq!(stores.count(1).expect("lccode recovery flag"), 1);
        assert_eq!(stores.count(2).expect("catcode restore flag"), 1);
        assert_eq!(stores.count(3).expect("lccode restore flag"), 1);
        assert_eq!(stores.catcode('?'), Catcode::Active);
        assert_eq!(admitted!(stores, |context| context.lccode('A')), 122);
        let terminal = terminal_text(stores);
        let log = pending_sink_text(stores, false);
        for output in [&terminal, &log] {
            let positions = [
                "Invalid code (16), should be in the range 0..15",
                "AFTER-CATCODE",
                "Invalid code (256), should be in the range 0..255",
                "AFTER-LCCODE",
            ]
            .map(|message| {
                output
                    .find(message)
                    .unwrap_or_else(|| panic!("missing {message:?} in {output:?}"))
            });
            assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        }
    });
}
#[test]
fn etex_zero_glue_parameter_reassignment_uses_canonical_pointer_identity() {
    // e-TeX §277 suppresses a local `eq_define` when both its type and
    // halfword identity are unchanged. TeX82 §1237 traps a scanned zero glue
    // specification to the shared `zero_glue` pointer before that test.
    // Separately scanned equal nonzero literals remain distinct pointers and
    // are the negative control.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\parfillskip=0pt \parfillskip=1pt \parfillskip=1pt \end",
        );
        let mut observations = ObservationRecorder::default();
        loop {
            match control
                .advance_with_observer(stores, &mut observations)
                .expect("e-TeX glue-parameter reassignments execute")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }

        let mutations: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if observation_name(&record.key) == Some("glue_parameter:14") =>
                {
                    Some(&record.value)
                }
                _ => None,
            })
            .collect();
        assert_eq!(mutations.len(), 2);
        assert_eq!(
            admitted!(stores, |context| {
                let glue = context
                    .glue_param(GlueParam::new(14))
                    .expect("glue parameter");
                context.glue(glue).width
            }),
            Scaled::from_raw(65_536)
        );
    });
}
#[test]
fn etex_signed_internal_glue_is_not_an_identical_pointer_reassignment() {
    // TeX82 §§430/461 negate all three components of an internal glue
    // specification. The transformed value no longer has the source register's
    // pointer identity, so e-TeX §277 must not discard either assignment as a
    // same-pointer reassignment.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\skip0=1pt plus 2fil minus 3fill \skip0=-\skip0
               \muskip0=4mu plus 5fill minus 6fil \muskip0=-\muskip0 \end",
        );

        run_to_end(&mut control, stores);

        admitted!(stores, |context| {
            let skip = context.glue(
                context
                    .glue_register(0)
                    .expect("skip register")
                    .expect("assigned skip"),
            );
            assert_eq!(skip.width, Scaled::from_raw(-Scaled::UNITY));
            assert_eq!(skip.stretch, Scaled::from_raw(-2 * Scaled::UNITY));
            assert_eq!(skip.stretch_order, tex_state::glue::Order::Fil);
            assert_eq!(skip.shrink, Scaled::from_raw(-3 * Scaled::UNITY));
            assert_eq!(skip.shrink_order, tex_state::glue::Order::Fill);

            let muskip = context.glue(context.muskip(0).expect("assigned muskip"));
            assert_eq!(muskip.width, Scaled::from_raw(-4 * Scaled::UNITY));
            assert_eq!(muskip.stretch, Scaled::from_raw(-5 * Scaled::UNITY));
            assert_eq!(muskip.stretch_order, tex_state::glue::Order::Fill);
            assert_eq!(muskip.shrink, Scaled::from_raw(-6 * Scaled::UNITY));
            assert_eq!(muskip.shrink_order, tex_state::glue::Order::Fil);
        });
    });
}
#[test]
fn etex_glue_expression_reassignment_retains_source_pointer_identity() {
    // e-TeX expression change [53a.4945--5360] leaves a glue factor's node
    // untouched when no operator requires a copy. Section 277 therefore
    // classifies the local assignment back to the same register as a
    // reassignment. An equal literal, an expression that applies an operator,
    // and a global assignment are controls: all allocate or define and remain
    // observable.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
        &mut control,
        br"\skip0=1pt \skip0=\glueexpr\skip0\relax \skip0=1pt \skip0=\glueexpr\skip0+0pt\relax \global\skip0=\glueexpr\skip0\relax \end",
    );
        let mut observations = ObservationRecorder::default();
        loop {
            match control
                .advance_with_observer(stores, &mut observations)
                .expect("e-TeX glue-expression reassignments execute")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }

        let mutations: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if observation_name(&record.key) == Some("skip:0") =>
                {
                    Some(record.global)
                }
                _ => None,
            })
            .collect();
        assert_eq!(mutations, [false, false, false, true]);
    });
}
#[test]
fn etex_sparse_skip_reassignment_keeps_sa_def_mutation_boundary() {
    // e-TeX 2.6 [49.1221--1237] sends the sparse shorthand through `sa_def`.
    // Its identical-pointer branch avoids saving or rewriting the element but
    // still completes the sparse assignment boundary, unlike §§277-278's
    // dense `eq_define` shortcut.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\skipdef\alias=32767 \alias=1pt \alias=\glueexpr\alias\relax \end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter(|observation| {
                matches!(
                    observation,
                    CommandObservation::Mutation(record)
                        if observation_name(&record.key) == Some("skip:32767")
                )
            })
            .count();
        assert_eq!(mutations, 2);
        assert_eq!(
            admitted!(stores, |context| {
                let glue = context
                    .glue_register(32_767)
                    .expect("skip register")
                    .expect("assigned sparse skip");
                context.glue(glue).width
            }),
            Scaled::from_raw(Scaled::UNITY)
        );
    });
}
#[test]
fn etex_penalty_array_assignments_are_mode_complete_and_consume_exactly_their_values() {
    // e-TeX 2.6 change [49.1248] routes all four selectors through
    // TeX82 §1248's `set_shape`; e-TeX §§6336-6366 define the selector
    // family and its repeated-last-value enquiry semantics.
    const MODES: [Mode; 6] = [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ];
    const ARRAYS: [(&str, PenaltyArrayKind); 4] = [
        ("interlinepenalties", PenaltyArrayKind::InterLine),
        ("clubpenalties", PenaltyArrayKind::Club),
        ("widowpenalties", PenaltyArrayKind::Widow),
        ("displaywidowpenalties", PenaltyArrayKind::DisplayWidow),
    ];

    for (name, kind) in ARRAYS {
        for mode in MODES {
            crate::test_harness::with_nonstop_plain_universe(|stores| {
                let mut control = etex_initex(stores);
                if mode != Mode::Vertical {
                    control.modes.push(mode).expect("test mode push");
                }
                let source = format!(r"\{name}  =  2  101  -202 \count0=17");
                register_source(&mut control, source.as_bytes());

                assert_eq!(
                    control.advance(stores).expect("penalty array assignment"),
                    StepResult::Progress(MainControlStep::Continue),
                    "selector {name}, mode {mode:?}"
                );
                assert_eq!(
                    admitted!(stores, |context| context.penalty_array(kind)),
                    vec![101, -202]
                );
                assert_eq!(
                    stores.count(0).expect("count register"),
                    0,
                    "following command was not consumed"
                );
                assert_eq!(control.current_mode(), mode);

                assert_eq!(
                    control.advance(stores).expect("following assignment"),
                    StepResult::Progress(MainControlStep::Continue),
                    "selector {name}, mode {mode:?}"
                );
                assert_eq!(
                    stores.count(0).expect("count register"),
                    17,
                    "following command stayed live"
                );
            });
        }
    }
}
#[test]
fn etex_penalty_array_mutations_use_their_extended_token_register_slots() {
    // e-TeX 2.6 [17.230] inserts these eqtb entries after the 256 dense token
    // registers, and [49.1248] assigns each with `define`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\interlinepenalties=1 10
           \global\clubpenalties=1 20
           \widowpenalties=1 30
           \global\displaywidowpenalties=1 40 \end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register =>
                {
                    Some((
                        observation_name(&record.key),
                        observation_tokens(&record.value),
                        record.global,
                    ))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            mutations,
            [
                (Some("toks:256"), Some([].as_slice()), false),
                (Some("toks:257"), Some([].as_slice()), true),
                (Some("toks:258"), Some([].as_slice()), false),
                (Some("toks:259"), Some([].as_slice()), true),
            ]
        );
    });
}
#[test]
fn etex_nonpositive_penalty_array_counts_clear_without_consuming_following_tokens() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\interlinepenalties=1 11 \interlinepenalties=0
           \clubpenalties=1 22 \clubpenalties=-1
           \widowpenalties=1 33 \widowpenalties=0
           \displaywidowpenalties=1 44 \displaywidowpenalties=-2
           \count0=19 \end",
        );

        run_to_end(&mut control, stores);

        for kind in [
            PenaltyArrayKind::InterLine,
            PenaltyArrayKind::Club,
            PenaltyArrayKind::Widow,
            PenaltyArrayKind::DisplayWidow,
        ] {
            assert!(
                admitted!(stores, |context| context.penalty_array(kind)).is_empty(),
                "array {kind:?}"
            );
        }
        assert_eq!(
            stores.count(0).expect("count register"),
            19,
            "zero and negative counts scan no values"
        );
    });
}
#[test]
fn etex_penalty_array_scope_enquiries_and_afterassignment_match_set_shape() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\clubpenalties=2 200 100
           {\clubpenalties=1 7}
           \widowpenalties=2 300 400
           {\widowpenalties=1 7}
           {\globaldefs=1 \displaywidowpenalties=1 500}
           \interlinepenalties=2 9 8
           {\globaldefs=-1 \global\interlinepenalties=-4}
           \def\aftermark{\global\advance\count0 by1}
           \afterassignment\aftermark\clubpenalties=1 42
           \end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array_value(PenaltyArrayKind::Widow, 0)),
            2
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array_value(PenaltyArrayKind::Widow, 1)),
            300
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array_value(PenaltyArrayKind::Widow, 8)),
            400
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::Club)),
            vec![42]
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::Widow)),
            vec![300, 400]
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::DisplayWidow)),
            vec![500]
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::InterLine)),
            vec![9, 8]
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            1,
            "afterassignment fired exactly once"
        );
    });
}
#[test]
fn readline_assignment_trace_precedes_the_next_command_trace() {
    // TeX82 §1225 calls `define(p,call,cur_val)` as soon as `read_toks`
    // returns. e-TeX [17.687-750] therefore renders both halves of that eqtb
    // write before §299 can trace the following command.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
        stores
            .world_mut()
            .push_memory_terminal_line("replacement")
            .expect("terminal line queues");
        let mut control = etex_initex(stores);
        register_source(
        &mut control,
        br"\def\line{\begingroup\scantokens{\message{level=\the\currentgrouplevel}}}\tracingassigns=1\tracingcommands=2\readline16to\line\endlinechar=-1\end",
    );

        run_to_end(&mut control, stores);

        let log = pending_sink_text(stores, false);
        let changing = log
            .find("{changing \\line =macro:->\\begingroup \\scantokens {\\message \\ETC.}")
            .unwrap_or_else(|| panic!("missing read target pre-image: {log:?}"));
        let into = log
            .find("{into \\line =macro:->replacement")
            .unwrap_or_else(|| panic!("missing read target post-image: {log:?}"));
        let next = log
            .find("{\\endlinechar}")
            .unwrap_or_else(|| panic!("missing following command trace: {log:?}"));
        assert!(changing < into && into < next, "{log:?}");
    });
}
#[test]
fn read_to_definition_preserves_effective_scope_and_replay() {
    // TeX82 §§1214/1225 select scope before `read_toks`, then install its
    // parameterless macro after collection. Exercise explicit prefixes and
    // both `\globaldefs` overrides through ordinary replay.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        // `\read-1` first reports §433's out-of-range stream number. Keep this
        // scope/replay test in scroll mode so §82's error-stop dialog does not
        // canonically consume the terminal lines intended for the reads.
        stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
        for line in ["local", "explicit", "forced-global", "forced-local"] {
            stores
                .world_mut()
                .push_memory_terminal_line(line)
                .expect("memory terminal accepts a line");
        }
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\def\local{old}{\read-1to\local}\def\explicit{old}{\global\read-1to\explicit}\globaldefs=1\def\forcedglobal{old}{\read-1to\forcedglobal}\globaldefs=-1\gdef\forcedlocal{old}{\global\read-1to\forcedlocal}\globaldefs=0\end",
    );
        run_to_end(&mut control, stores);

        assert_eq!(
            macro_semantic_tokens(stores, "local")[0],
            Token::Char {
                ch: 'o',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(
            macro_semantic_tokens(stores, "explicit")[0],
            Token::Char {
                ch: 'e',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(
            macro_semantic_tokens(stores, "forcedglobal")[0],
            Token::Char {
                ch: 'f',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(
            macro_semantic_tokens(stores, "forcedlocal")[0],
            Token::Char {
                ch: 'o',
                cat: Catcode::Letter,
            }
        );
    });
}
#[test]
fn read_to_mutation_precedes_afterassignment_replay_and_carries_exact_meaning() {
    // TeX82 §1225 commits `define(p,call,cur_val)` before §1211 reaches
    // §1269's `done:` and backs up the saved afterassignment token.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
        stores
            .world_mut()
            .push_memory_terminal_line("alpha")
            .expect("memory terminal accepts a line");
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\target{old}\afterassignment\relax\global\read-1to\target\end",
        );
        let mut observations = ObservationRecorder::default();
        loop {
            if matches!(
                control
                    .advance_with_observer(stores, &mut observations)
                    .expect("read and its replay execute"),
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput)
            ) {
                break;
            }
        }

        let mutation_index = observations
            .0
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::Mutation(record)
                        if observation_name(&record.key) == Some("target")
                            && matches!(record.value, ObservationValue::Tokens(_))
                            && record.global
                )
            })
            .expect("read meaning mutation is observed");
        let replay_index = observations
            .0
            .iter()
            .enumerate()
            .skip(mutation_index + 1)
            .position(|observation| {
                matches!(
                    observation.1,
                    CommandObservation::Input(record)
                        if record.transition == InputTransition::Backup
                            && record.reason == InputReason::Backup
                )
            })
            .map(|offset| mutation_index + 1 + offset)
            .expect("afterassignment replay is observed");
        assert!(mutation_index < replay_index, "{:?}", observations.0);
        let CommandObservation::Mutation(mutation) = &observations.0[mutation_index] else {
            unreachable!()
        };
        assert!(matches!(
            observation_tokens(&mutation.value),
            Some([
                tex_command::ObservedToken::MacroEndMatch,
                tex_command::ObservedToken::Character {
                    character: 'a',
                    catcode: Catcode::Letter,
                },
                ..
            ])
        ));
    });
}
#[test]
fn hot_definition_publication_precedes_afterassignment_and_its_host_effect() {
    // TeX82 §§1211/1269 commits `define`, publishes its mutation evidence,
    // and invokes §325 `back_input` before the saved macro can execute its
    // §1375 immediate write on the following main-control operation.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\mark{\immediate\write16{after}}\def\target{old}\afterassignment\mark\global\def\target{new}\end",
        );
        let mut observations = ObservationRecorder::default();
        loop {
            if matches!(
                control
                    .advance_with_observer(stores, &mut observations)
                    .expect("hot definition and saved token execute"),
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput)
            ) {
                break;
            }
        }

        let mutation = observations
            .0
            .iter()
            .rposition(|observation| {
                matches!(
                    observation,
                    CommandObservation::Mutation(record)
                        if observation_name(&record.key) == Some("target") && record.global
                )
            })
            .expect("global hot definition mutation");
        let backup = observations
            .0
            .iter()
            .enumerate()
            .skip(mutation + 1)
            .find_map(|(index, observation)| {
                matches!(
                    observation,
                    CommandObservation::Input(record)
                        if record.transition == InputTransition::Backup
                            && record.reason == InputReason::Backup
                )
                .then_some(index)
            })
            .expect("afterassignment backup");
        let write = observations
            .0
            .iter()
            .enumerate()
            .skip(backup + 1)
            .find_map(|(index, observation)| {
                matches!(
                    observation,
                    CommandObservation::Effect(record)
                        if record.kind == ObservationEffectKind::Write
                )
                .then_some(index)
            })
            .expect("saved macro host effect");
        assert!(mutation < backup && backup < write, "{:?}", observations.0);
        assert_eq!(macro_character_text(stores, "target"), "new");
    });
}
#[test]
fn setbox_scope_is_globaldefs_adjusted_before_the_box_is_scanned() {
    // TeX82 §1214's `<Adjust for the setting of \globaldefs>` runs inside
    // `prefixed_command`, so a positive `\globaldefs` makes an unprefixed
    // `\setbox` global and a negative one makes `\global\setbox` local.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\globaldefs=1 {\setbox0=\hbox{\kern1pt}}\globaldefs=-1 {\global\setbox1=\hbox{\kern1pt}}\globaldefs=0 \end",
    );
        run_to_end(&mut control, stores);

        assert!(
            stores.copy_box_to_page(0).is_some(),
            "positive globaldefs is global"
        );
        assert!(
            stores.copy_box_to_page(1).is_none(),
            "negative globaldefs is local"
        );
    });
}
#[test]
fn effective_scope_is_shared_by_provisional_and_committed_meaning_mutations() {
    // TeX82 §§1211/1214 resolve the assignment scope before §1224/§1257
    // install their provisional meanings. §§277-279 then expose that same
    // resolved choice for both provisional and final definitions.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"{\globaldefs=1\chardef\forcedchar=65\countdef\forcedregister=2}{\globaldefs=-1\global\chardef\localchar=66\global\countdef\localregister=3}\globaldefs=0\end",
    );
        let mut observations = ObservationRecorder::default();
        loop {
            if matches!(
                control
                    .advance_with_observer(stores, &mut observations)
                    .expect("scope matrix executes"),
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput)
            ) {
                break;
            }
        }

        for (name, expected_global) in [
            ("forcedchar", true),
            ("forcedregister", true),
            ("localchar", false),
            ("localregister", false),
        ] {
            let scopes: Vec<_> = observations
                .0
                .iter()
                .filter_map(|observation| match observation {
                    CommandObservation::Mutation(record)
                        if observation_name(&record.key) == Some(name) =>
                    {
                        Some(record.global)
                    }
                    _ => None,
                })
                .collect();
            assert!(!scopes.is_empty(), "{name} has an observed mutation");
            assert!(
                scopes.iter().all(|scope| *scope == expected_global),
                "{name} used one effective scope across provisional and final mutations: {scopes:?}"
            );
        }

        for name in ["forcedchar", "forcedregister"] {
            assert_ne!(
                admitted!(stores, |context| {
                    let symbol = context.intern_control_sequence(name);
                    context.meaning(symbol)
                }),
                ResolvedMeaning::Static(Meaning::Undefined),
                "{name} survived its group"
            );
        }
        for name in ["localchar", "localregister"] {
            assert_eq!(
                admitted!(stores, |context| {
                    let symbol = context.intern_control_sequence(name);
                    context.meaning(symbol)
                }),
                ResolvedMeaning::Static(Meaning::Undefined),
                "{name} was restored at group end"
            );
        }
    });
}
#[test]
fn every_non_eqtb_assignment_family_fires_afterassignment_once() {
    // TeX82 §1210 includes all ten families below in prefixed_command, and
    // §1269 reaches `done` after each completed assignment. The saved token
    // must enter through ordinary §325 back_input exactly once.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\def\mark{\global\advance\count0 by1}\afterassignment\mark\nullfont\afterassignment\mark\textfont0=\nullfont\afterassignment\mark\setbox0=\hbox{}\afterassignment\mark\prevdepth=0pt x\afterassignment\mark\spacefactor=1000\par\afterassignment\mark\prevgraf=0\afterassignment\mark\pagegoal=1pt\afterassignment\mark\deadcycles=0\afterassignment\mark\hyphenation{word}\afterassignment\mark\nonstopmode\end",
    );
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 10);
    });
}
#[test]
fn fontdimen_reports_an_unusable_parameter_number_and_leaves_the_font_alone() {
    // TeX82 §578 resolves `n<=0` to the scratch `fmem_ptr`; §579 reports it
    // and §1253 still consumes `=<dimen>`, so the next command runs.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\fontdimen0\nullfont=1pt \count0=1\end");
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(
            admitted!(stores, |context| context
                .hyphen_positions_for_language(0, "ab", 0, 0)),
            Vec::<usize>::new(),
            "§963 diagnoses the duplicate before replacing it with a2b"
        );
        let output = terminal_text(stores);
        assert!(
            output.contains("! Font \\nullfont has only 7 fontdimen parameters."),
            "{output}"
        );
    });
}
#[test]
fn dimension_advance_accepts_the_negative_max_dimen_boundary() {
    // TeX82 §104 deliberately leaves dimension addition unchecked, and
    // §1238 applies `advance` with a plain sum. Thus `-max_dimen-1sp`
    // commits the representable `-2^30sp` value instead of setting
    // `arith_error`. This is the e-TRIP line-781 boundary case.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\dimen44=-1073741823sp \advance\dimen44 by-1sp \end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        assert_eq!(
            admitted!(stores, |context| context.dimen(44)),
            Scaled::from_raw(-1_073_741_824)
        );
        assert!(!terminal_text(stores).contains("Arithmetic overflow"));
        assert!(observations.0.iter().any(|observation| {
            matches!(
                observation,
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register
                        && observation_name(&record.key) == Some("dimen:44")
                        && record.value == ObservationValue::Scaled(-1_073_741_824)
                        && !record.global
            )
        }));
    });
}
#[test]
fn arithmetic_overflow_reports_and_leaves_the_target_unchanged() {
    // TeX82 §1236 returns before `word_define` when `arith_error` is set.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\count0=2000000000 \multiply\count0 by2 \count1=7 \divide\count1 by0 \count2=1\end",
        );
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 2_000_000_000);
        assert_eq!(stores.count(1).expect("count register"), 7);
        assert_eq!(stores.count(2).expect("count register"), 1);
        let output = terminal_text(stores);
        assert_eq!(
            output.matches("! Arithmetic overflow.").count(),
            2,
            "{output}"
        );
    });
}
#[test]
fn math_group_collapses_only_one_undecorated_ord_nucleus() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let empty_list = tex_state::page_node_arena::PageListId::empty();
        let ch = MathChar {
            family: 0,
            character: 'x',
            origin: tex_state::token::OriginId::UNKNOWN,
        };
        for nucleus in [
            MathField::Empty,
            MathField::MathChar(ch),
            MathField::SubBox(empty_list),
            MathField::SubMlist(empty_list),
        ] {
            let list = crate::test_harness::publish_page_nodes(
                stores,
                [Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Ord),
                    nucleus,
                ))],
            );
            assert_eq!(
                collapse_singleton_math_group(
                    &stores.command_context().expect("live generation"),
                    list,
                ),
                nucleus
            );
        }

        let scripted = crate::test_harness::publish_page_nodes(
            stores,
            [Node::MathNoad(MathNoad {
                kind: NoadKind::Normal(NoadClass::Ord),
                nucleus: MathField::MathChar(ch),
                subscript: MathField::MathChar(ch),
                superscript: MathField::Empty,
            })],
        );
        let non_ord = crate::test_harness::publish_page_nodes(
            stores,
            [Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Open),
                MathField::MathChar(ch),
            ))],
        );
        let multiple = crate::test_harness::publish_page_nodes(
            stores,
            [
                Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Ord),
                    MathField::MathChar(ch),
                )),
                Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Ord),
                    MathField::MathChar(ch),
                )),
            ],
        );
        for list in [scripted, non_ord, multiple] {
            assert_eq!(
                collapse_singleton_math_group(
                    &stores.command_context().expect("live generation"),
                    list,
                ),
                MathField::SubMlist(list)
            );
        }
    });
}
#[test]
fn parshape_and_hanging_parameters_reset_after_paragraph() {
    with_etex(
        br"\parshape=1 3pt 40pt\hangindent=5pt\hangafter=2\looseness=2 x\par",
        |stores| {
            assert_eq!(
                stores
                    .dimen_param(DimenParam::HANG_INDENT)
                    .expect("dimension parameter")
                    .raw(),
                0
            );
            assert_eq!(stores.int_param(IntParam::HANG_AFTER), 1);
            assert_eq!(stores.int_param(IntParam::LOOSENESS), 0);
            assert!(admitted!(stores, |context| context.paragraph_shape()).is_empty());
        },
    );
}
#[test]
fn parshape_assignment_obeys_local_and_global_grouping() {
    with_etex(br"\parshape=1 3pt 40pt{\parshape=0}\end", |local| {
        assert_eq!(
            admitted!(local, |context| context.paragraph_shape()).len(),
            1
        );
        assert_eq!(
            admitted!(local, |context| context.paragraph_shape())[0]
                .indent
                .raw(),
            3 * 65_536
        );
        with_etex(br"{\global\parshape=1 7pt 80pt}\end", |global| {
            assert_eq!(
                admitted!(global, |context| context.paragraph_shape()).len(),
                1
            );
            assert_eq!(
                admitted!(global, |context| context.paragraph_shape())[0]
                    .indent
                    .raw(),
                7 * 65_536
            );
        });
    });
}
#[test]
fn etex_parshape_enquiries_return_explicit_and_repeated_components() {
    with_etex(
        br"\parshape=2 1pt 2pt 3pt 4pt
          \edef\result{\the\parshapeindent1/\the\parshapelength1/\the\parshapedimen3/\the\parshapedimen4/\the\parshapeindent8/\the\parshapelength8/\the\parshapeindent0}\end",
    |stores| {
    assert_eq!(
        macro_character_text(stores, "result"),
        "1.0pt/2.0pt/3.0pt/4.0pt/3.0pt/4.0pt/0.0pt"
    );
    });
}
#[test]
fn etex_penalty_arrays_assign_query_restore_and_reset_interline_at_par() {
    with_etex(
        br"\clubpenalties=2 200 100 \widowpenalties=2 300 400
          \displaywidowpenalties=1 500 {\clubpenalties=1 7}
          \interlinepenalties=2 8 7
          \edef\before{\number\clubpenalties0/\the\clubpenalties1/\the\clubpenalties8/\the\widowpenalties1/\the\widowpenalties8/\the\displaywidowpenalties0/\the\displaywidowpenalties8/\the\interlinepenalties0}
          \noindent\par \edef\after{\the\interlinepenalties0}\end",
    |stores| {
    assert_eq!(
        macro_character_text(stores, "before"),
        "2/200/100/300/400/1/500/2"
    );
    assert_eq!(macro_character_text(stores, "after"), "0");
    });
}
#[test]
fn interactionmode_reads_and_assigns_globally() {
    with_etex(
        br"\edef\before{\the\interactionmode}\begingroup\interactionmode=1\endgroup\edef\after{\the\interactionmode}",
    |stores| {
    assert_eq!(macro_character_text(stores, "before"), "3");
    assert_eq!(macro_character_text(stores, "after"), "1");
    assert_eq!(
        stores.interaction_mode(),
        tex_state::InteractionMode::Nonstop
    );
    });
}
