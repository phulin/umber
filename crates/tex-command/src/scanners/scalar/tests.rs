use tex_state::Universe;
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandRuntime, CommandState, InputTransition, ObservedToken,
};

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn push(command: &mut CommandState, tokens: Vec<Token>) {
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(
            tokens
                .into_iter()
                .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN))
                .collect::<Vec<_>>(),
        )),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
}

fn char_token(ch: char) -> Token {
    Token::Char {
        ch,
        cat: if ch.is_ascii_alphabetic() {
            Catcode::Letter
        } else {
            Catcode::Other
        },
    }
}

#[test]
fn scalar_forms_recovery_and_snapshot_use_only_command_input() {
    let mut command = CommandState::default();
    let snapshot = command.snapshot();
    push(
        &mut command,
        " --12 3.5pt plus 2pt minus 1pt"
            .chars()
            .map(char_token)
            .collect(),
    );
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );

        assert_eq!(processor.scan_integer().expect("integer scans").value, 12);
        let glue = processor.scan_glue(false).expect("glue scans");
        assert_eq!(glue.value.width.raw(), 3 * 65_536 + 32_768);
        assert_eq!(glue.value.stretch.raw(), 2 * 65_536);
        assert_eq!(glue.value.shrink.raw(), 65_536);
        assert_eq!(
            processor
                .scan_integer()
                .expect("EOF recovery scans")
                .recovery,
            ScalarRecovery::InsertedZero
        );
    }
    command.rollback(snapshot).expect("snapshot rolls back");
    assert!(command.publish_summary().is_ok());
}

#[test]
fn integer_radix_prefixes_deliver_digits_before_scanner_completion() {
    let mut command = CommandState::default();
    push(
        &mut command,
        "\"2A '17 42 ".chars().map(char_token).collect(),
    );
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(processor.scan_integer().expect("hex scans").value, 42);
    assert_eq!(processor.scan_integer().expect("octal scans").value, 15);
    assert_eq!(processor.scan_integer().expect("decimal scans").value, 42);
}

#[test]
fn integer_scanner_accepts_chardef_values() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let active = universe.intern("active").symbol();
    universe.set_meaning(active, Meaning::CharGiven('\r'));
    push(&mut command, vec![Token::Cs(active)]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);

        assert_eq!(processor.scan_integer().expect("chardef scans").value, 13);
    }

    assert_eq!(scanner_kinds(&recorder), vec!["internal", "integer"]);
}

#[test]
fn integer_scanner_accepts_mathchardef_values() {
    // TeX82 §413 groups `char_given` and `math_given` under one
    // `scanned_result(cur_chr)(int_val)` case. plain.tex's
    // `\mathchardef\@M=10000` must scan as the integer 10000 wherever an
    // internal integer is accepted, e.g. `\penalty-\@M` inside `\break`.
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let at_m = universe.intern("@M").symbol();
    universe.set_meaning(at_m, Meaning::MathCharGiven(10_000));
    push(&mut command, vec![Token::Cs(at_m)]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);

        assert_eq!(
            processor.scan_integer().expect("mathchardef scans").value,
            10_000
        );
    }

    assert_eq!(scanner_kinds(&recorder), vec!["internal", "integer"]);
}

#[test]
fn internal_scanner_commits_the_requested_level_not_the_quantitys_own() {
    // TeX82 §413 runs §429's `while cur_val_level>level` cascade before its
    // single exit, so the level it commits is the one the caller asked for.
    // plain.tex's `\def\rm{\fam\z@\tenrm}` asks at `int_val` for the dimension
    // register `\z@`: §429 lowers `dimen_val` to `int_val` keeping the scaled
    // representation, and TeX commits an integer. Committing the register's
    // own `dimen_val` instead reports a scaled dimension where TeX reports an
    // integer (umber2-johp.163).
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let zero = universe.intern("z@").symbol();
    universe.set_meaning(zero, Meaning::DimenRegister(0));
    universe.set_dimen(0, tex_state::scaled::Scaled::from_raw(3 * Scaled::UNITY));
    push(&mut command, vec![Token::Cs(zero)]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);

        assert_eq!(
            processor
                .scan_integer()
                .expect("a dimension register scans as an integer")
                .value,
            3 * Scaled::UNITY
        );
    }

    assert_eq!(scanner_kinds(&recorder), vec!["internal", "integer"]);
    assert_eq!(
        scanner_values(&recorder),
        vec![
            (3 * Scaled::UNITY).to_string(),
            (3 * Scaled::UNITY).to_string(),
        ]
    );
}

/// The committed scanner results a replay observed, in order.
///
/// TeX82 §413's `scan_something_internal` commits its own result before the
/// §440 `scan_int` (or §448 `scan_dimen`, §461 `scan_glue`) result that
/// consumes it, so a `\chardef`/`\mathchardef` constant must produce both.
fn scanner_kinds(recorder: &Recorder) -> Vec<&'static str> {
    recorder
        .0
        .iter()
        .filter_map(|record| match record {
            CommandObservation::Scanner(scanner) => Some(scanner.kind),
            _ => None,
        })
        .collect()
}

/// The rendered payloads of those same scanner results, in order.
///
/// The rendering carries the committed level -- a bare integer for `int_val`,
/// a `scaled:` prefix for `dimen_val` -- so it is what distinguishes §429's
/// cascade having run from its not having run.
fn scanner_values(recorder: &Recorder) -> Vec<String> {
    recorder
        .0
        .iter()
        .filter_map(|record| match record {
            CommandObservation::Scanner(scanner) => Some(scanner.value.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn character_code_scanning_accepts_an_active_character_before_optional_equals() {
    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
            char_token('`'),
            Token::Char {
                ch: '~',
                cat: Catcode::Active,
            },
            char_token('='),
            char_token('1'),
            char_token('3'),
        ],
    );
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);

        assert_eq!(
            processor
                .scan_integer()
                .expect("active character code")
                .value,
            126
        );
        assert!(
            processor
                .scan_optional_equals()
                .expect("equals scans")
                .value
        );
        assert_eq!(processor.scan_integer().expect("value scans").value, 13);
    }

    let raw_equals = recorder
        .0
        .iter()
        .position(|record| {
            matches!(record, CommandObservation::Command(command)
            if command.boundary == crate::CommandDeliveryBoundary::Raw
                && matches!(command.spelling, ObservedToken::Character { character: '=', .. }))
        })
        .expect("optional-space probe delivers equals raw");
    let expanded_equals = recorder
        .0
        .iter()
        .position(|record| {
            matches!(record, CommandObservation::Command(command)
            if command.boundary == crate::CommandDeliveryBoundary::Expanded
                && matches!(command.spelling, ObservedToken::Character { character: '=', .. }))
        })
        .expect("optional-space probe expands equals");
    let backup = recorder
        .0
        .iter()
        .position(|record| {
            matches!(record, CommandObservation::Input(record)
            if record.transition == InputTransition::Backup)
        })
        .expect("non-space optional-space probe backs equals up");
    assert!(raw_equals < expanded_equals && expanded_equals < backup);
}

#[test]
fn integer_internal_register_scans_its_index_through_command_input() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let count = universe.intern("count").symbol();
    universe.set_meaning(
        count,
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
    );
    universe.set_count(0, 42);
    push(&mut command, vec![Token::Cs(count), char_token('0')]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(
        processor
            .scan_integer()
            .expect("internal count scans")
            .value,
        42
    );
}

#[test]
fn glue_scan_accepts_internal_box_height_without_backing_up_the_primitive() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let height = universe.intern("ht").symbol();
    universe.set_meaning(
        height,
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Ht),
    );
    push(&mut command, vec![Token::Cs(height), char_token('0')]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    assert_eq!(
        processor
            .scan_glue(false)
            .expect("internal box dimension scans as glue")
            .value
            .width
            .raw(),
        0
    );
    assert!(
        !recorder.0.iter().any(|observation| {
            matches!(
                observation,
                CommandObservation::Input(record) if record.transition == InputTransition::Backup
            )
        }),
        "TeX82 consumes the internal dimension directly instead of backing up \\ht"
    );
}

#[test]
fn internal_dimension_register_scans_and_bounds_its_index_through_command_input() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let dimen = universe.intern("dimen").symbol();
    universe.set_meaning(
        dimen,
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Dimen),
    );
    universe.set_dimen(20, tex_state::scaled::Scaled::from_raw(42));
    universe.set_dimen(0, tex_state::scaled::Scaled::from_raw(7));
    let mut capabilities = CommandHostCapabilities::default();

    push(
        &mut command,
        vec![Token::Cs(dimen), char_token('2'), char_token('0')],
    );
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        assert_eq!(
            processor
                .scan_dimension()
                .expect("internal dimension scans")
                .value
                .raw(),
            42
        );
    }

    push(
        &mut command,
        vec![
            Token::Cs(dimen),
            char_token('2'),
            char_token('5'),
            char_token('6'),
        ],
    );
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );
    assert_eq!(
        processor
            .scan_dimension()
            .expect("out-of-range dimension register recovers")
            .value
            .raw(),
        7
    );
}

#[test]
fn a_whole_internal_dimension_operand_is_never_backed_up_and_redelivered() {
    // tex.web §448: `<Get the next non-blank non-sign token>` leaves the
    // token in hand, and only the branch for a command code outside
    // §208/§209's `min_internal..=max_internal` runs `back_input`. A
    // `\dimendef` name (`\maxdimen`) is `assign_dimen`, so `\splitmaxdepth=
    // \maxdimen` delivers it exactly once, with no backup level and no
    // recovery record, before `scan_something_internal` publishes its value
    // (`umber2-johp.135`).
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let maxdimen = universe.intern("maxdimen").symbol();
    universe.set_meaning(maxdimen, Meaning::DimenRegister(0));
    universe.set_dimen(0, tex_state::scaled::Scaled::from_raw(1_073_741_823));
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();

    push(&mut command, vec![Token::Cs(maxdimen)]);
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);

        assert_eq!(
            processor
                .scan_dimension()
                .expect("internal dimension scans")
                .value
                .raw(),
            1_073_741_823
        );
    }

    assert!(
        !recorder.0.iter().any(|record| matches!(
            record,
            CommandObservation::Input(record) if record.transition == InputTransition::Backup
        )),
        "an internal dimension operand must not install a backup level: {:?}",
        recorder.0
    );
    assert!(
        !recorder
            .0
            .iter()
            .any(|record| matches!(record, CommandObservation::Recovery(_))),
        "an internal dimension operand must not record a recovery: {:?}",
        recorder.0
    );
    assert_eq!(
        recorder
            .0
            .iter()
            .filter(
                |record| matches!(record, CommandObservation::Command(command)
                if command.boundary == crate::CommandDeliveryBoundary::Raw)
            )
            .count(),
        1,
        "the operand is delivered exactly once: {:?}",
        recorder.0
    );
    assert_eq!(scanner_kinds(&recorder), vec!["internal", "dimension"]);
}

#[test]
fn dimension_scanner_accepts_an_internal_dimension_as_its_unit() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let unit = universe.intern("unit").symbol();
    universe.set_meaning(unit, Meaning::DimenRegister(0));
    universe.set_dimen(0, tex_state::scaled::Scaled::from_raw(Scaled::UNITY));
    let mut capabilities = CommandHostCapabilities::default();

    push(
        &mut command,
        vec![
            char_token('8'),
            char_token('.'),
            char_token('5'),
            Token::Cs(unit),
        ],
    );
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(
        processor
            .scan_dimension()
            .expect("internal unit scans")
            .value
            .raw(),
        8 * Scaled::UNITY + Scaled::UNITY / 2
    );
}

#[test]
fn an_internal_dimension_unit_leaves_the_following_space_in_the_input() {
    // tex.web §455's internal-dimension exit sets `v` and jumps straight to
    // `found:`, whose `goto attach_sign` bypasses both §455's own
    // `<Scan an optional space>` (which only the `em`/`ex` path runs) and
    // §448's trailing one. So `3\unit x` consumes no space at all.
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let unit = universe.intern("unit").symbol();
    universe.set_meaning(unit, Meaning::DimenRegister(0));
    universe.set_dimen(0, tex_state::scaled::Scaled::from_raw(Scaled::UNITY));
    let mut capabilities = CommandHostCapabilities::default();

    push(
        &mut command,
        vec![
            char_token('3'),
            Token::Cs(unit),
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            char_token('x'),
        ],
    );
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(
        processor
            .scan_dimension()
            .expect("internal unit scans")
            .value
            .raw(),
        3 * Scaled::UNITY
    );
    assert!(matches!(
        processor
            .get_x_token()
            .expect("the following token delivers")
            .expect("the following token exists")
            .meaning(),
        Meaning::CharToken {
            ch: ' ',
            cat: Catcode::Space
        }
    ));
    assert!(matches!(
        processor
            .get_x_token()
            .expect("the next token delivers")
            .expect("the next token exists")
            .meaning(),
        Meaning::CharToken { ch: 'x', .. }
    ));
}

#[test]
fn an_em_unit_consumes_exactly_one_following_space() {
    // §455's `em`/`ex` path is the only one that runs
    // `<Scan an optional space>` before `found:`, and `found:`'s
    // `goto attach_sign` still skips §448's trailing one. So `3em  x`
    // consumes exactly one of its two spaces.
    use tex_state::font::NULL_FONT;

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    universe
        .set_font_dimen(NULL_FONT, 6, Scaled::from_raw(10 * Scaled::UNITY))
        .expect("nullfont has a quad parameter");
    let space = Token::Char {
        ch: ' ',
        cat: Catcode::Space,
    };
    push(
        &mut command,
        vec![
            char_token('3'),
            char_token('e'),
            char_token('m'),
            space,
            space,
            char_token('x'),
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(
        processor
            .scan_dimension()
            .expect("em dimension scans")
            .value
            .raw(),
        30 * Scaled::UNITY
    );
    assert!(matches!(
        processor
            .get_x_token()
            .expect("the second space delivers")
            .expect("the second space exists")
            .meaning(),
        Meaning::CharToken {
            ch: ' ',
            cat: Catcode::Space
        }
    ));
    assert!(matches!(
        processor
            .get_x_token()
            .expect("the next token delivers")
            .expect("the next token exists")
            .meaning(),
        Meaning::CharToken { ch: 'x', .. }
    ));
}

#[test]
fn dimension_scanner_recognizes_current_font_em_and_ex_units() {
    use tex_state::font::NULL_FONT;
    use tex_state::scaled::Scaled;

    let mut command = CommandState::default();
    push(&mut command, "1em 1ex 42".chars().map(char_token).collect());
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    universe
        .set_font_dimen(NULL_FONT, 6, Scaled::from_raw(10 * Scaled::UNITY))
        .expect("nullfont has a quad parameter");
    universe
        .set_font_dimen(NULL_FONT, 5, Scaled::from_raw(4 * Scaled::UNITY))
        .expect("nullfont has an x-height parameter");
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    assert_eq!(
        processor
            .scan_dimension()
            .expect("em dimension scans")
            .value
            .raw(),
        10 * Scaled::UNITY
    );
    assert_eq!(
        processor
            .scan_dimension()
            .expect("ex dimension scans")
            .value
            .raw(),
        4 * Scaled::UNITY
    );
    assert_eq!(
        processor
            .scan_integer()
            .expect("post-unit token remains available")
            .value,
        42
    );
    let scanned_dimensions = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Scanner(record) if record.kind == "dimension" => {
                Some(record.value.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        scanned_dimensions,
        vec![
            (10 * Scaled::UNITY).to_string(),
            (4 * Scaled::UNITY).to_string(),
        ]
    );
}

#[test]
fn internal_values_and_failed_keywords_replay_canonically() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    universe.set_count(17, 41);
    let keyword = "pto".chars().map(char_token).collect::<Vec<_>>();
    push(&mut command, keyword);
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        assert!(!processor.scan_keyword("plus").expect("keyword scans").value);
        assert!(matches!(
            processor
                .get_x_token()
                .expect("replayed token delivers")
                .expect("replayed token exists")
                .meaning(),
            Meaning::CharToken { ch: 'p', .. }
        ));
        let _ = processor
            .get_x_token()
            .expect("second replay token delivers");
        let _ = processor
            .get_x_token()
            .expect("third replay token delivers");
    }
    push(
        &mut command,
        vec![Token::Cs(universe.intern("count17").symbol())],
    );
    let symbol = universe.intern("count17").symbol();
    universe.set_meaning(symbol, Meaning::CountRegister(17));
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );
    assert_eq!(
        processor
            .scan_internal_value()
            .expect("internal scan succeeds")
            .expect("count register is internal")
            .value,
        InternalValue::Integer(41)
    );
}

#[test]
fn fractional_in_unit_retires_the_final_probe_backup_before_n() {
    let mut command = CommandState::default();
    push(&mut command, "1.25in ".chars().map(char_token).collect());
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    assert_eq!(
        processor
            .scan_dimension()
            .expect("dimension scans")
            .value
            .raw(),
        5_920_358
    );

    let n = recorder
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Command(record)
                    if matches!(record.spelling, ObservedToken::Character { character: 'n', .. })
                        && record.boundary == crate::CommandDeliveryBoundary::Raw
            )
        })
        .expect("in suffix n is delivered raw");
    assert!(matches!(
        recorder.0.get(n - 1),
        Some(CommandObservation::Input(record)) if record.transition == InputTransition::Retire
    ));
}

#[test]
fn leading_decimal_dimension_replays_the_point_before_scanning_its_fraction() {
    let mut command = CommandState::default();
    push(&mut command, ".75in 42".chars().map(char_token).collect());
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(
        processor
            .scan_dimension()
            .expect("leading-decimal dimension scans")
            .value
            .raw(),
        3_552_215
    );
    assert_eq!(
        processor
            .scan_integer()
            .expect("following token remains available")
            .value,
        42
    );
}

#[test]
fn leading_decimal_point_never_reaches_the_integer_scanner() {
    // TeX82 §448's non-internal branch reads `cur_tok` after `back_input`
    // without fetching it again:
    //
    //     back_input;
    //     if cur_tok<>point_token then scan_int
    //     else begin radix:=10; cur_val:=0; end;
    //
    // So a leading decimal point never enters `scan_int` at all, and §452's
    // `get_token` -- not `get_x_token` -- is the one delivery that re-scans
    // it. Routing the point through `scan_int` instead produced an expanded
    // redelivery, a §444 `vacuous` scan with §446's second `back_error`
    // backup, and an integer scanner result TeX never computes.
    let mut command = CommandState::default();
    push(&mut command, ".5cm".chars().map(char_token).collect());
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);
        assert_eq!(
            processor
                .scan_dimension()
                .expect("leading-decimal dimension scans")
                .value
                .raw(),
            932_339
        );
    }

    // The exact §448/§452 prefix: one raw/expanded pair from `<Get the next
    // non-blank non-sign token>`, `back_input`, then §452's single raw
    // `get_token` and the backup it exhausts. Everything after this belongs
    // to the fraction digits and §453's unit keywords.
    let point_delivery = |observation: &CommandObservation, boundary| {
        matches!(
            observation,
            CommandObservation::Command(record)
                if matches!(
                    record.spelling,
                    ObservedToken::Character { character: '.', .. }
                ) && record.boundary == boundary
        )
    };
    assert!(point_delivery(
        &recorder.0[0],
        crate::CommandDeliveryBoundary::Raw
    ));
    assert!(point_delivery(
        &recorder.0[1],
        crate::CommandDeliveryBoundary::Expanded
    ));
    assert!(matches!(
        &recorder.0[2],
        CommandObservation::Input(record)
            if record.transition == InputTransition::Backup
    ));
    assert!(matches!(&recorder.0[3], CommandObservation::Recovery(_)));
    assert!(point_delivery(
        &recorder.0[4],
        crate::CommandDeliveryBoundary::Raw
    ));
    assert!(matches!(
        &recorder.0[5],
        CommandObservation::Input(record)
            if record.transition == InputTransition::Retire
    ));
    // `scan_int` is never called, so it commits no result at all.
    assert!(!recorder.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Scanner(record) if record.kind == "integer"
    )));
}

/// Builds a processor over one pushed token list, for the level-coercion
/// tests below. Each of them scans exactly one scalar from state that the
/// caller has already installed.
fn scan_with<T>(
    universe: &mut Universe,
    tokens: Vec<Token>,
    scan: impl FnOnce(&mut CommandProcessor<'_>) -> T,
) -> T {
    let mut command = CommandState::default();
    push(&mut command, tokens);
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );
    scan(&mut processor)
}

fn glue(width: i32, stretch: i32, shrink: i32) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(width),
        stretch: Scaled::from_raw(stretch),
        shrink: Scaled::from_raw(shrink),
        ..GlueSpec::ZERO
    }
}

#[test]
fn integer_scanner_coerces_internal_glue_and_mu_glue_to_their_width() {
    // TeX82 §429: `scan_int` fetches at `int_val`, so §413's coercion loop
    // lowers `glue_val` to its width and then reinterprets that width's
    // scaled representation as an integer. `\count0=\skip3` and
    // `\ifnum\parskip>0` both depend on this; treating the glue as "not a
    // number" silently reads zero instead.
    let mut universe = Universe::new();
    let skip = universe.intern("skip").symbol();
    universe.set_meaning(
        skip,
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Skip),
    );
    let spec = universe.intern_glue(glue(7 * Scaled::UNITY, Scaled::UNITY, 0));
    universe.set_skip(3, spec);
    let muskip = universe.intern("muskip").symbol();
    universe.set_meaning(muskip, Meaning::MuskipRegister(1));
    let mu = universe.intern_glue(glue(5 * Scaled::UNITY, 0, 0));
    universe.set_muskip(1, mu);

    assert_eq!(
        scan_with(
            &mut universe,
            vec![Token::Cs(skip), char_token('3')],
            |processor| processor.scan_integer().expect("glue coerces").value,
        ),
        7 * Scaled::UNITY
    );
    // The `mu_val` step reports `mu_error` and then falls through the same
    // width/int cascade.
    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(muskip)], |processor| {
            processor.scan_integer().expect("mu glue coerces").value
        }),
        5 * Scaled::UNITY
    );
}

#[test]
fn dimension_scanner_coerces_internal_glue_to_its_width() {
    // TeX82 §429/§449: `scan_dimen` fetches at `dimen_val`, so a glue
    // parameter becomes its width and is the complete answer
    // (`\hsize=\parskip`).
    let mut universe = Universe::new();
    let parskip = universe.intern("parskip").symbol();
    universe.set_meaning(parskip, Meaning::GlueParam(2));
    let spec = universe.intern_glue(glue(3 * Scaled::UNITY, 2 * Scaled::UNITY, 0));
    universe.set_glue_param(tex_state::env::banks::GlueParam::PAR_SKIP, spec);

    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(parskip)], |processor| {
            processor
                .scan_dimension()
                .expect("glue coerces")
                .value
                .raw()
        }),
        3 * Scaled::UNITY
    );
}

#[test]
fn dimension_scanner_negates_a_signed_internal_glue_width() {
    // TeX82 §448's `attach_sign`: an internal quantity reached through the
    // leading-sign loop is negated after the level cascade, so `-\skip0`
    // scans as the negated width rather than as a missing number.
    let mut universe = Universe::new();
    let skip = universe.intern("skip").symbol();
    universe.set_meaning(skip, Meaning::SkipRegister(0));
    let spec = universe.intern_glue(glue(4 * Scaled::UNITY, 0, Scaled::UNITY));
    universe.set_skip(0, spec);

    assert_eq!(
        scan_with(
            &mut universe,
            vec![char_token('-'), Token::Cs(skip)],
            |processor| processor
                .scan_dimension()
                .expect("signed glue scans")
                .value
                .raw(),
        ),
        -4 * Scaled::UNITY
    );
}

#[test]
fn mu_dimension_scanner_accepts_a_bare_internal_mu_glue_quantity() {
    // TeX82 §449/§451: with `mu` set, `scan_dimen` fetches at `mu_val` and
    // "Coerce glue to a dimension" replaces the specification by its width
    // without changing `cur_val_level`, so `\mkern\thinmuskip` uses the
    // parameter's width directly.
    let mut universe = Universe::new();
    let thinmuskip = universe.intern("thinmuskip").symbol();
    universe.set_meaning(thinmuskip, Meaning::MuGlueParam(15));
    let spec = universe.intern_glue(glue(3 * Scaled::UNITY, Scaled::UNITY, 0));
    universe.set_glue_param(tex_state::env::banks::GlueParam::new(15), spec);

    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(thinmuskip)], |processor| {
            processor
                .scan_mu_dimension()
                .expect("mu glue scans as a mu dimension")
                .value
                .raw()
        }),
        3 * Scaled::UNITY
    );
}

#[test]
fn glue_scanner_negates_all_three_components_of_a_signed_internal_glue() {
    // TeX82 §430's "Negate all three glue components": `\skip0=-\skip1`
    // negates the width, stretch, and shrink together. Routing the signed
    // quantity through the width-only dimension scanner would drop the
    // stretch and shrink entirely.
    let mut universe = Universe::new();
    let skip = universe.intern("skip").symbol();
    universe.set_meaning(skip, Meaning::SkipRegister(1));
    let spec = universe.intern_glue(glue(6 * Scaled::UNITY, 2 * Scaled::UNITY, Scaled::UNITY));
    universe.set_skip(1, spec);

    let scanned = scan_with(
        &mut universe,
        vec![char_token('-'), Token::Cs(skip)],
        |processor| {
            processor
                .scan_glue(false)
                .expect("signed internal glue scans")
                .value
        },
    );
    assert_eq!(scanned.width.raw(), -6 * Scaled::UNITY);
    assert_eq!(scanned.stretch.raw(), -2 * Scaled::UNITY);
    assert_eq!(scanned.shrink.raw(), -Scaled::UNITY);
}

#[test]
fn dimension_scanner_uses_an_internal_integer_as_its_numeric_prefix() {
    // TeX82 §449: an internal quantity that settles at `int_val` is not the
    // answer but the numeric prefix of an ordinary units scan, and §448's
    // `if cur_val<0` moves its sign to `attach_sign` so the fixed-point
    // conversion still sees a nonnegative operand (`\dimen0=\count5 pt`).
    let mut universe = Universe::new();
    let count = universe.intern("count").symbol();
    universe.set_meaning(count, Meaning::CountRegister(5));
    universe.set_count(5, -3);

    assert_eq!(
        scan_with(
            &mut universe,
            vec![Token::Cs(count), char_token('p'), char_token('t')],
            |processor| processor
                .scan_dimension()
                .expect("internal integer prefix scans")
                .value
                .raw(),
        ),
        -3 * Scaled::UNITY
    );
}

#[test]
fn glue_scanner_scans_units_after_an_internal_integer_prefix() {
    // TeX82 §461: `if cur_val_level=int_val then scan_dimen(mu,false,true)`
    // -- the internal integer is the width's numeric prefix, and the glue's
    // stretch and shrink keywords still follow.
    let mut universe = Universe::new();
    let count = universe.intern("count").symbol();
    universe.set_meaning(count, Meaning::CountRegister(0));
    universe.set_count(0, 2);

    let scanned = scan_with(
        &mut universe,
        vec![Token::Cs(count)]
            .into_iter()
            .chain("pt plus 1pt".chars().map(char_token))
            .collect(),
        |processor| {
            processor
                .scan_glue(false)
                .expect("internal integer glue width scans")
                .value
        },
    );
    assert_eq!(scanned.width.raw(), 2 * Scaled::UNITY);
    assert_eq!(scanned.stretch.raw(), Scaled::UNITY);
}

#[test]
fn dimension_scanner_scales_true_units_by_the_prepared_magnification() {
    // TeX82 §457's "Adjust for the magnification ratio": `true` divides the
    // scanned quantity by `mag/1000` before the physical unit is converted,
    // so a `true` unit still measures one physical unit on the magnified
    // page. Recognizing the keyword and discarding it silently produces
    // `mag/1000`-times-too-large scaled points in every `\mag`-scaled job.
    let mut universe = Universe::new();
    universe.set_mag_global(2000);

    assert_eq!(
        scan_with(
            &mut universe,
            "1truept".chars().map(char_token).collect(),
            |processor| processor
                .scan_dimension()
                .expect("true dimension scans")
                .value
                .raw(),
        ),
        Scaled::UNITY / 2
    );
    // `\mag=1000` is the identity, and a plain unit is never scaled.
    let mut unmagnified = Universe::new();
    assert_eq!(
        scan_with(
            &mut unmagnified,
            "1truept 1pt".chars().map(char_token).collect(),
            |processor| (
                processor.scan_dimension().expect("true scans").value.raw(),
                processor.scan_dimension().expect("plain scans").value.raw(),
            ),
        ),
        (Scaled::UNITY, Scaled::UNITY)
    );
}

#[test]
fn true_units_scale_fractional_dimensions_before_converting_the_unit() {
    // §457 rescales `cur_val` and the fraction `f` together, carrying the
    // remainder, and does so *before* §458 converts the physical unit --
    // scaling the assembled scaled value instead loses the carried
    // remainder. Hand-evaluating tex.web for `\mag=1440` and `1.5truein`:
    // `f=round_decimals("5")=32768`; `xn_over_d(1,1000,1440)` is 0 remainder
    // 1000, so `f=(1000*32768+65536*1000) div 1440=68266`, `cur_val=1`, and
    // `f=2730`; then `in`'s 7227/100 gives `cur_val=75` and `f=18383`, so
    // `attach_fraction` yields `75*65536+18383`.
    let mut universe = Universe::new();
    universe.set_mag_global(1440);

    assert_eq!(
        scan_with(
            &mut universe,
            "1.5truein".chars().map(char_token).collect(),
            |processor| processor
                .scan_dimension()
                .expect("true fractional dimension scans")
                .value
                .raw(),
        ),
        75 * Scaled::UNITY + 18_383
    );
}

#[test]
fn unknown_unit_recovers_by_assuming_points() {
    // TeX82 §459's "Complain about unknown unit": TeX reports "Illegal unit
    // of measure (pt inserted)", assumes `pt`, and finishes the job. A hard
    // scanner failure would abandon a run that real pdfTeX completes.
    let mut universe = Universe::new();

    assert_eq!(
        scan_with(
            &mut universe,
            "3xy".chars().map(char_token).collect(),
            |processor| processor
                .scan_dimension()
                .expect("unknown unit recovers")
                .value
                .raw(),
        ),
        3 * Scaled::UNITY
    );
}

#[test]
fn mu_dimension_with_a_non_mu_unit_recovers_by_assuming_mu() {
    // TeX82 §456's "Illegal unit of measure (mu inserted)": the scanned
    // quantity is kept as mu and the offending unit text stays in the input.
    let mut universe = Universe::new();

    let (value, following) = scan_with(
        &mut universe,
        "2pt".chars().map(char_token).collect(),
        |processor| {
            let value = processor
                .scan_mu_dimension()
                .expect("mu mismatch recovers")
                .value
                .raw();
            (value, processor.scan_keyword("pt").expect("keyword").value)
        },
    );
    assert_eq!(value, 2 * Scaled::UNITY);
    assert!(following, "the rejected unit text is left to be re-read");
}

#[test]
fn oversized_dimension_clamps_to_max_dimen() {
    // TeX82 §460's "Report that this dimension is out of range": TeX prints
    // "Dimension too large", uses `max_dimen`, and clears `arith_error`.
    let mut universe = Universe::new();

    assert_eq!(
        scan_with(
            &mut universe,
            "20000pt".chars().map(char_token).collect(),
            |processor| processor
                .scan_dimension()
                .expect("oversized dimension recovers")
                .value,
        ),
        Scaled::MAX_DIMEN
    );
}

#[test]
fn continental_decimal_comma_introduces_a_fraction_like_a_point() {
    // TeX82 §448 aliases `continental_point_token` to `point_token` twice in
    // `scan_dimen`, so `3,5pt` is exactly `3.5pt` and a leading `,5pt` is
    // `0.5pt`. Without it an embedded comma survives the integer scan and no
    // unit keyword can match, and a leading one reads as a missing number.
    let mut universe = Universe::new();

    assert_eq!(
        scan_with(
            &mut universe,
            "3,5pt ,5pt".chars().map(char_token).collect(),
            |processor| (
                processor.scan_dimension().expect("embedded").value.raw(),
                processor.scan_dimension().expect("leading").value.raw(),
            ),
        ),
        (3 * Scaled::UNITY + Scaled::UNITY / 2, Scaled::UNITY / 2)
    );
}

#[test]
fn radix_prefixed_dimension_constants_scan_and_admit_no_fraction() {
    // TeX82 §448's own example is `-'77 pt`, so §444's octal and hexadecimal
    // introducers are legal dimension prefixes: `scan_int` owns them, and
    // §448 has no digit test of its own that could reject them.
    //
    // They are also why §448 guards §452 with `(radix=10)`: §440 initializes
    // `radix:=0` and only §444's decimal branch sets it to 10, so the point
    // in `'77.5pt` is not a decimal point. It reaches §453's unit scan
    // instead, which reports "Illegal unit of measure" and assumes `pt`,
    // leaving `.5pt` for the next scan.
    let mut universe = Universe::new();

    assert_eq!(
        scan_with(
            &mut universe,
            "'77pt".chars().map(char_token).collect(),
            |processor| processor.scan_dimension().expect("octal").value.raw(),
        ),
        63 * Scaled::UNITY
    );
    assert_eq!(
        scan_with(
            &mut universe,
            "\"1Fpt".chars().map(char_token).collect(),
            |processor| processor.scan_dimension().expect("hexadecimal").value.raw(),
        ),
        31 * Scaled::UNITY
    );
    assert_eq!(
        scan_with(
            &mut universe,
            "'77.5pt".chars().map(char_token).collect(),
            |processor| (
                processor.scan_dimension().expect("octal").value.raw(),
                processor.scan_dimension().expect("remainder").value.raw(),
            ),
        ),
        (63 * Scaled::UNITY, Scaled::UNITY / 2)
    );
}

#[test]
fn excess_l_suffixes_past_filll_are_consumed_rather_than_left_in_the_input() {
    // TeX82 §454's `while scan_keyword("l") do`: `filllll` yields `filll`
    // plus one error per extra `l`, and every `l` is consumed. Stopping the
    // loop at `filll` would leak the extra letters into later parsing.
    let mut universe = Universe::new();

    let (glue, following) = scan_with(
        &mut universe,
        "0pt plus 1filllll 5".chars().map(char_token).collect(),
        |processor| {
            let glue = processor
                .scan_glue(false)
                .expect("infinite stretch scans")
                .value;
            (glue, processor.scan_integer().expect("integer").value)
        },
    );
    assert_eq!(glue.stretch.raw(), Scaled::UNITY);
    assert_eq!(glue.stretch_order, Order::Filll);
    assert_eq!(following, 5);
}

#[test]
fn code_table_primitives_read_at_the_integer_level() {
    // TeX82 §414's "Fetch a character code from some table": `\catcode`,
    // `\lccode`, `\uccode`, `\sfcode`, `\mathcode`, and `\delcode` all scan
    // a character selector and read at `int_val`. They were wired for
    // assignment only, so `\ifnum\catcode`\~=13` silently compared zero.
    use tex_state::meaning::UnexpandablePrimitive as P;

    for (name, primitive, expected) in [
        ("catcode", P::CatCode, 11),
        ("lccode", P::LcCode, i32::from(b'a')),
        ("uccode", P::UcCode, i32::from(b'A')),
        ("sfcode", P::SfCode, 999),
        ("mathcode", P::MathCode, 7),
        ("delcode", P::DelCode, 42),
    ] {
        let mut universe = Universe::new();
        universe.set_catcode('A', Catcode::Letter);
        universe.set_lccode('A', u32::from(b'a'));
        universe.set_uccode('A', u32::from(b'A'));
        universe.set_sfcode('A', 999);
        universe.set_mathcode('A', 7);
        universe.set_delcode('A', 42);
        let symbol = universe.intern(name).symbol();
        universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));

        assert_eq!(
            scan_with(
                &mut universe,
                vec![Token::Cs(symbol), char_token('`'), char_token('A')],
                |processor| processor.scan_integer().expect("code table scans").value,
            ),
            expected,
            "\\{name} reads at int_val"
        );
    }
}

#[test]
fn parshape_reads_its_line_count() {
    // TeX82 §423's "Fetch the par_shape size": `\parshape` reads the number
    // of lines in the current shape, or zero when none is set.
    let mut universe = Universe::new();
    let parshape = universe.intern("parshape").symbol();
    universe.set_meaning(
        parshape,
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::ParShape),
    );

    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(parshape)], |processor| {
            processor.scan_integer().expect("empty shape scans").value
        }),
        0
    );

    universe.set_paragraph_shape(
        &[
            tex_state::ParagraphShapeLine {
                indent: Scaled::from_raw(0),
                width: Scaled::from_raw(Scaled::UNITY),
            },
            tex_state::ParagraphShapeLine {
                indent: Scaled::from_raw(0),
                width: Scaled::from_raw(Scaled::UNITY),
            },
        ],
        false,
    );
    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(parshape)], |processor| {
            processor.scan_integer().expect("shape size scans").value
        }),
        2
    );
}

#[test]
fn prev_depth_and_prev_graf_read_through_the_host_capability() {
    // TeX82 §418's "Fetch the space_factor or the prev_depth" and §422's
    // "Fetch the prev_graf": `\prevdepth` reads at `dimen_val` in vertical
    // mode and `\prevgraf` at `int_val`. Both are executor-owned mode-nest
    // facts, refreshed per operation like `space_factor`. Without the arms,
    // `\ifdim\prevdepth>-1000pt` silently compared zero and changed every
    // vertical spacing decision that idiom guards.
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let prev_depth = universe.intern("prevdepth").symbol();
    universe.set_meaning(prev_depth, Meaning::UnexpandablePrimitive(P::PrevDepth));
    let prev_graf = universe.intern("prevgraf").symbol();
    universe.set_meaning(prev_graf, Meaning::UnexpandablePrimitive(P::PrevGraf));
    push(
        &mut command,
        vec![Token::Cs(prev_depth), Token::Cs(prev_graf)],
    );

    let mut capabilities = CommandHostCapabilities::default();
    capabilities.set_prev_depth(Some(Scaled::from_raw(3 * Scaled::UNITY)));
    capabilities.set_prev_graf(Some(4));
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(
        processor
            .scan_dimension()
            .expect("prev_depth scans")
            .value
            .raw(),
        3 * Scaled::UNITY
    );
    assert_eq!(processor.scan_integer().expect("prev_graf scans").value, 4);
}

#[test]
fn prev_depth_outside_vertical_mode_reads_zero() {
    // §418's `if abs(mode)<>m then ... scanned_result(0)(dimen_val)`: an
    // absent capability is the improper-mode case, which reads zero rather
    // than the last vertical list's depth.
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let prev_depth = universe.intern("prevdepth").symbol();
    universe.set_meaning(prev_depth, Meaning::UnexpandablePrimitive(P::PrevDepth));
    push(&mut command, vec![Token::Cs(prev_depth)]);

    let mut capabilities = CommandHostCapabilities::default();
    capabilities.set_prev_depth(None);
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    let scanned = processor.scan_dimension().expect("improper mode recovers");
    assert_eq!(scanned.value.raw(), 0);
    assert_eq!(scanned.recovery, ScalarRecovery::InsertedZero);
}

#[test]
fn font_integers_fetch_hyphen_and_skew_characters_of_the_current_font() {
    // tex.web §426's "Fetch a font integer": `scan_font_ident` selects the
    // font, then `m=0` reads `hyphen_char[f]` and `m<>0` reads `skew_char[f]`,
    // both at `int_val`. Without this arm `\hyphenchar\font` fell through to
    // `scan_int`'s missing-number recovery and silently scanned as zero.
    use tex_state::font::NULL_FONT;
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let hyphen_char = universe.intern("hyphenchar").symbol();
    let skew_char = universe.intern("skewchar").symbol();
    let current_font = universe.intern("font").symbol();
    universe.set_meaning(hyphen_char, Meaning::UnexpandablePrimitive(P::HyphenChar));
    universe.set_meaning(skew_char, Meaning::UnexpandablePrimitive(P::SkewChar));
    universe.set_meaning(current_font, Meaning::UnexpandablePrimitive(P::Font));
    universe.set_font_hyphen_char(NULL_FONT, -1);
    universe.set_font_skew_char(NULL_FONT, 127);

    let mut tokens = vec![
        Token::Cs(hyphen_char),
        Token::Cs(current_font),
        Token::Cs(skew_char),
        Token::Cs(current_font),
    ];
    tokens.extend("42".chars().map(char_token));
    push(&mut command, tokens);

    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    let hyphen = processor.scan_integer().expect("hyphenchar scans");
    assert_eq!(hyphen.value, -1);
    assert_eq!(hyphen.recovery, ScalarRecovery::None);
    let skew = processor.scan_integer().expect("skewchar scans");
    assert_eq!(skew.value, 127);
    assert_eq!(skew.recovery, ScalarRecovery::None);
    assert_eq!(
        processor
            .scan_integer()
            .expect("the following number is still available")
            .value,
        42
    );
}

#[test]
fn font_integers_read_the_named_font_rather_than_the_current_one() {
    // §426 fetches through `scan_font_ident`, so `\skewchar\tenrm` reads the
    // named font's `skew_char` while a different font is selected.
    use tex_state::font::NULL_FONT;
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let tenrm = universe.intern("tenrm").symbol();
    let named = universe
        .try_copy_font_with_identifier(NULL_FONT, tenrm)
        .expect("a font identifier is definable from nullfont");
    universe.set_meaning(tenrm, Meaning::Font(named));
    let skew_char = universe.intern("skewchar").symbol();
    universe.set_meaning(skew_char, Meaning::UnexpandablePrimitive(P::SkewChar));
    universe.set_font_skew_char(NULL_FONT, 11);
    universe.set_font_skew_char(named, 96);

    push(&mut command, vec![Token::Cs(skew_char), Token::Cs(tenrm)]);

    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(processor.scan_integer().expect("skewchar scans").value, 96);
}
