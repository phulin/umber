use std::fs;
use std::sync::Arc;

use test_support::{CompileFailDependency, assert_compile_fail};
use tex_command::{RegisteredSourceKind, SourceRegistration};
use tex_exec::{CanonicalMainControl, MainControlStep};
use tex_state::{EffectRecord, InteractionMode, PrintSink, Universe};

#[test]
fn restricted_horizontal_hrule_reports_source_before_rule_spec_lookahead() {
    // TeX82 §1095 diagnoses this command in `head_for_vmode`, before §463
    // scans a rule specification. §82 must therefore display the physical
    // source line, not a token level created by keyword lookahead.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    stores.set_int_param(tex_state::env::banks::IntParam::TRACING_ONLINE, 1);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\setbox0=\\hbox{\n\\hrule\n}\\end".as_slice()),
        ))
        .expect("restricted-horizontal rule source registers");

    loop {
        match control
            .step(&mut stores)
            .expect("restricted-horizontal rule source executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let committed = stores
        .world()
        .memory_log_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::Log | PrintSink::TerminalAndLog,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let transcript = committed + &pending;
    let diagnostic = transcript
        .find("! You can't use `\\hrule' here except with leaders.")
        .unwrap_or_else(|| panic!("TeX82 §1095 diagnostic: {transcript:?}"));
    let context = &transcript[diagnostic..];
    assert!(context.contains("l.2 \\hrule"), "{transcript:?}");
    assert!(!context.contains("<to be read again>"), "{transcript:?}");
}

#[test]
fn restricted_horizontal_prevdepth_reports_before_scanning_an_operand() {
    // TeX82 §1243's `alter_aux` compares `cur_chr` with `abs(mode)` before
    // `scan_optional_equals` and `scan_normal_dimen`. The following `\relax`
    // therefore remains an ordinary command instead of becoming a rejected
    // dimension operand.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(br"\setbox0=\hbox{\prevdepth\relax X}\end".as_slice()),
        ))
        .expect("restricted-horizontal prevdepth source registers");

    loop {
        match control
            .step(&mut stores)
            .expect("restricted-horizontal prevdepth source executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let committed = stores
        .world()
        .memory_log_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::Log | PrintSink::TerminalAndLog,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let transcript = committed + &pending;
    assert!(
        transcript.contains("! You can't use `\\prevdepth' in restricted horizontal mode."),
        "{transcript}"
    );
    assert!(
        !transcript.contains("Missing number, treated as zero."),
        "{transcript}"
    );
}

#[test]
fn alignment_closing_brace_reports_inserted_cr_and_followup_brace() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    stores.set_int_param(tex_state::env::banks::IntParam::TRACING_ONLINE, 1);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(
                br"\long\def\l#1{}\let\PAR=\par\def\par{\relax\PAR}\halign{#&#&\l{#}\cr a&b&c&&&.}\par\cr}\end"
                    .as_slice(),
            ),
        ))
        .expect("alignment-recovery source registers");

    loop {
        match control
            .step(&mut stores)
            .expect("alignment-recovery source executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let committed = stores
        .world()
        .memory_log_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::Log | PrintSink::TerminalAndLog,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let transcript = committed + &pending;
    let diagnostic = transcript
        .find("! Missing \\cr inserted.")
        .unwrap_or_else(|| panic!("TeX82 §§82/1132 diagnostic: {transcript:?}"));
    let inserted = transcript[diagnostic..]
        .find("<inserted text>")
        .map(|offset| diagnostic + offset)
        .unwrap_or_else(|| panic!("inserted frozen \\cr context: {transcript:?}"));
    assert!(diagnostic < inserted, "{transcript:?}");
    let missing_left_brace = transcript[diagnostic..]
        .find("! Missing { inserted.")
        .map(|offset| diagnostic + offset)
        .unwrap_or_else(|| panic!("TeX82 §1127 diagnostic: {transcript:?}"));
    assert!(diagnostic < missing_left_brace, "{transcript:?}");
}

#[test]
fn misplaced_tab_in_v_template_retains_synchronous_error_context() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(
                br"\let\lb={\let\rb=}\halign\relax{\span\iffalse}\fi\cr#&\ifnum0=`{\fi\cr\cr}\end"
                    .as_slice(),
            ),
        ))
        .expect("alignment-template context source registers");

    loop {
        match control
            .step(&mut stores)
            .expect("alignment-template context source executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let committed = stores
        .world()
        .memory_log_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::Log | PrintSink::TerminalAndLog,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let transcript = committed + &pending;
    assert!(
        transcript.contains("<template> &\n            \\ifnum 0=`{\\fi \\endtemplate "),
        "TeX82 §§82,1128 diagnose before the retained v-template retires: {transcript}"
    );
}

#[test]
fn paragraph_start_page_build_reports_backed_up_context_before_help() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    stores.set_int_param(tex_state::env::banks::IntParam::TRACING_ONLINE, 1);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(
                br"\topskip=0pt \vsize=100pt \setbox1=\hbox{}\copy1 \vskip0pt minus 1fil$x$\end"
                    .as_slice(),
            ),
        ))
        .expect("page-error source registers");

    loop {
        match control
            .step(&mut stores)
            .expect("page-error source executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let committed = stores
        .world()
        .memory_log_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::Log | PrintSink::TerminalAndLog,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let transcript = committed + &pending;
    let error = transcript
        .find("! Infinite glue shrinkage found on current page.")
        .expect("error line");
    let context = transcript[error..]
        .find("<to be read again>")
        .map(|offset| error + offset)
        .unwrap_or_else(|| panic!("live command context: {transcript:?}"));
    let help = transcript[error..]
        .find("The page about to be output contains some infinitely")
        .map(|offset| error + offset)
        .expect("page-error help");
    assert!(error < context && context < help, "{transcript:?}");
}

#[test]
fn text_accent_in_math_reports_before_scanning_its_character() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    stores.set_int_param(tex_state::env::banks::IntParam::TRACING_ONLINE, 1);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(br"\setbox3=\hbox{x}$\unhcopy3\accent65x$\end".as_slice()),
        ))
        .expect("accent source registers");

    loop {
        match control.step(&mut stores).expect("accent source executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let committed = stores
        .world()
        .memory_log_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::Log | PrintSink::TerminalAndLog,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let transcript = committed + &pending;
    assert!(
        transcript.contains("Please use \\mathaccent for accents in math mode"),
        "{transcript:?}"
    );
    assert!(
        transcript.contains("\n<recently read> \\accent \n"),
        "TeX82 §§82,1110 retain the exhausted command level through the diagnostic: {transcript:?}"
    );
    assert!(
        !transcript.contains("<to be read again> 6"),
        "§436 must not consume the operand before §1110 reports"
    );
}

fn production_rust_sources(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).expect("read production source directory") {
            let path = entry.expect("read production source entry").path();
            if path.is_dir() {
                if path.file_name().is_none_or(|name| name != "tests") {
                    pending.push(path);
                }
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_none_or(|name| name != "tests.rs")
            {
                sources.push(path);
            }
        }
    }
    sources
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_token_rendering_stays_on_the_state_owner() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    for path in production_rust_sources(&manifest_dir.join("src")) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in [
            "tex_expand::append_token_show_text",
            "tex_expand::append_token_string_text",
            "tex_expand::append_token_selector_text",
            "tex_expand::token_text",
            "tex_expand::semantic_token",
            "tex_expand::meaning_text",
            "tex_expand::bounded_meaning_text",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must use tex_state::token_show instead of `{forbidden}`",
                path.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn command_savepoints_keep_paragraph_histories_persistent() {
    let source = fs::read_to_string(
        test_support::repository_root().join("crates/tex-exec/src/canonical_main_control.rs"),
    )
    .expect("read canonical main-control source");
    assert!(source.contains("finished: Arc<Vec<CanonicalParagraphRegion>>"));
    assert!(source.contains("replay: Arc<Vec<CanonicalParagraphRegion>>"));
    assert!(source.contains("Arc::make_mut(&mut self.finished).push(region)"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_replay_kinds_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        assert!(
            !source.contains("tex_lex::TokenListReplayKind"),
            "{} must use tex_state::TokenListReplayKind",
            path.display()
        );
        assert!(
            !source
                .lines()
                .any(|line| line.contains("tex_lex::{") && line.contains("TokenListReplayKind")),
            "{} must not import TokenListReplayKind through tex-lex",
            path.display()
        );
        assert!(
            !source.contains("tex_lex::TokenListReplayMarker"),
            "{} must use tex_state::TokenListReplayMarker",
            path.display()
        );
        assert!(
            !source.lines().any(|line| {
                line.contains("tex_lex::{") && line.contains("TokenListReplayMarker")
            }),
            "{} must not import TokenListReplayMarker through tex-lex",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_page_output_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let source = fs::read_to_string(source_root.join("canonical_page_output.rs"))
        .expect("read canonical page-output module");
    for forbidden in [
        "tex_lex",
        "InputStack",
        "ExecutionContext",
        "crate::executor",
        "legacy_output",
        "run_main_control_until",
    ] {
        assert!(
            !source.contains(forbidden),
            "canonical_page_output.rs must not reference legacy boundary `{forbidden}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn legacy_output_has_no_shipped_command_control_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("legacy_output") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some(
                        "lib.rs"
                            | "executor.rs"
                            | "align/legacy_execution.rs"
                            | "assignments/mod.rs"
                            | "assignments/shipout.rs"
                            | "legacy_output.rs"
                    )
                ),
                "{} must not call the retired output front",
                relative.display()
            );
        }
    }
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("legacy_output"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_diagnostics_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let source = fs::read_to_string(source_root.join("canonical_diagnostics.rs"))
        .expect("read canonical diagnostics module");
    for forbidden in [
        "tex_expand",
        "tex_lex",
        "InputStack",
        "ExecutionContext",
        "crate::executor",
        "legacy_diagnostics",
        "raw_delivery",
    ] {
        assert!(
            !source.contains(forbidden),
            "canonical_diagnostics.rs must not reference legacy boundary `{forbidden}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn legacy_diagnostics_has_no_canonical_command_control_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("legacy_diagnostics") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some(
                        "lib.rs"
                            | "executor.rs"
                            | "assignments/mod.rs"
                            | "assignments/legacy_scan.rs"
                            | "assignments/boxes/packaging.rs"
                            | "legacy_diagnostics.rs"
                    )
                ),
                "{} must not call the retired diagnostic scanner front",
                relative.display()
            );
        }
    }
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("legacy_diagnostics"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_assignment_family_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let owner_root = source_root.join("canonical_assignments");
    for path in production_rust_sources(&owner_root) {
        let source = fs::read_to_string(&path).expect("read canonical assignment source");
        for forbidden in [
            "tex_expand",
            "tex_lex",
            "InputStack",
            "ExecutionContext",
            "crate::executor",
            "crate::assignments",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must not reference legacy boundary `{forbidden}`",
                path.strip_prefix(&source_root)
                    .expect("canonical assignment source below root")
                    .display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_assignment_owner_has_only_declared_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("canonical_assignments") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some(
                        "lib.rs"
                            | "canonical_main_control.rs"
                            | "assignments/mod.rs"
                            | "assignments/legacy_variables.rs"
                            | "canonical_assignments/mod.rs"
                    )
                ),
                "{} must not bypass the canonical assignment owner",
                relative.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_box_runtime_has_no_legacy_dependencies_or_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let owner_root = source_root.join("canonical_box_runtime");
    for path in production_rust_sources(&owner_root) {
        let source = fs::read_to_string(&path).expect("read canonical box runtime source");
        for forbidden in [
            "tex_expand",
            "tex_lex",
            "InputStack",
            "ExecutionContext",
            "crate::executor",
            "legacy_assignments",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must not reference legacy boundary `{forbidden}`",
                path.strip_prefix(&source_root)
                    .expect("canonical box source below root")
                    .display()
            );
        }
    }

    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("canonical_box_runtime") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some(
                        "lib.rs"
                            | "canonical_main_control.rs"
                            | "canonical_box_runtime/mod.rs"
                            | "canonical_box_runtime/hmode.rs"
                            | "canonical_box_runtime/leaders.rs"
                            | "canonical_box_runtime/material.rs"
                            | "canonical_box_runtime/packaging.rs"
                            | "canonical_box_runtime/vsplit.rs"
                            | "canonical_paragraph_end.rs"
                            | "canonical_paragraph_end/hyphenation.rs"
                            | "canonical_paragraph_end/runtime.rs"
                            | "assignments/boxes/leaders.rs"
                            | "assignments/boxes/mod.rs"
                            | "assignments/boxes/packaging.rs"
                            | "assignments/boxes/vsplit.rs"
                            | "assignments/hmode.rs"
                            | "assignments/hyphenation.rs"
                            | "assignments/legacy_variables/streams.rs"
                            | "assignments/mod.rs"
                            | "assignments/paragraph.rs"
                            | "align/legacy_execution.rs"
                            | "math/display.rs"
                    )
                ),
                "{} must not bypass the canonical box runtime owner",
                relative.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_vsplit_physically_owns_its_source_free_closure() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let owner = fs::read_to_string(source_root.join("canonical_box_runtime/vsplit.rs"))
        .expect("read canonical vsplit owner");
    let legacy = source_root.join("assignments/boxes/vsplit.rs");
    assert!(!legacy.exists(), "retired vsplit scanner must stay deleted");

    for forbidden in [
        "assignments",
        "legacy",
        "executor",
        "ExecutionContext",
        "InputStack",
        "tex_expand",
        "tex_lex",
    ] {
        assert!(
            !owner.contains(forbidden),
            "canonical vsplit owner references `{forbidden}`"
        );
    }

    for implementation in [
        "fn split_vbox_register(",
        "fn normalize_split_infinite_shrink(",
        "fn replace_split_source(",
        "fn update_split_marks(",
        "fn clear_split_marks(",
        "fn vertical_break_error(",
    ] {
        assert!(
            owner.contains(implementation),
            "canonical owner lacks `{implementation}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_packaging_physically_owns_its_source_free_closure() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let owner = fs::read_to_string(source_root.join("canonical_box_runtime/packaging.rs"))
        .expect("read canonical packaging owner");
    let legacy = source_root.join("assignments/boxes/packaging.rs");
    assert!(
        !legacy.exists(),
        "retired packaging scanner must stay deleted"
    );

    for forbidden in [
        "assignments",
        "legacy",
        "executor",
        "ExecutionContext",
        "InputStack",
        "tex_expand",
        "tex_lex",
    ] {
        assert!(
            !owner.contains(forbidden),
            "canonical packaging owner references `{forbidden}`"
        );
    }
    for implementation in [
        "fn hpack_with_overfull_rule(",
        "fn hpack_owned_with_overfull_rule(",
        "fn project_short_diagnostic_discs(",
        "fn first_box_node(",
        "fn take_last_box(",
        "fn reset_removed_box_shift(",
        "fn report_cannot_take_last_box(",
    ] {
        assert!(
            owner.contains(implementation),
            "canonical owner lacks `{implementation}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_hmode_physically_owns_pending_character_runtime() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let owner = fs::read_to_string(source_root.join("canonical_box_runtime/hmode.rs"))
        .expect("read canonical hmode owner");
    let legacy = source_root.join("assignments/hmode.rs");
    assert!(!legacy.exists(), "retired hmode scanner must stay deleted");

    for forbidden in [
        "assignments",
        "legacy",
        "executor",
        "ExecutionContext",
        "InputStack",
        "tex_expand",
        "tex_lex",
    ] {
        assert!(
            !owner.contains(forbidden),
            "canonical hmode owner references `{forbidden}`"
        );
    }
    for implementation in [
        "fn append_canonical_character_with_fuel(",
        "fn flush_pending_hchars(",
        "fn flush_pending_hchar_run_with_fuel(",
        "fn append_space_after_flush(",
        "fn append_hchar_with_fuel(",
        "fn shape_open_type_chars(",
        "fn run_tfm_ligature_machine(",
        "fn reconstitute_with_fuel(",
        "fn literal_hyphen_disc(",
        "fn interword_glue(",
        "fn append_italic_correction_with_fuel(",
    ] {
        assert!(
            owner.contains(implementation),
            "canonical owner lacks `{implementation}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_box_material_physically_owns_post_scan_mutations() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let owner = fs::read_to_string(source_root.join("canonical_box_runtime/material.rs"))
        .expect("read canonical box material owner");
    let legacy = source_root.join("assignments/boxes/mod.rs");
    assert!(!legacy.exists(), "retired box scanner must stay deleted");

    for forbidden in [
        "assignments",
        "legacy",
        "executor",
        "ExecutionContext",
        "InputStack",
        "tex_expand",
        "tex_lex",
    ] {
        assert!(
            !owner.contains(forbidden),
            "canonical material owner references `{forbidden}`"
        );
    }
    for implementation in [
        "fn execute_scanned_unbox(",
        "fn execute_scanned_saved_vertical_discards(",
        "fn execute_delete_last(",
        "fn execute_delete_last_outer_vertical(",
        "fn append_box_register(",
        "fn append_box_node_to_current_list(",
        "fn extract_box_migrations(",
        "fn split_hpack_migrations(",
        "fn append_unboxed(",
        "fn report_incompatible_unbox(",
        "fn apply_box_shift_delta(",
        "fn acquire_box_register(",
        "fn assign_box_dimension(",
        "fn box_dimension_for_primitive(",
    ] {
        assert!(
            owner.contains(implementation),
            "canonical owner lacks `{implementation}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_leaders_physically_own_payload_and_contribution_runtime() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let owner = fs::read_to_string(source_root.join("canonical_box_runtime/leaders.rs"))
        .expect("read canonical leader owner");
    let legacy = source_root.join("assignments/boxes/leaders.rs");
    assert!(!legacy.exists(), "retired leader scanner must stay deleted");

    for forbidden in [
        "assignments",
        "legacy",
        "executor",
        "ExecutionContext",
        "InputStack",
        "tex_expand",
        "tex_lex",
    ] {
        assert!(
            !owner.contains(forbidden),
            "canonical leader owner references `{forbidden}`"
        );
    }
    for implementation in [
        "fn payload_from_node(",
        "fn leader_glue_kind(",
        "fn infinite_glue_for_skip_primitive(",
        "fn take_register_payload(",
        "fn append_leader_contribution(",
    ] {
        assert!(
            owner.contains(implementation),
            "canonical owner lacks `{implementation}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_main_has_no_mixed_box_or_hmode_calls() {
    let source = fs::read_to_string(
        test_support::repository_root().join("crates/tex-exec/src/canonical_main_control.rs"),
    )
    .expect("read canonical command control");
    for retired in [
        "assignments::append_box_node_to_current_list",
        "assignments::append_canonical_",
        "assignments::append_italic_correction_with_fuel",
        "assignments::append_whatsit",
        "assignments::apply_box_shift_delta",
        "assignments::commit_current_list",
        "assignments::control_space_glue_spec",
        "assignments::execute_delete_last",
        "assignments::execute_scanned_",
        "assignments::first_box_node",
        "assignments::fixed_infinite_glue",
        "assignments::flush_pending_hchars",
        "assignments::hpack_with_overfull_rule",
        "assignments::indent_in_hmode",
        "assignments::norm_min",
        "assignments::split_hpack_migrations",
        "assignments::split_vbox_register",
        "assignments::take_last_box",
    ] {
        assert!(
            !source.contains(retired),
            "canonical command control must bypass mixed runtime `{retired}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_command_control_bypasses_legacy_assignment_front() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("legacy_assignments"));

    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("legacy_assignments") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some(
                        "lib.rs"
                            | "executor.rs"
                            | "legacy_assignments.rs"
                            | "legacy_dispatch.rs"
                            | "legacy_output.rs"
                            | "math/legacy_front.rs"
                            | "math/legacy_scan.rs"
                    )
                ),
                "{} must not call the retired assignment scanner facade",
                relative.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_paragraph_memo_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let source = fs::read_to_string(source_root.join("canonical_paragraph_memo.rs"))
        .expect("read canonical paragraph-memo module");
    for forbidden in [
        "tex_expand",
        "tex_lex",
        "InputStack",
        "ExecutionContext",
        "crate::executor",
        "legacy_paragraph_memo",
    ] {
        assert!(
            !source.contains(forbidden),
            "canonical_paragraph_memo.rs must not reference legacy boundary `{forbidden}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_paragraph_replay_bypasses_the_legacy_memo_front() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    for helper in [
        "validate_dependencies",
        "same_mutation_entry_class",
        "validate_mutations",
        "replay_mutations",
    ] {
        assert!(
            canonical.contains(&format!("canonical_paragraph_memo::{helper}")),
            "canonical command control must call canonical paragraph helper `{helper}`"
        );
    }
    let retired = "crate::legacy_paragraph_memo";
    assert!(
        !canonical.contains(retired),
        "canonical command control must bypass retired paragraph front `{retired}`"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_paragraph_end_closure_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let owner = [
        "canonical_paragraph_end.rs",
        "canonical_paragraph_end/runtime.rs",
        "canonical_paragraph_end/hyphenation.rs",
    ]
    .into_iter()
    .map(|path| fs::read_to_string(source_root.join(path)).expect("read paragraph-end closure"))
    .collect::<String>();
    for forbidden in [
        "tex_expand",
        "tex_lex",
        "InputStack",
        "ExecutionContext",
        "crate::executor",
        "legacy_",
        "ParagraphMemoConsumer",
    ] {
        assert!(
            !owner.contains(forbidden),
            "canonical paragraph-end owner must not reference `{forbidden}`"
        );
    }

    assert!(owner.contains("LineMaterializer::new"));
    assert!(owner.contains("hpack_owned_with_overfull_rule"));
    assert!(owner.contains("append_vertical_contribution"));

    let legacy_front = source_root.join("assignments/paragraph.rs");
    assert!(
        !legacy_front.exists(),
        "retired paragraph adapter must stay deleted"
    );

    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("assignments::end_paragraph_with_fuel"));
    assert!(canonical.contains("canonical_paragraph_end::end_canonical_paragraph_without_source"));
    assert!(!canonical.contains("assignments::end_canonical_paragraph"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_shipout_transaction_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let owner = fs::read_to_string(source_root.join("canonical_shipout.rs"))
        .expect("read canonical shipout owner");
    for forbidden in [
        "tex_expand",
        "tex_lex",
        "InputStack",
        "ExecutionContext",
        "crate::executor",
        "legacy_",
    ] {
        assert!(
            !owner.contains(forbidden),
            "canonical shipout owner must not reference `{forbidden}`"
        );
    }

    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    for retired in [
        "crate::assignments::stage_pdf_form",
        "crate::assignments::shipout_node_with_input_summary",
        "crate::assignments::ShipoutOrigin",
        "crate::assignments::ReplayTextKind",
    ] {
        assert!(
            !canonical.contains(retired),
            "canonical command control must bypass mixed shipout boundary `{retired}`"
        );
    }
    assert!(canonical.contains("canonical_shipout::CanonicalShipoutTransaction"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_command_control_has_no_legacy_paragraph_front_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("legacy_paragraph_memo"));

    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("legacy_paragraph_memo") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some(
                        "lib.rs"
                            | "executor.rs"
                            | "assignments/paragraph.rs"
                            | "math/legacy_front.rs"
                    )
                ),
                "{} must not call the retired paragraph recording front",
                relative.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_math_family_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src/math");
    for relative in ["mod.rs", "display.rs", "lower.rs", "support.rs"] {
        let source = fs::read_to_string(source_root.join(relative))
            .expect("read canonical math-family source");
        for forbidden in [
            "tex_expand",
            "tex_lex",
            "InputStack",
            "ExecutionContext",
            "crate::executor",
            "legacy_front::",
            "legacy_scan::",
        ] {
            assert!(
                !source.contains(forbidden),
                "canonical math source {relative} must not reference legacy boundary `{forbidden}`"
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_command_control_has_no_legacy_math_front_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("math::legacy_front"));
    assert!(!canonical.contains("math::legacy_scan"));

    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("math::legacy_front") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some("legacy_dispatch.rs" | "legacy_paragraph_memo.rs" | "assignments/mod.rs")
                ),
                "{} must not call the retired math front",
                relative.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_alignment_family_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src/align");
    let mut canonical = vec![
        source_root.join("mod.rs"),
        source_root.join("canonical_execution.rs"),
        source_root.join("packaging.rs"),
        source_root.join("support.rs"),
        source_root.join("transitions.rs"),
    ];
    canonical.extend(production_rust_sources(&source_root.join("widths")));
    for path in canonical {
        let source = fs::read_to_string(&path).expect("read canonical alignment source");
        for forbidden in [
            "tex_expand",
            "tex_lex",
            "InputStack",
            "ExecutionContext",
            "crate::executor",
            "legacy_front::",
            "legacy_execution::",
        ] {
            assert!(
                !source.contains(forbidden),
                "canonical alignment source {} must not reference legacy boundary `{forbidden}`",
                path.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_command_control_has_no_legacy_alignment_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("align::legacy_front"));
    assert!(!canonical.contains("align::legacy_execution"));

    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("align::legacy_front") || source.contains("align::legacy_execution") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some("legacy_dispatch.rs" | "assignments/mod.rs" | "math/legacy_front.rs")
                ),
                "{} must not call the retired alignment front",
                relative.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_dispatch_contract_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let source = fs::read_to_string(source_root.join("dispatch.rs"))
        .expect("read canonical dispatch contract");
    for forbidden in [
        "tex_expand",
        "tex_lex",
        "InputStack",
        "ExecutionContext",
        "crate::executor",
        "legacy_dispatch",
        "assignments",
    ] {
        assert!(
            !source.contains(forbidden),
            "canonical dispatch contract must not reference legacy boundary `{forbidden}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_command_control_has_no_legacy_dispatch_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("legacy_dispatch"));

    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("legacy_dispatch") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some(
                        "lib.rs"
                            | "legacy_dispatch.rs"
                            | "executor.rs"
                            | "align/legacy_execution.rs"
                            | "assignments/hmode.rs"
                    )
                ),
                "{} must not call the retired dispatch front",
                relative.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_raw_token_delivery_bypasses_the_expand_compatibility_boundary() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in [
            "tex_expand::next_semantic_raw_token",
            "tex_expand::get_token",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must use the input owner's raw delivery instead of `{forbidden}`",
                path.display()
            );
        }
        assert!(
            !source.contains("tex_lex::next_semantic_raw_token"),
            "{} must not regain the retired raw-delivery bridge",
            path.display()
        );
    }
    assert!(
        !source_root.join("raw_delivery.rs").exists(),
        "retired raw-delivery bridge must stay deleted"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_mode_snapshots_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in ["tex_expand::EngineMode", "tex_expand::EngineStateSnapshot"] {
            assert!(
                !source.contains(forbidden),
                "{} must use the tex-state-owned mode snapshot instead of `{forbidden}`",
                path.display()
            );
        }
        assert!(
            !source.lines().any(|line| {
                line.contains("tex_expand::{")
                    && (line.contains("EngineMode") || line.contains("EngineStateSnapshot"))
            }),
            "{} must not import mode snapshot types through tex-expand",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_dimension_diagnostics_stay_on_the_command_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        assert!(
            !source.contains("tex_expand::scan_dimen::DimensionDiagnostic"),
            "{} must use tex_command::DimensionDiagnostic",
            path.display()
        );
        assert!(
            !source.lines().any(|line| {
                line.contains("tex_expand::{") && line.contains("DimensionDiagnostic")
            }),
            "{} must not import DimensionDiagnostic through tex-expand",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_recoverable_expansion_diagnostics_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        assert!(
            !source.contains("tex_expand::RecoverableExpansionDiagnostic"),
            "{} must use tex_state::RecoverableExpansionDiagnostic",
            path.display()
        );
        assert!(
            !source.lines().any(|line| {
                line.contains("tex_expand::{") && line.contains("RecoverableExpansionDiagnostic")
            }),
            "{} must not import RecoverableExpansionDiagnostic through tex-expand",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_paragraph_barriers_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in [
            "tex_expand::ParagraphExpansionBarrier",
            "tex_expand::PARAGRAPH_SCANTOKENS_BARRIER_DOMAIN",
            "tex_expand::PARAGRAPH_INPUT_OPEN_BARRIER_DOMAIN",
            "tex_expand::PARAGRAPH_END_INPUT_BARRIER_DOMAIN",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must use the tex-state-owned paragraph barrier contract instead of `{forbidden}`",
                path.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_inserted_input_stays_on_the_input_stack_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        assert!(
            !source.contains("tex_expand::insert_input"),
            "{} must insert never-delivered tokens through InputStack instead of tex-expand",
            path.display()
        );
        assert!(
            !source
                .lines()
                .any(|line| line.contains("use tex_expand::{") && line.contains("insert_input")),
            "{} must not import insert_input through tex-expand",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_backed_up_input_stays_on_the_input_stack_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in ["tex_expand::back_input", "tex_expand::back_error_input"] {
            assert!(
                !source.contains(forbidden),
                "{} must return delivered tokens through InputStack instead of `{forbidden}`",
                path.display()
            );
        }
        assert!(
            !source.lines().any(|line| {
                line.contains("use tex_expand::{")
                    && (line.contains("back_input") || line.contains("back_error_input"))
            }),
            "{} must not import token-backup helpers through tex-expand",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn resource_results_stay_on_the_execution_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let host_api =
        fs::read_to_string(source_root.join("host_api.rs")).expect("read execution host API");
    let public_surface =
        fs::read_to_string(source_root.join("lib.rs")).expect("read public surface");

    assert!(host_api.contains("pub enum ResourceLookup<T>"));
    assert!(host_api.contains("pub struct ResourceNeed"));
    assert!(
        !source_root.join("executor.rs").exists(),
        "retired executor must stay deleted"
    );
    for (source_name, source) in [
        ("execution host API", host_api),
        ("public surface", public_surface),
    ] {
        for forbidden in [
            "tex_expand::ResourceLookup",
            "tex_expand::ResourceResult",
            "pub use tex_expand::ResourceNeed",
            "pub use tex_expand::{ResourceLookup",
        ] {
            assert!(
                !source.contains(forbidden),
                "{source_name} must not regain the retired resource-result owner through `{forbidden}`"
            );
        }
    }

    match tex_exec::ResourceLookup::Available(21_u8).map(u16::from) {
        tex_exec::ResourceLookup::Available(value) => assert_eq!(value, 21),
        _ => panic!("available executor resource must remain available after mapping"),
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn expansion_resource_lookup_values_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in [
            "tex_expand::ResourceLookup",
            "tex_expand::ResourceResult",
            "tex_expand::ResourceNeed",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must use shared state resource values instead of {forbidden}",
                path.display()
            );
        }
    }
}

#[test]
fn command_fuel_can_only_be_owned_by_a_session_ledger() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    let tex_command_dir = manifest_dir.join("../tex-command");
    let dependencies = [CompileFailDependency::path("tex-command", &tex_command_dir)];
    assert_compile_fail(
        "command-fuel-construction-forbidden",
        &manifest_dir.join("tests/ui/command_fuel_construction_forbidden.rs"),
        &dependencies,
        &[
            "associated function `new` is private",
            "the trait bound `CommandFuel: Default` is not satisfied",
        ],
    );
    assert_compile_fail(
        "command-fuel-fields-forbidden",
        &manifest_dir.join("tests/ui/command_fuel_fields_forbidden.rs"),
        &dependencies,
        &["fields `limit` and `burned` of struct `CommandFuel` are private"],
    );
}

#[test]
fn session_ledger_lends_typed_fuel_without_transferring_ownership() {
    fn leaf_operation(fuel: &mut tex_command::CommandFuel) {
        fuel.charge().expect("session funds leaf operation");
    }

    let mut session =
        tex_command::CommandFuelLedger::new(2).expect("valid top-level session limit");
    leaf_operation(session.fuel_mut());
    leaf_operation(session.fuel_mut());
    assert_eq!(session.burned(), 2);
}

#[test]
fn engine_checkpoint_cannot_be_forged_by_callers() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    let tex_state_dir = manifest_dir.join("../tex-state");
    let dependencies = [
        CompileFailDependency::path("tex-exec", &manifest_dir),
        CompileFailDependency::path("tex-state", &tex_state_dir),
    ];
    assert_compile_fail(
        "engine-checkpoint-forgery-forbidden",
        &manifest_dir.join("tests/ui/engine_checkpoint_forgery_forbidden.rs"),
        &dependencies,
        &["cannot construct `EngineCheckpoint`", "private fields"],
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn checkpoints_store_only_command_owned_restart_state() {
    let crate_root = test_support::repository_root().join("crates/tex-exec");
    let checkpoint =
        fs::read_to_string(crate_root.join("src/checkpoint.rs")).expect("read checkpoint boundary");
    let incremental = fs::read_to_string(crate_root.join("../tex-incr/src/lib.rs"))
        .expect("read incremental session");
    let public_surface =
        fs::read_to_string(crate_root.join("src/lib.rs")).expect("read public surface");

    assert!(checkpoint.contains("command: Box<CommandSummary>"));
    assert!(
        !checkpoint.contains("pub fn restore_checkpoint<E"),
        "the dead generic InputStack reconstruction API must not return"
    );
    assert!(!checkpoint.contains("pub enum EngineRestoreError"));
    assert!(!public_surface.contains("EngineRestoreError"));
    for forbidden in [
        "CheckpointContinuation",
        "LegacyInput",
        "InputSummary",
        "InputStack::from_summary",
        "MemoryInput::from_offset",
        "WorldInput::from_content_at_offset",
        "LayoutCursor::new",
        "restore_editor_checkpoint",
    ] {
        assert!(
            !checkpoint.contains(forbidden),
            "checkpoint schema must not reconstruct retired editor input through {forbidden}"
        );
    }
    for forbidden in [
        "execute_revision",
        "execute_advance",
        "InputStack",
        "Executor::new()",
    ] {
        assert!(
            !incremental.contains(forbidden),
            "incremental sessions must not retain legacy restart path {forbidden}"
        );
    }
    assert!(!crate_root.join("src/legacy_editor_restart.rs").exists());
}

#[test]
fn scoped_execution_transaction_cannot_escape_public_api() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    let dependencies = [CompileFailDependency::path("tex-exec", &manifest_dir)];
    assert_compile_fail(
        "execution-transaction-private",
        &manifest_dir.join("tests/ui/execution_transaction_private.rs"),
        &dependencies,
        &["E0603", "module `transaction` is private"],
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn mode_list_mutation_capabilities_do_not_expose_mutable_aggregate_references() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    let mode = fs::read_to_string(manifest_dir.join("src/mode.rs"))
        .expect("read mode-list mutation boundary");

    for forbidden in [
        "fn current_list_mut(",
        "fn list_mut(",
        "fn reconstitution_target(",
        "fn align_state_mut(",
        "impl DerefMut for ModeListMutation",
        "impl AsMut<ModeList> for ModeListMutation",
        "impl BorrowMut<ModeList> for ModeListMutation",
        "fn apply<R>(self",
    ] {
        assert!(
            !mode.contains(forbidden),
            "mode-list mutation boundary must not expose `{forbidden}`"
        );
    }
    for forbidden_return in [
        "-> &mut ModeList",
        "-> Option<&mut ModeList>",
        "-> &mut Vec<Node>",
        "-> Option<&mut Vec<Node>>",
        "-> &mut Node",
        "-> Option<&mut Node>",
        "-> &mut AlignState",
        "-> Option<&mut AlignState>",
    ] {
        assert!(
            !mode.contains(forbidden_return),
            "mode-list API must not return `{forbidden_return}`"
        );
    }
    assert!(
        mode.contains("impl for<'a> FnOnce(&'a mut Node)")
            && mode.contains("impl for<'a> FnOnce(&'a mut Vec<Node>)")
            && mode.contains("impl for<'a> FnOnce(&'a mut AlignState)"),
        "pre-existing aggregate edits must remain behind higher-ranked write barriers"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_main_control_has_one_command_owned_delivery_and_aggregate_rollback_boundary() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    let driver = fs::read_to_string(manifest_dir.join("src/canonical_main_control.rs"))
        .expect("read canonical main-control boundary");

    for forbidden in [
        "use tex_lex",
        ": InputStack",
        "&mut InputStack",
        "next_semantic_raw_token",
        "crate::executor",
        "Executor::",
    ] {
        assert!(
            !driver.contains(forbidden),
            "canonical main control must receive command-owned delivery, not {forbidden}"
        );
    }
    assert!(
        driver.contains("match command.meaning()"),
        "canonical dispatch must classify typed CurrentCommand meanings"
    );
    assert!(
        !driver.contains("command.token()"),
        "canonical main control must not classify a raw token from CurrentCommand"
    );
    assert_eq!(
        driver.matches("fn snapshot_step(").count(),
        1,
        "canonical main control must have one aggregate snapshot constructor"
    );
    assert_eq!(
        driver.matches("fn rollback_step(").count(),
        1,
        "canonical main control must have one aggregate rollback implementation"
    );
    assert_eq!(
        driver.matches("stores.rollback_for_local_retry(").count(),
        1,
        "no family may introduce a separate Universe rollback path"
    );
    assert!(
        !driver.contains("cached_command") && !driver.contains("retained_command"),
        "canonical retries must start a fresh command-owned processor episode"
    );
    for owned_root in [
        "command: CommandState",
        "runtime: CommandRuntime",
        "fuel: tex_command::CommandFuelLedger",
        "capabilities: CommandHostCapabilities",
    ] {
        assert!(
            driver.contains(owned_root),
            "the engine boundary must explicitly own `{owned_root}`"
        );
    }
    for borrowed_root in [
        "command: &'a mut CommandState",
        "runtime: &'a mut CommandRuntime",
        "capabilities: &'a mut CommandHostCapabilities",
        "CommandHostContext::new(capabilities)",
    ] {
        assert!(
            driver.contains(borrowed_root),
            "the sole processor boundary must borrow `{borrowed_root}`"
        );
    }
    assert!(
        driver.contains("struct ObservationBuffer") && driver.contains("pending.flush_into"),
        "observation must be transaction-buffered output from the same command processor, not a cached delivery path"
    );
    // One main-control operation runs several command-processor episodes: the
    // delivery episode, the nested math-field/math-script/`\mathchoice`
    // episodes a host-applied step runs, and the deferred `\output` episode.
    // Each construction site used to decide for itself whether to install the
    // operation's observer, and the nested math episodes never did, so a
    // `^{...}` script field was scanned with zero observations
    // (umber2-johp.195). One constructor, taking the commit slot as a
    // parameter, is what makes that unrepresentable.
    assert_eq!(
        driver.matches("CommandProcessor::new(").count(),
        1,
        "canonical main control must construct every processor episode through one constructor"
    );
    assert_eq!(
        driver.matches(".with_observer(").count(),
        1,
        "whether an episode is observed must be decided in that one constructor"
    );
    assert_eq!(
        driver.matches(".with_fuel(fuel)").count(),
        1,
        "the one constructor must lend the shared run ledger to every processor episode"
    );
    assert_eq!(
        driver.matches("fn command_processor<").count(),
        1,
        "that constructor must be `command_processor`"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_alignment_scanner_phases_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let mut pending = vec![source_root];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).expect("read tex-exec production source") {
            let entry = entry.expect("read tex-exec production source entry");
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "tests") {
                    continue;
                }
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_none_or(|name| name != "tests.rs")
            {
                let source = fs::read_to_string(&path).expect("read production Rust source");
                assert!(
                    !source.contains("tex_lex::AlignmentScannerPhase")
                        && !source.contains("use tex_lex::{AlignmentScannerPhase")
                        && !source.contains(", AlignmentScannerPhase"),
                    "{} must use tex-state's alignment scanner phase identity",
                    path.display()
                );
            }
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn expansion_read_transactions_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let mut pending = vec![source_root];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).expect("read tex-exec production source") {
            let entry = entry.expect("read tex-exec production source entry");
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "tests") {
                    continue;
                }
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_none_or(|name| name != "tests.rs")
            {
                let source = fs::read_to_string(&path).expect("read production Rust source");
                for forbidden in [
                    "tex_expand::ReadRecorder",
                    "tex_expand::ReadRecorderBatch",
                    "tex_expand::ReadSetRecorder",
                ] {
                    assert!(
                        !source.contains(forbidden),
                        "{} must use tex-state's transactional read observation owner, not {forbidden}",
                        path.display()
                    );
                }
                for import in source.lines().filter(|line| line.contains("tex_expand")) {
                    assert!(
                        !import.contains("ReadRecorder"),
                        "{} must not import state-owned read observation through tex-expand: {import}",
                        path.display()
                    );
                }
            }
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_main_control_recovery_does_not_destructure_expand_errors() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let mut pending = vec![source_root];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).expect("read tex-exec production source") {
            let entry = entry.expect("read tex-exec production source entry");
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "tests") {
                    continue;
                }
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_none_or(|name| name != "tests.rs")
            {
                let source = fs::read_to_string(&path).expect("read production Rust source");
                for forbidden in [
                    "tex_expand::ExpandError::UndefinedControlSequence",
                    "tex_expand::ExpandError::Captured",
                    "tex_expand::ExpandError::MacroCall",
                    "tex_expand::ExpandError::ExtraConditionalControl",
                    "tex_expand::args::MacroCallError",
                ] {
                    assert!(
                        !source.contains(forbidden),
                        "{} must consume state-owned expansion recovery, not {forbidden}",
                        path.display()
                    );
                }
            }
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn profiling_feature_forwards_only_to_the_axis_owner() {
    let manifest =
        fs::read_to_string(test_support::repository_root().join("crates/tex-exec/Cargo.toml"))
            .expect("read tex-exec manifest");
    assert!(
        manifest.contains("profiling = [\"tex-state/profiling\"]"),
        "tex-exec profiling must forward only to the tex-state axis owner"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn compatibility_collapse_removes_legacy_execution_from_every_graph() {
    let root = test_support::repository_root().join("crates/tex-exec");
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read tex-exec manifest");
    let normal_dependencies = manifest
        .split("[dependencies]")
        .nth(1)
        .and_then(|tail| tail.split("[dev-dependencies]").next())
        .expect("bounded normal dependency section");
    for dependency in ["tex-expand", "tex-lex"] {
        assert!(
            !normal_dependencies
                .lines()
                .any(|line| line.trim_start().starts_with(dependency)),
            "shipping tex-exec must not retain a normal {dependency} edge"
        );
    }
    let dev_dependencies = manifest
        .split("[dev-dependencies]")
        .nth(1)
        .and_then(|tail| tail.split("[lints]").next())
        .expect("bounded dev dependency section");
    for dependency in ["tex-expand", "tex-lex"] {
        assert!(
            !dev_dependencies
                .lines()
                .any(|line| line.trim_start().starts_with(dependency)),
            "tex-exec must not retain a dev {dependency} edge"
        );
    }

    let lib = fs::read_to_string(root.join("src/lib.rs")).expect("read tex-exec root");
    for module in [
        "assignments",
        "executor",
        "legacy_assignments",
        "legacy_diagnostics",
        "legacy_dispatch",
        "legacy_output",
        "legacy_paragraph_memo",
        "raw_delivery",
    ] {
        assert!(
            !lib.contains(&format!("mod {module};")),
            "retired {module} must be absent from every module graph"
        );
    }
    for export in [
        "pub use executor::",
        "pub use legacy_assignments::",
        "pub use legacy_dispatch::",
    ] {
        assert!(
            !lib.contains(export),
            "retired compatibility export {export} must be absent"
        );
    }

    let canonical = fs::read_to_string(root.join("src/canonical_main_control.rs"))
        .expect("read canonical main control");
    for forbidden in [
        "crate::assignments",
        "crate::legacy_",
        "ExecutionContext",
        "Executor",
        "tex_expand",
        "tex_lex",
    ] {
        assert!(
            !canonical.contains(forbidden),
            "shipping canonical control must not reach {forbidden}"
        );
    }
}
