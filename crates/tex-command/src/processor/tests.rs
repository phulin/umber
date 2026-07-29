//! `CommandProcessor::publish_context` -- see the doc comment on that method
//! and `crate::context`'s module documentation for why every construction
//! site calls it once, right before its borrow ends, rather than the type
//! doing so on `Drop`.

use tex_state::Universe;
use tex_state::error_context::ContextEntry;

use crate::{
    CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandRuntime, CommandState,
    RegisteredSourceKind, SourceRegistration,
};

fn source_state(text: &[u8]) -> CommandState {
    let mut state = CommandState::default();
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            text.to_vec(),
        ))
        .expect("valid source");
    state
        .open_registered_source(source)
        .expect("registered source opens");
    state.load_next_source_line(13).expect("first line");
    state
}

fn advance_characters(state: &mut CommandState, count: usize) {
    for _ in 0..count {
        state.next_source_character().expect("character available");
    }
}

fn published_level_text(universe: &Universe) -> (String, String) {
    let entries = universe.world().error_context().entries();
    let [ContextEntry::Level(level)] = entries else {
        panic!("expected exactly one source level, got {entries:?}");
    };
    (level.before.clone(), level.after.clone())
}

#[test]
fn publish_context_reflects_input_position_with_no_error_raised() {
    let mut command = source_state(b"abcdef");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();

    advance_characters(&mut command, 3);
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        // No error is raised in this episode at all: `publish_context` is
        // exactly the call every `tex-exec` construction site now makes
        // unconditionally, whether or not the episode itself errored.
        processor.publish_context();
    }

    assert_eq!(
        published_level_text(&universe),
        ("abc".to_owned(), "def".to_owned())
    );
}

/// The scenario `umber2-alfh.8` names as worse than printing no context at
/// all: a narrow, non-erroring episode (an accent base, a math field, an
/// `\output` open) runs between one delivered command's error and the next
/// print that reads `World`'s published context, and moves the input stack
/// further while doing so. Without this episode also publishing, that next
/// print would render the *first* episode's now-stale position instead of
/// where input actually stands.
#[test]
fn publish_context_overwrites_a_stale_context_left_by_an_earlier_episode() {
    let mut command = source_state(b"abcdef");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();

    // First episode: an ordinary tex.web `print_err`/`error` call, which
    // self-publishes at position 1 ("a|bcdef") the same way every scanner
    // error in this crate does.
    advance_characters(&mut command, 1);
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        processor.print_err("first episode error").error();
    }
    assert_eq!(
        published_level_text(&universe),
        ("a".to_owned(), "bcdef".to_owned())
    );

    // Second episode: scans further to position 3 ("abc|def") but never
    // itself raises an error, exactly the shape of `tex-exec`'s narrow
    // nested episodes. It publishes once, unconditionally, right before its
    // borrow ends.
    advance_characters(&mut command, 2);
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        processor.publish_context();
    }

    // Had the second episode not republished, `World` would still report the
    // first episode's stale "a|bcdef" position here -- exactly what a
    // `tex-exec` execution-time error printed straight through `World`,
    // after this second episode's processor has already gone out of scope,
    // would read back.
    assert_eq!(
        published_level_text(&universe),
        ("abc".to_owned(), "def".to_owned())
    );
}
