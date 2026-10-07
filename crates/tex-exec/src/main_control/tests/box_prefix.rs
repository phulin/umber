//! TeX82 §§403/407/645/1083: consuming box prefixes and their real group.
use super::*;

#[test]
fn setbox_consumes_optional_equals_and_box_selection_in_one_grammar() {
    // §§405/1084/1241: one optional literal sign, then spaces/relax and a
    // genuine box head. An equals after relax is already the box offender.
    for observed in [false, true] {
        for (prefix, valid) in [
            (r"\head", true),
            (r" = \head", true),
            (r"\relax\head", true),
            (r"=\relax\head", true),
            (r"\relax=\head", false),
            (r"==\head", false),
            (r"\equal\head", false),
        ] {
            crate::test_harness::with_nonstop_plain_universe(|stores| {
                let mut control = MainControl::tex82_initex(stores);
                let source = format!(
                    "\\setbox0=\\hbox{{\\kern2pt}}\\def\\head{{\\hbox}}\\let\\equal==\\setbox0{prefix}{{\\kern1pt}}\\end"
                );
                register_source(&mut control, source.as_bytes());
                if observed {
                    run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());
                } else {
                    run_to_end(&mut control, stores);
                }
                let root = stores
                    .copy_box_to_page(0)
                    .expect("box register survives recovery");
                let Some(Node::HList(boxed)) = first_published_node(stores, root) else {
                    panic!("expected hbox");
                };
                assert_eq!(
                    boxed.width.raw(),
                    if valid {
                        Scaled::UNITY
                    } else {
                        2 * Scaled::UNITY
                    },
                    "{source}"
                );
                let terminal = terminal_text(stores);
                assert_eq!(
                    terminal.matches("A <box> was supposed to be here").count(),
                    usize::from(!valid),
                    "{source}: {terminal}"
                );
                assert_eq!(
                    terminal.matches("! ").count(),
                    usize::from(!valid),
                    "{source}: {terminal}"
                );
            });
        }
    }
}

#[test]
fn vcenter_prefix_uses_the_same_group_boundary_and_everyvbox_body() {
    // §§645/1167: vcenter uses scan_spec too. Pinned pdfTeX agrees on
    // these three scope controls and the single null-math-font diagnostic.
    for (prefix, body, expected) in [
        (r"\relax\csname scopedprefix\endcsname", "", r"\undefined"),
        ("", r"\csname scopedprefix\endcsname", r"\undefined"),
        (r"\csname scopedprefix\endcsname", "", r"\relax"),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            let source = format!(
                "\\everyvbox={{\\ifvmode\\else\\errmessage{{wrong body mode}}\\fi\\global\\advance\\count2 by1 }}\\setbox0=\\hbox{{$\\vcenter{prefix}{{{body}}}\\ifx\\scopedprefix{expected}\\global\\count0=1 \\else\\global\\count0=2 \\fi$}}\\end"
            );
            register_source(&mut control, source.as_bytes());
            run_to_end(&mut control, stores);
            assert_eq!(stores.count(0).expect("scope control"), 1, "{source}");
            assert_eq!(stores.count(2).expect("everyvbox"), 1);
            let terminal = terminal_text(stores);
            assert_eq!(terminal.matches("! ").count(), 1, "{terminal}");
            assert!(
                terminal.contains("! Math formula deleted: Insufficient symbol fonts."),
                "{terminal}"
            );
        });
    }
}

#[test]
fn box_prefix_expansion_belongs_to_the_scan_spec_group_boundary() {
    // §645 opens the group after packing, before scan_left_brace. These
    // independent §372 csname controls also pass with pinned pdfTeX.
    for profile in [
        CommandProfile::TEX82,
        CommandProfile::ETEX26,
        CommandProfile::PDFTEX14029,
    ] {
        for kind in ["hbox", "vbox", "vtop"] {
            for (prefix, body, expected) in [
                (r"\relax\csname scopedprefix\endcsname", "", r"\undefined"),
                ("", r"\csname scopedprefix\endcsname", r"\undefined"),
                (r"\csname scopedprefix\endcsname", "", r"\relax"),
            ] {
                crate::test_harness::with_nonstop_plain_universe(|stores| {
                    let mut control = match profile {
                        CommandProfile::TEX82 => MainControl::tex82_initex(stores),
                        CommandProfile::ETEX26 => etex_initex(stores),
                        _ => pdftex_initex(stores),
                    };
                    let source = format!(
                        "\\setbox0=\\{kind}{prefix}{{{body}}}\\ifx\\scopedprefix{expected}\\count0=1 \\else\\count0=2 \\fi\\end"
                    );
                    register_source(&mut control, source.as_bytes());
                    run_to_end(&mut control, stores);
                    assert_eq!(stores.count(0).expect("count register"), 1, "{source}");
                    assert!(
                        !terminal_text(stores).contains("! "),
                        "{source}: {}",
                        terminal_text(stores)
                    );
                });
            }
        }
    }
}

#[test]
fn box_prefix_consumes_packing_keywords_and_meaning_brace_aliases() {
    // §407 ignores keyword case/category; §403 accepts a begin_group meaning.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\let\open={\let\close=}
            \setbox0=\hbox To3pt\open\kern1pt\close
            \setbox1=\hbox SPREAD2pt{\kern1pt}
            \def\prefix{to4pt}\setbox2=\hbox\prefix{\kern1pt}
            \def\boxmaker{\setbox3=\hbox}\catcode`t=12 \boxmaker to5pt{\kern1pt}\end",
        );
        run_to_end(&mut control, stores);
        for (index, width) in [(0, 3), (1, 3), (2, 4), (3, 5)] {
            let root = stores.copy_box_to_page(index).expect("completed box");
            let Some(Node::HList(boxed)) = first_published_node(stores, root) else {
                panic!("expected hbox {index}");
            };
            assert_eq!(boxed.width.raw(), width * Scaled::UNITY);
        }
        assert!(
            !terminal_text(stores).contains("! "),
            "{}",
            terminal_text(stores)
        );
    });
}

#[test]
fn box_prefix_failed_keyword_restores_the_prefix_and_offender() {
    // §407 restores both words; §403 inserts a brace before the actual
    // prefix. Its \kern remains body material, and the real } closes the box.
    for prefix in [r"t\kern1pt", r"sp\kern1pt", r"\letter\kern1pt"] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            let source = format!("\\let\\letter=t\\setbox0=\\hbox {prefix}}}\\count0=7 \\end");
            register_source(&mut control, source.as_bytes());
            run_to_end(&mut control, stores);
            let terminal = terminal_text(stores);
            assert_eq!(
                terminal.matches("! Missing { inserted.").count(),
                1,
                "{terminal}"
            );
            assert_eq!(stores.count(0).expect("following assignment"), 7);
            let nodes = box_child_nodes(stores, 0);
            assert!(
                matches!(nodes.as_slice(), [Node::Kern { amount, .. }] if amount.raw() == Scaled::UNITY),
                "{nodes:?}"
            );
        });
    }
}

#[test]
fn box_prefix_opens_group_before_brace_expansion_and_enters_mode_after_it() {
    // §§645/1083: brace expansion observes the new group and enclosing mode;
    // everyhbox starts only in the new restricted-horizontal body.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\tracinggroups=1\tracingonline=1\let\brace={
            \def\open{\ifnum\currentgrouptype=2 \else\errmessage{wrong group}\fi
                \ifvmode\else\errmessage{wrong prefix mode}\fi\brace}
            \everyhbox={\ifhmode\else\errmessage{wrong body mode}\fi\global\advance\count0 by1 }
            \setbox0=\hbox\relax\open}\end",
        );
        run_to_end(&mut control, stores);
        let terminal = terminal_text(stores);
        assert!(!terminal.contains("! "), "{terminal}");
        assert!(terminal.contains("entering hbox group"), "{terminal}");
        assert!(terminal.contains("leaving hbox group"), "{terminal}");
        assert_eq!(stores.count(0).expect("everyhbox assignment"), 1);
    });
}

#[test]
fn page_succession_waits_for_empty_box_construction_owners() {
    // Ownership regression, not a TeX output oracle: stage the deferred page
    // transition while a real §1083 box has no nodes yet. §§1086/1089 keep its
    // construction live through package and the final register assignment.
    for kind in ["hbox", "vbox", "vtop"] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            let source = format!("\\setbox0=\\{kind}{{\\kern1pt}}\\end");
            register_source(&mut control, source.as_bytes());
            control.advance(stores).expect("open real box construction");
            assert_eq!(control.execution.boxes.active_boxes.len(), 1);
            assert!(!control.execution.modes.retains_page_node_handles());

            control.execution.page_region_succession_pending = true;
            control.finish_pending_page_region_succession(stores);
            assert!(control.execution.page_region_succession_pending);

            run_to_end(&mut control, stores);
            assert!(control.execution.boxes.active_boxes.is_empty());
            assert!(!control.execution.page_region_succession_pending);
            let children = box_child_nodes(stores, 0);
            let [Node::Kern { amount, .. }] = children.as_slice() else {
                panic!("completed {kind} retains its body");
            };
            assert_eq!(amount.raw(), Scaled::UNITY);
            assert!(!terminal_text(stores).contains("! "));
        });
    }
}

#[test]
fn blocked_page_succession_keeps_empty_box_body_in_the_command_loop() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\hbox{\global\count0=1 \global\count1=2 \kern1pt}\end",
        );
        control.advance(stores).expect("open actual construction");
        control.execution.page_region_succession_pending = true;

        // A request that cannot rotate this live construction must not end
        // the native reader after each assignment. The box close itself is
        // the real host boundary, once its construction authority is gone.
        let mut completed = 0;
        control
            .interpret_chunk(
                stores,
                &mut DiagnosticEffects::new(),
                false,
                &mut None,
                128,
                &mut completed,
            )
            .expect("consume the box body through its actual closing boundary");
        assert_eq!(stores.count(0).expect("first assignment"), 1);
        assert_eq!(stores.count(1).expect("second assignment"), 2);
        assert!(control.execution.boxes.active_boxes.is_empty());
        run_to_end(&mut control, stores);
        assert!(stores.copy_box_to_page(0).is_some());
        let terminal = terminal_text(stores);
        assert!(!terminal.contains("! "), "{terminal}");
    });
}
