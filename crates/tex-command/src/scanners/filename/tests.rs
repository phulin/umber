use super::*;

use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandError, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandRuntime,
    CommandState, RegisteredSourceKind, SourceRegistration,
};

fn traced(token: Token) -> TracedTokenWord {
    TracedTokenWord::pack(token, OriginId::UNKNOWN)
}

fn text_tokens(text: &str) -> Vec<Token> {
    text.chars()
        .map(|ch| Token::Char {
            ch,
            cat: match ch {
                '{' => Catcode::BeginGroup,
                '}' => Catcode::EndGroup,
                ' ' => Catcode::Space,
                'a'..='z' | 'A'..='Z' => Catcode::Letter,
                _ => Catcode::Other,
            },
        })
        .collect()
}

fn push(command: &mut CommandState, tokens: impl IntoIterator<Item = Token>) {
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(
            tokens.into_iter().map(traced).collect::<Vec<_>>(),
        )),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
}

fn processor<'a>(
    command: &'a mut CommandState,
    runtime: &'a mut CommandRuntime,
    universe: &'a mut Universe,
    capabilities: &'a mut CommandHostCapabilities,
) -> CommandProcessor<'a> {
    CommandProcessor::new(
        command,
        runtime,
        universe.command_context(),
        CommandHostContext::new(capabilities),
    )
}

fn scan_text(text: &str) -> ScannedFileName {
    let mut command = CommandState::default();
    push(&mut command, text_tokens(text));
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_file_name()
        .expect("filename scans")
}

#[test]
fn filename_components_split_area_name_and_extension_canonically() {
    for (input, area, name, extension, packed, termination) in [
        (
            "paper ",
            "",
            "paper",
            "",
            "paper",
            FileNameTermination::Space,
        ),
        (
            "area/final.ext ",
            "area/",
            "final",
            ".ext",
            "area/final.ext",
            FileNameTermination::Space,
        ),
        (
            "area.with.dot/final.part.ext ",
            "area.with.dot/",
            "final",
            ".part.ext",
            "area.with.dot/final.part.ext",
            FileNameTermination::Space,
        ),
        (
            "a:b.c.d ",
            "a:",
            "b",
            ".c.d",
            "a:b.c.d",
            FileNameTermination::Space,
        ),
        (
            "a>b.c.d ",
            "",
            "a>b",
            ".c.d",
            "a>b.c.d",
            FileNameTermination::Space,
        ),
        (
            "b.c.d ",
            "",
            "b",
            ".c.d",
            "b.c.d",
            FileNameTermination::Space,
        ),
        (
            "before.dot/after.ext.more ",
            "before.dot/",
            "after",
            ".ext.more",
            "before.dot/after.ext.more",
            FileNameTermination::Space,
        ),
        (
            "{area/final.name.ext}",
            "area/",
            "final",
            ".name.ext",
            "area/final.name.ext",
            FileNameTermination::Group,
        ),
        (
            "\"area with spaces/final.ext\" ",
            "area with spaces/",
            "final",
            ".ext",
            "area with spaces/final.ext",
            FileNameTermination::Space,
        ),
    ] {
        let scanned = scan_text(input);
        assert_eq!(scanned.components.area, area, "area for {input:?}");
        assert_eq!(scanned.components.name, name, "name for {input:?}");
        assert_eq!(
            scanned.components.extension, extension,
            "extension for {input:?}"
        );
        assert_eq!(scanned.packed(), packed);
        assert_eq!(scanned.termination, termination);
    }

    let scanned = scan_text("area..with...dots/final..part...ext ");
    assert_eq!(scanned.components.area, "area..with...dots/");
    assert_eq!(scanned.components.name, "final");
    assert_eq!(scanned.components.extension, "..part...ext");

    let scanned = scan_text("first.name.ext/second.part.ext ");
    assert_eq!(scanned.components.area, "first.name.ext/");
    assert_eq!(scanned.components.name, "second");
    assert_eq!(scanned.components.extension, ".part.ext");

    let mut defaulted = scan_text("area/paper ");
    defaulted.components.apply_default_extension(".tex");
    assert_eq!(defaulted.packed(), "area/paper.tex");
    defaulted.components.apply_default_extension(".dvi");
    assert_eq!(defaulted.packed(), "area/paper.tex");
}

#[test]
fn filename_component_terminators_and_string_overflow_follow_tex82() {
    let long_name = "a".repeat(8_192);
    let scanned = scan_text(&format!("{long_name} "));
    assert_eq!(scanned.packed(), long_name);
    assert_eq!(scanned.termination, FileNameTermination::Space);

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let terminator = universe.intern("stop").symbol();
    universe.set_meaning(terminator, Meaning::Relax);
    let mut tokens = text_tokens("plain");
    tokens.push(Token::Cs(terminator));
    tokens.extend(text_tokens("tail"));
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let (scanned, replayed) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let scanned = processor.scan_file_name().expect("unbraced filename scans");
        let replayed = processor
            .get_x_token()
            .expect("terminator replay succeeds")
            .expect("terminator is present");
        (scanned, replayed)
    };
    assert_eq!(scanned.packed(), "plain");
    assert_eq!(scanned.termination, FileNameTermination::NonCharacter);
    assert_eq!(replayed.meaning(), Meaning::Relax);

    let scanned = scan_text("terminal");
    assert_eq!(scanned.termination, FileNameTermination::EndOfInput);

    let mut command = CommandState::default();
    push(
        &mut command,
        text_tokens(&format!("{} ", "a".repeat(FILE_NAME_POOL_CAPACITY + 1))),
    );
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let error = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_file_name()
        .expect_err("filename string-pool capacity is bounded");
    assert_eq!(
        error,
        CommandError::Fatal(crate::FatalError::overflow(
            "pool size",
            FILE_NAME_POOL_CAPACITY as i32,
        ))
    );
    assert!(!command.name_in_progress());
}

#[test]
fn filename_scan_expands_characters_and_backs_up_first_noncharacter() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let macro_name = universe.intern("stem").symbol();
    let terminator = universe.intern("stop").symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&text_tokens("paper"));
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    universe.set_meaning(terminator, Meaning::Relax);
    let mut tokens = vec![
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(macro_name),
    ];
    tokens.extend(text_tokens(".tex"));
    tokens.push(Token::Cs(terminator));
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();

    let (scanned, next) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let scanned = processor.scan_file_name().expect("expanded filename scans");
        let next = processor
            .get_x_token()
            .expect("terminator replay succeeds")
            .expect("terminator is present");
        (scanned, next)
    };
    assert_eq!(scanned.packed(), "paper.tex");
    assert_eq!(scanned.termination, FileNameTermination::NonCharacter);
    assert_eq!(next.meaning(), Meaning::Relax);
}

#[test]
fn filename_scan_recursion_guard_and_retry_modes_follow_tex82() {
    let mut command = CommandState::default();
    push(&mut command, text_tokens("chapter.tex "));
    let snapshot = command.snapshot();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();

    let error = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .open_registered_input()
        .expect_err("unavailable input requests host retry");
    assert_eq!(
        error,
        CommandError::MissingInput {
            name: "chapter.tex".to_owned(),
            original_name: "chapter.tex".to_owned(),
        }
    );

    command
        .rollback(snapshot)
        .expect("failed attempt rolls back");
    capabilities.register_input(
        "chapter.tex",
        SourceRegistration::new(
            RegisteredSourceKind::Generated,
            std::sync::Arc::<[u8]>::from(b"x".as_slice()),
        ),
    );
    let opened = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .open_registered_input()
        .expect("registered retry opens");
    assert_eq!(opened.file_name.packed(), "chapter.tex");

    let input = universe.intern("input").symbol();
    universe.set_meaning(
        input,
        Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::Input),
    );
    let mut command = CommandState::default();
    let mut nested = vec![Token::Cs(input), Token::Cs(input)];
    nested.extend(text_tokens("inc "));
    push(&mut command, nested);
    universe.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
    let error = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect_err("empty outer filename requests interactive recovery")
    };
    assert_eq!(
        error,
        CommandError::MissingInput {
            name: ".tex".to_owned(),
            original_name: String::new(),
        }
    );
    assert!(!command.name_in_progress());
    {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let sentinel = processor
            .get_x_token()
            .expect("sentinel delivery succeeds")
            .expect("sentinel remains above the restored input");
        assert_eq!(sentinel.meaning(), Meaning::Relax);
        let restored_input = processor
            .get_token()
            .expect("restored input delivery succeeds")
            .expect("restored input follows the sentinel");
        assert_eq!(
            restored_input.meaning(),
            Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::Input)
        );
        for expected in ['i', 'n', 'c'] {
            assert_eq!(
                processor
                    .get_token()
                    .expect("filename character delivery succeeds")
                    .expect("filename character remains")
                    .meaning(),
                Meaning::CharToken {
                    ch: expected,
                    cat: Catcode::Letter,
                }
            );
        }
    }

    let snapshot = command.snapshot();
    command.begin_file_name().expect("guard begins");
    command.rollback(snapshot).expect("guard state rolls back");
    assert!(!command.name_in_progress());

    let mut command = CommandState::default();
    push(&mut command, text_tokens("missing "));
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    capabilities.mark_input_unavailable("missing.tex");
    capabilities.mark_input_unavailable("TeXinputs:missing.tex");
    let error = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .open_registered_input()
        .expect_err("noninteractive missing input is fatal");
    assert_eq!(
        error,
        CommandError::Fatal(crate::FatalError::emergency_stop(
            "job aborted, file error in nonstop mode",
        ))
    );
}
