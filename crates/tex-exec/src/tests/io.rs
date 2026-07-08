use super::support::*;
use super::*;
use tex_expand::ReadRecorder;
use tex_out::{EffectSink, PageArtifact, PageEffect, PageNode};
use tex_state::interner::Symbol;

#[test]
fn openin_read_defines_control_sequence_from_world_stream() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("stream.tex", b"abc\nnext".to_vec())
        .expect("seed stream");
    let mut input = InputStack::new(MemoryInput::new(
        "\\openin1=stream.tex \\read1 to \\foo \\message{\\foo}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("read from opened stream");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("abc"));
    assert!(
        !stores
            .world()
            .input_stream_eof(tex_state::StreamSlot::new(1))
    );
}

#[test]
fn read_consumes_additional_stream_lines_until_braces_balance() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("stream.tex", b"{abc\ndef}\nnext".to_vec())
        .expect("seed stream");
    let mut input = InputStack::new(MemoryInput::new(
        "\\openin1=stream.tex \\read1 to \\foo \\message{\\foo}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("read balanced multiline stream");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("abc"));
    assert!(output.contains("def"));
    assert!(
        !stores
            .world()
            .input_stream_eof(tex_state::StreamSlot::new(1))
    );
}

#[test]
fn read_stream_cursor_rolls_back_with_universe_snapshot() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("stream.tex", b"one\ntwo".to_vec())
        .expect("seed stream");
    let mut open = InputStack::new(MemoryInput::new("\\openin1=stream.tex"));
    Executor::new()
        .run(&mut open, &mut stores)
        .expect("open stream");
    let snapshot = stores.snapshot();

    let mut first = InputStack::new(MemoryInput::new("\\read1 to \\foo \\message{\\foo}\\end"));
    Executor::new()
        .run(&mut first, &mut stores)
        .expect("first read");
    assert!(terminal_effect_text(&stores).contains("one"));

    stores.rollback(&snapshot);
    let mut second = InputStack::new(MemoryInput::new("\\read1 to \\foo \\message{\\foo}\\end"));
    Executor::new()
        .run(&mut second, &mut stores)
        .expect("reread after rollback");

    assert!(terminal_effect_text(&stores).contains("one"));
    assert!(
        !stores
            .world()
            .input_stream_eof(tex_state::StreamSlot::new(1))
    );
}

#[test]
fn read_at_open_stream_eof_defines_empty_line_and_closes_stream() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("stream.tex", b"abc".to_vec())
        .expect("seed stream");
    let mut input = InputStack::new(MemoryInput::new(
        "\\openin1=stream.tex \\read1 to \\foo \\read1 to \\bar \\message{[\\bar]}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("read EOF line");

    assert!(
        stores
            .world()
            .stream_bufs()
            .read_stream_target(tex_state::StreamSlot::new(1))
            .is_none()
    );
    let bar = stores.symbol("bar").expect("bar was defined");
    assert!(
        stores.macro_meaning(bar).is_some(),
        "EOF read still defines the target macro"
    );
}

#[test]
fn read_missing_stream_in_nonstop_mode_errors_without_terminal_prompt() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores.set_interaction_mode(InteractionMode::Nonstop);
    let mut input = InputStack::new(MemoryInput::new("\\openin1=missing.tex \\read1 to \\foo"));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("nonstop mode cannot read terminal");

    assert_eq!(
        err.to_string(),
        "I can't \\read from terminal in nonstop modes"
    );
}

#[test]
fn read_missing_stream_in_errorstop_mode_uses_terminal_line() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores.set_interaction_mode(InteractionMode::ErrorStop);
    stores
        .world_mut()
        .push_memory_terminal_line("typed")
        .expect("seed terminal");
    let mut input = InputStack::new(MemoryInput::new(
        "\\openin1=missing.tex \\read1 to \\foo \\message{\\foo}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("terminal read");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("\\foo="));
    assert!(output.contains("typed"));
}

#[test]
fn openout_closeout_append_world_effect_records() {
    let mut stores = Universe::new();
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\openout2=out.aux \\closeout2\\end"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("openout closeout");

    assert!(matches!(
        stores.world().effect_records(),
        [
            EffectRecord::StreamOpen { slot, target },
            EffectRecord::StreamClose { slot: close_slot }
        ] if *slot == tex_state::StreamSlot::new(2)
            && *close_slot == tex_state::StreamSlot::new(2)
            && target.path() == std::path::Path::new("out.aux")
    ));
}

#[test]
fn shipout_expands_write_against_barrier_state_and_stores_artifact() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\count0=7 \\setbox0=\\hbox{\\write16{p:\\the\\count0}}\
         \\count0=9 \\shipout\\box0\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout succeeds");
    let effect_pos = stores.world().effect_pos();
    stores
        .commit_effects(effect_pos)
        .expect("final commit is idempotent");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(memory_terminal_text(&stores), "p:9");
    assert_eq!(memory_log_text(&stores), "p:9");
    assert!(
        stores.world().effect_records().is_empty(),
        "shipout should flush the committed effect prefix"
    );

    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    assert_eq!(artifact.counts[0], 9);
    assert!(matches!(
        artifact.effects.as_slice(),
        [PageEffect::Write {
            sink: EffectSink::TerminalAndLog,
            text
        }] if text == "p:9"
    ));
    assert!(matches!(
        artifact.root,
        PageNode::HList(ref box_node)
            if matches!(box_node.children.as_slice(), [PageNode::WhatsitAnchor { effect_index: 0 }])
    ));
}

#[test]
fn shipout_copy_expands_deferred_write_each_time() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox{\\write16{p:\\the\\count0}}\
         \\count0=1 \\shipout\\copy0\
         \\count0=2 \\shipout\\copy0\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout copies succeed");

    assert_eq!(stats.shipped_artifacts.len(), 2);
    assert_ne!(stats.shipped_artifacts[0], stats.shipped_artifacts[1]);
    assert_eq!(memory_terminal_text(&stores), "p:1p:2");
}

#[test]
fn rollback_after_shipout_does_not_replay_committed_effects() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut ship = InputStack::new(MemoryInput::new("\\shipout\\hbox{\\write16{once}}\\end"));

    Executor::new()
        .run(&mut ship, &mut stores)
        .expect("shipout succeeds");
    let snapshot = stores.snapshot();

    let mut later = InputStack::new(MemoryInput::new("\\message{later}\\end"));
    Executor::new()
        .run(&mut later, &mut stores)
        .expect("later message succeeds");
    stores.rollback(&snapshot);
    let effect_pos = stores.world().effect_pos();
    stores
        .commit_effects(effect_pos)
        .expect("post-rollback final commit succeeds");

    assert_eq!(memory_terminal_text(&stores), "once");
}

#[test]
fn shipout_write_expansion_uses_active_read_recorder() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\count0=5 \\shipout\\hbox{\\write16{\\the\\count0}}\\end",
    ));
    let mut recorder = SawTheRecorder::default();
    let mut hooks = NoopExecHooks;

    Executor::new()
        .run_with_recorder_and_hooks(&mut input, &mut stores, &mut recorder, &mut hooks)
        .expect("shipout succeeds");

    assert!(
        recorder.saw_the,
        "shipout-time deferred write expansion should use the active recorder"
    );
}

#[derive(Default)]
struct SawTheRecorder {
    saw_the: bool,
}

impl ReadRecorder for SawTheRecorder {
    fn record_meaning(&mut self, _symbol: Symbol, meaning: Meaning) {
        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::The) {
            self.saw_the = true;
        }
    }
}

fn memory_terminal_text(stores: &Universe) -> String {
    String::from_utf8_lossy(
        stores
            .world()
            .memory_terminal_output()
            .expect("memory terminal output"),
    )
    .into_owned()
}

fn memory_log_text(stores: &Universe) -> String {
    String::from_utf8_lossy(
        stores
            .world()
            .memory_log_output()
            .expect("memory log output"),
    )
    .into_owned()
}
