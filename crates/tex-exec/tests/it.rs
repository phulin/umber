use std::sync::Arc;

use test_support::{CompileFailDependency, assert_compile_fail};
use tex_command::{RegisteredSourceKind, SourceRegistration};
use tex_exec::{MainControl, MainControlStep};
use tex_state::{
    EffectRecord, InteractionMode, PrintSink, Universe,
    meaning::{Meaning, UnexpandablePrimitive},
};

fn run_tex82(source: &[u8], tracing_online: bool) -> String {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    if tracing_online {
        stores.set_int_param(tex_state::env::banks::IntParam::TRACING_ONLINE, 1);
    }
    let mut control = MainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("test source registers");

    loop {
        match control.step(&mut stores).expect("test source executes") {
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
    committed + &pending
}

#[test]
fn let_endgroup_alias_runs_off_save_and_restores_the_primitive() {
    // TeX82 §§1215/1063--1066: `\let` copies the `end_group` command,
    // so the alias must take `off_save` inside a `simple_group`, regardless
    // of its spelling. The inserted right brace then runs §283 `unsave`
    // before the alias is replayed, restoring the locally redefined
    // `\endgroup`. Frozen alignment sentinels have distinct `EndV`/
    // `EndTemplate` meanings and continue through the alignment dispatch
    // covered by the tests below.
    let mut stores = Universe::new_with_plain_catcodes();
    let alias = stores.intern("alias");
    let restored = stores.intern("restored");
    stores.set_interaction_mode(InteractionMode::Nonstop);
    let mut control = MainControl::tex82_initex(&mut stores);
    control.set_fuel_limit(128).expect("bounded command fuel");
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(
                br"\let\alias=\endgroup{\def\endgroup{\alias\alias}\alias\let\restored=\endgroup\count0=17"
                    .as_slice(),
            ),
        ))
        .expect("test source registers");

    loop {
        match control.step(&mut stores).expect("alias recovery executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert_eq!(
        stores.meaning(alias),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndGroup)
    );
    assert_eq!(stores.meaning(restored), stores.meaning(alias));
    assert_eq!(stores.count(0), 17);
    assert!(control.fuel_burned() < 128);
    let transcript = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(transcript.matches("! Missing } inserted.").count(), 1);
    assert_eq!(transcript.matches("! Extra \\endgroup.").count(), 1);
}

#[test]
fn restricted_horizontal_hrule_reports_source_before_rule_spec_lookahead() {
    // TeX82 §1095 diagnoses this command in `head_for_vmode`, before §463
    // scans a rule specification. §82 must therefore display the physical
    // source line, not a token level created by keyword lookahead.
    let transcript = run_tex82(b"\\setbox0=\\hbox{\n\\hrule\n}\\end", true);
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
    // `scan_optional_equals` and `scan_normal_dimen`.
    let transcript = run_tex82(br"\setbox0=\hbox{\prevdepth\relax X}\end", false);
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
    let transcript = run_tex82(
        br"\long\def\l#1{}\let\PAR=\par\def\par{\relax\PAR}\halign{#&#&\l{#}\cr a&b&c&&&.}\par\cr}\end",
        true,
    );
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
    let transcript = run_tex82(
        br"\let\lb={\let\rb=}\halign\relax{\span\iffalse}\fi\cr#&\ifnum0=`{\fi\cr\cr}\end",
        false,
    );
    assert!(
        transcript.contains("<template> &\n            \\ifnum 0=`{\\fi \\endtemplate "),
        "TeX82 §§82,1128 diagnose before the retained v-template retires: {transcript}"
    );
}

#[test]
fn paragraph_start_page_build_reports_backed_up_context_before_help() {
    let transcript = run_tex82(
        br"\topskip=0pt \vsize=100pt \setbox1=\hbox{}\copy1 \vskip0pt minus 1fil$x$\end",
        true,
    );
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
    let transcript = run_tex82(br"\setbox3=\hbox{x}$\unhcopy3\accent65x$\end", true);
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

#[test]
fn resource_lookup_maps_available_values() {
    match tex_exec::ResourceLookup::Available(21_u8).map(u16::from) {
        tex_exec::ResourceLookup::Available(value) => assert_eq!(value, 21),
        _ => panic!("available execution resource must remain available after mapping"),
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
