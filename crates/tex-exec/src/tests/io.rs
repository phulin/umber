use super::support::*;
use super::*;
use test_support::{corpus_root, read_fixture};
use tex_out::dvi::write_dvi;
use tex_out::{
    DiscKind as PageDiscKind, EffectSink, PageArtifact, PageEffect, PageNode, PageToken,
};
use tex_state::scaled::Scaled;

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
fn readline_uses_only_space_and_other_catcodes() {
    // e-TeX short reference manual section 3.2: unlike \read, \readline
    // assigns catcode 10 to spaces and catcode 12 to every other character.
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    stores
        .world_mut()
        .set_memory_file("stream.tex", b"a {\\x".to_vec())
        .expect("seed stream");
    let mut input = InputStack::new(MemoryInput::new(
        "\\openin1=stream.tex \\readline1 to \\foo \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("readline from opened stream");

    let foo = stores.symbol("foo").expect("readline target was defined");
    let replacement = stores
        .macro_meaning(foo)
        .expect("readline target is a macro")
        .replacement_text();
    let tokens = stores.tokens(replacement);
    assert_eq!(
        tokens
            .iter()
            .filter_map(|token| match token {
                Token::Char { ch, .. } => Some(*ch),
                _ => None,
            })
            .collect::<String>(),
        "a {\\x\r"
    );
    assert!(tokens.iter().all(|token| matches!(
        token,
        Token::Char {
            ch: ' ',
            cat: Catcode::Space
        } | Token::Char {
            cat: Catcode::Other,
            ..
        }
    )));
}

#[test]
fn read_consumes_invalid_category_characters_without_unwinding() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores.set_catcode('0', Catcode::Invalid);
    stores
        .world_mut()
        .set_memory_file("stream.tex", b"a0b".to_vec())
        .expect("seed stream");
    let mut input = InputStack::new(MemoryInput::new(
        "\\openin1=stream.tex \\read1 to \\foo \\message{\\foo}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("read skips invalid-category input characters");

    assert!(terminal_effect_text(&stores).contains("ab"));
}

#[test]
fn read_closes_partial_group_and_stops_at_outer_macro() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("stream.tex", b"{a\\stop".to_vec())
        .expect("seed stream");
    let mut input = InputStack::new(MemoryInput::new(
        "\\outer\\def\\stop{}\\openin1=stream.tex \\read1 to \\foo",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("outer token aborts read recoverably");

    let foo = stores.symbol("foo").expect("read target");
    let meaning = stores.macro_meaning(foo).expect("read-defined macro");
    assert_eq!(stores.tokens(meaning.replacement_text()).len(), 3);
}

#[test]
fn read_loop_observes_eof_after_outer_aborted_final_line() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("tripos", b"\\uppercase{0[".to_vec())
        .expect("seed stream");
    let mut input = InputStack::new(MemoryInput::new(
        "\\openin0=tripos \\def\\loop{\\ifeof0\\let\\loop=\\relax\\else{\\global\\read0to\\a}\\fi\\loop}\\catcode`0=15\\catcode`[=1\\outer\\def\\uppercase{}\\loop\\count1=7",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("read loop reaches eof after final unterminated line");

    assert_eq!(stores.count(1), 7);
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
fn openout_closeout_append_deferred_whatsits_before_shipout() {
    let mut stores = Universe::new();
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\openout2=out.aux \\closeout2"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("openout closeout");

    assert!(
        stores.world().effect_records().is_empty(),
        "non-immediate openout/closeout should wait for shipout"
    );
    let contributions = stores.page_contributions();
    assert_eq!(contributions.len(), 2);
    assert!(matches!(
        (contributions.front(), contributions.back()),
        (
            Some(tex_state::node::Node::Whatsit(tex_state::node::Whatsit::OpenOut { slot, path })),
            Some(tex_state::node::Node::Whatsit(tex_state::node::Whatsit::CloseOut { slot: close_slot }))
        ) if *slot == tex_state::StreamSlot::new(2)
            && *close_slot == tex_state::StreamSlot::new(2)
            && path == "out.aux"
    ));
}

#[test]
fn immediate_openout_write_closeout_append_world_effect_records() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\count0=3 \
         \\immediate\\openout2=imm.out \
         \\immediate\\write2{p:\\the\\count0}\
         \\immediate\\closeout2",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("immediate stream commands");

    assert!(matches!(
        stores.world().effect_records(),
        [
            EffectRecord::StreamOpen { slot, target },
            EffectRecord::StreamWrite {
                sink: tex_state::PrintSink::Stream(write_slot),
                text
            },
            EffectRecord::StreamClose { slot: close_slot },
        ] if *slot == tex_state::StreamSlot::new(2)
            && *write_slot == tex_state::StreamSlot::new(2)
            && *close_slot == tex_state::StreamSlot::new(2)
            && target.path() == std::path::Path::new("imm.out")
            && text == "p:3\n"
    ));
}

#[test]
fn immediate_openout_defaults_an_extensionless_name_at_execution() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"\immediate\openout10=tripos \immediate\write10{line}\immediate\closeout10",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("immediate stream commands");

    assert!(matches!(
        stores.world().effect_records(),
        [
            EffectRecord::StreamOpen { target, .. },
            EffectRecord::StreamWrite { .. },
            EffectRecord::StreamClose { .. },
        ] if target.path() == std::path::Path::new("tripos.tex")
    ));
}

#[test]
fn newlinechar_is_honored_by_message_and_immediate_write() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\newlinechar=`| \
         \\message{m|n}\
         \\errmessage{e|f}\
         \\immediate\\openout2=nl.out \
         \\immediate\\write2{w|x}\
         \\immediate\\closeout2",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("newlinechar output executes");

    assert!(terminal_effect_text(&stores).contains("m\nn"));
    assert!(terminal_effect_text(&stores).contains("e\nf"));
    assert!(matches!(
        stores.world().effect_records(),
        [
            EffectRecord::StreamWrite { .. },
            EffectRecord::StreamWrite { .. },
            EffectRecord::StreamOpen { .. },
            EffectRecord::StreamWrite {
                sink: tex_state::PrintSink::Stream(write_slot),
                text
            },
            EffectRecord::StreamClose { .. },
        ] if *write_slot == tex_state::StreamSlot::new(2) && text == "w\nx\n"
    ));
}

#[test]
fn protected_macros_are_preserved_in_immediate_write_expansion() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\protected\\def\\p{expanded}\\immediate\\write2{\\p}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("protected write expansion");

    let output = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(output.contains("\\p "));
    assert!(!output.contains("expanded"));
}

#[test]
fn print_cs_spacing_is_shared_by_diagnostics_and_immediate_and_deferred_writes() {
    const DEFINITIONS: &str = r"\let\foo=\relax
          \let\@=\relax
          \expandafter\def\expandafter\multiother\expandafter{\csname @@\endcsname X}
          \def\multiletter{\foo X}
          \def\single{\@X}
          \catcode`\@=11
          \catcode`\~=13 \let~=\relax \def\active{~X}
          ";

    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(format!(
        "{DEFINITIONS}\\show\\multiother \\show\\single \\show\\active \\end"
    )));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("control-sequence diagnostic fixture executes");
    let diagnostic = terminal_effect_text(&stores);
    assert!(diagnostic.contains("> \\multiother=macro:\n->\\@@ X."));
    assert!(diagnostic.contains("> \\single=macro:\n->\\@ X."));
    assert!(diagnostic.contains("> \\active=macro:\n->~X."));

    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        [
            DEFINITIONS,
            r"
          \immediate\openout2=printcs.out
          \immediate\write2{\multiother|\multiletter|\single|\active}
          \catcode`\@=12
          \shipout\hbox{\write2{\multiother|\multiletter|\single|\active}}
          \immediate\closeout2
          \end",
        ]
        .concat(),
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("control-sequence rendering fixture executes");

    assert_eq!(
        stores.world().memory_output("printcs.out"),
        Some(&b"\\@@ X|\\foo X|\\@ X|~X\n\\@@ X|\\foo X|\\@X|~X\n"[..])
    );
}

#[test]
fn shipout_commits_deferred_openout_closeout_whatsits() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\shipout\\hbox{\\openout2=out.aux \\write2{alpha}\\closeout2}\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout succeeds");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(
        stores.world().memory_output("out.aux"),
        Some(&b"alpha\n"[..])
    );
    assert!(
        stores.world().effect_records().is_empty(),
        "shipout should flush deferred open/write/close effects"
    );

    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    assert!(matches!(
        artifact.effects.as_slice(),
        [
            PageEffect::OpenOut { stream: 2, path },
            PageEffect::Write {
                sink: EffectSink::Stream(2),
                text
            },
            PageEffect::CloseOut { stream: 2 },
        ] if path == "out.aux" && text == "alpha\n"
    ));
    assert!(matches!(
        artifact.root,
        PageNode::HList(ref box_node)
            if matches!(
                box_node.children.as_slice(),
                [
                    PageNode::WhatsitAnchor { effect_index: 0 },
                    PageNode::WhatsitAnchor { effect_index: 1 },
                    PageNode::WhatsitAnchor { effect_index: 2 },
                ]
            )
    ));
}

#[test]
fn shipped_extensionless_openout_is_visible_to_same_job_input() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\shipout\\hbox{\\openout10=tripos \\write10{} \\write10{\\uppercase{0{\\outputpenalty}}} \\write10{[\\uppercase{mmmmmmmmmm}[} \\closeout10}\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout succeeds");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(stores.world().memory_output("tripos"), None);
    let expected = b"\n\\uppercase {0{\\outputpenalty }}\n[\\uppercase {mmmmmmmmmm}[\n";
    assert_eq!(
        stores.world().memory_output("tripos.tex"),
        Some(&expected[..])
    );
    let content = stores
        .world_mut()
        .read_file("tripos.tex")
        .expect("committed shipout output is readable");
    assert_eq!(content.bytes(), expected);

    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    assert!(matches!(
        artifact.effects.first(),
        Some(PageEffect::OpenOut { path, .. }) if path == "tripos.tex"
    ));
}

#[test]
fn shipout_artifacts_ignore_source_token_provenance() {
    let left = shipout_artifact_bytes("\\shipout\\hbox{}\\end");
    let right = shipout_artifact_bytes("   \\shipout\\hbox{}\\end");

    assert_eq!(left, right);
    assert_eq!(
        tex_state::ContentHash::from_bytes(&left),
        tex_state::ContentHash::from_bytes(&right)
    );
}

#[test]
fn newlinechar_is_honored_by_deferred_shipout_write() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\newlinechar=`| \
         \\shipout\\hbox{\\openout2=ship.out \\write2{s|t}\\closeout2}\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout write executes");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(
        stores.world().memory_output("ship.out"),
        Some(&b"s\nt\n"[..])
    );
    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    assert!(matches!(
        artifact.effects.as_slice(),
        [
            PageEffect::OpenOut { .. },
            PageEffect::Write {
                sink: EffectSink::Stream(2),
                text
            },
            PageEffect::CloseOut { .. },
        ] if text == "s\nt\n"
    ));
}

#[test]
fn top_level_deferred_openout_closeout_without_write_materializes_empty_output() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\openout2=empty.out \\closeout2\\end"));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("final cleanup succeeds");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(stores.world().memory_output("empty.out"), Some(&b""[..]));
    assert!(
        stores.world().effect_records().is_empty(),
        "final cleanup should commit the shipped open/close effects"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side committed parity fixture.
fn top_level_deferred_openout_closeout_ship_during_final_cleanup() {
    let source = read_io_source("top_open_close");
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(&source));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("final cleanup succeeds");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(stores.world().memory_output("top.out"), Some(&b"top\n"[..]));

    let reference = read_fixture("tex_exec_io", "top_open_close", "out");
    assert_eq!(reference.as_bytes(), b"top\n");
}

#[test]
fn copied_box_replays_deferred_openout_closeout_per_shipout() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox{\\openout2=copy.out \\write2{p:\\the\\count0}\\closeout2}\
         \\count0=1 \\shipout\\copy0\
         \\count0=2 \\shipout\\copy0\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout copies succeed");

    assert_eq!(stats.shipped_artifacts.len(), 2);
    assert_eq!(
        stores.world().memory_output("copy.out"),
        Some(&b"p:2\n"[..]),
        "second replayed openout truncates the stream like TeX"
    );

    for (index, expected) in stats.shipped_artifacts.iter().zip(["p:1\n", "p:2\n"]) {
        let bytes = stores
            .world()
            .read_artifact(*index)
            .expect("read artifact")
            .expect("artifact stored");
        let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
        assert!(matches!(
            artifact.effects.as_slice(),
            [
                PageEffect::OpenOut { stream: 2, path },
                PageEffect::Write {
                    sink: EffectSink::Stream(2),
                    text
                },
                PageEffect::CloseOut { stream: 2 },
            ] if path == "copy.out" && text == expected
        ));
    }
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
    assert_eq!(memory_terminal_text(&stores), "p:9\n");
    assert_eq!(memory_log_text(&stores), "p:9\n");
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
        }] if text == "p:9\n"
    ));
    assert!(matches!(
        artifact.root,
        PageNode::HList(ref box_node)
            if matches!(box_node.children.as_slice(), [PageNode::WhatsitAnchor { effect_index: 0 }])
    ));
}

#[test]
fn shipout_reports_illegal_magnification_diagnostic() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\mag=40000 \\shipout\\hbox{}\\end"));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout succeeds");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(stores.mag(), 1000);
    assert_eq!(stores.prepared_mag(), Some(1000));
    assert!(
        memory_terminal_text(&stores)
            .contains("! Illegal magnification has been changed to 1000 (40000).")
    );
    assert!(
        memory_log_text(&stores)
            .contains("! Illegal magnification has been changed to 1000 (40000).")
    );

    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    assert_eq!(artifact.job.mag, 1000);
}

#[test]
fn shipout_artifact_captures_page_offsets() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\hoffset=12sp \\voffset=-34sp \\shipout\\hbox{}\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout succeeds");
    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");

    assert_eq!(artifact.job.h_offset, Scaled::from_raw(12));
    assert_eq!(artifact.job.v_offset, Scaled::from_raw(-34));
}

#[test]
fn huge_shipout_is_diagnosed_without_committing_an_artifact() {
    let mut stores = support::stores_with_fonts();
    let before = stores.snapshot();
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\vbox to8192pt{}\\shipout\\vbox{\\copy0\\box0}\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("huge shipout should recover");

    assert!(stats.shipped_artifacts.is_empty());
    assert!(stores.world().artifact_commits().is_empty());
    assert!(support::terminal_effect_text(&stores).contains("Huge page cannot be shipped out"));
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&before);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\vbox to8192pt{}\\shipout\\vbox{\\copy0\\box0}\\end",
    ));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("huge shipout replay should recover");
    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
fn shipout_reports_incompatible_magnification_diagnostic() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\mag=1200 \\shipout\\hbox{} \\mag=2000 \\shipout\\hbox{}\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipouts succeed");

    assert_eq!(stats.shipped_artifacts.len(), 2);
    assert_eq!(stores.mag(), 1200);
    assert_eq!(stores.prepared_mag(), Some(1200));
    assert!(
        memory_terminal_text(&stores)
            .contains("! Incompatible magnification (2000); the previous value will be retained.")
    );
    assert!(
        memory_log_text(&stores)
            .contains("! Incompatible magnification (2000); the previous value will be retained.")
    );

    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[1])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    assert_eq!(artifact.job.mag, 1200);
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
    assert_eq!(memory_terminal_text(&stores), "p:1\np:2\n");
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

    assert_eq!(memory_terminal_text(&stores), "once\n");
}

#[test]
fn shipout_write_expansion_uses_active_read_recorder() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\count0=5 \\shipout\\hbox{\\write16{\\the\\count0}}\\end",
    ));
    let mut recorder = TestRecorder::default();
    let mut hooks = NoopExecHooks;

    Executor::new()
        .run_with_recorder_and_hooks(&mut input, &mut stores, &mut recorder, &mut hooks)
        .expect("shipout succeeds");

    assert!(
        recorder
            .meanings
            .contains(&Meaning::ExpandablePrimitive(ExpandablePrimitive::The)),
        "shipout-time deferred write expansion should use the active recorder"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side committed parity fixture.
fn source_special_lowers_to_anchored_dvi_xxx_payload() {
    let source = read_io_source("special_payload");
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(&source));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout succeeds");

    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    assert!(matches!(
        artifact.effects.as_slice(),
        [PageEffect::Special { class, payload }]
            if class == "dvi" && payload == b"pre abc-42"
    ));
    assert!(matches!(
        artifact.root,
        PageNode::HList(ref box_node)
            if matches!(box_node.children.as_slice(), [PageNode::WhatsitAnchor { effect_index: 0 }])
    ));

    let dvi = write_dvi(std::slice::from_ref(&artifact)).expect("DVI writes");
    assert_eq!(
        format_special_payloads(&dvi_special_payloads(&dvi)),
        read_fixture("tex_exec_io", "special_payload", "specials")
    );
}

#[test]
fn source_special_preserves_tex_character_bytes() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox{\\special{\u{80}}}\\shipout\\box0",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout succeeds");
    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");

    assert!(matches!(
        artifact.effects.as_slice(),
        [PageEffect::Special { payload, .. }] if payload == &[0x80]
    ));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side committed parity fixture.
fn leader_payload_suppresses_deferred_write_but_keeps_specials() {
    let source = read_io_source("leader_payload_effects");
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(&source));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout succeeds");
    let effect_pos = stores.world().effect_pos();
    stores
        .commit_effects(effect_pos)
        .expect("final commit is idempotent");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(stores.world().memory_output("leader.out"), None);
    assert!(!memory_terminal_text(&stores).contains("leader-write"));
    assert!(!memory_log_text(&stores).contains("leader-write"));
    assert!(
        stores.world().effect_records().is_empty(),
        "shipout should flush only the committed, non-suppressed effect prefix"
    );

    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    assert!(
        artifact
            .effects
            .iter()
            .all(|effect| matches!(effect, PageEffect::Special { .. })),
        "leader-contained deferred stream whatsits should not become page effects"
    );
    assert!(
        artifact
            .effects
            .iter()
            .any(|effect| matches!(effect, PageEffect::Special { payload, .. } if payload == b"leader-special"))
    );

    let dvi = write_dvi(std::slice::from_ref(&artifact)).expect("DVI writes");
    let actual_specials = dvi_special_payloads(&dvi);
    assert!(
        !actual_specials.is_empty(),
        "leader-contained specials should still emit DVI xxx output"
    );

    let reference_effects = read_fixture("tex_exec_io", "leader_payload_effects", "effects");
    assert!(reference_effects.contains("leader.out: absent"));
    assert!(reference_effects.contains("leader-write-in-log: false"));
    assert_eq!(
        format_special_payloads(&actual_specials),
        read_fixture("tex_exec_io", "leader_payload_effects", "specials")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side committed parity fixture.
fn ordinary_shipped_openout_closeout_matches_reference_file_effect() {
    let source = read_io_source("ordinary_open_close");
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(&source));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout succeeds");

    assert_eq!(
        stores.world().memory_output("ordinary.out"),
        Some(&b"ordinary\n"[..])
    );

    let reference = read_fixture("tex_exec_io", "ordinary_open_close", "out");
    assert_eq!(reference.as_bytes(), b"ordinary\n");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side committed parity fixture.
fn openout_closeout_without_write_matches_reference_materialization() {
    let source = read_io_source("open_close_without_write");
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(&source));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("open close without writes succeeds");

    let actual = format_output_presence(
        &stores,
        &["immediate.out", "shipped.out", "boxed.out", "top.out"],
    );
    assert_eq!(
        actual,
        read_fixture("tex_exec_io", "open_close_without_write", "effects")
    );
}

#[test]
fn copied_special_reuses_scan_time_expansion() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\count0=1 \\setbox0=\\hbox{\\special{\\the\\count0}}\
         \\count0=2 \\shipout\\copy0\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout succeeds");
    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");

    assert!(matches!(
        artifact.effects.as_slice(),
        [PageEffect::Special { payload, .. }] if payload == b"1"
    ));
}

#[test]
fn shipout_converts_deferred_math_lists_before_artifact_lowering() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores.set_dimen_param(DimenParam::MATH_SURROUND, Scaled::from_raw(123));

    let content = stores.freeze_node_list(&[tex_state::node::Node::MathNoad(
        tex_state::math::MathNoad::new(
            tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Ord),
            tex_state::math::MathField::Empty,
        ),
    )]);
    let children = stores.freeze_node_list(&[tex_state::node::Node::MathList(
        tex_state::math::MathListNode {
            display: false,
            content,
        },
    )]);
    let root = tex_state::node::Node::HList(tex_state::node::BoxNode::new(
        tex_state::node::BoxNodeFields {
            width: Scaled::from_raw(246),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: tex_state::scaled::GlueSetRatio::ZERO,
            glue_sign: tex_state::node::Sign::Normal,
            glue_order: tex_state::glue::Order::Normal,
            children,
        },
    ));
    let root_list = stores.freeze_node_list(&[root]);
    stores.set_box_reg(0, root_list);
    let mut input = InputStack::new(MemoryInput::new("\\shipout\\box0\\end"));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("deferred math list shipout succeeds");
    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    let PageNode::HList(box_node) = &artifact.root else {
        panic!("shipout root should lower to hlist");
    };
    assert!(matches!(
        box_node.children.as_slice(),
        [PageNode::MathOn(width), PageNode::MathOff(end_width)]
            if width.raw() == 123 && end_width.raw() == 123
    ));
}

#[test]
fn etex_shipout_reorders_nested_tex_xet_segments_into_visual_order() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"\font\f=cmr10 \relax \f\TeXXeTstate=1
          \shipout\hbox{A\beginR BC\beginL DE\endL FG\endR H}\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("nested TeX--XeT shipout succeeds");
    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    let PageNode::HList(box_node) = &artifact.root else {
        panic!("shipout root should be an hlist");
    };
    fn append_visual_text(nodes: &[PageNode], text: &mut String) {
        for node in nodes {
            match node {
                PageNode::Char { ch, .. } | PageNode::Lig { ch, .. } => {
                    text.push(char::from_u32(*ch).expect("stored character is valid"));
                }
                PageNode::HList(box_node) | PageNode::VList(box_node) => {
                    append_visual_text(&box_node.children, text);
                }
                _ => {}
            }
        }
    }
    let mut visual_text = String::new();
    append_visual_text(&box_node.children, &mut visual_text);

    assert_eq!(visual_text, "AGFDECBH", "{artifact:#?}");
}

#[test]
fn inline_math_restores_normal_space_for_dvi_movement_reuse() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"\font\f=cmr10 \relax \f \textfont0=\f \scriptfont0=\f \scriptscriptfont0=\f
          \shipout\hbox{A B\spacefactor=2000 $0$ if}\end",
    ));
    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("inline-math shipout succeeds");
    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    let dvi = write_dvi(std::slice::from_ref(&artifact)).expect("DVI writes");

    assert!(
        dvi.windows(4).any(|window| window == [150, 3, 85, 85]),
        "the first 218453sp font space should establish DVI w"
    );
    assert!(
        dvi.windows(2).any(|window| window == [147, b'i']),
        "the post-math font space should reuse the normal-space w value"
    );
}

#[test]
fn shipout_lowers_supported_whatsit_adjacent_nodes_without_reordering_effects() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let cs = stores.intern("markcs");
    let mark_tokens = stores.intern_token_list(&[
        Token::Char {
            ch: 'm',
            cat: Catcode::Letter,
        },
        Token::Cs(cs.symbol()),
        Token::param(2),
    ]);
    let disc_pre = stores.freeze_node_list(&[tex_state::node::Node::Kern {
        amount: Scaled::from_raw(11),
        kind: tex_state::node::KernKind::Explicit,
    }]);
    let disc_post = stores.freeze_node_list(&[tex_state::node::Node::Penalty(22)]);
    let disc_replace = stores.freeze_node_list(&[tex_state::node::Node::Rule {
        width: Some(Scaled::from_raw(33)),
        height: Some(Scaled::from_raw(44)),
        depth: Some(Scaled::from_raw(0)),
    }]);
    let insert_content = stores.freeze_node_list(&[tex_state::node::Node::Penalty(55)]);
    let adjust_content = stores.freeze_node_list(&[tex_state::node::Node::Kern {
        amount: Scaled::from_raw(66),
        kind: tex_state::node::KernKind::Explicit,
    }]);
    let children = stores.freeze_node_list(&[
        tex_state::node::Node::Whatsit(tex_state::node::Whatsit::Special {
            class: "dvi".to_owned(),
            payload: b"before".to_vec(),
        }),
        tex_state::node::Node::Disc {
            kind: tex_state::node::DiscKind::Discretionary,
            pre: disc_pre,
            post: disc_post,
            replace: disc_replace,
        },
        tex_state::node::Node::Mark {
            class: 7,
            tokens: mark_tokens,
        },
        tex_state::node::Node::Ins {
            class: 3,
            size: Scaled::from_raw(0),
            split_top_skip: stores.glue_param(GlueParam::SPLIT_TOP_SKIP),
            split_max_depth: Scaled::MAX_DIMEN,
            floating_penalty: 0,
            content: insert_content,
        },
        tex_state::node::Node::Adjust(adjust_content),
        tex_state::node::Node::Whatsit(tex_state::node::Whatsit::Special {
            class: "dvi".to_owned(),
            payload: b"after".to_vec(),
        }),
    ]);
    let root = tex_state::node::Node::HList(tex_state::node::BoxNode::new(
        tex_state::node::BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: tex_state::scaled::GlueSetRatio::ZERO,
            glue_sign: tex_state::node::Sign::Normal,
            glue_order: tex_state::glue::Order::Normal,
            children,
        },
    ));
    let root_list = stores.freeze_node_list(&[root]);
    stores.set_box_reg(0, root_list);
    let mut input = InputStack::new(MemoryInput::new("\\shipout\\box0\\end"));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("seeded shipout succeeds");

    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = PageArtifact::from_bytes(&bytes).expect("artifact parses");
    assert_eq!(artifact.to_bytes().expect("artifact serializes"), bytes);
    assert!(matches!(
        artifact.effects.as_slice(),
        [
            PageEffect::Special { payload: before, .. },
            PageEffect::Special { payload: after, .. },
        ] if before == b"before" && after == b"after"
    ));
    let PageNode::HList(box_node) = &artifact.root else {
        panic!("shipout root should lower to hlist");
    };
    assert!(matches!(
        box_node.children.as_slice(),
        [
            PageNode::WhatsitAnchor { effect_index: 0 },
            PageNode::Disc {
                kind: PageDiscKind::Discretionary,
                pre,
                post,
                replace,
            },
            PageNode::Mark { class: 7, tokens },
            PageNode::Insert { class: 3, content },
            PageNode::Adjust(adjust),
            PageNode::WhatsitAnchor { effect_index: 1 },
        ] if matches!(pre.as_slice(), [PageNode::Kern { .. }])
            && matches!(post.as_slice(), [PageNode::Penalty(22)])
            && matches!(replace.as_slice(), [PageNode::Rule { .. }])
            && matches!(
                tokens.as_slice(),
                [
                    PageToken::Char { ch, .. },
                    PageToken::ControlSequence(name),
                    PageToken::Param(2),
                ] if *ch == 'm' as u32 && name == "markcs"
            )
            && matches!(content.as_slice(), [PageNode::Penalty(55)])
            && matches!(adjust.as_slice(), [PageNode::Kern { .. }])
    ));

    let dvi = write_dvi(std::slice::from_ref(&artifact)).expect("DVI writes");
    assert_eq!(
        dvi_special_payloads(&dvi),
        vec![b"before".to_vec(), b"after".to_vec()]
    );
}

#[allow(clippy::disallowed_methods)] // host-side committed fixture source read.
fn read_io_source(stem: &str) -> String {
    let path = corpus_root()
        .join("tex_exec_io")
        .join(format!("{stem}.tex"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn format_special_payloads(payloads: &[Vec<u8>]) -> String {
    let mut output = String::new();
    for payload in payloads {
        output.push_str(&String::from_utf8_lossy(payload));
        output.push('\n');
    }
    output
}

#[test]
fn deferred_write_does_not_absorb_following_par() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def\\x{x\\write10{\\the\\spacefactor}\\par}\\x",
    ));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("write followed by par executes");

    assert_eq!(executor.nest().current_mode(), crate::Mode::Vertical);
}

fn shipout_artifact_bytes(source: &str) -> Vec<u8> {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(source));
    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shipout succeeds");
    assert_eq!(stats.shipped_artifacts.len(), 1);
    stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
        .expect("read artifact")
        .expect("artifact stored")
}

#[test]
fn shipout_nested_in_box_scan_is_reported_to_driver() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox{\\shipout\\hbox{A}}\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("nested shipout succeeds");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(stats.dvi_pages.len(), 1);
    assert_eq!(stores.world().artifact_commits(), stats.shipped_artifacts);
}

fn format_output_presence(stores: &Universe, paths: &[&str]) -> String {
    let mut output = String::new();
    for path in paths {
        let state = stores
            .world()
            .memory_output(path)
            .map_or("absent".to_owned(), |bytes| {
                format!("present:{} bytes", bytes.len())
            });
        output.push_str(path);
        output.push_str(": ");
        output.push_str(&state);
        output.push('\n');
    }
    output
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

fn dvi_special_payloads(dvi: &[u8]) -> Vec<Vec<u8>> {
    const XXX1: u8 = 239;
    const XXX4: u8 = 242;

    let mut payloads = Vec::new();
    let mut index = 0usize;
    while index < dvi.len() {
        match dvi[index] {
            XXX1 if index + 2 <= dvi.len() => {
                let len = dvi[index + 1] as usize;
                let start = index + 2;
                let end = start + len;
                if end <= dvi.len() {
                    payloads.push(dvi[start..end].to_vec());
                    index = end;
                    continue;
                }
                break;
            }
            XXX4 if index + 5 <= dvi.len() => {
                let Ok(len) = usize::try_from(i32::from_be_bytes([
                    dvi[index + 1],
                    dvi[index + 2],
                    dvi[index + 3],
                    dvi[index + 4],
                ])) else {
                    break;
                };
                let start = index + 5;
                let end = start + len;
                if end <= dvi.len() {
                    payloads.push(dvi[start..end].to_vec());
                    index = end;
                    continue;
                }
                break;
            }
            _ => index += 1,
        }
    }
    payloads
}
