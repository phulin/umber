//! Resource admission, suspension, retry, and checkpoint publication.

use super::*;

#[test]
fn tracked_advance_abandons_before_resource_suspension_rollback() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\font\missing=not-installed");

        let tracked = control
            .advance_with_tracked_region(stores)
            .expect("resource suspension is a step result");

        assert!(matches!(
            tracked.step,
            StepResult::Suspended(ResourceNeed::Font { .. })
        ));
        assert_eq!(tracked.region, None);
        assert!(!admitted!(stores, |context| context.tracked_region_is_active()));
    });
}
#[test]
fn ready_input_provider_stays_in_one_execution_for_observed_and_ordinary_routes() {
    let ordinary = run_immediate_input_provider_route(false);
    let observed = run_immediate_input_provider_route(true);
    assert_eq!(ordinary.0, observed.0);
    assert_eq!(ordinary.1, 1, "ordinary route resolves input exactly once");
    assert_eq!(observed.1, 1, "observed route resolves input exactly once");
    assert_eq!(
        ordinary.2, 0,
        "ready input does not rewind ordinary execution"
    );
    assert_eq!(
        observed.2, 0,
        "ready input does not rewind observed execution"
    );
}
#[test]
fn unavailable_input_provider_continues_to_existing_diagnostic_path() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\input missing \end");
        let mut host = UnavailableInputResourceHost { calls: 0 };
        let mut provider = ResourceHostProvider::new(&mut host);
        let mut suspended = false;
        let mut finished = false;
        for _ in 0..TEST_STEP_LIMIT {
            match control
                .advance_with_resource_provider(stores, &mut provider)
                .expect("unavailable provider route executes")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => {
                    finished = true;
                    break;
                }
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(_) => {
                    suspended = true;
                    break;
                }
            }
        }
        assert!(finished, "unavailable input route exceeded the step bound");
        assert!(!suspended, "authoritative absence must not suspend");
        assert_eq!(host.calls, 1, "unavailable input is resolved once");
    });
}
#[test]
fn ready_font_provider_stays_in_one_execution() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.tfm",
                include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
            )
            .expect("font installs");
        register_source(&mut control, br"\font\f=cmr10 \f A\end");
        let mut host = ImmediateFontResourceHost { calls: 0 };
        let mut provider = ResourceHostProvider::new(&mut host);
        let mut finished = false;
        for _ in 0..TEST_STEP_LIMIT {
            match control
                .advance_with_resource_provider(stores, &mut provider)
                .expect("font provider route executes")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => {
                    finished = true;
                    break;
                }
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => {
                    panic!("ready font unexpectedly suspended: {need:?}")
                }
            }
        }
        assert!(
            finished,
            "ready font provider route exceeded the step bound"
        );
        assert_eq!(host.calls, 1);
        assert_eq!(control.advance_telemetry().resource_replayed_dispatches, 0);
    });
}
#[test]
fn ready_input_probe_provider_stays_in_one_execution() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        stores
            .world_mut()
            .set_memory_file("child.tex", br"probe".to_vec())
            .expect("probe installs");
        register_source(&mut control, br"\openin0=child \ifeof0\fi \closein0\end");
        let mut host = ImmediateInputProbeResourceHost { calls: 0 };
        let mut provider = ResourceHostProvider::new(&mut host);
        let mut finished = false;
        for _ in 0..TEST_STEP_LIMIT {
            match control
                .advance_with_resource_provider(stores, &mut provider)
                .expect("probe provider route executes")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => {
                    finished = true;
                    break;
                }
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => {
                    panic!("ready probe unexpectedly suspended: {need:?}")
                }
            }
        }
        assert!(
            finished,
            "ready probe provider route exceeded the step bound"
        );
        assert_eq!(host.calls, 1);
        assert_eq!(
            stores.world().input_records().len(),
            1,
            "active World probe is opened from its existing record"
        );
        assert_eq!(control.advance_telemetry().resource_replayed_dispatches, 0);
    });
}
#[test]
fn math_choice_nested_input_and_probe_execute_without_duplicate_effects() {
    let child = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(&br"\global\advance\count2 by1 \endinput"[..]),
    );
    for probe in [false, true] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_initex(stores);
            register_cmr10_as(&mut control, stores, "cmr10.tfm");
            if probe {
                control.capabilities_mut().register_input_probe(
                    "child.tex",
                    tex_command::FileEnquiryResource::new(child.clone(), None),
                );
            } else {
                control
                    .capabilities_mut()
                    .register_input("child.tex", child.clone());
            }
            let source = if probe {
                br"\font\body=cmr10 \body $\mathchoice{\global\advance\count0 by1 \openin0=child \ifeof0\fi A}{B}{C}{D}$\global\count1=23\end".as_slice()
            } else {
                br"\font\body=cmr10 \body $\mathchoice{\global\advance\count0 by1 \input child A}{B}{C}{D}$\global\count1=23\end".as_slice()
            };
            register_source(&mut control, source);
            assert_eq!(stores.count(0).expect("count register"), 0);
            run_to_end(&mut control, stores);
            assert_eq!(stores.count(0).expect("count register"), 1);
            assert_eq!(stores.count(1).expect("count register"), 23);
            assert_eq!(
                stores.count(2).expect("count register"),
                usize::from(!probe) as i32
            );
        });
    }
}
#[test]
fn etex_raw_font_character_enquiry_checkpoint_retry_is_atomic() {
    // The `last_item` command identity is serialized in an e-TeX format.
    // Restoring a quiescent checkpoint must restore both the diagnostic
    // effect and the unconsumed operand so a retry takes the identical path.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        control.set_fuel_limit(1_000).expect("bounded fuel");
        register_source(&mut control, br"\nonstopmode \fontcharwd a\end");
        assert_eq!(
            control.advance(stores).expect("interaction mode executes"),
            StepResult::Progress(MainControlStep::Continue)
        );
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("raw font enquiry checkpoints");

        assert_eq!(
            control.advance(stores).expect("raw font enquiry recovers"),
            StepResult::Progress(MainControlStep::Continue)
        );
        let first_hash = stores.journal_cursor().expect("state cursor");
        let first_output = terminal_text(stores);
        assert!(first_output.contains("You can't use `\\fontcharwd' in vertical mode"));

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("raw font enquiry state restores");
        assert_eq!(
            control
                .advance(stores)
                .expect("raw font enquiry retry recovers"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(stores.journal_cursor().expect("state cursor"), first_hash);
        assert_eq!(terminal_text(stores), first_output);
    });
}
#[test]
fn etex_parshape_enquiry_checkpoint_retry_is_atomic() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        control.set_fuel_limit(1_000).expect("bounded fuel");
        register_source(&mut control, br"\nonstopmode \parshapelength1\end");
        assert_eq!(
            control.advance(stores).expect("interaction mode executes"),
            StepResult::Progress(MainControlStep::Continue)
        );
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("raw parshape enquiry checkpoints");

        assert_eq!(
            control
                .advance(stores)
                .expect("raw parshape enquiry recovers"),
            StepResult::Progress(MainControlStep::Continue)
        );
        let first_hash = stores.journal_cursor().expect("state cursor");
        let first_output = terminal_text(stores);
        assert!(first_output.contains("You can't use `\\parshapelength' in vertical mode"));

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("raw parshape enquiry state restores");
        assert_eq!(
            control
                .advance(stores)
                .expect("raw parshape enquiry retry recovers"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(stores.journal_cursor().expect("state cursor"), first_hash);
        assert_eq!(terminal_text(stores), first_output);
    });
}
#[test]
fn bare_macro_parameter_commit_survives_later_input_need_without_duplication() {
    // The §1045 diagnostic is part of the parameter command's committed
    // operation. A later resource need must neither erase nor duplicate the
    // earlier report, even though this direct control cannot be retried.
    // The mode is the harness's `\nonstopmode` rather than an explicit
    // `\errorstopmode`: §1045's report is routed to the terminal either way,
    // and errorstop would send §82 into §83's dialog, which this harness's
    // terminal cannot answer and §71 ends the job over.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"#\input child\end");

        assert!(matches!(
            control.advance(stores).expect("parameter recovers"),
            StepResult::Progress(ReplayStep::Continue)
        ));
        let committed = terminal_text(stores);
        assert_eq!(committed.matches("macro parameter character #").count(), 1);
        let first = control
            .first_recoverable_diagnostic()
            .expect("committed stomach diagnostic");
        assert_eq!(first.kind, "command-recoverable");
        assert!(first.message.contains("macro parameter character #"));
        let first_message = first.message.clone();

        assert!(matches!(
            control.advance(stores).expect("missing input suspends"),
            StepResult::Suspended(ResourceNeed::Input {
                name,
                original_name,
            }) if name == "child.tex" && original_name == "child"
        ));
        assert_eq!(terminal_text(stores), committed);
        assert_eq!(
            control
                .first_recoverable_diagnostic()
                .expect("first diagnostic survives suspension")
                .message,
            first_message
        );
        assert!(matches!(
            control.advance(stores),
            Err(ExecError::ResourceReplayRequired)
        ));
    });
}
#[test]
fn committed_recoverable_diagnostic_survives_later_input_need() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\first{\badness \input child}\first\end",
        );

        let mut suspended = false;
        for _ in 0..8 {
            let step = control
                .advance(stores)
                .expect("diagnostic operation advances");
            if matches!(
                step,
                StepResult::Suspended(ResourceNeed::Input { ref name, .. })
                    if name == "child.tex"
            ) {
                suspended = true;
                break;
            }
        }
        assert!(
            suspended,
            "diagnostic operation eventually requests child input"
        );
        let first_message = control
            .first_recoverable_diagnostic()
            .expect("earlier diagnostic is committed before the input suspension")
            .message
            .clone();

        let first = control
            .first_recoverable_diagnostic()
            .expect("resource need preserves the committed diagnostic");
        assert_eq!(first.kind, "command-recoverable");
        assert!(first.message.contains("\\badness"));
        assert_eq!(first.message, first_message);
        assert!(matches!(
            control.advance(stores),
            Err(ExecError::ResourceReplayRequired)
        ));
    });
}
#[test]
fn production_batch_keeps_ordinary_prefix_and_rejects_direct_retry() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\count0=11 \input child\end");

        let batch_step = control.advance_episode(stores).expect("batch suspends");
        assert!(
            matches!(
                batch_step,
                StepResult::Suspended(ResourceNeed::Input { ref name, .. }) if name == "child.tex"
            ),
            "unexpected batch step: {batch_step:?}"
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            11,
            "the successful ordinary prefix commits"
        );
        let suspended = control.advance_telemetry();
        assert_eq!(suspended.rollbacks, 1);
        assert_eq!(suspended.resource_replayed_delivered_tokens, 0);
        assert_eq!(suspended.resource_replayed_dispatches, 0);
        let suspended_interpreter = control.command.lifecycle_stats();
        assert_eq!(
            suspended_interpreter.processor_entries, suspended_interpreter.processor_completions,
            "resource suspension retires the interpreter facade"
        );
        assert_eq!(suspended_interpreter.live_processors, 0);

        control.capabilities_mut().register_input(
            "child.tex",
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(&b""[..])),
        );
        assert!(matches!(
            control.advance_episode(stores),
            Err(ExecError::ResourceReplayRequired)
        ));
        assert_eq!(stores.count(0).expect("count register"), 11);
        let telemetry = control.advance_telemetry();
        let after_rejection = control.command.lifecycle_stats();
        assert_eq!(
            after_rejection.processor_entries, suspended_interpreter.processor_entries,
            "a rejected direct retry must not re-enter the discarded interpreter"
        );
        assert_eq!(after_rejection.live_processors, 0);
        assert_eq!(after_rejection.maximum_live_processors, 1);
        assert_eq!(telemetry.rollbacks, 1);
        assert_eq!(telemetry.resource_replayed_delivered_tokens, 0);
        assert_eq!(telemetry.resource_replayed_dispatches, 0);
    });
}
#[test]
fn prepared_openin_probe_loads_after_the_blocked_macro_command() {
    let source = br"\font\bodyfont=cmr10 \bodyfont A\def\sectionref{0}\def\pagerefvalue{0}\def\newlabel#1#2{\gdef\sectionref{1}\gdef\pagerefvalue{1}}\def\load{\openin0=child \ifeof0\else\closein0\input child\fi \openin2=second \ifeof2\else\closein2\input second\fi \count0=7}\load\end";
    let child = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(
            &br"\newlabel{sec:intro}{{1}{1}}
"[..],
        ),
    );
    let second = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(&br"\global\count2=11\endinput"[..]),
    );

    let run = || {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_cmr10_as(&mut control, stores, "cmr10.tfm");
            control
                .capabilities_mut()
                .register_input("child.tex", child.clone());
            control
                .capabilities_mut()
                .register_input("second.tex", second.clone());
            register_source(&mut control, source);
            run_to_end(&mut control, stores);
            (
                stores.count(0).expect("count register"),
                stores.count(1).expect("count register"),
                stores.count(2).expect("count register"),
                terminal_text(stores),
            )
        })
    };

    let result = run();
    assert_eq!(result.0, 7);
    assert_eq!(result.1, 0);
    assert_eq!(result.2, 11);
}
#[test]
fn retained_native_probe_actual_use_tracks_pending_and_committed_overwrites() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        let path = Path::new("child.tex");
        stores
            .world_mut()
            .set_memory_file(path, b"external".to_vec())
            .expect("external probe input stages");
        let initial = stores
            .world_mut()
            .read_file(path)
            .expect("external probe input reads");
        let initial_hash = initial.hash();
        let initial_bytes = initial.shared_bytes();
        let dependency = tex_state::InputDependency::new(
            path,
            tex_state::InputDependencyOutcome::Present(initial_hash),
            tex_state::InputDependencyAccess::AuthoritativeProbe,
        );
        control
            .capabilities_mut()
            .register_input_probe_with_dependencies(
                "child.tex",
                tex_command::FileEnquiryResource::world(initial),
                vec![dependency],
            );
        let retained = control
            .capabilities_mut()
            .input_probe_resource("child.tex")
            .expect("retained native probe");
        assert!(
            retained.source().active_world_record().is_none(),
            "capability must not retain the acquisition record"
        );
        assert!(tex_state::SharedBytes::ptr_eq(
            &initial_bytes,
            &retained.source().shared_bytes()
        ));

        let first = retained
            .actual_use(&mut stores.input_open_context())
            .expect("external retained probe materializes")
            .expect("external retained probe remains available");
        assert_eq!(first.source().bytes(), b"external");
        assert!(first.source().active_world_record().is_some());
        assert!(tex_state::SharedBytes::ptr_eq(
            &initial_bytes,
            &first.source().shared_bytes()
        ));
        let first_dependencies = retained.dependencies_for_actual_use(&first);
        admitted!(stores, |context| context
            .record_input_dependencies(&first_dependencies)
            .expect("external probe dependency records"));

        let slot = tex_state::StreamSlot::new(1);
        stores.world_mut().open_out(slot, path);
        stores
            .world_mut()
            .write_text(tex_state::PrintSink::Stream(slot), "pending");
        stores.world_mut().close_out(slot);
        let pending = retained
            .actual_use(&mut stores.input_open_context())
            .expect("pending generated probe materializes")
            .expect("pending generated probe remains available");
        assert_eq!(pending.source().bytes(), b"pending");
        assert_ne!(
            tex_state::ContentHash::from_bytes(pending.source().bytes()),
            initial_hash
        );
        let pending_dependencies = retained.dependencies_for_actual_use(&pending);
        admitted!(stores, |context| context
            .record_input_dependencies(&pending_dependencies)
            .expect("pending dependency records"));

        let effect_pos = stores.world().effect_pos();
        stores
            .publish_effect_prefix(effect_pos)
            .expect("pending output commits");
        let committed = retained
            .actual_use(&mut stores.input_open_context())
            .expect("committed generated probe materializes")
            .expect("committed generated probe remains available");
        assert_eq!(committed.source().bytes(), b"pending");

        stores.world_mut().open_out(slot, path);
        stores
            .world_mut()
            .write_text(tex_state::PrintSink::Stream(slot), "committed-new");
        stores.world_mut().close_out(slot);
        let effect_pos = stores.world().effect_pos();
        stores
            .publish_effect_prefix(effect_pos)
            .expect("replacement output commits");
        let replaced = retained
            .actual_use(&mut stores.input_open_context())
            .expect("replacement generated probe materializes")
            .expect("replacement generated probe remains available");
        assert_eq!(replaced.source().bytes(), b"committed-new");
        let replaced_hash = tex_state::ContentHash::from_bytes(replaced.source().bytes());
        assert_ne!(
            replaced_hash,
            tex_state::ContentHash::from_bytes(committed.source().bytes())
        );
        let replaced_dependencies = retained.dependencies_for_actual_use(&replaced);
        admitted!(stores, |context| context
            .record_input_dependencies(&replaced_dependencies)
            .expect("replacement dependency records"));
        let current_dependency = stores
            .world()
            .input_dependencies()
            .find(|dependency| dependency.path() == path)
            .expect("current accepted probe dependency");
        assert_eq!(
            current_dependency.outcome(),
            tex_state::InputDependencyOutcome::Present(replaced_hash)
        );
        assert_eq!(
            current_dependency.access(),
            tex_state::InputDependencyAccess::AuthoritativeProbe
        );

        let generated_only = {
            let mut transaction = stores.begin_shipout();
            let generated_path = Path::new("rolled-back.tex");
            let generated_slot = tex_state::StreamSlot::new(2);
            transaction
                .world_mut()
                .open_out(generated_slot, generated_path);
            transaction
                .world_mut()
                .write_text(tex_state::PrintSink::Stream(generated_slot), "discarded");
            transaction.world_mut().close_out(generated_slot);
            let content = transaction
                .world_mut()
                .read_file(generated_path)
                .expect("generated-only content reads before rollback");
            let hash = content.hash();
            control
                .capabilities_mut()
                .register_input_probe_with_dependencies(
                    "rolled-back.tex",
                    tex_command::FileEnquiryResource::world(content),
                    vec![tex_state::InputDependency::new(
                        generated_path,
                        tex_state::InputDependencyOutcome::Present(hash),
                        tex_state::InputDependencyAccess::AuthoritativeProbe,
                    )],
                );
            control
                .capabilities_mut()
                .input_probe_resource("rolled-back.tex")
                .expect("generated-only retained capability")
        };
        let after_rollback = generated_only
            .actual_use(&mut stores.input_open_context())
            .expect("rolled-back generated-only probe checks current output");
        assert!(
            after_rollback.is_none(),
            "a generated-only capability must not revive bytes after rollback"
        );
    });
}
#[test]
fn superscript_math_group_propagates_through_preloaded_input_probe() {
    // TeX82 §1153 returns from `scan_math` immediately after `push_math`;
    // §1030 ordinary main control executes this braced superscript until
    // §1186's right-brace command stores the finished mlist. The global
    // increment before the probe proves that the nested body does not replay
    // its opener or restart already committed commands.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.capabilities_mut().register_input_probe(
            "child.tex",
            tex_command::FileEnquiryResource::new(
                SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(&b""[..]),
                ),
                None,
            ),
        );
        register_source(
            &mut control,
            br"$^{\global\advance\count0 by1 \openin0=child \ifeof0\else\closein0\fi}\global\count1=23",
        );
        run_to_end(&mut control, stores);
        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(stores.count(1).expect("count register"), 23);
        assert!(control.active_math_fields.is_empty());
        assert!(control.pending_resource_site().is_none());
        assert_eq!(
            control
                .advance_telemetry()
                .resource_replayed_delivered_tokens,
            0
        );
        assert_eq!(control.advance_telemetry().resource_replayed_dispatches, 0);
    });
}
#[test]
fn nested_file_probe_executes_expandafter_collector_csname_and_integer_frames_with_preloaded_resource()
 {
    // e-TeX [27.465] enters a nested general-text collector for `\unexpanded`.
    // The direct semantic test admits the fixture before execution. The
    // incremental session tests own full-checkpoint replay for the same
    // scanner families, so this test can focus on the nested collectors and
    // exact expansion destinations without retrying a discarded control.
    let child = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(&b"AB"[..]),
    );

    let run = |source: &[u8]| {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_initex(stores);
            control.capabilities_mut().register_input_probe(
                "child",
                tex_command::FileEnquiryResource::new(child.clone(), None),
            );
            register_source(&mut control, source);
            run_to_end(&mut control, stores);
            terminal_text(stores)
        })
    };

    for (source, expected) in [
        (
            br"\edef\result{\unexpanded\expandafter{\pdffiledump length 2{child}}}\message{[\result]}\end"
                .as_slice(),
            "[4142]",
        ),
        (
            br"\edef\result{P\unexpanded\expandafter{{A\pdffiledump length 2{child}B}}Q}\message{[\result]}\end"
                .as_slice(),
            "[P{A4142B}Q]",
        ),
        (
            br"\edef\result{\csname a\pdffiledump length 2{child}b\endcsname}\message{[\meaning\result]}\end"
                .as_slice(),
            "a4142b",
        ),
        (
            br"\edef\result{\romannumeral\pdffilesize{child}}\message{[\result]}\end".as_slice(),
            "[ii]",
        ),
        (
            br"\nonstopmode\def\gobble#1X{Z}\def\term{\expandafter\gobble\pdffilesize{child}X}\edef\result{\romannumeral0\term}\message{[\result]}\end"
                .as_slice(),
            "[Z]",
        ),
        (
            br"\catcode0=13 \protected\def^^@{}\edef\result{\romannumeral`^^@\pdffilesize{child}}\message{[\result]}\end"
                .as_slice(),
            "[2]",
        ),
    ] {
        let output = run(source);
        assert!(output.contains(expected), "{output:?}");
    }
}
#[test]
fn nested_csname_and_ifcsname_accumulators_execute_with_preloaded_probes() {
    // TeX82 §372 and e-TeX [17.4765--4779] use the same expanded name scan,
    // Each invocation owns a different accumulated spelling and `\ifcsname`
    // additionally owns an already-pushed condition frame. A caller-order name
    // stack can make these examples appear to work only by matching recursive
    // return order; each name must stay with its enclosing expansion or
    // conditional phase.
    let source = br"\expandafter\def\csname inner4143\endcsname{Z}\expandafter\def\csname outerZtail\endcsname{OK}\expandafter\def\csname inner4\endcsname{Y}\expandafter\def\csname outerYtail\endcsname{YES}\edef\result{\csname outer\csname inner\pdffiledump length 1{second}\pdffiledump length 1{third}\endcsname tail\endcsname}\ifcsname outer\csname inner\pdffilesize{first}\endcsname tail\endcsname\message{[\result:YES]}\else\message{[bad-ifcsname]}\fi\unless\ifcsname missing\pdffilesize{4}\endcsname\message{[UNLESS]}\else\message{[bad-unless]}\fi\end";

    let preloaded_terminal = run_pdftex_file_probe_job(source, &["second", "third", "first", "4"]);
    assert!(
        preloaded_terminal.contains("[OK:YES] [UNLESS]"),
        "{preloaded_terminal:?}"
    );
}
#[test]
fn count_assignment_scans_exact_integer_operand_with_preloaded_probe() {
    // The literal prefix is part of the assignment before `\pdffilesize`
    // contributes its scalar. The assignment and integer scanner must retain
    // that prefix and continue the same radix tail, yielding 12 followed by 2.
    for (source, resources, expected) in [
        (
            br"\count0=12\pdffilesize{second}\message{[\the\count0]}\end".as_slice(),
            &["second"][..],
            "[122]",
        ),
        (
            br"\count0=12\pdffilesize{second}\pdffilesize{third}\message{[\the\count0]}\end"
                .as_slice(),
            &["second", "third"][..],
            "[1222]",
        ),
    ] {
        let preloaded_terminal = run_pdftex_file_probe_job(source, resources);
        assert!(
            preloaded_terminal.contains(expected),
            "{preloaded_terminal:?}"
        );
    }

    let source = br"\dimen0=12\pdffilesize{second}pt\message{[\the\dimen0]}\end";
    let preloaded_terminal = run_pdftex_file_probe_job(source, &["second"]);
    assert!(
        preloaded_terminal.contains("[122.0pt]"),
        "{preloaded_terminal:?}"
    );

    let source =
        br"\skip0=1\pdffilesize{second}pt plus 3\pdffilesize{third}fil\message{[\the\skip0]}\end";
    let resources = &["second", "third"];
    let preloaded_terminal = run_pdftex_file_probe_job(source, resources);
    assert!(
        preloaded_terminal.contains("[12.0pt plus 32.0fil]"),
        "{preloaded_terminal:?}"
    );
}
#[test]
fn expanding_command_executes_nested_expanded_scanner_with_preloaded_probe() {
    // TeX82 §§380 and 473--479 settle the expandable preflight to `\edef`.
    // pdfTeX §§495/1535 then run the outer macro-definition collector before
    // its nested `\expanded` collector in exact LIFO order.
    let source = br"\def\afterfirst#1{\edef\result{\expanded{\pdffiledump length 2{second}}}}\expandafter\afterfirst\pdffilesize{first}\message{[\result]}\end";

    let preloaded_terminal = run_pdftex_file_probe_job(source, &["first", "second"]);
    assert!(preloaded_terminal.contains("[4142]"));
}
#[test]
fn directly_delivered_edef_executes_inner_expanded_scanner_with_preloaded_probe() {
    // Negative control: without the earlier expanding-preflight suspension,
    // the directly delivered `\edef` already owns the nested scanner retry.
    let source = br"\edef\result{\expanded{\pdffiledump length 2{second}}}\message{[\result]}\end";

    let preloaded_terminal = run_pdftex_file_probe_job(source, &["second"]);
    assert!(preloaded_terminal.contains("[4142]"));

    // The old same-stack assertion was intentionally removed: a resource
    // miss now discards this direct operation, and the retained-generation
    // tests cover the full-checkpoint replay contract.
}
#[test]
fn scalar_optional_space_keeps_a_nested_scanner_as_its_child_with_preloaded_probe() {
    // The nested csname/expandafter shape matches LaTeX's format-time
    // primitive-name construction. `\number` has finished its scalar before
    // its optional-space lookahead enters the suspended `\expanded` scanner.
    let source = br"\edef\result{\csname outer\expandafter\csname inner\expandafter\expandafter\expandafter\number1\expanded{\unexpanded{A}\pdffiledump length 2{second}}\endcsname\endcsname}\message{[done]}\end";

    let preloaded_terminal = run_pdftex_file_probe_job(source, &["second"]);
    assert!(
        preloaded_terminal.contains("[done]"),
        "{preloaded_terminal:?}"
    );
}
#[test]
fn expandafter_child_completion_scans_its_owning_expanded_collector_with_preloaded_resources() {
    // The outer macro-definition collector owns `\expandafter`; that frame
    // owns its second-command `\expanded` invocation; and the nested scanner
    // owns each file-enquiry expansion. Preloading the fixtures keeps this
    // direct semantic test focused on the exact child edge and collector
    // ownership; the session tests own resource replay.
    let source = br"\edef\result{\expandafter Q\expanded{\unexpanded{U}\pdffiledump length 2{second}\pdffiledump length 2{third}}}\message{[\result]}\end";

    let preloaded_terminal = run_pdftex_file_probe_job(source, &["second", "third"]);
    assert!(preloaded_terminal.contains("[QU41424344]"));
}
#[test]
fn preloaded_resource_fuel_abort_releases_its_scanner_child() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\edef\result{\expanded{\pdffiledump length 2{second}}}\end",
        );
        register_named_file_size_probe(&mut control, "second", b"AB");
        control.set_fuel_limit(1).expect("bounded abort fuel");

        let aborted = control.advance_episode(stores);
        assert!(
            matches!(
                &aborted,
                Err(ExecError::Command(CommandError::FuelExhausted { .. }))
            ) || matches!(
                &aborted,
                Err(ExecError::Captured { error, .. })
                    if matches!(**error, ExecError::Command(CommandError::FuelExhausted { .. }))
            ),
            "unexpected scalar abort: {aborted:?}"
        );
    });
}
#[test]
fn preloaded_scalar_operation_fuel_abort_releases_parent_and_deepest_child() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\count0=12\pdffilesize{second}\message{unreachable}\end",
        );
        register_named_file_size_probe(&mut control, "second", b"AB");
        control.set_fuel_limit(1).expect("bounded abort fuel");

        let aborted = control.advance_episode(stores);
        assert!(
            matches!(
                &aborted,
                Err(ExecError::Command(CommandError::FuelExhausted { .. }))
            ) || matches!(
                &aborted,
                Err(ExecError::Captured { error, .. })
                    if matches!(**error, ExecError::Command(CommandError::FuelExhausted { .. }))
            ),
            "unexpected scalar operation abort: {aborted:?}"
        );
        assert!(
            control.pending_resource_site().is_none(),
            "fuel abort must leave no resource site after the direct operation is discarded"
        );
    });
}
#[test]
fn sequential_generated_reference_probes_preserve_the_macro_cursor() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            include_bytes!("../../../../../tests/corpus/stabilization/latex-references/source.tex"),
        );
        for (name, bytes) in [
            ("main.aux", GENERATED_MAIN_AUX),
            ("main.toc", GENERATED_MAIN_TOC),
        ] {
            control.capabilities_mut().register_input_probe(
                name,
                tex_command::FileEnquiryResource::new(
                    SourceRegistration::new(
                        RegisteredSourceKind::Generated,
                        Arc::<[u8]>::from(bytes),
                    ),
                    None,
                ),
            );
        }
        let mut host = GeneratedReferenceInputHost::default();
        let mut provider = ResourceHostProvider::new(&mut host);
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();
        let mut terminal_step = None;
        for _ in 0..512 {
            let result = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step_with_resource_provider(&mut checkpoints, &cancellation, &mut provider);
            match result {
                crate::CanonicalStepResult::ResourceNeed(need) => panic!(
                    "preloaded reference probe unexpectedly crossed a resource boundary: {need:?}"
                ),
                crate::CanonicalStepResult::Completed(step @ ReplayStep::End) => {
                    terminal_step = Some(step);
                    break;
                }
                crate::CanonicalStepResult::Progress(_)
                | crate::CanonicalStepResult::Committed(_) => {}
                other => panic!("unexpected reference step {other:?}"),
            }
        }
        assert_eq!(
            host.calls, 2,
            "required reads use the host provider separately from probes"
        );
        assert_eq!(control.pending_resource_site(), None);
        ledger
            .terminal_receipt(&control, stores, terminal_step.expect("terminal step"))
            .expect("answered probes leave terminal completion quiescent");
    });
}
#[test]
fn unavailable_input_probe_releases_its_diagnostic_site_before_direct_retry_rejection() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\message{[\pdffilesize{missing-resource}]}\end",
        );
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();
        let need = {
            let mut pending = None;
            for _ in 0..TEST_STEP_LIMIT {
                match crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                    .step(&mut checkpoints, &cancellation)
                {
                    crate::CanonicalStepResult::ResourceNeed(
                        need @ ResourceNeed::InputProbe { .. },
                    ) => {
                        assert!(control.pending_resource_site().is_some());
                        pending = Some(need);
                        break;
                    }
                    crate::CanonicalStepResult::Completed(_) => {
                        panic!("unavailable probe completed before its resource need")
                    }
                    crate::CanonicalStepResult::Progress(_)
                    | crate::CanonicalStepResult::Committed(_) => {}
                    other => panic!("unexpected unavailable-probe step: {other:?}"),
                }
            }
            pending.expect("unavailable probe need must be reached in bounds")
        };
        ledger.mark_unavailable(&mut control, &need, false);
        assert_eq!(control.pending_resource_site(), None);
        assert!(matches!(
            crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation),
            crate::CanonicalStepResult::Failed(crate::CanonicalStepFailure::Execution(
                ExecError::ResourceReplayRequired
            ))
        ));
    });
}
#[test]
fn prefixed_definition_scanner_rejects_stale_retry_after_first_child_need() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(&mut control, PREFIXED_DEFINITION_RESOURCE_SOURCE);

        let first = next_input_probe(&mut control, stores);
        assert!(matches!(&first, ResourceNeed::InputProbe { .. }));
        register_file_size_probe(&mut control, &first, b"ABCD");
        assert!(matches!(
            control.advance_episode(stores),
            Err(ExecError::ResourceReplayRequired)
        ));
    });
}
#[test]
fn prefixed_definition_scanner_fuel_abort_releases_its_operation_child() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(&mut control, PREFIXED_DEFINITION_RESOURCE_SOURCE);
        register_named_file_size_probe(&mut control, "first", b"ABCD");
        register_named_file_size_probe(&mut control, "second", b"AB");
        control.set_fuel_limit(1).expect("bounded abort fuel");

        let aborted = control.advance_episode(stores);
        assert!(
            matches!(
                &aborted,
                Err(ExecError::Command(CommandError::FuelExhausted { .. }))
            ) || matches!(
                &aborted,
                Err(ExecError::Captured { error, .. })
                    if matches!(**error, ExecError::Command(CommandError::FuelExhausted { .. }))
            ),
            "unexpected prefixed-definition abort: {aborted:?}"
        );
        assert!(
            control.pending_resource_site().is_none(),
            "fuel abort leaves no resource site after the direct operation is discarded"
        );
    });
}
#[test]
fn main_source_paragraph_checkpoint_precedes_the_next_command() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(&mut control, br"\count0=1 A\par\count0=2\end");
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();

        let committed = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
            .step(&mut checkpoints, &cancellation);
        assert!(
            matches!(committed, crate::CanonicalStepResult::Committed(_)),
            "paragraph boundary result: {committed:?}"
        );
        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(
            checkpoints
                .iter()
                .filter(|checkpoint| {
                    checkpoint.boundary() == crate::EngineBoundary::OuterParagraphEnd
                })
                .count(),
            1
        );

        for _ in 0..32 {
            if matches!(
                crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger,)
                    .step(&mut checkpoints, &cancellation),
                crate::CanonicalStepResult::Completed(_)
            ) {
                break;
            }
        }
        assert_eq!(stores.count(0).expect("count register"), 2);
        assert_eq!(
            checkpoints
                .iter()
                .filter(|checkpoint| {
                    checkpoint.boundary() == crate::EngineBoundary::OuterParagraphEnd
                })
                .count(),
            1,
            "one paragraph intent publishes exactly once"
        );
    });
}
#[test]
fn observed_resource_need_publishes_no_uncommitted_prefix() {
    let source = br"\input child\end";
    let child =
        SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(&b""[..]));

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, source);
        let mut retried = ObservationRecorder::default();
        assert!(matches!(
            control
                .advance_with_observer(stores, &mut retried)
                .expect("missing input suspends"),
            StepResult::Suspended(ResourceNeed::Input { ref name, .. }) if name == "child.tex"
        ));
        assert!(
            retried.0.is_empty(),
            "a suspended observed operation publishes no prefix"
        );
        control
            .capabilities_mut()
            .register_input("child.tex", child.clone());
        assert!(matches!(
            control.advance_with_observer(stores, &mut retried),
            Err(ExecError::ResourceReplayRequired)
        ));
        assert!(
            retried.0.is_empty(),
            "a discarded observed operation cannot publish a prefix on stale retry"
        );
    });
}
#[test]
fn committed_token_scanner_attempt_is_discarded_before_named_checkpoint() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\toks0={A}");

        assert_eq!(
            control.advance(stores).expect("token assignment commits"),
            StepResult::Progress(ReplayStep::Continue)
        );
        let stored = admitted!(stores, |context| {
            let tokens = context
                .token_register(0)
                .expect("token register lookup")
                .expect("token assignment installs its promoted root");
            context.token_list(tokens).iter().collect::<Vec<_>>()
        });
        assert_eq!(
            stored,
            [tex_state::token::TokenWord::pack(Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            })]
        );
        control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("committed scanner attempt no longer blocks a named checkpoint");
    });
}
#[test]
fn diagnostic_assignment_discards_font_need_before_checkpoint_capture() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::JobStart,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("quiescent diagnostic control captures a checkpoint");
        register_source(&mut control, br"\font\body=cmr10 X");

        assert!(matches!(
            control
                .diagnostic_expand_step(stores)
                .expect("font request suspends"),
            DiagnosticStepResult::Suspended(ResourceNeed::Font { .. })
        ));
        let state_before = stores.journal_cursor().expect("state cursor");
        control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("discarded diagnostic need leaves a quiescent control");
        assert_eq!(
            stores.journal_cursor().expect("state cursor"),
            state_before,
            "discarding the attempt must not mutate the semantic state"
        );
        drop(checkpoint);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        assert!(matches!(
            control.diagnostic_expand_step(stores),
            Err(ExecError::ResourceReplayRequired)
        ));
        assert_eq!(
            control.pending_resource_site(),
            None,
            "diagnostic resource discard leaves no scanner continuation"
        );
        assert_eq!(control.advance_telemetry().maximum_live_savepoints, 0);
    });
}
#[test]
fn diagnostic_input_retry_rejects_the_discarded_delivery_attempt() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\input child after");
        assert!(matches!(
            control
                .diagnostic_expand_step(stores)
                .expect("input request suspends"),
            DiagnosticStepResult::Suspended(ResourceNeed::Input { ref name, .. })
                if name == "child.tex"
        ));

        control.capabilities_mut().register_input(
            "child.tex",
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(
                    &br"\def\frominput{IN}%
\frominput
"[..],
                ),
            ),
        );
        assert!(matches!(
            control.diagnostic_expand_step(stores),
            Err(ExecError::ResourceReplayRequired)
        ));
        assert_eq!(
            control.pending_resource_site(),
            None,
            "diagnostic resource discard leaves no scanner continuation"
        );
    });
}
#[test]
fn etex_penalty_array_assignment_restores_checkpoint_and_retries_atomically() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(&mut control, br"\clubpenalties=2 7 5 \count0=23 \end");
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("penalty array state checkpoints");

        assert_eq!(
            control.advance(stores).expect("first assignment"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::Club)),
            vec![7, 5]
        );
        let assigned_hash = stores.journal_cursor().expect("state cursor");

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("penalty array state restores");
        assert!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::Club))
            .is_empty()
        );
        assert_eq!(stores.count(0).expect("count register"), 0);

        assert_eq!(
            control.advance(stores).expect("retried assignment"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(
            stores.journal_cursor().expect("state cursor"),
            assigned_hash
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::Club)),
            vec![7, 5]
        );
        assert_eq!(
            control.advance(stores).expect("following assignment"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(stores.count(0).expect("count register"), 23);
    });
}
#[test]
fn preloaded_output_routine_preserves_end_job_progress_and_observation_order() {
    // TeX82 §§1025--1026/1054: immutable input acquisition inside an
    // explicit output routine cannot publish or roll back the page-builder
    // progress that admitted that routine. The direct semantic test admits
    // the fixture before execution; incremental checkpoint replay owns the
    // resource-resume equivalence test.
    let source = br"\output={\input child\shipout\box255}\hrule\end";
    let child =
        SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(&b""[..]));

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control
            .capabilities_mut()
            .register_input("child.tex", child);
        register_source(&mut control, source);
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        assert_eq!(stores.world().artifact_commits().len(), 1);
        let shipout = observations
            .0
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::Effect(effect)
                        if effect.kind == ObservationEffectKind::Shipout
                )
            })
            .expect("output routine publishes one shipout");
        let terminate = observations
            .0
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::Effect(effect)
                        if effect.kind == ObservationEffectKind::Terminate
                )
            })
            .expect("end-job termination is observed");
        assert!(
            shipout < terminate,
            "shipout observation must precede termination"
        );
    });
}
#[test]
fn hot_definition_checkpoint_restore_replays_one_atomic_mutation() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\global\def\target{new}\end");
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("quiescent hot definition checkpoints");

        assert_eq!(
            control.advance(stores).expect("hot definition executes"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(macro_character_text(stores, "target"), "new");
        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("hot definition checkpoint restores");
        admitted!(stores, |context| {
            let target = context.intern_control_sequence("target");
            assert!(matches!(
                context.meaning(target),
                ResolvedMeaning::Static(Meaning::Undefined)
            ));
        });
        assert_eq!(
            control.advance(stores).expect("hot definition retries"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(macro_character_text(stores, "target"), "new");
    });
}
#[test]
fn error_stop_deletes_requested_tokens_before_retry() {
    // TeX82 §§84--85: a one- or two-digit response consumes that many
    // unexpanded tokens, displays the resulting context, and prompts again.
    crate::test_harness::with_plain_universe(|stores| {
        stores
            .world_mut()
            .push_memory_terminal_line("2")
            .expect("deletion response queues");
        stores
            .world_mut()
            .push_memory_terminal_line("")
            .expect("retry response queues");
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\show\errorstopmode ab\count0=17\end");

        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            17,
            "only the two ignored letters disappear"
        );
        let terminal = terminal_text(stores);
        assert_eq!(terminal.matches("? ").count(), 2, "{terminal:?}");
    });
}
#[test]
fn etex_showbox_invalid_register_checkpoint_retry_recovers_to_zero() {
    // e-TeX 2.6 etex.ch [49.1296] replaces TeX82's `scan_eight_bit_int`
    // with `scan_register_num`, whose restricted scan diagnoses -1, recovers
    // it to zero, and leaves the following token for the next command.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        control.set_fuel_limit(1_000).expect("bounded fuel");
        register_source(&mut control, br"\showbox-1\count0=23\end");
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("showbox checkpoints");

        assert_eq!(
            control
                .advance(stores)
                .expect("invalid showbox register recovers"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            0,
            "following assignment remains unread"
        );
        let first_hash = stores.journal_cursor().expect("state cursor");
        let first_output = terminal_text(stores);
        assert!(
            first_output.contains("Bad register code (-1)"),
            "{first_output}"
        );
        assert!(first_output.contains("> \\box0="), "{first_output}");

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("showbox state restores");
        assert_eq!(
            control
                .advance(stores)
                .expect("invalid showbox register retries identically"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(stores.journal_cursor().expect("state cursor"), first_hash);
        assert_eq!(terminal_text(stores), first_output);

        run_to_end(&mut control, stores);
        assert_eq!(
            stores.count(0).expect("count register"),
            23,
            "following token executes after recovery"
        );
        assert!(control.fuel_burned() < 1_000);
    });
}
#[test]
fn frozen_page_scalar_rejection_is_checkpoint_atomic() {
    // TeX82 §1236 rejects set_page_dimen as an arithmetic target before
    // scanning an operand. Restoring the command checkpoint must restore both
    // the live frozen page values and the rejected target for an identical
    // retry through §1269's recovery path.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode \topskip=0pt \setbox0=\hbox{}\copy0
           \pagegoal=12pt \insertpenalties=4
           \advance\pagegoal by 3pt \edef\snapshot{\the\pagegoal/\the\insertpenalties}",
        );
        while admitted!(stores, |context| context
            .page_dimension(PageDimension::Goal))
        .raw()
            != 12 * Scaled::UNITY
            || admitted!(stores, |context| context
                .page_integer(PageInteger::InsertPenalties))
                != 4
        {
            assert_eq!(
                control.advance(stores).expect("setup executes"),
                StepResult::Progress(MainControlStep::Continue)
            );
        }
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("frozen page checkpoint captures");

        run_to_end(&mut control, stores);
        let first_output = terminal_text(stores);
        let first_snapshot = macro_semantic_tokens(stores, "snapshot").to_vec();
        assert_eq!(
            admitted!(stores, |context| context
                .page_dimension(PageDimension::Goal))
            .raw(),
            12 * Scaled::UNITY
        );
        assert!(first_output.contains("You can't use `\\pagegoal' after \\advance"));

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("frozen page checkpoint restores");
        run_to_end(&mut control, stores);
        assert_eq!(
            admitted!(stores, |context| context
                .page_dimension(PageDimension::Goal))
            .raw(),
            12 * Scaled::UNITY
        );
        assert_eq!(
            admitted!(stores, |context| context
                .page_integer(PageInteger::InsertPenalties)),
            4
        );
        assert_eq!(macro_semantic_tokens(stores, "snapshot"), first_snapshot);
        assert_eq!(terminal_text(stores), first_output);
    });
}
#[test]
fn invalid_arithmetic_target_commit_survives_later_resource_need() {
    // The §1236 recovery and §1269 afterassignment replay are a committed
    // operation. A later missing-resource boundary cannot duplicate either;
    // direct callers must reject a retry after the need escapes.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\mark{\global\advance\count0 by1}
           \afterassignment\mark\advance x
           \input child\end",
        );

        for _ in 0..8 {
            if stores.count(0).expect("count register") == 1 {
                break;
            }
            assert!(matches!(
                control.advance(stores).expect("setup executes"),
                StepResult::Progress(ReplayStep::Continue)
            ));
        }
        assert_eq!(stores.count(0).expect("count register"), 1);
        let committed = terminal_text(stores);
        assert_eq!(committed.matches("the letter x").count(), 1);

        assert!(matches!(
            control.advance(stores).expect("missing input suspends"),
            StepResult::Suspended(ResourceNeed::Input {
                name,
                original_name,
            }) if name == "child.tex" && original_name == "child"
        ));
        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(terminal_text(stores), committed);
        assert!(matches!(
            control.advance(stores),
            Err(ExecError::ResourceReplayRequired)
        ));
    });
}
#[test]
fn unavailable_font_need_rejects_direct_retry_without_mode_drift() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\batchmode\font\missingfont=absent\missing\scrollmode\end",
        );
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();
        let need = {
            let mut pending = None;
            for _ in 0..TEST_STEP_LIMIT {
                match crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                    .step_with_observer(
                        &mut checkpoints,
                        &cancellation,
                        &mut ObservationRecorder::default(),
                    ) {
                    crate::CanonicalStepResult::ResourceNeed(need @ ResourceNeed::Font { .. }) => {
                        pending = Some(need);
                        break;
                    }
                    crate::CanonicalStepResult::Progress(_)
                    | crate::CanonicalStepResult::Committed(_) => {}
                    crate::CanonicalStepResult::Completed(_) => {
                        panic!("unavailable font completed before its resource need")
                    }
                    other => panic!("unexpected unavailable-font step: {other:?}"),
                }
            }
            pending.expect("unavailable font need must be reached in bounds")
        };
        ledger.mark_unavailable(&mut control, &need, false);
        assert!(matches!(
            crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger).step_with_observer(
                &mut checkpoints,
                &cancellation,
                &mut ObservationRecorder::default(),
            ),
            crate::CanonicalStepResult::Failed(crate::CanonicalStepFailure::Execution(
                ExecError::ResourceReplayRequired
            ))
        ));

        assert_eq!(stores.interaction_mode(), tex_state::InteractionMode::Batch);
    });
}
