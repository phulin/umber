use super::support::*;
use super::*;

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
