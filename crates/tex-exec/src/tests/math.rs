use super::support::{stores_with_fonts, terminal_effect_text};
use super::*;
use tex_command::{
    CommandProfile, FontResource, RegisteredSourceKind, SourceRegistration,
    install_etex_expandable_primitives, install_tex82_expandable_primitives,
};
use tex_state::InputOpenState;
use tex_state::math::{
    FractionThickness, LimitType, MathChoice, MathField, MathListNode, MathNoad, NoadClass,
    NoadKind,
};
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::page::INF_PENALTY;
use tex_state::provenance::{InsertedOriginKind, OriginRecord};
use tex_state::scaled::Scaled;

#[test]
fn null_math_fonts_are_insufficient_for_formula_conversion() {
    assert_eq!(
        crate::math::testing_math_font_failure(&crate::test_harness::universe_with_plain_catcodes()),
        Some("symbol")
    );
}

#[test]
fn missing_extension_fonts_are_distinguished_after_symbol_fonts_validate() {
    let (stores, _) = run_canonical_math_recovery(
        stores_with_fonts(),
        CommandProfile::TEX82,
        r"\font\sym=cmsy10 \relax
          \textfont2=\sym \scriptfont2=\sym \scriptscriptfont2=\sym \end",
        true,
    );

    assert_eq!(
        crate::math::testing_math_font_failure(&stores),
        Some("extension")
    );
}

/// TeX82 §§1194-1195 delete the entire formula after the symbol-family check
/// succeeds but any extension-family size lacks thirteen parameters.  The
/// diagnostic is recoverable: following input still executes normally.
#[test]
fn display_exit_deletes_formula_when_extension_font_is_missing() {
    let (stores, _) = run_canonical_math_recovery(
        stores_with_fonts(),
        CommandProfile::TEX82,
        r"\font\sym=cmsy10 \relax
          \textfont2=\sym \scriptfont2=\sym \scriptscriptfont2=\sym
          \count0=0 $$x$$ \count0=73 \end",
        true,
    );

    let transcript = terminal_effect_text(&stores);
    assert!(transcript.contains("Math formula deleted: Insufficient extension fonts"));
    assert!(transcript.contains("Sorry, but I can't typeset math unless \\textfont 3"));
    assert!(transcript.contains("and \\scriptfont 3 and \\scriptscriptfont 3 have all"));
    assert!(transcript.contains("the \\fontdimen values needed in math extension fonts."));
    assert_eq!(stores.count(0), 73, "following assignment must execute");
}

#[test]
fn char_primitive_scans_as_a_direct_math_field() {
    let (stores, executor) = run_math_source(r"$\mathop\char66");
    let nodes = math_nodes(&stores, &executor);
    let operator = math_noad(&nodes[0]);

    assert!(matches!(operator.nucleus, MathField::MathChar(_)));
}

#[test]
fn mathchar_command_outside_math_inserts_math_shift_and_retries() {
    let (stores, control) = run_canonical_math_recovery(
        stores_with_fonts(),
        CommandProfile::TEX82,
        r#"\mathchardef\circ="020E \circ"#,
        false,
    );

    assert_eq!(control.current_mode(), Mode::Math);
    assert!(terminal_effect_text(&stores).contains("Missing $ inserted"));
    assert_eq!(control.current_list().nodes().len(), 1);
}

#[test]
fn remove_item_commands_apply_to_math_lists() {
    let (stores, executor) =
        run_math_source(r"$\penalty10\unpenalty\kern1pt\unkern\hskip1pt\unskip");

    assert!(math_nodes(&stores, &executor).is_empty());
}

#[test]
fn control_space_in_math_appends_normal_interword_glue() {
    let (stores, executor) = run_math_source("$\\ X");
    let nodes = math_nodes(&stores, &executor);

    assert!(
        matches!(
            nodes,
            [
                Node::Glue {
                    kind: GlueKind::Normal,
                    ..
                },
                ..
            ]
        ),
        "{nodes:?}"
    );
}

#[test]
fn lastbox_in_math_reports_recovery_and_yields_no_node() {
    let (stores, executor) = run_math_source(r"$\lastbox");

    assert!(math_nodes(&stores, &executor).is_empty());
    assert!(terminal_effect_text(&stores).contains("lastbox will be void"));
}

#[test]
fn indent_in_math_appends_an_ord_sub_box() {
    let (stores, executor) = run_math_source(r"$\indent");
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 1);
    assert!(matches!(math_noad(&nodes[0]).nucleus, MathField::SubBox(_)));
}

#[test]
fn text_accent_in_math_uses_mathaccent_semantics() {
    let (stores, executor) = run_math_source(r"\chardef\x=65 $\accent\x a");
    let nodes = math_nodes(&stores, &executor);

    assert!(matches!(math_noad(&nodes[0]).kind, NoadKind::Accent { .. }));
    assert!(terminal_effect_text(&stores).contains("Please use \\mathaccent"));
}

#[test]
fn moveleft_in_math_is_ignored_without_consuming_lastbox() {
    let (stores, executor) = run_math_source(r"$\moveleft\lastbox");

    assert!(math_nodes(&stores, &executor).is_empty());
    let output = terminal_effect_text(&stores);
    assert!(output.contains("moveleft"));
    assert!(output.contains("lastbox will be void"));
}

#[test]
fn halign_in_inline_math_reports_illegal_case_without_scanning_a_preamble() {
    let (stores, executor) = run_math_source(r"$\halign a");
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 1, "the token after \\halign must remain input");
    assert_math_char(&math_noad(&nodes[0]).nucleus, 1, 'a');
    assert!(terminal_effect_text(&stores).contains("You can't use `\\halign' in math mode"));
}

#[test]
fn raw_font_character_dimensions_in_math_do_not_scan_operands() {
    let (stores, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::ETEX26,
        r"$\fontcharwd a\fontcharht b\fontchardp c\fontcharic d",
        false,
    );
    let nodes = control.current_list().nodes();

    assert_eq!(
        nodes.len(),
        4,
        "raw font-character dimensions must leave following tokens in the math list"
    );
    assert_math_char(&math_noad(&nodes[0]).nucleus, 1, 'a');
    assert_math_char(&math_noad(&nodes[1]).nucleus, 1, 'b');
    assert_math_char(&math_noad(&nodes[2]).nucleus, 1, 'c');
    assert_math_char(&math_noad(&nodes[3]).nucleus, 1, 'd');
    let output = terminal_effect_text(&stores);
    for primitive in ["fontcharwd", "fontcharht", "fontchardp", "fontcharic"] {
        assert!(
            output.contains(&format!("You can't use `\\{primitive}' in math mode")),
            "{output}"
        );
    }
}

#[test]
fn vertical_skip_in_math_inserts_math_shift_and_retries() {
    let (stores, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"$\vfill",
        false,
    );

    assert_eq!(control.current_mode(), Mode::Vertical);
    assert!(terminal_effect_text(&stores).contains("Missing $ inserted"));
}

#[test]
fn end_in_math_inserts_math_shift_and_retries() {
    let (stores, control) =
        run_canonical_math_recovery(stores_with_fonts(), CommandProfile::TEX82, r"$x\end", false);

    assert_ne!(control.current_mode(), Mode::Math);
    assert_eq!(stores.world().artifact_commits().len(), 1);
}

fn run_canonical_math_recovery(
    mut stores: Universe,
    profile: CommandProfile,
    source: &str,
    register_symbol_font: bool,
) -> (Universe, CanonicalMainControl) {
    let mut control = match profile {
        CommandProfile::TEX82 => CanonicalMainControl::tex82_initex(&mut stores),
        CommandProfile::ETEX26 => {
            install_tex82_expandable_primitives(&mut stores);
            install_unexpandable_primitives(&mut stores);
            install_etex_expandable_primitives(&mut stores);
            install_etex_unexpandable_primitives(&mut stores);
            CanonicalMainControl::prepared_initex(profile)
        }
        _ => panic!("math recovery helper supports TeX82 and e-TeX only"),
    };
    if let Ok(metrics) = tex_state::InputReadState::read_input_file(
        &mut stores.input_open_context(),
        std::path::Path::new("cmr10.tfm"),
    ) {
        control.capabilities_mut().register_font(
            "cmr10.tfm",
            FontResource::Tfm {
                metrics,
                opentype: None,
            },
        );
    }
    if register_symbol_font {
        let metrics = tex_state::InputReadState::read_input_file(
            &mut stores.input_open_context(),
            std::path::Path::new("cmsy10.tfm"),
        )
        .expect("seeded symbol font fixture reads through the world");
        control.capabilities_mut().register_font(
            "cmsy10.tfm",
            FontResource::Tfm {
                metrics,
                opentype: None,
            },
        );
        let metrics = tex_state::InputReadState::read_input_file(
            &mut stores.input_open_context(),
            std::path::Path::new("cmex10.tfm"),
        )
        .expect("seeded extension font fixture reads through the world");
        control.capabilities_mut().register_font(
            "cmex10.tfm",
            FontResource::Tfm {
                metrics,
                opentype: None,
            },
        );
    }
    run_registered_canonical_math_source(stores, control, source)
}

fn run_registered_canonical_math_source(
    stores: Universe,
    control: CanonicalMainControl,
    source: &str,
) -> (Universe, CanonicalMainControl) {
    run_registered_canonical_math_source_with_limit(stores, control, source, 1024).0
}

fn run_registered_canonical_math_source_with_limit(
    mut stores: Universe,
    mut control: CanonicalMainControl,
    source: &str,
    step_limit: usize,
) -> ((Universe, CanonicalMainControl), usize) {
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("register canonical math-recovery source");
    for step in 1..=step_limit {
        if control
            .step(&mut stores)
            .expect("canonical math-recovery step")
            != MainControlStep::Continue
        {
            return ((stores, control), step);
        }
    }
    panic!("canonical math-recovery source did not stop within {step_limit} steps");
}

#[test]
fn math_mode_builds_noads_styles_choices_and_mu_nodes() {
    let (stores, executor) = run_math_source(
        r"$a_b^c\mathbin+\mathop{x}\limits_y\overline{z}\mskip3mu\mkern2mu\nonscript\displaystyle\mathchoice{d}{t}{s}{u}",
    );
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 9);
    let noad = math_noad(&nodes[0]);
    assert!(matches!(
        noad.kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Ord)
    ));
    assert_math_char(&noad.nucleus, 1, 'a');
    assert_math_char(&noad.subscript, 1, 'b');
    assert_math_char(&noad.superscript, 1, 'c');

    assert!(matches!(
        math_noad(&nodes[1]).kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Bin)
    ));

    let op = math_noad(&nodes[2]);
    assert!(matches!(
        op.kind,
        tex_state::math::NoadKind::Operator(LimitType::Limits)
    ));
    assert_math_char(&op.nucleus, 1, 'x');
    assert_math_char(&op.subscript, 1, 'y');

    let overline = math_noad(&nodes[3]);
    assert!(matches!(overline.kind, tex_state::math::NoadKind::Overline));
    assert_math_char(&overline.nucleus, 1, 'z');

    assert!(matches!(
        nodes[4],
        Node::Glue {
            kind: GlueKind::MuSkip,
            ..
        }
    ));
    assert!(matches!(
        nodes[5],
        Node::Kern {
            kind: KernKind::Mu,
            ..
        }
    ));
    assert!(matches!(
        nodes[6],
        Node::Glue {
            kind: GlueKind::NonScript,
            ..
        }
    ));
    assert!(matches!(
        nodes[7],
        Node::MathStyle(tex_state::math::MathStyle::Display)
    ));

    let Node::MathChoice(MathChoice {
        display,
        text,
        script,
        script_script,
    }) = nodes[8]
    else {
        panic!("expected math choice");
    };
    assert_one_char_list(&stores, display, 'd');
    assert_one_char_list(&stores, text, 't');
    assert_one_char_list(&stores, script, 's');
    assert_one_char_list(&stores, script_script, 'u');
}

#[test]
fn invalid_superscript_command_inserts_a_group_around_following_material() {
    let (stores, executor) = run_math_source(r"^\leaders\vrule\mskip0mu M}");
    let nodes = math_nodes(&stores, &executor);
    let scripted = math_noad(&nodes[0]);
    let MathField::SubMlist(list) = scripted.superscript else {
        panic!("recovered superscript should be a sub-mlist");
    };
    let contains_m = stores.nodes(list).iter().any(|node| {
        matches!(
            node,
            tex_state::node_arena::NodeRef::MathNoad(noad)
                if matches!(
                    noad.nucleus,
                    MathField::MathChar(tex_state::math::MathChar {
                        family: 1,
                        character: 'M',
                        ..
                    })
                )
        )
    });

    assert!(
        contains_m,
        "M must remain inside the recovered superscript group"
    );
    assert!(terminal_effect_text(&stores).contains("Missing { inserted"));
    assert!(terminal_effect_text(&stores).contains("Missing $ inserted"));
}

#[test]
fn limit_switch_applies_to_mathchardef_operator() {
    let (stores, executor) = run_math_source(r#"\mathchardef\op="1352 $\op\nolimits"#);
    let nodes = math_nodes(&stores, &executor);

    let op = math_noad(&nodes[0]);
    assert!(matches!(op.kind, NoadKind::Operator(LimitType::NoLimits)));
    assert_math_char(&op.nucleus, 3, 'R');
    assert!(!terminal_effect_text(&stores).contains("Limit controls must follow"));
}

#[test]
fn generalized_fraction_absorbs_prior_list_and_reports_doubled_fraction() {
    let (mut stores, mut executor) = run_math_source(r"$a\over b\over c");
    let content = executor.finish_current_math_list_for_test(&mut stores);
    let nodes = stores.nodes(content).testing_decoded();

    assert_eq!(nodes.len(), 1);
    let Node::FractionNoad(fraction) = &nodes[0] else {
        panic!("expected fraction noad");
    };
    assert_eq!(fraction.thickness, FractionThickness::Default);
    assert_one_char_list(&stores, fraction.numerator, 'a');
    assert_char_list(&stores, fraction.denominator, &['b', 'c']);
    assert!(
        terminal_effect_text(&stores).contains("! Ambiguous; you need another { and }."),
        "doubled fraction should emit TeX's ambiguity diagnostic"
    );
}

#[test]
fn grouped_fraction_inside_hbox_keeps_box_brace_accounting_balanced() {
    let stores = super::core::run_canonical_tex82(
        r"\setbox0=\hbox{${a+b\over c+d}$}\setbox1=\hbox{$x$}\end",
    );

    let Some(box0) = stores.box_reg(0) else {
        panic!("first hbox should be assigned");
    };
    assert!(
        matches!(stores.nodes(box0).testing_decoded(), [Node::HList(_)]),
        "first hbox should be stored as an hlist"
    );
    let Some(box1) = stores.box_reg(1) else {
        panic!("following hbox should still parse after grouped math");
    };
    assert!(
        matches!(stores.nodes(box1).testing_decoded(), [Node::HList(_)]),
        "second hbox should be stored as an hlist"
    );
}

#[test]
fn semi_simple_groups_execute_assignments_and_aftergroup_in_math_mode() {
    let stores = super::core::run_canonical_tex82(
        r"\def\after{\global\count2=7}\count0=1\count1=1$\begingroup\count0=2\global\count1=3\aftergroup\after\endgroup$\end",
    );

    assert_eq!(stores.count(0), 1, "local assignment should be restored");
    assert_eq!(stores.count(1), 3, "global assignment should survive");
    assert_eq!(
        stores.count(2),
        7,
        "aftergroup token should replay in math mode"
    );
}

#[test]
fn token_register_macros_resume_expansion_in_math_mode() {
    let stores = super::core::run_canonical_tex82(
        r"\def\fromtoks{\global\count0=7\relax}\toks0={\fromtoks}$\the\toks0 x$\end",
    );

    assert_eq!(stores.count(0), 7);
}

#[test]
fn semi_simple_math_aftergroup_replay_has_aftergroup_provenance() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            br"$\begingroup\aftergroup\input\endgroup".to_vec(),
        ))
        .expect("register semi-simple aftergroup source");
    let err = (0..32)
        .find_map(|_| control.advance(&mut stores).err())
        .expect("replayed input without a file name should fail");
    let origin = err
        .diagnostic_site()
        .primary_origin()
        .expect("replayed token origin");
    let OriginRecord::Inserted(inserted) = stores.origin(origin) else {
        panic!("aftergroup replay should have inserted provenance");
    };
    assert_eq!(inserted.kind(), InsertedOriginKind::AfterGroup);
    assert_ne!(inserted.parent(), OriginId::UNKNOWN);
}

#[test]
fn math_shift_groups_restore_locals_keep_globals_and_reset_fam_per_formula() {
    let stores = super::core::run_canonical_tex82(
        r"\fam=7 \count0=1 \count1=1
          $\fam=4 \count0=2 \global\count1=3$
          \count2=\fam
          $\global\count3=\fam$\end",
    );

    assert_eq!(stores.int_param(IntParam::FAM), 7);
    assert_eq!(stores.count(0), 1, "local formula assignment restores");
    assert_eq!(stores.count(1), 3, "global formula assignment survives");
    assert_eq!(stores.count(2), 7, "outer fam is restored after math");
    assert_eq!(stores.count(3), -1, "the next formula resets fam to -1");
}

#[test]
fn math_shift_groups_restore_code_tables_and_replay_aftergroup_after_restore() {
    let stores = super::core::run_canonical_tex82(
        r#"\fam=8 \mathcode`x="7131
            \def\after{\global\count4=\fam}
            $\mathcode`x="7231 \global\mathcode`y="7332 \aftergroup\after$\end"#,
    );

    assert_eq!(stores.mathcode('x'), 0x7131);
    assert_eq!(stores.mathcode('y'), 0x7332);
    assert_eq!(stores.count(4), 8, "aftergroup runs after fam restoration");
}

#[test]
fn math_shift_aftergroup_replay_has_inserted_provenance() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            br"$\aftergroup\input$".to_vec(),
        ))
        .expect("register math-shift aftergroup source");
    let err = (0..32)
        .find_map(|_| control.advance(&mut stores).err())
        .expect("math-shift aftergroup token should replay");
    let origin = err
        .diagnostic_site()
        .primary_origin()
        .expect("replayed token origin");
    let OriginRecord::Inserted(inserted) = stores.origin(origin) else {
        panic!("math-shift aftergroup replay should have inserted provenance");
    };
    assert_eq!(inserted.kind(), InsertedOriginKind::AfterGroup);
    assert_ne!(inserted.parent(), OriginId::UNKNOWN);
}

#[test]
fn math_shift_group_replay_converges_after_snapshot_rollback() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_int_param(IntParam::FAM, 6);
    let mut stores = super::core::run_canonical_tex82_with_universe(
        stores,
        r"\def\after{\global\count1=\fam}\end",
    );
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    let source = r#"\count0=4 $\count0=9 \mathcode`x="7231
                     \aftergroup\after$"#;
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("register canonical math replay source");
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("math replay state checkpoints");
    while control.step(&mut stores).expect("first math replay step") == MainControlStep::Continue {}
    let first_hash = stores.testing_state_hash();
    assert_eq!(stores.count(0), 4);
    assert_eq!(stores.count(1), 6);

    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("math replay state restores");
    while control.step(&mut stores).expect("second math replay step") == MainControlStep::Continue {
    }
    assert_eq!(stores.testing_state_hash(), first_hash);
}

#[test]
fn inline_math_uses_local_layout_parameters_before_restoring_them() {
    let (stores, nodes) =
        super::core::run_canonical_tex82_current_list(r"\mathsurround=2pt $\mathsurround=7pt a$");

    assert_eq!(
        stores.dimen_param(DimenParam::MATH_SURROUND).raw(),
        2 * Scaled::UNITY
    );
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node, Node::MathOn(width) if width.raw() == 7 * Scaled::UNITY))
    );
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node, Node::MathOff(width) if width.raw() == 7 * Scaled::UNITY))
    );
}

#[test]
fn plain_active_prime_shape_closes_brace_alias_math_field() {
    let (stores, executor) = run_math_source(
        r"\let\bgroup={\let\egroup=}\def\prime{p}\def\prim@s{\prime\futurelet\next\pr@m@s}\def\pr@m@s{\let\nxt\egroup\nxt}$x^\bgroup\prim@s",
    );
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 1);
    let noad = math_noad(&nodes[0]);
    assert_math_char(&noad.nucleus, 1, 'x');
    assert_math_char(&noad.superscript, 1, 'p');
}

#[test]
fn math_field_groups_remove_braces_around_single_unscripted_ord_box() {
    let (stores, executor) = run_math_source(r"$\mathopen{{\hbox{}}}");
    let nodes = math_nodes(&stores, &executor);

    let [node] = nodes else {
        panic!("expected one math-open noad")
    };
    let noad = math_noad(node);
    assert!(matches!(noad.kind, NoadKind::Normal(NoadClass::Open)));
    let MathField::SubBox(list) = noad.nucleus else {
        panic!("TeX's math-group simplification should expose the hbox nucleus")
    };
    assert!(matches!(
        stores.nodes(list).testing_decoded(),
        [Node::HList(_)]
    ));
}

#[test]
fn bare_math_brace_nucleus_owns_both_following_scripts() {
    let (stores, nodes) = super::core::run_canonical_tex82_current_list(r"${ab}^c_d");

    let [Node::MathNoad(noad)] = nodes.as_slice() else {
        panic!("a bare braced formula and its scripts should make one Ord noad")
    };
    assert!(matches!(noad.kind, NoadKind::Normal(NoadClass::Ord)));
    let MathField::SubMlist(nucleus) = noad.nucleus else {
        panic!("the bare brace should remain the scripted noad's nucleus")
    };
    let nucleus = stores.nodes(nucleus).testing_decoded();
    assert_eq!(nucleus.len(), 2);
    assert_math_char(&math_noad(&nucleus[0]).nucleus, 1, 'a');
    assert_math_char(&math_noad(&nucleus[1]).nucleus, 1, 'b');
    assert_math_char(&noad.superscript, 1, 'c');
    assert_math_char(&noad.subscript, 1, 'd');
}

#[test]
fn canonical_display_entry_publishes_paragraph_geometry() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(tex_command::SourceRegistration::new(
            tex_command::RegisteredSourceKind::Generated,
            br"\hsize=100pt \hangindent=20pt \hangafter=1 x$$ $$\end".to_vec(),
        ))
        .expect("register display geometry source");
    for _ in 0..128 {
        control.step(&mut stores).expect("enter canonical display");
        if control.current_mode() == Mode::DisplayMath {
            break;
        }
    }
    assert_eq!(control.current_mode(), Mode::DisplayMath);

    assert_eq!(
        stores.dimen_param(DimenParam::DISPLAY_WIDTH).raw(),
        80 * Scaled::UNITY
    );
    assert_eq!(
        stores.dimen_param(DimenParam::DISPLAY_INDENT).raw(),
        20 * Scaled::UNITY
    );
    assert_ne!(
        stores.dimen_param(DimenParam::PRE_DISPLAY_SIZE),
        Scaled::from_raw(-Scaled::MAX_DIMEN.raw()),
        "a nonempty interrupted paragraph publishes its last visible position"
    );

    for _ in 0..128 {
        if control.step(&mut stores).expect("finish canonical display") == MainControlStep::End {
            break;
        }
    }
    assert_eq!(
        stores.dimen_param(DimenParam::DISPLAY_WIDTH),
        Scaled::from_raw(0)
    );
    assert_eq!(
        stores.dimen_param(DimenParam::DISPLAY_INDENT),
        Scaled::from_raw(0)
    );
    assert_eq!(
        stores.dimen_param(DimenParam::PRE_DISPLAY_SIZE),
        Scaled::from_raw(0)
    );
}

#[test]
fn math_atom_group_around_accent_replaces_the_ord_wrapper() {
    let (stores, executor) = run_math_source(r#"${\mathaccent"7013 y}"#);
    let nodes = math_nodes(&stores, &executor);

    let [Node::MathNoad(noad)] = nodes else {
        panic!("grouped accent should remain one noad")
    };
    assert!(matches!(noad.kind, NoadKind::Accent { .. }));
}

#[test]
fn mathaccent_skips_relax_before_its_nucleus() {
    let (stores, executor) = run_math_source(r#"$\mathaccent"7013\relax a"#);
    let nodes = math_nodes(&stores, &executor);

    let [Node::MathNoad(noad)] = nodes else {
        panic!("the accent and its nucleus should form one noad")
    };
    assert!(matches!(noad.kind, NoadKind::Accent { .. }));
    assert_math_char(&noad.nucleus, 1, 'a');
}

#[test]
fn math_group_mismatch_reports_the_closing_token_origin() {
    let stores = super::core::run_canonical_tex82(r"$\begingroup}\end");
    let output = terminal_effect_text(&stores);
    assert!(output.contains("Extra }, or forgotten \\endgroup"));
    assert!(output.contains("l.1 $\\begingroup}"));

    let stores = super::core::run_canonical_tex82(r"$\endgroup\end");
    let output = terminal_effect_text(&stores);
    assert!(output.contains("Extra \\endgroup"));
    assert!(output.contains("l.1 $\\endgroup"));

    let stores = super::core::run_canonical_tex82(r"$}\end");
    assert!(support::terminal_effect_text(&stores).contains("Extra }, or forgotten $"));

    let stores = super::core::run_canonical_tex82(r"$\begingroup$\end");
    assert!(support::terminal_effect_text(&stores).contains("Missing \\endgroup inserted"));
}

#[test]
fn inline_math_entry_lookahead_preserves_source_origin() {
    assert_replayed_math_error_is_source_backed(r"$\noexpand\input");
}

#[test]
fn mismatched_display_closer_preserves_following_source_origin() {
    assert_replayed_math_error_is_source_backed(r"\noindent$$a$\noexpand\input");
}

#[test]
fn post_display_replay_preserves_following_source_origin() {
    assert_replayed_math_error_is_source_backed(r"\noindent$$a$$\noexpand\input");
}

#[test]
fn post_display_alignment_replay_preserves_following_source_origin() {
    assert_replayed_math_error_is_source_backed(r"\noindent$$\halign{#\cr a\cr}$$\noexpand\input");
}

#[test]
fn equation_number_math_shift_group_restores_before_outer_display_group() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_int_param(IntParam::FAM, 9);
    stores.set_count(0, 1);
    let (stores, control) = run_canonical_math_recovery(
        stores,
        CommandProfile::TEX82,
        r"\noindent $$\count0=2 a\eqno\count0=3 b$$",
        false,
    );

    assert_eq!(stores.count(0), 1);
    assert_eq!(stores.int_param(IntParam::FAM), 9);
    assert_eq!(control.current_mode(), Mode::Horizontal);
}

#[test]
fn equation_number_uses_a_checkpointable_nested_math_level() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.enter_group_with_kind(tex_state::GroupKind::MathShift);
    let mut nest = ModeNest::new();
    nest.push(Mode::DisplayMath).expect("test mode push");

    crate::math::testing_start_eq_no(&mut nest, &mut stores, UnexpandablePrimitive::EqNo)
        .expect("equation number should enter ordinary math");

    assert_eq!(nest.depth(), 3);
    assert_eq!(nest.current_mode(), Mode::Math);
    assert!(nest.current_list().display_eq_no().is_some());
    let summary = nest.summary();
    assert_eq!(
        ModeNest::from_summary(summary.clone())
            .expect("equation-number mode summary should restore")
            .summary(),
        summary
    );
    assert_eq!(tex_state::ExpansionState::execution_group_depth(&stores), 2);
}

#[test]
fn equation_number_aftergroup_runs_after_its_nested_math_group_closes() {
    let (stores, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"\def\mark{\global\count1=7}\noindent $$a\eqno\aftergroup\mark b$$",
        false,
    );

    assert_eq!(stores.count(1), 7);
    assert_eq!(control.current_mode(), Mode::Horizontal);
    assert_eq!(tex_state::ExpansionState::execution_group_depth(&stores), 0);
}

#[test]
fn equation_number_expands_the_outer_display_closer() {
    let (stores, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"\def\close{$}\noindent $$a\eqno b$\close\count1=7",
        false,
    );

    assert_eq!(stores.count(1), 7);
    assert_eq!(control.current_mode(), Mode::Horizontal);
    assert!(!terminal_effect_text(&stores).contains("Display math should end with $$"));
}

#[test]
fn equation_number_outside_display_reports_illegal_case_without_starting_math() {
    let (stores, _) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"\eqno x$\count0=7",
        false,
    );

    assert_eq!(stores.count(0), 7);
    let output = support::terminal_effect_text(&stores);
    assert!(!output.contains("Missing $ inserted"));
    assert!(output.contains("You can't use `\\eqno' in vertical mode"));
}

#[test]
fn math_shift_inserts_endgroup_for_open_semisimple_group() {
    let (stores, _) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"\count0=1 $x\begingroup\count0=2$\count1=3",
        false,
    );

    assert_eq!(stores.count(0), 1);
    assert_eq!(stores.count(1), 3);
    assert!(support::terminal_effect_text(&stores).contains("Missing \\endgroup inserted"));
}

#[test]
fn math_shift_inserts_right_brace_for_open_simple_group() {
    let (stores, _) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"\count0=1 $x{\count0=2$\count1=3",
        false,
    );

    assert_eq!(stores.count(0), 1);
    assert_eq!(stores.count(1), 3);
    assert!(support::terminal_effect_text(&stores).contains("Missing } inserted"));
}

#[test]
fn vadjust_is_accepted_in_math_mode() {
    let (_, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"$x\vadjust{\penalty7}\prevgraf=8 \insert255{\penalty9}y",
        false,
    );

    assert_eq!(control.current_mode(), Mode::Math);
}

#[test]
fn vcenter_accepts_a_spread_pack_specification() {
    let (_, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"$\vcenter spread -2pt{}",
        false,
    );

    assert!(matches!(
        math_noad(&control.current_list().nodes()[0]).kind,
        NoadKind::VCenter
    ));
}

#[test]
fn vcenter_accepts_a_begin_group_control_sequence_alias() {
    let (_, control) = run_canonical_math_recovery(
        stores_with_fonts(),
        CommandProfile::TEX82,
        r"\let\bgroup={\let\egroup=}$\vcenter\bgroup\hrule\egroup",
        false,
    );

    assert!(matches!(
        math_noad(&control.current_list().nodes()[0]).kind,
        NoadKind::VCenter
    ));
}

#[test]
fn char_primitive_uses_the_characters_mathcode_in_math_mode() {
    let (_, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"$\char`+",
        false,
    );

    assert_math_char(
        &math_noad(&control.current_list().nodes()[0]).nucleus,
        0,
        '+',
    );
}

#[test]
fn explicit_kern_is_accepted_in_math_mode() {
    let (_, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"$x\kern1pt y",
        false,
    );

    assert!(matches!(
        control.current_list().nodes()[1],
        Node::Kern { amount, kind: KernKind::Explicit } if amount == Scaled::from_raw(65_536)
    ));
}

#[test]
fn italic_correction_in_math_appends_a_zero_kern() {
    let (_, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"$x\/",
        false,
    );

    assert!(matches!(
        control.current_list().nodes()[1],
        Node::Kern { amount, kind: KernKind::Font } if amount == Scaled::from_raw(0)
    ));
}

#[test]
fn math_discretionary_deletes_a_nonempty_replacement_part() {
    let (stores, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"$\discretionary{\kern1pt}{\kern2pt}{\kern3pt}\kern4pt",
        false,
    );

    let transcript = support::terminal_effect_text(&stores);
    assert!(transcript.contains("Illegal math \\discretionary"));
    assert!(transcript.contains("I had to delete your third part."));
    let [
        Node::Disc {
            pre, post, replace, ..
        },
        Node::Kern { amount, .. },
    ] = control.current_list().nodes()
    else {
        panic!("recovered discretionary and following input must survive")
    };
    assert_eq!(stores.nodes(*pre).len(), 1);
    assert_eq!(stores.nodes(*post).len(), 1);
    assert!(stores.nodes(*replace).is_empty());
    assert_eq!(*amount, Scaled::from_raw(4 * Scaled::UNITY));
}

#[test]
fn math_discretionary_diagnostic_respects_escapechar() {
    let (stores, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"\escapechar=`! $\discretionary{}{}{\kern1pt}",
        false,
    );

    let transcript = support::terminal_effect_text(&stores);
    assert!(transcript.contains("Illegal math !discretionary"));
    let [Node::Disc { replace, .. }] = control.current_list().nodes() else {
        panic!("recovered discretionary must survive")
    };
    assert!(stores.nodes(*replace).is_empty());
}

#[test]
fn vrule_is_accepted_in_math_mode() {
    let (_, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"$\vrule height 9pt",
        false,
    );

    assert!(matches!(
        control.current_list().nodes(),
        [Node::Rule { .. }]
    ));
}

#[test]
fn spacefactor_in_math_reports_illegal_case_without_scanning_an_assignment() {
    let (stores, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"$\spacefactor1",
        false,
    );

    assert!(support::terminal_effect_text(&stores).contains("You can't use `\\spacefactor'"));
    assert_math_char(
        &math_noad(&control.current_list().nodes()[0]).nucleus,
        0,
        '1',
    );
}

#[test]
fn misplaced_alignment_commands_and_mark_recover_in_math_mode() {
    let (stores, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"$\span\omit\mark{a}\cr",
        false,
    );

    let output = support::terminal_effect_text(&stores);
    assert!(output.contains("Misplaced \\span"));
    assert!(output.contains("Misplaced \\omit"));
    assert!(output.contains("Misplaced \\cr"));
    assert!(matches!(
        control.current_list().nodes(),
        [Node::Mark { .. }]
    ));
}

#[test]
fn math_brace_groups_restore_local_box_assignments_and_keep_globals() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let baseline = stores.freeze_node_list(&[Node::Kern {
        amount: tex_state::scaled::Scaled::from_raw(17),
        kind: KernKind::Explicit,
    }]);
    stores.set_box_reg(0, baseline);

    let stores = super::core::run_canonical_tex82_with_universe(
        stores,
        r"${\setbox0=\hbox{x}\global\setbox1=\hbox{y}}$\end",
    );

    let restored = stores.box_reg(0).expect("local box should be restored");
    assert!(matches!(
        stores.nodes(restored).testing_decoded(),
        [Node::Kern { amount, kind: KernKind::Explicit }] if amount.raw() == 17
    ));
    assert!(stores.box_reg(1).is_some(), "global box should survive");
}

#[test]
fn explicit_groups_in_math_restore_local_box_assignments_and_keep_globals() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let baseline = stores.freeze_node_list(&[Node::Kern {
        amount: tex_state::scaled::Scaled::from_raw(23),
        kind: KernKind::Explicit,
    }]);
    stores.set_box_reg(0, baseline);

    let stores = super::core::run_canonical_tex82_with_universe(
        stores,
        r"$\begingroup\setbox0=\hbox{x}\global\setbox1=\hbox{y}\endgroup$\end",
    );

    let restored = stores.box_reg(0).expect("local box should be restored");
    assert!(matches!(
        stores.nodes(restored).testing_decoded(),
        [Node::Kern { amount, kind: KernKind::Explicit }] if amount.raw() == 23
    ));
    assert!(stores.box_reg(1).is_some(), "global box should survive");
}

#[test]
fn penalty_builds_ordinary_list_material_in_inline_math() {
    let (_, nodes) = super::core::run_canonical_tex82_current_list(r"$a\penalty123 b");

    assert!(matches!(
        nodes.as_slice(),
        [Node::MathNoad(_), Node::Penalty(123), Node::MathNoad(_)]
    ));
}

#[test]
fn penalty_builds_ordinary_list_material_in_display_math() {
    let (_, control) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"\noindent$$a\penalty456 b",
        false,
    );

    assert_eq!(control.current_mode(), Mode::DisplayMath);
    assert!(matches!(
        control.current_list().nodes(),
        [Node::MathNoad(_), Node::Penalty(456), Node::MathNoad(_)]
    ));
}

#[test]
fn forced_postdisplay_penalty_builds_page_after_horizontal_resume() {
    let mut stores = support::stores_with_fonts();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    let metrics = tex_state::InputReadState::read_input_file(
        &mut stores.input_open_context(),
        std::path::Path::new("cmr10.tfm"),
    )
    .expect("seeded font fixture reads");
    control.capabilities_mut().register_font(
        "cmr10.tfm",
        FontResource::Tfm {
            metrics,
            opentype: None,
        },
    );
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"\\font\\f=cmr10 \\f \\hsize=50pt \\postdisplaypenalty=-10000 \
              \\noindent x$$x$$"
                .to_vec(),
        ))
        .expect("register canonical post-display source");
    for _ in 0..1024 {
        assert_eq!(
            control.step(&mut stores).expect("canonical step"),
            MainControlStep::Continue,
            "forced display penalty must fire before source exhaustion"
        );
        if !stores.world().artifact_commits().is_empty() {
            break;
        }
    }

    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert_eq!(control.take_prepared_dvi_pages().len(), 1);
    assert!(matches!(
        stores.page_contribution_front(),
        Some(Node::Penalty(value)) if *value == INF_PENALTY
    ));
    assert!(
        stores
            .page_contributions()
            .iter()
            .any(|node| matches!(node, Node::Penalty(-10_000))),
        "the later forced post-display penalty waits for the next builder invocation"
    );
}

#[test]
fn lowered_math_box_rolls_back_without_leaking_arena_handles() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let baseline = stores.freeze_node_list(&[Node::Kern {
        amount: tex_state::scaled::Scaled::from_raw(17),
        kind: KernKind::Explicit,
    }]);
    stores.set_box_reg(0, baseline);
    let snapshot = stores.snapshot();
    let before = snapshot.state_hash();

    let mut stores = super::core::run_canonical_tex82_with_universe(
        stores,
        r"\setbox0=\hbox{$\left({a+b\over c_d^e}\right)$}\end",
    );
    let converted = stores.box_reg(0).expect("converted box should be assigned");
    assert_ne!(converted, baseline);
    assert_ne!(stores.snapshot().state_hash(), before);

    stores.rollback(&snapshot);

    assert_eq!(stores.snapshot().state_hash(), before);
    let restored = stores.box_reg(0).expect("baseline box should be restored");
    assert!(matches!(
        stores.nodes(restored).testing_decoded(),
        [Node::Kern { amount, kind: KernKind::Explicit }] if amount.raw() == 17
    ));
}

#[test]
fn mathcode_8000_uses_current_active_meaning_and_fam_overrides_variable_family() {
    let run = |source: &str| {
        let mut stores = crate::test_harness::universe_with_plain_catcodes();
        stores.set_mathcode('?', 0x8000);
        let active_question = stores.intern_active_character('?');
        stores.set_meaning(active_question, Meaning::MathCharGiven(0x0231));
        run_canonical_math_recovery(stores, CommandProfile::TEX82, source, false)
    };

    let (_, first_control) = run(r#"\mathcode`x="7131 $?"#);
    let first = first_control.current_list().nodes();
    assert_eq!(first.len(), 1);
    assert_math_char(&math_noad(&first[0]).nucleus, 2, '1');

    let (_, second_control) = run(r#"\mathcode`x="7131 $\fam=5 x"#);
    let second = second_control.current_list().nodes();
    assert_eq!(second.len(), 1);
    assert_math_char(&math_noad(&second[0]).nucleus, 5, '1');

    let (_, third_control) = run(r#"\mathcode`x="7131 $x^?"#);
    let third = third_control.current_list().nodes();
    assert_eq!(third.len(), 1);
    assert_math_char(&math_noad(&third[0]).nucleus, 1, '1');
    assert_math_char(&math_noad(&third[0]).superscript, 2, '1');
}

#[test]
fn initex_letter_mathcodes_use_variable_family_one_and_honor_fam() {
    let (default_stores, default_executor) = run_math_source(r"$a");
    assert_eq!(default_stores.mathcode('a'), 0x7161);
    assert_eq!(default_stores.mathcode('S'), 0x7153);
    let default = math_nodes(&default_stores, &default_executor);
    assert_math_char(&math_noad(&default[0]).nucleus, 1, 'a');

    let (overridden_stores, overridden_executor) = run_math_source(r"$\fam=2 S");
    let overridden = math_nodes(&overridden_stores, &overridden_executor);
    assert_math_char(&math_noad(&overridden[0]).nucleus, 2, 'S');
}

#[test]
fn showlists_reports_unfinished_math_noad_fields() {
    let (stores, _) = run_math_source(r"$a_b^c\mathchoice{d}{t}{s}{u}\showlists$");
    let log = terminal_effect_text(&stores);

    assert!(log.contains("### math mode entered at line 1"), "{log}");
    assert!(log.contains("\\mathord"));
    assert!(log.contains(".\\fam1 a"));
    assert!(log.contains("^\\fam1 c"));
    assert!(log.contains("_\\fam1 b"));
    assert!(log.contains("\\mathchoice"));
}

#[test]
fn showlists_reports_adjustments_embedded_in_a_display_math_list() {
    // TeX82 §§182/692: `show_node_list` retains subsidiary prefixes while
    // walking a noad, then returns to the enclosing mlist for generic nodes.
    // In particular, §1099's adjustment wrapper owns its vertical body; the
    // temporary internal-vmode list used to build that body is not a nest
    // level visible to the later §218 `\showlists`.
    let stores = super::core::run_canonical_tex82(
        r"\nonstopmode\showboxdepth=10\showboxbreadth=100$$A^{\hbox{\kern1pt}}_{B-}\vadjust{\penalty7}\mkern-9mu\the\prevgraf \prevgraf=8 \insert255{\penalty999}\showlists$$\end",
    );
    let log = terminal_effect_text(&stores);

    assert!(
        log.contains(concat!(
            "### display math mode entered at line 1\n",
            "\\mathord\n",
            ".\\fam1 A\n",
            "^\\hbox(0.0+0.0)x1.0\n",
            "^.\\kern 1.0\n",
            "_\\mathord\n",
            "_.\\fam1 B\n",
            "_\\mathord\n",
            "_.\\fam0 -\n",
            "\\vadjust\n",
            ".\\penalty 7\n",
            "\\mkern-9.0mu\n",
        )),
        "{log}"
    );
    assert!(!log.contains("### internal vertical mode entered"), "{log}");
    assert!(
        log.contains("\\insert0, natural size 0.0; split(0.0,0.0); float cost 0\n.\\penalty 999"),
        "{log}"
    );
}

#[test]
fn showlists_roots_an_unfinished_display_while_scanning_its_equation_number() {
    // TeX82 §§1194/218: `fin_mlist(null)` saves the display while the
    // equation number is scanned, but `show_activities` still visits that
    // display through its live display-mode list root. The nested braced
    // fields ensure the equation-number math level is not current when the
    // diagnostic walks back to the display level.
    let stores = super::core::run_canonical_tex82(
        r"\nonstopmode\showboxdepth=10\showboxbreadth=100$$\vadjust{\penalty7}\mkern-9mu\eqno\mathpunct{AA}^{\hbox{A}}{\above9pt{v\over p\showlists q}}$$\end",
    );
    let log = terminal_effect_text(&stores);

    assert!(
        log.contains(concat!(
            "### display math mode entered at line 1\n",
            "\\vadjust\n",
            ".\\penalty 7\n",
            "\\mkern-9.0mu\n",
            "### vertical mode entered at line 0\n",
        )),
        "{log}"
    );
}

/// TeX82 §1151 allocates each ordinary noad before `scan_math` enters a
/// braced subsidiary field. Section 218 therefore displays that parent noad,
/// without a nucleus child, after the nested math level and before the outer
/// display level. The complete §687 ordinary-noad interval proves that the
/// ownership rule is generic rather than specific to `ord_noad`.
#[test]
fn showlists_inside_braced_fields_displays_the_reserved_parent_noad_matrix() {
    for (constructor, noad_name) in [
        (r"\mathord", r"\mathord"),
        (r"\mathop", r"\mathop"),
        (r"\mathbin", r"\mathbin"),
        (r"\mathrel", r"\mathrel"),
        (r"\mathopen", r"\mathopen"),
        (r"\mathclose", r"\mathclose"),
        (r"\mathpunct", r"\mathpunct"),
        (r"\mathinner", r"\mathinner"),
    ] {
        let source = format!(
            r"\nonstopmode\showboxdepth=10\showboxbreadth=10$\displaystyle{constructor}{{\showlists}}$\end"
        );
        let stores = super::core::run_canonical_tex82(&source);
        let log = terminal_effect_text(&stores);
        let expected = format!(
            "### math mode entered at line 1\n### math mode entered at line 1\n\\displaystyle\n{noad_name}\n"
        );
        assert!(log.contains(&expected), "{constructor}: {log}");
    }
}

#[test]
fn showlists_reports_incomplete_fraction_numerator() {
    let stores = super::core::run_canonical_tex82(
        r"\nonstopmode\showboxdepth=10\showboxbreadth=10$a\above1pt\showlists b$\end",
    );
    let log = terminal_effect_text(&stores);

    let numerator = log
        .find("this will begin denominator of:\n")
        .expect("incomplete fraction diagnostic");
    assert!(
        log[numerator..].contains(
            "this will begin denominator of:\n\\fraction, thickness 1.0\n\\\\mathord\n\\.\\fam1 a"
        ),
        "{log}"
    );
}

#[test]
fn showlists_reports_empty_submlist_in_incomplete_fraction_numerator() {
    let stores = super::core::run_canonical_tex82(
        r"\nonstopmode\showboxdepth=10\showboxbreadth=10${}\over\showlists b$\end",
    );
    let log = terminal_effect_text(&stores);

    assert!(
        log.contains(
            "this will begin denominator of:\n\\fraction, thickness = default\n\\\\mathord\n\\.{}"
        ),
        "{log}"
    );
}

#[test]
fn showlists_projects_fraction_across_middle_at_canonical_depths() {
    let stores = super::core::run_canonical_etex(
        r"\nonstopmode\showboxdepth=10\showboxbreadth=10$\left.p\middle.q\over r\showlists\right.\showlists$\end",
    );
    let log = terminal_effect_text(&stores);

    assert!(log.contains(
        "this will begin denominator of:\n\\fraction, thickness = default\n\\\\left\"0\n\\\\mathord\n\\.\\fam1 p\n\\\\middle\"0\n\\\\mathord\n\\.\\fam1 q"
    ), "{log}");
    assert!(log.contains(
        "\\mathinner\n.\\left\"0\n.\\mathord\n..\\fam1 p\n.\\middle\"0\n.\\fraction, thickness = default\n.\\\\mathord\n.\\.\\fam1 q\n./\\mathord\n./.\\fam1 r\n.\\right\"0"
    ), "{log}");
}

#[test]
fn par_in_math_finishes_math_with_tex_error_text() {
    let (stores, executor) = run_math_source(r"$a\par");
    assert_eq!(executor.current_mode(), Mode::Vertical);
    assert!(terminal_effect_text(&stores).contains("! Missing $ inserted."));
}

#[test]
fn left_right_scans_nested_list_as_inner_noad() {
    let (stores, executor) = run_math_source(r"$\left. a \right.");
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 1);
    let inner = math_noad(&nodes[0]);
    assert!(matches!(
        inner.kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Inner)
    ));
    let MathField::SubMlist(list) = inner.nucleus else {
        panic!("expected left/right inner noad to hold a sub-mlist");
    };
    let enclosed = stores.nodes(list).testing_decoded();
    assert!(matches!(
        math_noad(&enclosed[0]).kind,
        tex_state::math::NoadKind::LeftDelimiter { delimiter: 0 }
    ));
    assert_math_char(&math_noad(&enclosed[1]).nucleus, 1, 'a');
    assert!(matches!(
        math_noad(&enclosed[2]).kind,
        tex_state::math::NoadKind::RightDelimiter { delimiter: 0 }
    ));
}

#[test]
fn etex_middle_stays_inside_left_right_and_has_its_own_noad_kind() {
    // e-TeX manual section 3.5: `\middle` is valid only in a matching
    // `\left...\right` group and is sized with those delimiters.
    let (stores, root) = super::core::run_canonical_etex_current_list(r"$\left(a\middle|b\right)");
    let inner = math_noad(&root[0]);
    let MathField::SubMlist(content) = inner.nucleus else {
        panic!("left/right inner noad")
    };
    assert!(stores.nodes(content).testing_decoded().iter().any(|node| {
        matches!(
            node,
            Node::MathNoad(MathNoad {
                kind: NoadKind::MiddleDelimiter { .. },
                ..
            })
        )
    }));
}

/// e-TeX etex.ch [48.1192] handles `middle_noad` only in the
/// `math_left_group` branch.  Its othercases arm consumes the delimiter,
/// reports the middle-specific error/help pair, and otherwise leaves the
/// current mlist unchanged.  In particular, a simple group nested inside a
/// left/right group is not itself a valid `\middle` context.
#[test]
fn canonical_etex_middle_invalid_context_matrix_recovers_exactly() {
    for source in [
        r"\nonstopmode$\middle|a$\end",
        r"\nonstopmode${\middle|a}$\end",
        r"\nonstopmode$\left(a{\middle|b}\right)$\end",
    ] {
        let stores = super::core::run_canonical_etex(source);
        let output = canonical_log_text(&stores);
        let missing = output
            .find("! Missing delimiter (. inserted).")
            .expect("invalid delimiter reports first");
        let extra = output.find("! Extra \\middle.").expect("extra middle");
        assert!(
            missing < extra,
            "wrong recovery order for {source:?}: {output:?}"
        );
        assert!(
            output.contains("\nI'm ignoring a \\middle that had no matching \\left.\n"),
            "wrong recovery help for {source:?}: {output:?}"
        );
        assert_eq!(
            output.matches("! Extra \\middle.").count(),
            1,
            "the consumed delimiter must not be replayed: {output:?}"
        );
    }
}

/// e-TeX etex.ch [48.1192] gives every middle noad the `right_noad` command
/// class.  Thus §687's `scripts_allowed` bound excludes it just as it excludes
/// `\right`: a following script starts a fresh empty Ord noad.  Multiple
/// middle noads remain legal in one left/right group.
#[test]
fn canonical_etex_repeated_middle_and_following_script_preserve_noad_boundaries() {
    let (_, nodes) = super::core::run_canonical_etex_current_list(r"$\left.a\middle.^2b\middle.c");

    assert_eq!(nodes.len(), 7);
    assert!(matches!(
        math_noad(&nodes[0]).kind,
        NoadKind::LeftDelimiter { .. }
    ));
    assert_math_char(&math_noad(&nodes[1]).nucleus, 1, 'a');
    assert!(matches!(
        math_noad(&nodes[2]).kind,
        NoadKind::MiddleDelimiter { .. }
    ));
    let scripted = math_noad(&nodes[3]);
    assert!(matches!(scripted.kind, NoadKind::Normal(NoadClass::Ord)));
    assert!(matches!(scripted.nucleus, MathField::Empty));
    assert_math_char(&scripted.superscript, 0, '2');
    assert_math_char(&math_noad(&nodes[4]).nucleus, 1, 'b');
    assert!(matches!(
        math_noad(&nodes[5]).kind,
        NoadKind::MiddleDelimiter { .. }
    ));
    assert_math_char(&math_noad(&nodes[6]).nucleus, 1, 'c');

    let stores = super::core::run_canonical_etex(r"\nonstopmode$\left.a\middle.b\middle.c$\end");
    let output = canonical_log_text(&stores);
    assert!(output.starts_with("\n! Missing \\right. inserted.\n"));
    assert!(output.contains(
        "\nI've inserted something that you may have forgotten.\n\
         (See the <inserted text> above.)\n\
         With luck, this will get me unwedged. But if you\n\
         really didn't forget anything, try typing `2' now; then\n\
         my insertion and my current dilemma will both disappear.\n"
    ));
    assert!(!output.contains("Extra \\middle"));
}

#[test]
fn etex_display_records_and_resumes_the_interrupted_text_direction() {
    let (stores, nodes) = super::core::run_canonical_etex_current_list(
        r"\TeXXeTstate=1
          \everydisplay{\message{PD=\the\predisplaydirection}}
          \noindent\beginR abc$$x$$def\endR",
    );

    assert!(terminal_effect_text(&stores).contains("PD=-1"));
    let directions: Vec<_> = nodes
        .iter()
        .filter_map(|node| match node {
            Node::Direction(direction) => Some(*direction),
            _ => None,
        })
        .collect();
    assert_eq!(
        directions,
        [
            tex_state::node::Direction::BeginR,
            tex_state::node::Direction::EndR,
        ]
    );
}

#[test]
fn doubled_math_shift_in_internal_vertical_mode_is_a_display() {
    // tex.web §§1090, 1092, and 1138: `new_graf` enters ordinary horizontal
    // mode even from an internal vlist, so doubled `$` opens display math and
    // `\ifinner` is false.
    // The probes are registers rather than `\message`s: §1195's math-font
    // report is unavoidable without loaded families, and §82's context display
    // echoes the source line, so any probe text would also appear in it.
    let stores = super::core::run_canonical_etex(
        r"\everypar{\global\count2=1}\setbox0=\vbox{$$\ifinner\global\count1=1\else\global\count1=2\fi$$}\end",
    );

    assert_eq!(stores.count(1), 2);
    assert_eq!(stores.count(2), 1);
}

#[test]
fn right_closes_left_group_whose_numerator_was_captured_by_fraction() {
    let (stores, executor) = run_math_source(r"$\left.A\over A\abovewithdelims.?\right(+A");

    assert_eq!(executor.current_mode(), Mode::Math);
    let transcript = support::terminal_effect_text(&stores);
    assert!(!transcript.contains("Extra \\right"));
    // TeX82 §§1160--1161 and 1182: `.` is the null delimiter, while the
    // invalid `?` is backed up and diagnosed before §448 starts the explicit
    // thickness scan. That scan therefore sees the same `?` and takes §446's
    // missing-number path only after the delimiter report.
    let missing_delimiter = transcript
        .find("! Missing delimiter (. inserted).")
        .expect("invalid right fraction delimiter is diagnosed");
    let missing_number = transcript
        .find("! Missing number, treated as zero.")
        .expect("thickness reports its vacuous numeric constant");
    assert!(missing_delimiter < missing_number, "{transcript}");
    let nodes = math_nodes(&stores, &executor);
    let inner = math_noad(&nodes[0]);
    let MathField::SubMlist(list) = inner.nucleus else {
        panic!("expected left/right inner noad to hold a sub-mlist");
    };
    let enclosed = stores.nodes(list).testing_decoded();
    assert!(matches!(
        math_noad(&enclosed[0]).kind,
        NoadKind::LeftDelimiter { .. }
    ));
    assert!(matches!(enclosed[1], Node::FractionNoad(_)));
    assert!(matches!(
        math_noad(&enclosed[2]).kind,
        NoadKind::RightDelimiter { .. }
    ));
}

#[test]
fn mismatched_right_and_missing_right_use_tex_error_text() {
    let (extra_stores, extra_executor) = run_math_source(r"$a\right.");
    let extra_nodes = math_nodes(&extra_stores, &extra_executor);
    assert_eq!(extra_nodes.len(), 1);
    assert_math_char(&math_noad(&extra_nodes[0]).nucleus, 1, 'a');
    assert!(terminal_effect_text(&extra_stores).contains("! Extra \\right."));

    let (missing_stores, missing_executor) = run_math_source(r"$\left. a$");
    assert_eq!(missing_executor.current_mode(), Mode::Horizontal);
    assert!(
        missing_executor
            .current_list()
            .nodes()
            .iter()
            .any(|node| matches!(node, Node::MathOn(_)))
    );
    assert!(
        terminal_effect_text(&missing_stores).contains("! Missing \\right. inserted."),
        "missing right delimiter should use reference primary wording"
    );
}

#[test]
fn inline_math_finishing_emits_mathsurround_markers_and_penalties() {
    let (mut stores, executor) = run_math_source_with_text_math_fonts(
        r"\mathsurround=3pt \binoppenalty=700 \relpenalty=500 $a\mathbin+b\mathrel=c",
    );
    let list = unfinished_math_list(&mut stores, &executor);

    let nodes = crate::math::finish_math_list_node(&mut stores, list, true);

    assert!(matches!(
        nodes.first(),
        Some(Node::MathOn(width)) if width.raw() == 3 * tex_state::scaled::Scaled::UNITY
    ));
    assert!(matches!(
        nodes.last(),
        Some(Node::MathOff(width)) if width.raw() == 3 * tex_state::scaled::Scaled::UNITY
    ));
    assert!(
        nodes.iter().any(|node| matches!(node, Node::Penalty(700))),
        "binoppenalty should be inserted in outer inline conversion"
    );
    assert!(
        nodes.iter().any(|node| matches!(node, Node::Penalty(500))),
        "relpenalty should be inserted in outer inline conversion"
    );
    assert!(
        nodes.iter().all(|node| !matches!(node, Node::MathList(_))),
        "paragraph line breaking must see converted hlist nodes"
    );
}

/// TeX82 §§722--724 and §755: `fetch` calls `char_warning` for a character
/// absent from a defined math font, drops that character, and continues the
/// surrounding formula. See umber2-e51h.63.7 for the missing conversion-event
/// boundary needed to make the warning observable outside `tex-typeset`.
#[test]
fn missing_math_character_reports_canonical_warning_and_omits_only_character() {
    // TeX82 §445 recognizes uppercase A--F in hexadecimal constants.
    let (mut stores, executor) = run_math_source_with_text_math_fonts("$a\\mathchar\"007F b");
    let list = unfinished_math_list(&mut stores, &executor);

    let nodes = crate::math::finish_math_list_node(&mut stores, list, false);

    let characters: Vec<_> = nodes
        .iter()
        .filter_map(|node| match node {
            Node::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert_eq!(characters, ['a', 'b']);
    let terminal = terminal_effect_text(&stores);
    assert!(
        terminal.contains("Missing character: There is no ^^? in font cmr10!"),
        "terminal output was {terminal:?}"
    );
}

/// TeX82 §§722--723: selecting nullfont through an undefined family resets
/// only that math field to empty and continues converting its siblings.
#[test]
fn undefined_math_family_reports_error_and_omits_only_character() {
    // TeX82 §445 recognizes uppercase A--F in hexadecimal constants.
    let (mut stores, executor) = run_math_source_with_text_math_fonts("$a\\mathchar\"0F61 b");
    let list = unfinished_math_list(&mut stores, &executor);

    let nodes = crate::math::finish_math_list_node(&mut stores, list, false);

    let characters = nodes
        .iter()
        .filter_map(|node| match node {
            Node::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(characters, ['a', 'b']);
    assert!(terminal_effect_text(&stores).contains("\\textfont 15 is undefined (character a)"));
}

/// TeX82 §§82/721: Appendix G reports an undefined family only after the
/// formula has ended, but `error` still shows the live command input at that
/// closing math shift.  All three size selectors share this boundary.
#[test]
fn undefined_math_family_reports_closing_source_context_before_help() {
    let cases = [
        (r#"\mathchar"0F61"#, "\\textfont"),
        (r#"x_{\mathchar"0F61}"#, "\\scriptfont"),
        (r#"x_{y_{\mathchar"0F61}}"#, "\\scriptscriptfont"),
    ];
    for (formula, selector) in cases {
        let source = format!(
            "\\font\\roman=cmr10 \\font\\symbol=cmsy10 \\font\\extension=cmex10\n\
             \\textfont2=\\symbol \\scriptfont2=\\symbol \\scriptscriptfont2=\\symbol\n\
             \\textfont3=\\extension \\scriptfont3=\\extension \\scriptscriptfont3=\\extension\n\
             before\n\
             ${formula}$ after\\end"
        );
        let (stores, _) =
            run_canonical_math_recovery(stores_with_fonts(), CommandProfile::TEX82, &source, true);
        let output = terminal_effect_text(&stores);
        let message = output
            .find(&format!("{selector} 15 is undefined (character a)"))
            .unwrap_or_else(|| panic!("missing family report in {output:?}"));
        let source_context = output[message..]
            .find("l.5 $")
            .map(|offset| message + offset)
            .unwrap_or_else(|| panic!("missing closing source context in {output:?}"));
        let help = output[message..]
            .find("Somewhere in the math formula just ended")
            .map(|offset| message + offset)
            .unwrap_or_else(|| panic!("missing family help in {output:?}"));
        assert!(
            message < source_context && source_context < help,
            "TeX82 §82 context must precede §721 help: {output:?}"
        );
    }
}

#[test]
fn detached_math_conversion_error_context_survives_snapshot_rollback() {
    let (mut stores, control) = run_math_source_with_text_math_fonts("$\\mathchar\"0F61");
    let list = unfinished_math_list(&mut stores, &control);
    let snapshot = stores.snapshot();
    let baseline = terminal_effect_text(&stores);
    let context = crate::math::MathConversionErrorContext::new(
        "\n<recently read> $\n                  \nl.9 $a$\n       ".to_owned(),
    );

    let _ = crate::math::finish_inline_math_list_node(&mut stores, list, false, context.clone());
    let first = terminal_effect_text(&stores)[baseline.len()..].to_owned();
    stores.rollback(&snapshot);
    let _ = crate::math::finish_inline_math_list_node(&mut stores, list, false, context);
    let second = terminal_effect_text(&stores)[baseline.len()..].to_owned();

    assert_eq!(
        first, second,
        "rollback must replay the detached report exactly"
    );
    assert!(first.contains("<recently read> $"));
    assert!(first.contains("l.9 $a$"));
}

#[test]
fn inline_math_resets_space_factor_before_following_space() {
    let (_stores, executor) = run_math_source(r"\noindent\spacefactor=2000 $a$\message{done}");

    assert_eq!(executor.current_list().space_factor(), 1000);
}

#[test]
fn restricted_inline_math_finishing_suppresses_line_break_penalties() {
    let (mut stores, executor) = run_math_source(r"$a\mathbin+b\mathrel=c");
    let list = unfinished_math_list(&mut stores, &executor);

    let nodes = crate::math::finish_math_list_node(&mut stores, list, false);

    assert!(
        nodes
            .iter()
            .all(|node| !matches!(node, Node::Penalty(700 | 500))),
        "restricted hbox math conversion should not emit line-break penalties"
    );
}

#[test]
fn converted_math_glue_becomes_ordinary_while_named_spacing_and_leaders_keep_their_subtypes() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let explicit = stores.intern_glue(tex_state::glue::GlueSpec::ZERO);
    let content = stores.freeze_node_list(&[
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Operator(LimitType::NoLimits),
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Bin),
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Rel),
            MathField::Empty,
        )),
        Node::Glue {
            spec: explicit,
            kind: GlueKind::MuSkip,
            leader: None,
        },
        Node::Glue {
            spec: explicit,
            kind: GlueKind::Leaders,
            leader: Some(tex_state::node::LeaderPayload::Rule {
                width: None,
                height: None,
                depth: None,
            }),
        },
    ]);
    let list = MathListNode {
        display: false,
        content,
    };

    let nodes = crate::math::finish_math_list_node(&mut stores, list, true);

    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Glue {
                kind: GlueKind::ThinMuSkip,
                ..
            }
        )),
        "ord-op spacing should lower as named thinmuskip"
    );
    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Glue {
                kind: GlueKind::MedMuSkip,
                ..
            }
        )),
        "ord-bin spacing should lower as named medmuskip"
    );
    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Glue {
                kind: GlueKind::ThickMuSkip,
                ..
            }
        )),
        "ord-rel spacing should lower as named thickmuskip"
    );
    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Glue {
                kind: GlueKind::Normal,
                leader: None,
                ..
            }
        )),
        "TeX82 §732 converts explicit \\mskip to ordinary glue"
    );
    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Glue {
                kind: GlueKind::Leaders,
                leader: Some(_),
                ..
            }
        )),
        "leader glue does not take TeX82 §732's mu-glue conversion branch"
    );
}

#[test]
fn delimiter_radical_accent_and_vcenter_parse_to_math_noads() {
    let (stores, executor) = run_math_source(
        r#"$\delimiter"4266308 \radical"270370 x \mathaccent"7013 y \vcenter{\hrule width1pt}"#,
    );
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 4);
    assert!(matches!(
        math_noad(&nodes[0]).kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Open)
    ));
    assert_math_char(&math_noad(&nodes[0]).nucleus, 2, 'f');

    let radical = math_noad(&nodes[1]);
    assert!(matches!(
        radical.kind,
        tex_state::math::NoadKind::Radical {
            delimiter: 0x270370
        }
    ));
    assert_math_char(&radical.nucleus, 1, 'x');

    let accent = math_noad(&nodes[2]);
    assert!(matches!(
        accent.kind,
        tex_state::math::NoadKind::Accent { .. }
    ));
    assert_math_char(&accent.nucleus, 1, 'y');

    let vcenter = math_noad(&nodes[3]);
    assert!(matches!(vcenter.kind, tex_state::math::NoadKind::VCenter));
    let MathField::SubBox(list) = vcenter.nucleus else {
        panic!("expected vcenter sub-box field");
    };
    assert!(matches!(
        stores.nodes(list).testing_decoded()[0],
        Node::VList(_)
    ));
}

#[test]
fn vcenter_restores_local_assignments_and_preserves_globals() {
    let (stores, _) = run_math_source(
        r"\lineskip=1pt \baselineskip=12pt $\vcenter{\lineskip=4pt \global\baselineskip=17pt \hrule}$",
    );

    assert_eq!(
        stores.glue(stores.glue_param(GlueParam::LINE_SKIP)).width,
        Scaled::from_raw(Scaled::UNITY)
    );
    assert_eq!(
        stores
            .glue(stores.glue_param(GlueParam::BASELINE_SKIP))
            .width,
        Scaled::from_raw(17 * Scaled::UNITY)
    );
}

#[test]
fn vcenter_replays_everyvbox_before_its_body() {
    let (stores, _) = run_math_source(
        r"\everyvbox{\global\count6=42}\count6=0 \count7=0 $\vcenter{\global\count7=\count6 \hrule}$",
    );

    assert_eq!(stores.count(6), 42, "vcenter executes the everyvbox hook");
    assert_eq!(
        stores.count(7),
        42,
        "the everyvbox hook precedes the vcenter body"
    );
}

#[test]
fn every_math_and_every_display_tokens_are_inserted_on_entry() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let control = CanonicalMainControl::tex82_initex(&mut stores);
    let displaystyle = stores.symbol("displaystyle").expect("displaystyle");
    let every_math = stores.intern_token_list(&[Token::Cs(displaystyle.symbol())]);
    stores.set_tok_param(TokParam::EVERY_MATH, every_math);
    // TeX82 §§1138 and 1145 insert the hook exactly once, then §357 advances
    // its token-list cursor and §360 retires the exhausted level. Keeping this
    // bound tight makes a cursor reset or repeated entry-hook push fail before
    // it can grow the input stack without limit.
    let ((_stores, mut control), inline_steps) =
        run_registered_canonical_math_source_with_limit(stores, control, "$a", 8);
    assert!(inline_steps <= 8);
    assert_eq!(
        control.command_mut().input_level_count(),
        0,
        "the everymath level and its enclosing source both retire"
    );
    let nodes = control.current_list().nodes();
    assert!(matches!(
        nodes[0],
        Node::MathStyle(tex_state::math::MathStyle::Display)
    ));

    let (display_stores, _) = run_canonical_math_recovery(
        crate::test_harness::universe_with_plain_catcodes(),
        CommandProfile::TEX82,
        r"\everydisplay{\message{ED}}\noindent$$b$$\end",
        false,
    );
    assert!(terminal_effect_text(&display_stores).contains("ED"));
}

fn run_math_source(source: &str) -> (Universe, CanonicalMainControl) {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_int_param(IntParam::SHOW_BOX_BREADTH, 100);
    stores.set_int_param(IntParam::SHOW_BOX_DEPTH, 100);
    run_canonical_math_recovery(stores, CommandProfile::TEX82, source, false)
}

fn run_math_source_with_text_math_fonts(source: &str) -> (Universe, CanonicalMainControl) {
    let mut stores = stores_with_fonts();
    let metrics = stores
        .world_mut()
        .read_file("cmr10.tfm")
        .expect("seeded math font is readable");
    // Make slot 127 absent while retaining the canonical font name used by
    // the warning contract; the committed cmr10 fixture defines that slot.
    let mut sparse_metrics = metrics.bytes().to_vec();
    let header_words = usize::from(u16::from_be_bytes([sparse_metrics[2], sparse_metrics[3]]));
    let first_character = usize::from(u16::from_be_bytes([sparse_metrics[4], sparse_metrics[5]]));
    let char_info = 24 + header_words * 4 + (0x7f - first_character) * 4;
    sparse_metrics[char_info] = 0;
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", sparse_metrics)
        .expect("install sparse metrics under the asserted diagnostic name");
    let source = format!(
        r"\font\rm=cmr10
          \tracinglostchars=1
          \textfont0=\rm \scriptfont0=\rm \scriptscriptfont0=\rm
          \textfont1=\rm \scriptfont1=\rm \scriptscriptfont1=\rm {source}"
    );
    run_canonical_math_recovery(stores, CommandProfile::TEX82, &source, false)
}

fn assert_replayed_math_error_is_source_backed(source: &str) {
    const PATH: &str = "math-origin.tex";

    #[derive(Default)]
    struct Recorder(Vec<tex_command::CommandObservation>);

    impl tex_command::CommandObserver for Recorder {
        fn committed(&mut self, observation: tex_command::CommandObservation) {
            self.0.push(observation);
        }
    }

    let mut stores = crate::test_harness::memory_universe_with_plain_catcodes();
    let source = source.replace(r"\noexpand\input", r"\global\count7=1\relax\noexpand\input");
    stores
        .world_mut()
        .set_memory_file(PATH, source.as_bytes().to_vec())
        .expect("memory source should be installed");
    let content = stores
        .world_mut()
        .read_file(PATH)
        .expect("memory source should be readable");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::world(content).with_name(PATH))
        .expect("register source-backed canonical math fixture");
    let mut recorder = Recorder::default();
    for _ in 0..1024 {
        if control
            .step_with_observer(&mut stores, &mut recorder)
            .expect("canonical provenance step")
            != MainControlStep::Continue
        {
            break;
        }
    }
    let sentinel = recorder
        .0
        .iter()
        .find_map(|observation| match observation {
            tex_command::CommandObservation::Command(command)
                if command.command == "relax"
                    && command.spelling
                        == tex_command::ObservedToken::ControlSequence("input".to_owned()) =>
            {
                Some(&command.provenance)
            }
            _ => None,
        })
        .expect("noexpand sentinel is observed as a one-delivery relax");
    let origin = sentinel.origin;
    let origin = match stores.origin(origin) {
        OriginRecord::Inserted(inserted) if inserted.kind() == InsertedOriginKind::NoExpand => {
            inserted.parent()
        }
        _ => origin,
    };
    let OriginRecord::SourceSpan(source_span) = stores.origin(origin) else {
        panic!("expected source span, got {:?}", stores.origin(origin));
    };
    let expected_offset =
        u64::try_from(source.rfind(r"\input").expect("sentinel token in fixture"))
            .expect("fixture offset should fit in u64");
    assert_eq!(
        source_span.lo(),
        stores
            .source_position(tex_state::SourceId::new(0), expected_offset)
            .expect("source position")
    );
}

fn math_nodes<'a>(stores: &'a Universe, control: &'a CanonicalMainControl) -> &'a [Node] {
    if matches!(control.current_mode(), Mode::Math | Mode::DisplayMath) {
        return control.current_list().nodes();
    }
    let lists = math_list_nodes(control);
    assert_eq!(lists.len(), 1);
    stores.nodes(lists[0].content).testing_decoded()
}

fn unfinished_math_list(stores: &mut Universe, control: &CanonicalMainControl) -> MathListNode {
    assert_eq!(control.current_mode(), Mode::Math);
    let content = stores.freeze_node_list(control.current_list().nodes());
    MathListNode {
        display: false,
        content,
    }
}

fn math_list_nodes(control: &CanonicalMainControl) -> Vec<MathListNode> {
    control
        .current_list()
        .nodes()
        .iter()
        .filter_map(|node| match node {
            Node::MathList(list) => Some(*list),
            _ => None,
        })
        .collect()
}

fn math_noad(node: &Node) -> &tex_state::math::MathNoad {
    match node {
        Node::MathNoad(noad) => noad,
        other => panic!("expected noad, got {other:?}"),
    }
}

fn assert_math_char(field: &MathField, family: u8, character: char) {
    match field {
        MathField::MathChar(ch) => {
            assert_eq!(ch.family, family);
            assert_eq!(ch.character, character);
        }
        other => panic!("expected math char field, got {other:?}"),
    }
}

fn assert_one_char_list(stores: &Universe, list: tex_state::ids::NodeListId, character: char) {
    assert_char_list(stores, list, &[character]);
}

fn assert_char_list(stores: &Universe, list: tex_state::ids::NodeListId, expected: &[char]) {
    let actual: Vec<_> = stores
        .nodes(list)
        .testing_decoded()
        .iter()
        .map(|node| {
            let noad = math_noad(node);
            match &noad.nucleus {
                MathField::MathChar(ch) => ch.character,
                other => panic!("expected math char nucleus, got {other:?}"),
            }
        })
        .collect();
    assert_eq!(actual, expected);
}

/// TeX82 §1155's `set_math_char` sends a `math_code` of `@'100000` to §1152's
/// `@<Treat |cur_chr| as an active character@>`, which loads the
/// `active_base + c` meaning into `cur_cmd`/`cur_chr`, runs §381's `x_token`
/// on it, and backs the settled token up. Plain TeX's ``\mathcode`\'="8000``
/// depends on the whole chain: without it every `'` in a formula vanishes
/// instead of expanding the active `'` macro that builds `\prime` lists.
#[test]
fn canonical_mathcode_8000_expands_the_active_character_in_math_mode() {
    let stores = super::core::run_canonical_tex82(
        r"\catcode`\?=13 \def?{\global\count0=7 }\catcode`\?=12
          \mathcode`\?=32768 $?$\end",
    );

    assert_eq!(stores.count(0), 7);
}

/// §1152 ends in `x_token; back_input`, so an active character bound to an
/// unexpandable meaning is still redispatched: `x_token` leaves that meaning
/// alone and `back_input` hands it to main control, which executes it in
/// place of the character that carried the `math_code`. The result is read
/// back inside the formula because `push_math` opens a group.
#[test]
fn canonical_mathcode_8000_backs_up_an_unexpandable_active_meaning() {
    let stores = super::core::run_canonical_tex82(
        r"\catcode`\?=13 \let?=\count \catcode`\?=12
          \mathcode`\?=32768 $?0=9 \global\count1=\count0 $\end",
    );

    assert_eq!(stores.count(1), 9);
}

/// §1155 guards the redispatch with `c>=@'100000` alone, so any smaller
/// `math_code` builds an ordinary noad and never consults the active
/// character's meaning at all.
#[test]
fn canonical_mathcode_below_32768_never_consults_the_active_character() {
    let stores = super::core::run_canonical_tex82(
        r"\catcode`\?=13 \def?{\global\count0=7 }\catcode`\?=12
          \mathcode`\?=28721 $?$\end",
    );

    assert_eq!(stores.count(0), 0);
}

/// TeX82 §1151 stores its result with `math_type(p):=math_char;
/// character(p):=qi(c mod 256)` and its own `fam` rule -- it builds no noad,
/// so `c`'s class bits are dropped. A `\mathchar` script field is therefore a
/// math character, never a one-noad sublist carrying the class the same code
/// would give an mlist entry (`umber2-johp.265`).
#[test]
fn canonical_mathchar_script_field_is_a_math_char_without_its_class() {
    let (_, nodes) = super::core::run_canonical_tex82_current_list("$y^\\mathchar\"3161");

    assert_eq!(nodes.len(), 1);
    let scripted = math_noad(&nodes[0]);
    assert_math_char(&scripted.nucleus, 1, 'y');
    assert_math_char(&scripted.superscript, 1, 'a');
}

/// §1151's `scan_math` carries the same branch as §1155: a script field whose
/// first token is a `math_code`-32768 character is redispatched through the
/// active character instead of becoming the field itself.
///
/// §1152 is `x_token; back_input`, so the field restarts on whatever the
/// active meaning's expansion *begins* with, and §1151 then classifies that
/// token like any other. A `{` therefore reaches §1153 and the assignment
/// runs as live `math_group` body -- the redispatched expansion is ordinary
/// input, never material the field absorbs.
#[test]
fn canonical_mathcode_8000_redispatches_inside_a_script_field() {
    let stores = super::core::run_canonical_tex82(
        r"\catcode`\?=13 \def?{{\global\count0=7 x}}\catcode`\?=12
          \mathcode`\?=32768 $y^?$\end",
    );

    assert_eq!(stores.count(0), 7);
}

/// tex.web §1176's `sub_sup` reaches §1177's dummy noad through `p=null`
/// whenever the tail fails §687's `scripts_allowed`, and §1177 prints only
/// under `if t<>empty`. A `\mskip` leaves a glue node as the tail, whose
/// `type` sorts below `ord_noad`, so the script starts a fresh Ord noad and
/// TeX reports nothing at all.
#[test]
fn canonical_script_after_a_non_noad_tail_is_silent() {
    let (_, nodes) = super::core::run_canonical_tex82_current_list(r"$= \mskip3mu ^2");

    assert_eq!(nodes.len(), 3);
    assert!(matches!(
        nodes[1],
        Node::Glue {
            kind: GlueKind::MuSkip,
            ..
        }
    ));
    let scripted = math_noad(&nodes[2]);
    assert!(matches!(
        scripted.kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Ord)
    ));
    assert!(matches!(scripted.nucleus, MathField::Empty));
    assert_math_char(&scripted.superscript, 0, '2');

    let stores = super::core::run_canonical_tex82(r"$= \mskip3mu ^2$\end");
    assert!(
        canonical_log_text(&stores).contains("Math formula deleted: Insufficient symbol fonts"),
        "the script operation itself stays silent; only §1194 diagnoses the null math fonts"
    );
}

/// §687's `scripts_allowed` stops below `left_noad`, so a script following
/// `\left` is a `p=null` case: it starts its own Ord noad instead of landing
/// on the left delimiter's noad, and stays silent. e-TeX's `\middle` is a
/// `right_noad` with `subtype=middle_noad`, so the same bound excludes it.
#[test]
fn canonical_script_after_left_delimiter_starts_a_fresh_ord_noad() {
    let (_, nodes) = super::core::run_canonical_tex82_current_list(r"$\left.^2");

    assert_eq!(nodes.len(), 2);
    let left = math_noad(&nodes[0]);
    assert!(matches!(
        left.kind,
        tex_state::math::NoadKind::LeftDelimiter { .. }
    ));
    assert!(matches!(left.superscript, MathField::Empty));
    let scripted = math_noad(&nodes[1]);
    assert!(matches!(
        scripted.kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Ord)
    ));
    assert_math_char(&scripted.superscript, 0, '2');

    let stores = super::core::run_canonical_tex82(r"$\left.^2\right.$\end");
    assert!(
        canonical_log_text(&stores).contains("Math formula deleted: Insufficient symbol fonts"),
        "the script operation itself stays silent; only §1194 diagnoses the null math fonts"
    );
}

/// §1177's two messages and `help1` lines run before §1176 calls §1151's
/// `scan_math`. The dummy noad is appended either way, so `x^1^2` behaves
/// like `x^1{}^2`.
#[test]
fn canonical_double_script_reports_tex82_message_and_help_before_field_scan() {
    let (_, nodes) = super::core::run_canonical_tex82_current_list(r"$a^1^2");

    assert_eq!(nodes.len(), 2);
    assert_math_char(&math_noad(&nodes[0]).superscript, 0, '1');
    assert_math_char(&math_noad(&nodes[1]).superscript, 0, '2');
    let stores = super::core::run_canonical_tex82(r"$a^1^{\message{SUP-FIELD-SCANNED}}$\end");
    assert!(canonical_log_text(&stores).starts_with(
        "! Double superscript.\n\
             I treat `x^1^2' essentially like `x^1{}^2'.\n\n\
             SUP-FIELD-SCANNED"
    ));

    let (_, nodes) = super::core::run_canonical_tex82_current_list(r"$a_1_2");

    assert_eq!(nodes.len(), 2);
    assert_math_char(&math_noad(&nodes[0]).subscript, 0, '1');
    assert_math_char(&math_noad(&nodes[1]).subscript, 0, '2');
    let stores = super::core::run_canonical_tex82(r"$a_1_{\message{SUB-FIELD-SCANNED}}$\end");
    assert!(canonical_log_text(&stores).starts_with(
        "! Double subscript.\n\
             I treat `x_1_2' essentially like `x_1{}_2'.\n\n\
             SUB-FIELD-SCANNED"
    ));
}

/// TeX82 §§1151–1153 and §§1176–1177 keep `p`, the selected field pointer,
/// live while a braced field runs ordinary main control. Nodes appended by
/// that nested execution must not make the completed field attach to the new
/// tail. The scalar cases also retain their own local source provenance.
#[test]
fn canonical_script_target_reservation_survives_nested_mlist_mutation() {
    let (stores, nodes) = super::core::run_canonical_tex82_current_list(r"$a^1^{b_c^d}_e");

    assert_eq!(nodes.len(), 2);
    assert_math_char(&math_noad(&nodes[0]).superscript, 0, '1');
    assert_math_char(&math_noad(&nodes[1]).subscript, 1, 'e');
    let MathField::SubMlist(nested) = math_noad(&nodes[1]).superscript else {
        panic!("reserved duplicate target should hold the completed sub-mlist");
    };
    let nested = stores.nodes(nested).testing_decoded();
    assert_eq!(nested.len(), 1);
    let inner = math_noad(&nested[0]);
    assert_math_char(&inner.nucleus, 1, 'b');
    assert_math_char(&inner.subscript, 1, 'c');
    assert_math_char(&inner.superscript, 1, 'd');
    let MathField::MathChar(subscript) = math_noad(&nodes[1]).subscript else {
        panic!("expected scalar subscript");
    };
    assert_ne!(subscript.origin, OriginId::UNKNOWN);
}

/// The reserved list position is intentionally stronger than "find the tail
/// again": §1153 recovery and a live math group may execute arbitrary commands
/// before §1151 fills `p`. Challenge both script slots, both empty and occupied
/// eligible fields, and a disallowed tail while appending a later noad.
#[test]
fn canonical_script_reservation_matrix_ignores_later_tail_appends() {
    for kind in [
        tex_command::MathScriptKind::Superscript,
        tex_command::MathScriptKind::Subscript,
    ] {
        for occupied in [false, true] {
            let mut stores = crate::test_harness::universe_with_plain_catcodes();
            let mut list = crate::ModeList::default();
            let mut first = MathNoad::new(NoadKind::Normal(NoadClass::Ord), MathField::Empty);
            if occupied {
                *crate::canonical_main_control::canonical_script_field_mut(&mut first, kind) =
                    MathField::MathChar(tex_state::math::MathChar {
                        family: 0,
                        character: '1',
                        origin: OriginId::UNKNOWN,
                    });
            }
            list.push(Node::MathNoad(first));
            let target = crate::canonical_main_control::reserve_canonical_script_target(
                crate::mode::ModeListMutation::for_test(&mut list),
                &mut stores,
                kind,
            )
            .expect("script reservation reports no fatal error");
            let reserved_index = usize::from(occupied);
            assert_eq!(target.node_index, reserved_index);

            list.push(Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::Empty,
            )));
            crate::canonical_main_control::fill_canonical_script_target(
                crate::mode::ModeListMutation::for_test(&mut list),
                target,
                MathField::MathChar(tex_state::math::MathChar {
                    family: 0,
                    character: '2',
                    origin: OriginId::UNKNOWN,
                }),
            );
            let reserved = math_noad(&list.nodes()[reserved_index]);
            assert_math_char(
                match kind {
                    tex_command::MathScriptKind::Superscript => &reserved.superscript,
                    tex_command::MathScriptKind::Subscript => &reserved.subscript,
                },
                0,
                '2',
            );
            let later_tail = math_noad(list.nodes().last().expect("later tail"));
            assert!(matches!(later_tail.superscript, MathField::Empty));
            assert!(matches!(later_tail.subscript, MathField::Empty));
        }

        let mut stores = crate::test_harness::universe_with_plain_catcodes();
        let mut list = crate::ModeList::default();
        list.push(Node::Glue {
            spec: stores.intern_glue(tex_state::glue::GlueSpec::ZERO),
            kind: GlueKind::MuSkip,
            leader: None,
        });
        let target = crate::canonical_main_control::reserve_canonical_script_target(
            crate::mode::ModeListMutation::for_test(&mut list),
            &mut stores,
            kind,
        )
        .expect("script reservation reports no fatal error");
        assert_eq!(target.node_index, 1);
        assert_eq!(canonical_log_text(&stores), "");
    }
}

fn canonical_log_text(stores: &Universe) -> String {
    String::from_utf8_lossy(
        stores
            .world()
            .memory_log_output()
            .expect("memory log output"),
    )
    .into_owned()
}

/// TeX82 §1167's `mmode+vcenter` opens a box, not a math text field:
/// `scan_spec(vcenter_group,false); normal_paragraph; push_nest; mode:=-vmode`.
/// Its body is therefore an internal *vertical* list, so vertical-mode-only
/// constructions -- above all §1130's `\halign` -- build normally inside it and
/// §1168 packages the result with `vpack` before wrapping it in a
/// `vcenter_noad`. Scanning the body as an mlist instead silently dropped every
/// alignment row, which is what collapsed plain's `\pmatrix`/`\matrix`/
/// `\cases`/`\eqalign` to their bare `\mathstrut` (`umber2-johp.260`).
#[test]
fn canonical_vcenter_body_is_an_internal_vertical_list() {
    let (stores, nodes) = super::core::run_canonical_tex82_current_list(
        r"\baselineskip=12pt \lineskip=0pt \lineskiplimit=0pt
          $\vcenter{\halign{#\cr\hbox to 7pt{}\cr\hbox to 9pt{}\cr}}",
    );

    assert_eq!(nodes.len(), 1);
    let vcenter = math_noad(&nodes[0]);
    assert!(matches!(vcenter.kind, tex_state::math::NoadKind::VCenter));
    let MathField::SubBox(list) = vcenter.nucleus else {
        panic!("§1168 stores the packaged box as the vcenter noad's nucleus");
    };
    let Node::VList(packaged) = &stores.nodes(list).testing_decoded()[0] else {
        panic!("§1168 vpacks the body");
    };
    let rows = stores.nodes(packaged.children).testing_decoded();
    assert_eq!(
        rows.len(),
        3,
        "two alignment rows separated by §799's interline glue: {rows:?}"
    );
    assert!(matches!(rows[0], Node::HList(_)));
    assert!(matches!(
        rows[1],
        Node::Glue {
            kind: GlueKind::BaselineSkip,
            ..
        }
    ));
    assert!(matches!(rows[2], Node::HList(_)));
}
