use tex_state::Universe;
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{EffectRecord, PrintSink};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandDialect, CommandHostCapabilities, CommandHostContext, CommandObservation,
    CommandObserver, CommandProfile, CommandRuntime, CommandState, InputTransition, ObservedToken,
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
        cat: catcode(ch),
    }
}

/// The plain-TeX category code of a test character.
///
/// A space is category 10, not category 12: tex.web's numeric scanners test
/// `cur_cmd<>spacer` (§443, §444, §452) and §407's `scan_keyword` skips a
/// leading `spacer`, so a fixture that spells a space as `other_char` is
/// exercising a token no tokenizer produces.
fn catcode(ch: char) -> Catcode {
    if ch.is_ascii_alphabetic() {
        Catcode::Letter
    } else if ch == ' ' {
        Catcode::Space
    } else {
        Catcode::Other
    }
}

fn scanner_tokens(source: &str) -> Vec<Token> {
    source
        .chars()
        .map(|ch| {
            if ch == ' ' {
                char_token(' ')
            } else {
                char_token(ch)
            }
        })
        .collect()
}

fn diagnostic_text(universe: &Universe) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
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

/// TeX82 §455's internal-unit probe opens with §406's "Get the next non-blank
/// non-call token", so spaces before an internal unit are skipped.
///
/// §449 reaches §453's unit scan directly when the coefficient was an
/// `int_val` internal quantity, and nothing on that path has absorbed an
/// optional space -- §445's and §452's trailing-space rules belong to the
/// digit scans, which never ran. `\dimen0=\pretolerance␣\dimen2` is therefore
/// the ordinary shape in which §455 meets a space. Reading a single token
/// instead ended the probe on it, so §453's keyword scan found no unit and
/// §459 recovered the whole dimension as `pt` (umber2-johp.115).
#[test]
fn an_internal_dimension_unit_may_be_preceded_by_a_space() {
    use tex_state::env::banks::IntParam;
    use tex_state::meaning::UnexpandablePrimitive as P;
    use tex_state::scaled::Scaled;

    const PRETOLERANCE: u16 = 0;

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    universe.set_int_param(IntParam::new(PRETOLERANCE), 3);
    universe.set_dimen(2, Scaled::from_raw(5 * Scaled::UNITY));
    let coefficient = universe.intern("pretolerance").symbol();
    universe.set_meaning(coefficient, Meaning::IntParam(PRETOLERANCE));
    let dimen = universe.intern("dimen").symbol();
    universe.set_meaning(dimen, Meaning::UnexpandablePrimitive(P::Dimen));

    push(
        &mut command,
        vec![
            Token::Cs(coefficient),
            char_token(' '),
            Token::Cs(dimen),
            char_token('2'),
        ],
    );

    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    let scanned = processor.scan_dimension().expect("dimension scans");
    assert_eq!(scanned.value.raw(), 15 * Scaled::UNITY);
    assert_eq!(scanned.recovery, ScalarRecovery::None);
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
fn a_decimal_fraction_absorbs_the_space_that_ends_it() {
    // tex.web §452's `<Scan decimal fraction>` ends with
    //
    //     if cur_cmd<>spacer then back_input;
    //
    // the same terminator rule §443's `<Scan an optional space>` and §444's
    // `<Scan a numeric constant>` use. So `.5 in` reaches §453's unit scan
    // with `i` as the very next token. Backing the space up instead installs
    // a backup input level, re-delivers the space, and leaves every later
    // delivery one step behind the oracle (umber2-johp.267).
    let mut command = CommandState::default();
    push(&mut command, ".5 in42".chars().map(char_token).collect());
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
                .expect("fractional dimension scans")
                .value
                .raw(),
            2_368_143
        );
        assert_eq!(
            processor
                .scan_integer()
                .expect("following token remains available")
                .value,
            42
        );
    }

    let raw_characters = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Command(record)
                if record.boundary == crate::CommandDeliveryBoundary::Raw =>
            {
                match record.spelling {
                    ObservedToken::Character { character, .. } => Some(character),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        raw_characters.iter().filter(|&&ch| ch == ' ').count(),
        1,
        "the fraction's terminating space is delivered once and absorbed: {raw_characters:?}"
    );
    let space = raw_characters
        .iter()
        .position(|&ch| ch == ' ')
        .expect("the terminating space is delivered");
    assert_eq!(
        raw_characters.get(space + 1),
        Some(&'i'),
        "the unit scan sees the unit's first letter next: {raw_characters:?}"
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
    scan_with_profile(universe, CommandProfile::TEX82, tokens, scan)
}

/// Runs a focused scalar scan under an explicit immutable character profile.
fn scan_with_profile<T>(
    universe: &mut Universe,
    profile: CommandProfile,
    tokens: Vec<Token>,
    scan: impl FnOnce(&mut CommandProcessor<'_>) -> T,
) -> T {
    let mut command = CommandState::new(profile);
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

/// Exercises TeX82 §416 through a public scalar caller, retaining the
/// recovery's diagnostic and replay state for its caller-specific assertions.
fn scan_missing_number_internal<T>(
    meaning: Meaning,
    scan: impl FnOnce(&mut CommandProcessor<'_>) -> ScannedScalar<T>,
) -> (ScalarRecovery, Vec<&'static str>, usize, String, Meaning) {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let rejected = universe.intern("rejected-internal").symbol();
    universe.set_meaning(rejected, meaning);
    push(&mut command, vec![Token::Cs(rejected)]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let (recovery, replayed) = {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);
        let recovery = scan(&mut processor).recovery;
        let replayed = processor
            .get_x_token()
            .expect("§416 replay remains available")
            .expect("§416 backs the rejected operand once")
            .meaning();
        (recovery, replayed)
    };
    let backups = recorder
        .0
        .iter()
        .filter(|record| matches!(record, CommandObservation::Input(record) if record.transition == InputTransition::Backup))
        .count();
    (
        recovery,
        scanner_kinds(&recorder),
        backups,
        diagnostic_text(&universe),
        replayed,
    )
}

#[test]
fn section_416_recovery_is_internal_once_and_each_scalar_caller_observes_its_result() {
    use tex_state::font::NULL_FONT;

    for meaning in [Meaning::Font(NULL_FONT), Meaning::ToksRegister(0)] {
        let (recovery, kinds, backups, diagnostic, replayed) =
            scan_missing_number_internal(meaning, |processor| {
                processor.scan_integer().expect("integer recovery")
            });
        assert_eq!(recovery, ScalarRecovery::None);
        assert_eq!(kinds, vec!["internal", "integer"]);
        assert_eq!(backups, 1);
        assert_eq!(replayed, meaning);
        assert_eq!(
            diagnostic
                .matches("Missing number, treated as zero")
                .count(),
            1
        );
        for help in [
            "A number should have been here; I inserted `0'.",
            "(If you can't figure out why I needed to see a number,",
            "look up `weird error' in the index to The TeXbook.)",
        ] {
            assert_eq!(diagnostic.matches(help).count(), 1);
        }

        let (recovery, kinds, backups, diagnostic, replayed) =
            scan_missing_number_internal(meaning, |processor| {
                processor.scan_dimension().expect("dimension recovery")
            });
        assert_eq!(recovery, ScalarRecovery::None);
        assert_eq!(kinds, vec!["internal", "dimension"]);
        assert_eq!(backups, 1);
        assert_eq!(replayed, meaning);
        assert_eq!(
            diagnostic
                .matches("Missing number, treated as zero")
                .count(),
            1
        );
        assert_eq!(
            diagnostic
                .matches("A number should have been here; I inserted `0'.")
                .count(),
            1
        );

        let (recovery, kinds, _backups, diagnostic, replayed) =
            scan_missing_number_internal(meaning, |processor| {
                processor.scan_glue(false).expect("glue recovery")
            });
        assert_eq!(recovery, ScalarRecovery::None);
        assert_eq!(kinds, vec!["internal", "glue"]);
        assert_eq!(replayed, meaning);
        assert_eq!(
            diagnostic
                .matches("Missing number, treated as zero")
                .count(),
            1
        );
        assert_eq!(
            diagnostic
                .matches("look up `weird error' in the index to The TeXbook.)")
                .count(),
            1
        );
    }
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
fn dimension_infinite_units_accept_mixed_case_repeated_suffixes() {
    // TeX82 §454 implements every repeated suffix as §407's
    // `scan_keyword("l")`. Exercise both entry paths: an adjacent leading
    // `f` is already the integer scan's terminator, while a space before
    // `fil` reaches the ordinary unit-keyword probe.
    for (source, expected) in [
        ("0pt plus 1fIl 7", Order::Fil),
        ("0pt plus 1fIlL 7", Order::Fill),
        ("0pt plus 1fIlLl 7", Order::Filll),
        ("0pt plus 1 FiL 7", Order::Fil),
        ("0pt plus 1 FiLl 7", Order::Fill),
        ("0pt plus 1 FiLlL 7", Order::Filll),
        // Each one-letter keyword scan skips leading spaces, including after
        // filll; excess mixed-case suffixes are consumed while the order
        // remains clamped, and the first non-l token is replayed.
        ("0pt plus 1fIl L l L 7", Order::Filll),
        ("0pt plus 1 FiLlLlL 7", Order::Filll),
    ] {
        let mut universe = Universe::new();
        let (glue, following) = scan_with(&mut universe, scanner_tokens(source), |processor| {
            let glue = processor
                .scan_glue(false)
                .expect("mixed-case infinite stretch scans")
                .value;
            let following = processor
                .scan_integer()
                .expect("boundary integer remains")
                .value;
            (glue, following)
        });
        assert_eq!(glue.stretch.raw(), Scaled::UNITY, "{source}");
        assert_eq!(glue.stretch_order, expected, "{source}");
        assert_eq!(following, 7, "{source}");
    }
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
    assert_eq!(scanned.recovery, ScalarRecovery::None);
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

/// TeX82 §415's font-identifier branch is `back_input; scan_font_ident`.
///
/// §415 never reads the font off the command it already holds: it pushes that
/// command back and re-reads it through §577. The backup level, its recovery
/// record, and the command's second delivery are all observable, and reading
/// the font in place emitted only the internal result -- five events short per
/// `\the<font identifier>` (umber2-johp.259).
#[test]
fn the_font_identifier_backs_the_command_up_and_rereads_it_through_scan_font_ident() {
    use tex_state::font::NULL_FONT;

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let tenrm = universe.intern("tenrm").symbol();
    let named = universe
        .try_copy_font_with_identifier(NULL_FONT, tenrm)
        .expect("a font identifier is definable from nullfont");
    universe.set_meaning(tenrm, Meaning::Font(named));
    push(&mut command, vec![Token::Cs(tenrm)]);

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
        let target = processor
            .get_x_token()
            .expect("the font identifier delivers")
            .expect("input is not exhausted");
        let value = processor
            .scan_the_internal_value(&target)
            .expect("§415 fetches a font identifier")
            .expect("`\\tenrm` is an internal quantity");
        assert_eq!(value, InternalValue::Font(tenrm));
    }

    let backups = recorder
        .0
        .iter()
        .filter(|record| {
            matches!(record, CommandObservation::Input(record)
                if record.transition == InputTransition::Backup)
        })
        .count();
    assert_eq!(backups, 1, "§415 backs the font identifier up exactly once");
    let deliveries = recorder
        .0
        .iter()
        .filter(|record| {
            matches!(record, CommandObservation::Command(record)
                if record.boundary == crate::CommandDeliveryBoundary::Raw)
        })
        .count();
    assert_eq!(
        deliveries, 2,
        "§415's `back_input` makes §577 deliver the same command a second time"
    );
    let backup = recorder
        .0
        .iter()
        .position(|record| {
            matches!(record, CommandObservation::Input(record)
                if record.transition == InputTransition::Backup)
        })
        .expect("the font identifier is backed up");
    assert!(
        matches!(
            &recorder.0[backup + 1],
            CommandObservation::Recovery(recovery)
                if recovery.tokens == vec![ObservedToken::ControlSequence("tenrm".into())]
        ),
        "the recovery record names the backed-up font identifier"
    );
}

/// TeX82 §577's `def_family` branch: `m:=cur_chr; scan_four_bit_int;
/// f:=equiv(m+cur_val)`.
///
/// `\textfont`, `\scriptfont`, and `\scriptscriptfont` are font identifiers
/// wherever `scan_font_ident` runs -- not only in §415 -- so `\skewchar
/// \textfont1` reads family 1's text font. Handling the family index outside
/// `scan_font_ident` left every other caller unable to name one.
#[test]
fn scan_font_ident_resolves_a_math_family_font() {
    use tex_state::font::NULL_FONT;
    use tex_state::math::MathFontSize;
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let teni = universe.intern("teni").symbol();
    let family_font = universe
        .try_copy_font_with_identifier(NULL_FONT, teni)
        .expect("a font identifier is definable from nullfont");
    universe.set_math_family_font(MathFontSize::Text, 1, family_font, false);
    universe.set_font_skew_char(family_font, 127);
    let text_font = universe.intern("textfont").symbol();
    universe.set_meaning(text_font, Meaning::UnexpandablePrimitive(P::TextFont));
    let skew_char = universe.intern("skewchar").symbol();
    universe.set_meaning(skew_char, Meaning::UnexpandablePrimitive(P::SkewChar));

    push(
        &mut command,
        vec![Token::Cs(skew_char), Token::Cs(text_font), char_token('1')],
    );

    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(
        processor.scan_integer().expect("skewchar scans").value,
        127,
        "§426 fetches through §577, which resolves `\\textfont1`"
    );
}

/// TeX82 §415 tests `level<>tok_val` before it scans any operand.
///
/// The whole `level<>tok_val` branch is `back_error;
/// scanned_result(0)(dimen_val)`: §415's own `scan_eight_bit_int`, §577's
/// `scan_four_bit_int`, and §415's `back_input; scan_font_ident` are all
/// unreachable on that path, so `\count0=\textfont1` leaves both `\textfont`
/// and `1` in the input rather than eating the family index.
#[test]
fn an_integer_request_for_a_font_identifier_scans_no_operand() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let text_font = universe.intern("textfont").symbol();
    universe.set_meaning(text_font, Meaning::UnexpandablePrimitive(P::TextFont));
    push(
        &mut command,
        vec![Token::Cs(text_font), char_token('1'), char_token('7')],
    );

    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    let scanned = processor.scan_integer().expect("§415 recovers");
    assert_eq!(scanned.value, 0);
    assert_eq!(scanned.recovery, ScalarRecovery::None);

    let mut replayed = Vec::new();
    while let Some(command) = processor.get_x_token().expect("replay delivers") {
        replayed.push(match command.meaning() {
            Meaning::CharToken { ch, .. } => ch,
            Meaning::UnexpandablePrimitive(P::TextFont) => '\\',
            other => panic!("unexpected replayed meaning {other:?}"),
        });
    }
    assert_eq!(
        replayed,
        vec!['\\', '1', '7'],
        "the rejected command is backed up and its family index is untouched"
    );
}

/// TeX82 §407's failed match restores the input as two levels, not one.
///
/// `back_input` undoes the offending delivery -- one observed backup push
/// carrying its recovery record -- and `back_list(link(backup_head))` then
/// pushes the already-matched prefix on top of it as a separate level with no
/// recovery record of its own. Collapsing both into a single level loses a
/// push transition the pinned oracle records for every partially matched
/// keyword, which is what `\lower.5ex` produced when `scan_keyword("em")`
/// consumed `e` and rejected `x`.
#[test]
fn a_failed_keyword_backs_the_offender_and_the_matched_prefix_up_separately() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    push(&mut command, "ex".chars().map(char_token).collect());
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let replayed = {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);

        assert!(!processor.scan_keyword("em").expect("keyword scans").value);
        let mut replayed = Vec::new();
        while let Some(command) = processor.get_x_token().expect("replay delivers") {
            match command.meaning() {
                Meaning::CharToken { ch, .. } => replayed.push(ch),
                other => panic!("unexpected replayed meaning {other:?}"),
            }
        }
        replayed
    };

    // The prefix is pushed above the offender, so it is reread first.
    assert_eq!(replayed, vec!['e', 'x']);

    let backups = recorder
        .0
        .iter()
        .filter(|record| {
            matches!(record, CommandObservation::Input(record)
                if record.transition == InputTransition::Backup)
        })
        .count();
    assert_eq!(backups, 2, "§407 pushes back_input and back_list");
    let recoveries = recorder
        .0
        .iter()
        .filter(|record| matches!(record, CommandObservation::Recovery(_)))
        .count();
    assert_eq!(
        recoveries, 1,
        "§323's back_list carries no recovery record; only §325's back_input does"
    );
    let first_backup = recorder
        .0
        .iter()
        .position(|record| {
            matches!(record, CommandObservation::Input(record)
                if record.transition == InputTransition::Backup)
        })
        .expect("the offender is backed up");
    assert!(
        matches!(
            &recorder.0[first_backup + 1],
            CommandObservation::Recovery(recovery)
                if recovery.tokens
                    == vec![ObservedToken::Character {
                        character: 'x',
                        catcode: Catcode::Letter,
                    }]
        ),
        "the recovery record names the offending token, not the matched prefix"
    );
}

/// TeX82 §407 consumes a space read before anything has matched and never
/// gives it back: `(cur_cmd<>spacer)or(p<>backup_head)` is false, so `k` does
/// not advance and the token is simply dropped. A space read *after* a
/// partial match is an ordinary mismatch instead.
#[test]
fn a_keyword_scan_drops_leading_spaces_and_rejects_interior_ones() {
    let mut universe = Universe::new();
    let leading = scan_with(
        &mut universe,
        vec![
            char_token(' '),
            char_token(' '),
            char_token('x'),
            char_token('y'),
        ],
        |processor| {
            assert!(!processor.scan_keyword("pt").expect("keyword scans").value);
            let mut replayed = Vec::new();
            while let Some(command) = processor.get_x_token().expect("replay delivers") {
                match command.meaning() {
                    Meaning::CharToken { ch, .. } => replayed.push(ch),
                    other => panic!("unexpected replayed meaning {other:?}"),
                }
            }
            replayed
        },
    );
    assert_eq!(leading, vec!['x', 'y'], "leading spaces are discarded");

    let interior = scan_with(
        &mut universe,
        vec![char_token('p'), char_token(' '), char_token('t')],
        |processor| {
            assert!(!processor.scan_keyword("pt").expect("keyword scans").value);
            let mut replayed = Vec::new();
            while let Some(command) = processor.get_x_token().expect("replay delivers") {
                match command.meaning() {
                    Meaning::CharToken { ch, .. } => replayed.push(ch),
                    other => panic!("unexpected replayed meaning {other:?}"),
                }
            }
            replayed
        },
    );
    assert_eq!(
        interior,
        vec!['p', ' ', 't'],
        "a space after a partial match is restored with the prefix"
    );
}

/// TeX82 §407's `cur_cs=0`: only a character token can spell a keyword
/// letter. A control sequence `\let` to one has the same `cur_cmd` and
/// `cur_chr` and still cannot match.
#[test]
fn a_control_sequence_let_to_a_keyword_letter_does_not_match() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let letter_p = universe.intern("p").symbol();
    universe.set_meaning(
        letter_p,
        Meaning::CharToken {
            ch: 'p',
            cat: Catcode::Letter,
        },
    );
    push(&mut command, vec![Token::Cs(letter_p), char_token('t')]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );
    assert!(!processor.scan_keyword("pt").expect("keyword scans").value);
}

fn scan_internal_with(
    universe: &mut Universe,
    tokens: Vec<Token>,
    configure_host: impl FnOnce(&mut CommandHostCapabilities),
) -> InternalValue {
    let mut command = CommandState::default();
    push(&mut command, tokens);
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    configure_host(&mut capabilities);
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );
    let target = processor
        .get_x_token()
        .expect("internal target delivers")
        .expect("internal target exists");
    processor
        .scan_the_internal_value(&target)
        .expect("internal target scans")
        .expect("target is internal")
}

fn internal_primitive(
    universe: &mut Universe,
    name: &str,
    primitive: tex_state::meaning::UnexpandablePrimitive,
) -> Token {
    let symbol = universe.intern(name).symbol();
    universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    Token::Cs(symbol)
}

fn scanner_test_box(
    universe: &mut Universe,
    width: Scaled,
    height: Scaled,
    depth: Scaled,
) -> tex_state::ids::NodeListId {
    use tex_state::glue::Order;
    use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
    use tex_state::scaled::GlueSetRatio;

    let children = universe.freeze_node_list(&[]);
    let node = BoxNode::new(BoxNodeFields {
        width,
        height,
        depth,
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    });
    universe.freeze_node_list(&[Node::HList(node)])
}

#[test]
fn internal_equality_table_sources_scan_each_code_family_and_character_boundary() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    for character in ['\0', 'A', 'ÿ'] {
        let mut universe = Universe::new();
        universe.set_catcode(character, Catcode::Letter);
        universe.set_lccode(character, 17);
        universe.set_uccode(character, 23);
        universe.set_sfcode(character, 999);
        universe.set_mathcode(character, 321);
        universe.set_delcode(character, 654);
        for (name, primitive, expected) in [
            ("catcode", P::CatCode, i32::from(Catcode::Letter as u8)),
            ("lccode", P::LcCode, 17),
            ("uccode", P::UcCode, 23),
            ("sfcode", P::SfCode, 999),
            ("mathcode", P::MathCode, 321),
            ("delcode", P::DelCode, 654),
        ] {
            let token = internal_primitive(&mut universe, name, primitive);
            let value = scan_with(
                &mut universe,
                vec![
                    token,
                    char_token('`'),
                    Token::Char {
                        ch: character,
                        cat: Catcode::Other,
                    },
                ],
                |processor| processor.scan_integer().expect("code table scans").value,
            );
            assert_eq!(value, expected, "{name} at U+{:04X}", u32::from(character));
        }
    }
}

#[test]
fn internal_token_sources_preserve_empty_nonempty_and_identifier_values() {
    use tex_state::font::NULL_FONT;
    use tex_state::ids::TokenListId;

    let mut universe = Universe::new();
    let nonempty = universe.intern_token_list(&[char_token('x'), char_token(' ')]);
    universe.set_toks(7, nonempty);
    let toks = universe.intern("saved").symbol();
    universe.set_meaning(toks, Meaning::ToksRegister(7));
    assert_eq!(
        scan_internal_with(&mut universe, vec![Token::Cs(toks)], |_| {}),
        InternalValue::Tokens { tokens: nonempty }
    );

    let empty = universe.intern("empty").symbol();
    universe.set_meaning(empty, Meaning::ToksRegister(0));
    assert_eq!(
        scan_internal_with(&mut universe, vec![Token::Cs(empty)], |_| {}),
        InternalValue::Tokens {
            tokens: TokenListId::EMPTY
        }
    );

    let identifier = universe.intern("namedfont").symbol();
    let font = universe
        .try_copy_font_with_identifier(NULL_FONT, identifier)
        .expect("font identity copies");
    universe.set_meaning(identifier, Meaning::Font(font));
    assert_eq!(
        scan_internal_with(&mut universe, vec![Token::Cs(identifier)], |_| {}),
        InternalValue::Font(identifier)
    );
}

#[test]
fn internal_token_sources_recover_illegal_requested_levels_and_indexes() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut universe = Universe::new();
    let saved = universe.intern("saved").symbol();
    universe.set_meaning(saved, Meaning::ToksRegister(4));
    let scanned = scan_with(&mut universe, vec![Token::Cs(saved)], |processor| {
        processor
            .scan_integer()
            .expect("token below tok_val recovers")
    });
    assert_eq!(scanned.value, 0);
    assert_eq!(scanned.recovery, ScalarRecovery::None);

    let zero = universe.intern_token_list(&[char_token('z')]);
    universe.set_toks(0, zero);
    for selector in ["-1", "256"] {
        let toks = internal_primitive(&mut universe, "toks", P::Toks);
        let tokens = std::iter::once(toks)
            .chain(selector.chars().map(char_token))
            .collect();
        assert_eq!(
            scan_internal_with(&mut universe, tokens, |_| {}),
            InternalValue::Tokens { tokens: zero }
        );
    }
}

#[test]
fn tex82_scanner_conditionals_observes_token_list_internal_results() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let saved = universe.intern_token_list(&[char_token('x'), char_token(' ')]);
    universe.set_toks(7, saved);
    let toks = internal_primitive(&mut universe, "observed-toks", P::Toks);
    push(&mut command, vec![toks, char_token('7')]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let value = {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);
        let target = processor
            .get_x_token()
            .expect("token-list primitive delivers")
            .expect("token-list primitive exists");
        processor
            .scan_the_internal_value(&target)
            .expect("token-list internal scans")
            .expect("token-list primitive is internal")
    };

    assert_eq!(value, InternalValue::Tokens { tokens: saved });
    assert_eq!(scanner_kinds(&recorder), vec!["integer", "internal"]);
    let selector = recorder
        .0
        .iter()
        .position(|record| {
            matches!(record, CommandObservation::Scanner(scanner)
                if scanner.kind == "integer" && scanner.value == "7")
        })
        .expect("register selector is observed");
    let result = recorder
        .0
        .iter()
        .position(|record| {
            matches!(record, CommandObservation::Scanner(scanner)
            if scanner.kind == "internal"
                && scanner.value == "tokens"
                && scanner.tokens.as_deref() == Some(&[
                    ObservedToken::Character {
                        character: 'x',
                        catcode: Catcode::Letter,
                    },
                    ObservedToken::Character {
                        character: ' ',
                        catcode: Catcode::Space,
                    },
                ]))
        })
        .expect("typed token-list result is observed with its spelling");
    assert!(
        selector < result,
        "selector commits before the token-list result"
    );
}

#[test]
fn internal_auxiliary_sources_cover_spacefactor_prevdepth_modes_and_recovery() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    for (primitive, expected, configure) in [
        (P::SpaceFactor, InternalValue::Integer(1200), 0_u8),
        (
            P::PrevDepth,
            InternalValue::Dimension(Scaled::from_raw(77)),
            1,
        ),
        (P::PrevGraf, InternalValue::Integer(9), 2),
    ] {
        let mut universe = Universe::new();
        let token = internal_primitive(&mut universe, "aux", primitive);
        let value = scan_internal_with(&mut universe, vec![token], |host| match configure {
            0 => host.set_space_factor(Some(1200)),
            1 => host.set_prev_depth(Some(Scaled::from_raw(77))),
            _ => host.set_prev_graf(Some(9)),
        });
        assert_eq!(value, expected);
    }

    for (primitive, scan_integer) in [(P::SpaceFactor, true), (P::PrevGraf, true)] {
        let mut universe = Universe::new();
        let token = internal_primitive(&mut universe, "unavailable-aux", primitive);
        if scan_integer {
            assert_eq!(
                scan_with(&mut universe, vec![token], |processor| processor
                    .scan_integer()
                    .expect("unavailable internal integer scans")),
                ScannedScalar {
                    value: 0,
                    recovery: ScalarRecovery::None,
                    provenance: ScalarProvenance {
                        primary: OriginId::UNKNOWN
                    },
                }
            );
        }
    }

    let mut universe = Universe::new();
    let prev_depth = internal_primitive(&mut universe, "prevdepth", P::PrevDepth);
    assert_eq!(
        scan_with(&mut universe, vec![prev_depth], |processor| processor
            .scan_dimension()
            .expect("wrong-mode prevdepth recovers")),
        ScannedScalar {
            value: Scaled::from_raw(0),
            recovery: ScalarRecovery::None,
            provenance: ScalarProvenance {
                primary: OriginId::UNKNOWN
            },
        }
    );
}

#[test]
fn input_line_group_and_conditional_internal_integers_read_live_command_state() {
    use crate::conditionals::{ConditionalKind, IfLimit};
    use crate::{RegisteredSourceKind, SourceRegistration};

    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            std::sync::Arc::<[u8]>::from(b"\\relax\n\\inputlineno".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    command
        .conditions
        .push_with_inversion(ConditionalKind::IfTrue, 0, true);
    assert!(
        command
            .conditions
            .change_if_limit(crate::processor::status::ConditionId(0), IfLimit::Fi)
    );

    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.enter_group_with_kind(tex_state::GroupKind::HBox);
    let input_line = universe.intern("inputlineno").symbol();
    let group_level = universe.intern("currentgrouplevel").symbol();
    let group_type = universe.intern("currentgrouptype").symbol();
    let if_level = universe.intern("currentiflevel").symbol();
    let if_type = universe.intern("currentiftype").symbol();
    let if_branch = universe.intern("currentifbranch").symbol();
    for (symbol, integer) in [
        (
            input_line,
            tex_state::meaning::InternalInteger::InputLineNumber,
        ),
        (
            group_level,
            tex_state::meaning::InternalInteger::CurrentGroupLevel,
        ),
        (
            group_type,
            tex_state::meaning::InternalInteger::CurrentGroupType,
        ),
        (
            if_level,
            tex_state::meaning::InternalInteger::CurrentIfLevel,
        ),
        (if_type, tex_state::meaning::InternalInteger::CurrentIfType),
        (
            if_branch,
            tex_state::meaning::InternalInteger::CurrentIfBranch,
        ),
    ] {
        universe.set_meaning(symbol, Meaning::InternalInteger(integer));
    }
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    let _relax = processor
        .get_x_token()
        .expect("first source token delivers")
        .expect("first source token exists");
    let input = processor
        .get_x_token()
        .expect("input-line token delivers")
        .expect("input-line token exists");
    assert_eq!(
        processor
            .scan_the_internal_value(&input)
            .expect("input line scans"),
        Some(InternalValue::Integer(2))
    );
    push(
        processor.command,
        vec![
            Token::Cs(group_level),
            Token::Cs(group_type),
            Token::Cs(if_level),
            Token::Cs(if_type),
            Token::Cs(if_branch),
        ],
    );
    for expected in [1, 2, 1, -15, 1] {
        let token = processor
            .get_x_token()
            .expect("state token delivers")
            .expect("state token exists");
        assert_eq!(
            processor
                .scan_the_internal_value(&token)
                .expect("state integer scans"),
            Some(InternalValue::Integer(expected))
        );
    }
}

#[test]
fn internal_page_shape_box_sources_cover_empty_active_and_register_boundaries() {
    use tex_state::meaning::UnexpandablePrimitive as P;
    use tex_state::page::{PageDimension, PageInteger};

    let mut universe = Universe::new();
    universe.set_page_integer(PageInteger::DeadCycles, 6);
    universe.set_page_dimension(PageDimension::Goal, Scaled::from_raw(123));
    let page_int = universe.intern("deadcycles").symbol();
    universe.set_meaning(page_int, Meaning::PageInteger(PageInteger::DeadCycles));
    let page_dim = universe.intern("pagegoal").symbol();
    universe.set_meaning(page_dim, Meaning::PageDimension(PageDimension::Goal));
    assert_eq!(
        scan_internal_with(&mut universe, vec![Token::Cs(page_int)], |_| {}),
        InternalValue::Integer(6)
    );
    assert_eq!(
        scan_internal_with(&mut universe, vec![Token::Cs(page_dim)], |_| {}),
        InternalValue::Dimension(Scaled::MAX_DIMEN),
        "an empty page exposes TeX82's max_dimen page-goal sentinel"
    );
    universe.set_page_contents(tex_state::page::PageContents::BoxThere);
    assert_eq!(
        scan_internal_with(&mut universe, vec![Token::Cs(page_dim)], |_| {}),
        InternalValue::Dimension(Scaled::from_raw(123))
    );

    let shape = internal_primitive(&mut universe, "parshape", P::ParShape);
    assert_eq!(
        scan_internal_with(&mut universe, vec![shape], |_| {}),
        InternalValue::Integer(0)
    );
    universe.set_paragraph_shape(
        &[tex_state::ParagraphShapeLine {
            indent: Scaled::from_raw(1),
            width: Scaled::from_raw(2),
        }],
        false,
    );
    let shape = internal_primitive(&mut universe, "parshape", P::ParShape);
    assert_eq!(
        scan_internal_with(&mut universe, vec![shape], |_| {}),
        InternalValue::Integer(1)
    );

    let box_node = scanner_test_box(
        &mut universe,
        Scaled::from_raw(11),
        Scaled::from_raw(22),
        Scaled::from_raw(33),
    );
    universe.set_box_reg(255, box_node);
    for (primitive, expected) in [(P::Wd, 11), (P::Ht, 22), (P::Dp, 33)] {
        let token = internal_primitive(&mut universe, "boxdim", primitive);
        let mut tokens = vec![token];
        tokens.extend("255".chars().map(char_token));
        assert_eq!(
            scan_internal_with(&mut universe, tokens, |_| {}),
            InternalValue::Dimension(Scaled::from_raw(expected))
        );
    }
    let width = internal_primitive(&mut universe, "wd", P::Wd);
    assert_eq!(
        scan_internal_with(&mut universe, vec![width, char_token('0')], |_| {}),
        InternalValue::Dimension(Scaled::from_raw(0))
    );
}

#[test]
fn internal_last_item_sources_cover_each_node_kind_and_empty_list_sentinels() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    let cases = [
        (
            P::LastPenalty,
            Some(crate::LastNodeItem::Penalty(-50)),
            InternalValue::Integer(-50),
        ),
        (
            P::LastKern,
            Some(crate::LastNodeItem::Kern(Scaled::from_raw(17))),
            InternalValue::Dimension(Scaled::from_raw(17)),
        ),
        (
            P::LastSkip,
            Some(crate::LastNodeItem::Glue(glue(1, 2, 3))),
            InternalValue::Glue(glue(1, 2, 3)),
        ),
        (
            P::LastSkip,
            Some(crate::LastNodeItem::MuGlue(glue(4, 5, 6))),
            InternalValue::MuGlue(glue(4, 5, 6)),
        ),
        (P::LastPenalty, None, InternalValue::Integer(0)),
        (
            P::LastKern,
            None,
            InternalValue::Dimension(Scaled::from_raw(0)),
        ),
        (P::LastSkip, None, InternalValue::Glue(GlueSpec::ZERO)),
    ];
    for (primitive, item, expected) in cases {
        let mut universe = Universe::new();
        let token = internal_primitive(&mut universe, "last", primitive);
        assert_eq!(
            scan_internal_with(&mut universe, vec![token], |host| host.set_last_node(item)),
            expected
        );
    }
}

#[test]
fn internal_font_dimensions_cover_first_last_missing_and_named_font_selection() {
    use tex_state::font::NULL_FONT;
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut universe = Universe::new();
    let named_symbol = universe.intern("named").symbol();
    let named = universe
        .try_copy_font_with_identifier(NULL_FONT, named_symbol)
        .expect("font copies");
    universe.set_meaning(named_symbol, Meaning::Font(named));
    universe
        .set_font_dimen(named, 1, Scaled::from_raw(11))
        .expect("first parameter");
    universe
        .set_font_dimen(named, 7, Scaled::from_raw(77))
        .expect("later parameter");
    let fontdimen = internal_primitive(&mut universe, "fontdimen", P::FontDimen);
    for (number, expected) in [(1, 11), (7, 77), (8, 0)] {
        let mut tokens = vec![fontdimen];
        tokens.extend(number.to_string().chars().map(char_token));
        tokens.push(Token::Cs(named_symbol));
        assert_eq!(
            scan_internal_with(&mut universe, tokens, |_| {}),
            InternalValue::Dimension(Scaled::from_raw(expected))
        );
    }
}

#[test]
fn internal_font_integers_cover_current_named_hyphen_and_skew_boundaries() {
    use tex_state::font::NULL_FONT;
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut universe = Universe::new();
    let font_token = internal_primitive(&mut universe, "font", P::Font);
    universe.set_font_hyphen_char(NULL_FONT, -1);
    universe.set_font_skew_char(NULL_FONT, 255);
    for (primitive, expected) in [(P::HyphenChar, -1), (P::SkewChar, 255)] {
        let query = internal_primitive(&mut universe, "fontint", primitive);
        assert_eq!(
            scan_internal_with(&mut universe, vec![query, font_token], |_| {}),
            InternalValue::Integer(expected)
        );
    }

    let named_symbol = universe.intern("namedfont").symbol();
    let named = universe
        .try_copy_font_with_identifier(NULL_FONT, named_symbol)
        .expect("font copies");
    universe.set_meaning(named_symbol, Meaning::Font(named));
    universe.set_font_skew_char(named, 96);
    let query = internal_primitive(&mut universe, "skewchar", P::SkewChar);
    assert_eq!(
        scan_internal_with(&mut universe, vec![query, Token::Cs(named_symbol)], |_| {}),
        InternalValue::Integer(96)
    );
}

#[test]
fn internal_register_sources_cover_all_families_indexes_and_selector_forms() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut universe = Universe::new();
    for index in [0_u16, 17, 65, 255] {
        universe.set_count(index, 1000 + i32::from(index));
        universe.set_dimen(index, Scaled::from_raw(2000 + i32::from(index)));
        let ordinary = universe.intern_glue(glue(3000 + i32::from(index), 1, 2));
        let mu = universe.intern_glue(glue(4000 + i32::from(index), 3, 4));
        universe.set_skip(index, ordinary);
        universe.set_muskip(index, mu);
    }

    for (selector, index) in [
        ("0", 0_u16),
        ("17", 17),
        ("'101", 65),
        ("`A", 65),
        ("255", 255),
    ] {
        let count = internal_primitive(&mut universe, "count", P::Count);
        let dimen = internal_primitive(&mut universe, "dimen", P::Dimen);
        let skip = internal_primitive(&mut universe, "skip", P::Skip);
        let muskip = internal_primitive(&mut universe, "muskip", P::Muskip);
        let suffix: Vec<_> = selector.chars().map(char_token).collect();
        let tokens = |head| {
            std::iter::once(head)
                .chain(suffix.iter().copied())
                .collect()
        };
        assert_eq!(
            scan_internal_with(&mut universe, tokens(count), |_| {}),
            InternalValue::Integer(1000 + i32::from(index))
        );
        assert_eq!(
            scan_internal_with(&mut universe, tokens(dimen), |_| {}),
            InternalValue::Dimension(Scaled::from_raw(2000 + i32::from(index)))
        );
        assert_eq!(
            scan_internal_with(&mut universe, tokens(skip), |_| {}),
            InternalValue::Glue(glue(3000 + i32::from(index), 1, 2))
        );
        assert_eq!(
            scan_internal_with(&mut universe, tokens(muskip), |_| {}),
            InternalValue::MuGlue(glue(4000 + i32::from(index), 3, 4))
        );
    }

    let selector = universe.intern("selector").symbol();
    universe.set_meaning(selector, Meaning::CountRegister(9));
    universe.set_count(9, 17);
    let count = internal_primitive(&mut universe, "count", P::Count);
    assert_eq!(
        scan_internal_with(&mut universe, vec![count, Token::Cs(selector)], |_| {}),
        InternalValue::Integer(1017)
    );
}

#[test]
fn internal_register_sources_recover_missing_negative_and_above_eight_bit_indexes() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut universe = Universe::new();
    universe.set_count(0, 73);
    for selector in ["", "-1", "256"] {
        let count = internal_primitive(&mut universe, "count", P::Count);
        let tokens = std::iter::once(count)
            .chain(selector.chars().map(char_token))
            .collect();
        assert_eq!(
            scan_internal_with(&mut universe, tokens, |_| {}),
            InternalValue::Integer(73),
            "selector {selector:?} recovers to register zero"
        );
    }
    let count = internal_primitive(&mut universe, "count", P::Count);
    let mut tokens = vec![count];
    tokens.extend("256 42".chars().map(char_token));
    let (value, following) = scan_with(&mut universe, tokens, |processor| {
        let target = processor
            .get_x_token()
            .expect("target")
            .expect("target exists");
        let value = processor
            .scan_the_internal_value(&target)
            .expect("scan")
            .expect("internal");
        let following = processor.scan_integer().expect("following integer").value;
        (value, following)
    });
    assert_eq!(value, InternalValue::Integer(73));
    assert_eq!(following, 42);
}

#[test]
fn internal_coercion_lowers_each_source_level_and_commits_requested_level() {
    let mut universe = Universe::new();
    let id = universe.intern_glue(glue(7 * Scaled::UNITY, 2, 3));
    universe.set_skip(3, id);
    let skip = universe.intern("skipthree").symbol();
    universe.set_meaning(skip, Meaning::SkipRegister(3));

    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(skip)], |processor| processor
            .scan_integer()
            .expect("glue lowers to int")
            .value),
        7 * Scaled::UNITY
    );
    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(skip)], |processor| processor
            .scan_dimension()
            .expect("glue lowers to dimen")
            .value
            .raw()),
        7 * Scaled::UNITY
    );
    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(skip)], |processor| processor
            .scan_glue(false)
            .expect("glue remains glue")
            .value),
        glue(7 * Scaled::UNITY, 2, 3)
    );
    assert_eq!(
        scan_internal_with(&mut universe, vec![Token::Cs(skip)], |_| {}),
        InternalValue::Glue(glue(7 * Scaled::UNITY, 2, 3))
    );

    let mu_id = universe.intern_glue(glue(5 * Scaled::UNITY, 4, 6));
    universe.set_muskip(2, mu_id);
    let muskip = universe.intern("muskiptwo").symbol();
    universe.set_meaning(muskip, Meaning::MuskipRegister(2));
    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(muskip)], |processor| {
            processor
                .scan_glue(false)
                .expect("muglue lowers to glue")
                .value
        }),
        glue(5 * Scaled::UNITY, 4, 6)
    );
}

#[test]
fn internal_coercion_balances_glue_references_for_return_lowering_and_error() {
    let mut universe = Universe::new();
    let id = universe.intern_glue(glue(9, 8, 7));
    universe.set_skip(8, id);
    let skip = universe.intern("skip8").symbol();
    universe.set_meaning(skip, Meaning::SkipRegister(8));

    for _ in 0..3 {
        assert_eq!(
            scan_internal_with(&mut universe, vec![Token::Cs(skip)], |_| {}),
            InternalValue::Glue(glue(9, 8, 7))
        );
        assert_eq!(
            scan_with(&mut universe, vec![Token::Cs(skip)], |processor| processor
                .scan_dimension()
                .expect("lowered glue")
                .value
                .raw()),
            9
        );
    }
    assert_eq!(
        universe.glue(id),
        glue(9, 8, 7),
        "immutable glue storage survives return and lowering"
    );
}

#[test]
fn internal_coercion_negates_scalar_glue_and_muglue_components() {
    let mut universe = Universe::new();
    universe.set_count(1, 12);
    universe.set_dimen(1, Scaled::from_raw(34));
    let ordinary = universe.intern_glue(glue(1, 2, 3));
    let mu = universe.intern_glue(glue(4, 5, 6));
    universe.set_skip(1, ordinary);
    universe.set_muskip(1, mu);
    let count = universe.intern("countone").symbol();
    let dimen = universe.intern("dimenone").symbol();
    let skip = universe.intern("skipone").symbol();
    let muskip = universe.intern("muskipone").symbol();
    universe.set_meaning(count, Meaning::CountRegister(1));
    universe.set_meaning(dimen, Meaning::DimenRegister(1));
    universe.set_meaning(skip, Meaning::SkipRegister(1));
    universe.set_meaning(muskip, Meaning::MuskipRegister(1));

    assert_eq!(
        scan_with(
            &mut universe,
            vec![char_token('-'), Token::Cs(count)],
            |p| p.scan_integer().expect("negative integer").value
        ),
        -12
    );
    assert_eq!(
        scan_with(
            &mut universe,
            vec![char_token('-'), Token::Cs(dimen)],
            |p| p.scan_dimension().expect("negative dimension").value.raw()
        ),
        -34
    );
    let negative = scan_with(&mut universe, vec![char_token('-'), Token::Cs(skip)], |p| {
        p.scan_glue(false).expect("negative glue").value
    });
    assert_eq!(
        (
            negative.width.raw(),
            negative.stretch.raw(),
            negative.shrink.raw()
        ),
        (-1, -2, -3)
    );
    let negative_mu = scan_with(
        &mut universe,
        vec![char_token('-'), Token::Cs(muskip)],
        |p| p.scan_glue(true).expect("negative muglue").value,
    );
    assert_eq!(
        (
            negative_mu.width.raw(),
            negative_mu.stretch.raw(),
            negative_mu.shrink.raw()
        ),
        (-4, -5, -6)
    );
}

#[test]
fn internal_coercion_recovers_noninternal_and_token_values_below_tok_level() {
    let mut universe = Universe::new();
    let token_list = universe.intern_token_list(&[char_token('x')]);
    universe.set_toks(4, token_list);
    let toks = universe.intern("saved").symbol();
    universe.set_meaning(toks, Meaning::ToksRegister(4));
    let scanned = scan_with(&mut universe, vec![Token::Cs(toks)], |processor| {
        processor.scan_integer().expect("token below tok_val")
    });
    assert_eq!((scanned.value, scanned.recovery), (0, ScalarRecovery::None));

    let scanned = scan_with(&mut universe, vec![char_token('x')], |processor| {
        processor
            .scan_integer()
            .expect("noninternal missing number")
    });
    assert_eq!(
        (scanned.value, scanned.recovery),
        (0, ScalarRecovery::InsertedZero)
    );
}

#[test]
fn dimension_prefixes_cover_sign_shortcut_internal_decimal_radix_and_character_forms() {
    let cases = [
        ("--3pt", 3 * Scaled::UNITY),
        ("-3pt", -3 * Scaled::UNITY),
        ("'10pt", 8 * Scaled::UNITY),
        ("\"10pt", 16 * Scaled::UNITY),
        ("`Apt", 65 * Scaled::UNITY),
    ];
    for (source, expected) in cases {
        let mut universe = Universe::new();
        assert_eq!(
            scan_with(
                &mut universe,
                source.chars().map(char_token).collect(),
                |processor| {
                    processor
                        .scan_dimension()
                        .expect("dimension prefix scans")
                        .value
                        .raw()
                }
            ),
            expected,
            "prefix {source:?}"
        );
    }

    let mut universe = Universe::new();
    let count = universe.intern("count-prefix").symbol();
    universe.set_meaning(count, Meaning::CountRegister(2));
    universe.set_count(2, -4);
    assert_eq!(
        scan_with(
            &mut universe,
            vec![Token::Cs(count), char_token('p'), char_token('t')],
            |processor| processor
                .scan_dimension()
                .expect("internal prefix scans")
                .value
                .raw(),
        ),
        -4 * Scaled::UNITY
    );

    let complete = universe.intern("complete-dimension").symbol();
    universe.set_meaning(complete, Meaning::DimenRegister(9));
    universe.set_dimen(9, Scaled::from_raw(123_456));
    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(complete)], |processor| {
            processor
                .scan_dimension()
                .expect("complete internal dimension scans")
                .value
                .raw()
        }),
        123_456
    );

    let mut universe = Universe::new();
    assert_eq!(
        scan_with(
            &mut universe,
            "pt".chars().map(char_token).collect(),
            |processor| {
                processor
                    .scan_dimension_shortcut(7, false)
                    .expect("prepared integer shortcut scans")
                    .raw()
            }
        ),
        7 * Scaled::UNITY
    );
}

#[test]
fn dimension_fractions_cover_point_catcodes_zero_seventeen_and_excess_digits() {
    let mut universe = Universe::new();
    let values = scan_with(
        &mut universe,
        scanner_tokens(
            ".pt .0pt .5pt ,5pt .12345678901234567pt .123456789012345679999pt .00000762939453125pt",
        ),
        |processor| {
            (0..7)
                .map(|_| {
                    processor
                        .scan_dimension()
                        .expect("fraction scans")
                        .value
                        .raw()
                })
                .collect::<Vec<_>>()
        },
    );
    assert_eq!(values[0], 0);
    assert_eq!(values[1], 0);
    assert_eq!(values[2], Scaled::UNITY / 2);
    assert_eq!(values[3], Scaled::UNITY / 2);
    assert_eq!(
        values[4], values[5],
        "digits after TeX82's 17-digit buffer are consumed but ignored"
    );
    assert_eq!(values[6], 1, "the half-sp decimal boundary rounds upward");

    let mut universe = Universe::new();
    let letter_point = Token::Char {
        ch: '.',
        cat: Catcode::Letter,
    };
    let (value, following) = scan_with(
        &mut universe,
        [
            vec![char_token('1')],
            vec![letter_point],
            "5pt".chars().map(char_token).collect(),
        ]
        .concat(),
        |processor| {
            let value = processor
                .scan_dimension()
                .expect("non-other point recovers")
                .value
                .raw();
            let following = processor
                .get_x_token()
                .expect("point delivers")
                .expect("point remains")
                .meaning();
            (value, following)
        },
    );
    assert_eq!(value, Scaled::UNITY);
    assert!(
        matches!(
            following,
            Meaning::CharToken {
                ch: '.',
                cat: Catcode::Letter
            }
        ),
        "only an other-category point starts the fraction"
    );
}

#[test]
fn dimension_prefixes_preserve_unit_terminator_and_scanner_observation_order() {
    let mut command = CommandState::default();
    push(&mut command, scanner_tokens(".5pt  9"));
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
        Scaled::UNITY / 2
    );
    assert!(matches!(
        processor
            .get_x_token()
            .expect("second space delivers")
            .expect("second space remains")
            .meaning(),
        Meaning::CharToken {
            cat: Catcode::Space,
            ..
        }
    ));
    assert_eq!(
        processor
            .scan_integer()
            .expect("following integer scans")
            .value,
        9
    );
    assert_eq!(scanner_kinds(&recorder), vec!["dimension", "integer"]);
}

#[test]
fn dimension_infinite_units_cover_orders_case_spaces_disabled_mode_and_excess_suffixes() {
    use tex_state::glue::Order;

    for (unit, expected) in [
        ("fil", Order::Fil),
        ("fill", Order::Fill),
        ("filll", Order::Filll),
        ("FIL", Order::Fil),
    ] {
        let mut universe = Universe::new();
        let source = format!("0pt plus 1 {unit} 7");
        let (spec, following) = scan_with(&mut universe, scanner_tokens(&source), |processor| {
            let spec = processor
                .scan_glue(false)
                .expect("infinite glue scans")
                .value;
            let following = processor
                .scan_integer()
                .expect("terminator remains usable")
                .value;
            (spec, following)
        });
        assert_eq!(
            (spec.stretch.raw(), spec.stretch_order, following),
            (Scaled::UNITY, expected, 7)
        );
    }

    let mut universe = Universe::new();
    let (value, suffix) = scan_with(
        &mut universe,
        "1fil".chars().map(char_token).collect(),
        |processor| {
            let value = processor
                .scan_dimension()
                .expect("infinite units are disabled for dimensions")
                .value
                .raw();
            let suffix = processor.scan_keyword("fil").expect("suffix probe").value;
            (value, suffix)
        },
    );
    assert_eq!(value, Scaled::UNITY);
    assert!(suffix);

    let mut universe = Universe::new();
    let (spec, following) = scan_with(
        &mut universe,
        scanner_tokens("0pt plus 1fillllll 8"),
        |processor| {
            let spec = processor
                .scan_glue(false)
                .expect("excess suffix recovers")
                .value;
            (
                spec,
                processor
                    .scan_integer()
                    .expect("following integer scans")
                    .value,
            )
        },
    );
    assert_eq!(spec.stretch_order, Order::Filll);
    assert_eq!(following, 8);
}

#[test]
fn dimension_internal_units_scale_whole_fraction_glue_width_and_muglue_compatibility() {
    let mut universe = Universe::new();
    universe.set_dimen(4, Scaled::from_raw(2 * Scaled::UNITY));
    let dimen = universe.intern("dimen-unit").symbol();
    universe.set_meaning(dimen, Meaning::DimenRegister(4));
    let ordinary_id = universe.intern_glue(glue(3 * Scaled::UNITY, 7, 8));
    universe.set_skip(5, ordinary_id);
    let ordinary = universe.intern("glue-unit").symbol();
    universe.set_meaning(ordinary, Meaning::SkipRegister(5));
    let mu_id = universe.intern_glue(glue(4 * Scaled::UNITY, 9, 10));
    universe.set_muskip(6, mu_id);
    let mu = universe.intern("mu-unit").symbol();
    universe.set_meaning(mu, Meaning::MuskipRegister(6));

    for (tokens, expected, is_mu) in [
        (
            vec![char_token('2'), Token::Cs(dimen)],
            4 * Scaled::UNITY,
            false,
        ),
        (
            vec![
                char_token('2'),
                char_token('.'),
                char_token('5'),
                Token::Cs(dimen),
            ],
            5 * Scaled::UNITY,
            false,
        ),
        (
            vec![char_token('2'), Token::Cs(ordinary)],
            6 * Scaled::UNITY,
            false,
        ),
        (
            vec![char_token('2'), Token::Cs(mu)],
            8 * Scaled::UNITY,
            true,
        ),
    ] {
        let actual = scan_with(&mut universe, tokens, |processor| {
            if is_mu {
                processor
                    .scan_mu_dimension()
                    .expect("internal mu unit scans")
                    .value
                    .raw()
            } else {
                processor
                    .scan_dimension()
                    .expect("internal unit scans")
                    .value
                    .raw()
            }
        });
        assert_eq!(actual, expected);
    }
}

#[test]
fn dimension_font_relative_units_cover_em_ex_fraction_and_optional_space_boundaries() {
    use tex_state::font::NULL_FONT;

    let mut universe = Universe::new();
    universe
        .set_font_dimen(NULL_FONT, 6, Scaled::from_raw(10 * Scaled::UNITY))
        .expect("quad sets");
    universe
        .set_font_dimen(NULL_FONT, 5, Scaled::from_raw(4 * Scaled::UNITY))
        .expect("x-height sets");
    let (em, ex, following) =
        scan_with(&mut universe, scanner_tokens("1.5em 2ex  7"), |processor| {
            let em = processor
                .scan_dimension()
                .expect("fractional em scans")
                .value
                .raw();
            let ex = processor.scan_dimension().expect("ex scans").value.raw();
            let token = processor
                .get_x_token()
                .expect("second space delivers")
                .expect("second space remains");
            assert!(matches!(
                token.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ));
            let following = processor
                .scan_integer()
                .expect("following integer scans")
                .value;
            (em, ex, following)
        });
    assert_eq!(
        (em, ex, following),
        (15 * Scaled::UNITY, 8 * Scaled::UNITY, 7)
    );

    universe
        .set_font_dimen(NULL_FONT, 5, Scaled::from_raw(0))
        .expect("zero x-height sets");
    assert_eq!(
        scan_with(
            &mut universe,
            "9ex".chars().map(char_token).collect(),
            |processor| {
                processor
                    .scan_dimension()
                    .expect("zero metric scans")
                    .value
                    .raw()
            }
        ),
        0
    );
}

#[test]
fn dimension_mu_units_cover_success_nonmu_and_mixed_internal_units() {
    let mut universe = Universe::new();
    assert_eq!(
        scan_with(
            &mut universe,
            "2mu".chars().map(char_token).collect(),
            |processor| {
                processor
                    .scan_mu_dimension()
                    .expect("mu unit scans")
                    .value
                    .raw()
            }
        ),
        2 * Scaled::UNITY
    );

    let mut universe = Universe::new();
    let (value, suffix) = scan_with(
        &mut universe,
        "2pt".chars().map(char_token).collect(),
        |processor| {
            let value = processor
                .scan_mu_dimension()
                .expect("non-mu unit recovers")
                .value
                .raw();
            (
                value,
                processor.scan_keyword("pt").expect("suffix probe").value,
            )
        },
    );
    assert_eq!(value, 2 * Scaled::UNITY);
    assert!(suffix);

    let mut universe = Universe::new();
    let (value, suffix) = scan_with(
        &mut universe,
        "2mu".chars().map(char_token).collect(),
        |processor| {
            let value = processor
                .scan_dimension()
                .expect("mu in ordinary mode recovers")
                .value
                .raw();
            (
                value,
                processor.scan_keyword("mu").expect("suffix probe").value,
            )
        },
    );
    assert_eq!(value, 2 * Scaled::UNITY);
    assert!(suffix);

    let mut universe = Universe::new();
    let ordinary_id = universe.intern_glue(glue(3 * Scaled::UNITY, 0, 0));
    universe.set_skip(1, ordinary_id);
    let ordinary = universe.intern("ordinary-unit").symbol();
    universe.set_meaning(ordinary, Meaning::SkipRegister(1));
    let (value, following) = scan_with(
        &mut universe,
        vec![char_token('2'), Token::Cs(ordinary)],
        |processor| {
            let value = processor
                .scan_mu_dimension()
                .expect("mixed internal unit scans")
                .value
                .raw();
            let following = processor
                .get_x_token()
                .expect("input remains available")
                .map(|command| command.meaning());
            (value, following)
        },
    );
    assert_eq!(value, 6 * Scaled::UNITY);
    assert_eq!(following, None, "§455 consumes an internal unit once");
    assert!(diagnostic_text(&universe).contains("Incompatible glue units"));
}

#[test]
fn dimension_internal_unit_probe_accepts_integer_and_missing_number_zero_without_keyword_replay() {
    // TeX82 §455 branches on `min_internal..max_internal`, not on the final
    // value level. An integer is therefore a scaled unit, and §416's zero is
    // likewise an accepted unit rather than an `em`/`ex`/physical-unit probe.
    let mut universe = Universe::new();
    let count = universe.intern("unit-count").symbol();
    universe.set_meaning(count, Meaning::CountRegister(1));
    universe.set_count(1, 3);
    let (ordinary, mu) = scan_with(
        &mut universe,
        vec![
            char_token('2'),
            Token::Cs(count),
            char_token('2'),
            Token::Cs(count),
        ],
        |processor| {
            (
                processor
                    .scan_dimension()
                    .expect("integer unit scans")
                    .value
                    .raw(),
                processor
                    .scan_mu_dimension()
                    .expect("integer mu unit scans")
                    .value
                    .raw(),
            )
        },
    );
    assert_eq!((ordinary, mu), (6, 6));
    assert!(diagnostic_text(&universe).contains("Incompatible glue units"));

    use tex_state::font::NULL_FONT;

    let mut universe = Universe::new();
    let font = universe.intern("unit-font").symbol();
    universe.set_meaning(font, Meaning::Font(NULL_FONT));
    let (value, following) = scan_with(
        &mut universe,
        vec![
            char_token('9'),
            Token::Cs(font),
            char_token('p'),
            char_token('t'),
        ],
        |processor| {
            let value = processor
                .scan_dimension()
                .expect("missing-number unit recovers as zero")
                .value
                .raw();
            let following = processor
                .get_x_token()
                .expect("remaining input scans")
                .map(|command| command.meaning());
            (value, following)
        },
    );
    assert_eq!(value, 0);
    assert_eq!(following, Some(Meaning::Font(NULL_FONT)));
    assert!(
        !diagnostic_text(&universe).contains("Illegal unit of measure"),
        "§455 must not fall through to physical-unit recovery for §416 zero"
    );
}

#[test]
fn dimension_physical_units_cover_all_factors_rounding_case_and_sp_shortcut() {
    for (actual, canonical) in [
        ("1pc", "12pt"),
        ("100in", "7227pt"),
        ("254cm", "7227pt"),
        ("2540mm", "7227pt"),
        ("7200bp", "7227pt"),
        ("1157dd", "1238pt"),
        ("1157cc", "14856pt"),
        ("65536sp", "1pt"),
        ("1PT", "1pt"),
    ] {
        let mut universe = Universe::new();
        let scanned = scan_with(
            &mut universe,
            actual.chars().map(char_token).collect(),
            |processor| {
                processor
                    .scan_dimension()
                    .expect("physical unit scans")
                    .value
                    .raw()
            },
        );
        let mut universe = Universe::new();
        let expected = scan_with(
            &mut universe,
            canonical.chars().map(char_token).collect(),
            |processor| {
                processor
                    .scan_dimension()
                    .expect("canonical unit scans")
                    .value
                    .raw()
            },
        );
        assert_eq!(scanned, expected, "{actual} equals {canonical}");
    }

    let mut universe = Universe::new();
    assert_eq!(
        scan_with(
            &mut universe,
            "1.5sp".chars().map(char_token).collect(),
            |processor| {
                processor
                    .scan_dimension()
                    .expect("fractional sp truncates")
                    .value
                    .raw()
            }
        ),
        1
    );

    let mut universe = Universe::new();
    assert_eq!(
        scan_with(
            &mut universe,
            "1.25in".chars().map(char_token).collect(),
            |processor| {
                processor
                    .scan_dimension()
                    .expect("fractional physical unit rounds")
                    .value
                    .raw()
            }
        ),
        5_920_358
    );
}

#[test]
fn dimension_true_units_cover_mag_1000_low_high_and_fraction_remainders() {
    for (mag, source, expected) in [
        (1000, "1truept", Scaled::UNITY),
        (500, "1truept", 2 * Scaled::UNITY),
        (2000, "1truept", Scaled::UNITY / 2),
        (1440, "1.5truein", 75 * Scaled::UNITY + 18_383),
    ] {
        let mut universe = Universe::new();
        universe.set_mag_global(mag);
        assert_eq!(
            scan_with(
                &mut universe,
                source.chars().map(char_token).collect(),
                |processor| {
                    processor
                        .scan_dimension()
                        .expect("true unit scans")
                        .value
                        .raw()
                }
            ),
            expected,
            "mag={mag}, source={source}"
        );
    }
}

#[test]
fn dimension_unknown_units_preserve_keyword_backup_and_assume_points() {
    for source in ["3xy", "3XY", "3truX"] {
        let mut universe = Universe::new();
        let suffix = &source[1..];
        let (value, preserved) = scan_with(
            &mut universe,
            source.chars().map(char_token).collect(),
            |processor| {
                let value = processor
                    .scan_dimension()
                    .expect("unknown unit recovers")
                    .value
                    .raw();
                let preserved = processor
                    .scan_keyword(suffix)
                    .expect("preserved suffix scans")
                    .value;
                (value, preserved)
            },
        );
        assert_eq!(value, 3 * Scaled::UNITY);
        assert!(preserved, "failed keyword probes preserve {suffix:?}");
    }
}

#[test]
fn dimension_range_recovery_covers_positive_negative_and_arithmetic_overflow() {
    let mut universe = Universe::new();
    let values = scan_with(
        &mut universe,
        scanner_tokens("1073741823sp 20000pt 1pt -20000pt 2pt 1073741824sp 3pt"),
        |processor| {
            (0..7)
                .map(|_| {
                    processor
                        .scan_dimension()
                        .expect("range recovery continues")
                        .value
                })
                .collect::<Vec<_>>()
        },
    );
    assert_eq!(values[0], Scaled::MAX_DIMEN);
    assert_eq!(values[1], Scaled::MAX_DIMEN);
    assert_eq!(values[2].raw(), Scaled::UNITY);
    assert_eq!(values[3], Scaled::from_raw(-Scaled::MAX_DIMEN.raw()));
    assert_eq!(values[4].raw(), 2 * Scaled::UNITY);
    assert_eq!(values[5], Scaled::MAX_DIMEN);
    assert_eq!(
        values[6].raw(),
        3 * Scaled::UNITY,
        "arith_error is cleared after recovery"
    );
}

#[test]
fn tex82_scanner_conditionals_observes_fractional_physical_and_true_units() {
    let mut command = CommandState::default();
    push(&mut command, scanner_tokens("1.25IN 1.5truein"));
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    universe.set_mag_global(1440);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let values = {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);
        [
            processor
                .scan_dimension()
                .expect("fractional physical dimension scans")
                .value,
            processor
                .scan_dimension()
                .expect("true dimension scans")
                .value,
        ]
    };

    assert_eq!(values[0].raw(), 5_920_358);
    assert_eq!(values[1].raw(), 75 * Scaled::UNITY + 18_383);
    assert_eq!(
        scanner_kinds(&recorder),
        vec!["integer", "dimension", "integer", "dimension"]
    );
    assert_eq!(
        recorder
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::Scanner(scanner) if scanner.kind == "dimension" => {
                    Some(scanner.value.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>(),
        values
            .iter()
            .map(|value| value.raw().to_string())
            .collect::<Vec<_>>()
    );
    let uppercase_i = recorder
        .0
        .iter()
        .position(|record| {
            matches!(record, CommandObservation::Command(command)
            if command.spelling == ObservedToken::Character {
                character: 'I',
                catcode: Catcode::Letter,
            })
        })
        .expect("physical-unit source spelling is observed");
    let first_result = recorder
        .0
        .iter()
        .position(|record| {
            matches!(record, CommandObservation::Scanner(scanner)
                if scanner.kind == "dimension")
        })
        .expect("physical result is observed");
    assert!(
        uppercase_i < first_result,
        "unit delivery precedes its result"
    );
}

#[test]
fn scanner_syntax_mandatory_brace_relax_expansion_and_inserted_recovery() {
    use tex_state::macro_store::MacroMeaning;
    use tex_state::meaning::MeaningFlags;

    let mut universe = Universe::new();
    let macro_symbol = universe.intern("brace").symbol();
    let empty = universe.intern_token_list(&[]);
    let opening = Token::Char {
        ch: '{',
        cat: Catcode::BeginGroup,
    };
    let replacement = universe.intern_token_list(&[opening]);
    universe.set_macro_meaning(
        macro_symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement),
    );
    let scanned = scan_with(
        &mut universe,
        vec![char_token(' '), char_token(' '), Token::Cs(macro_symbol)],
        |processor| {
            processor
                .scan_left_brace(true)
                .expect("expanded brace scans")
                .meaning()
        },
    );
    assert!(matches!(
        scanned,
        Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        }
    ));

    let mut universe = Universe::new();
    let (recovered, following) = scan_with(&mut universe, vec![char_token('x')], |processor| {
        let recovered = processor.scan_left_brace(true).is_err();
        let following = processor
            .get_x_token()
            .expect("rejected token delivers")
            .expect("rejected token remains")
            .meaning();
        (recovered, following)
    });
    assert!(
        recovered,
        "the caller receives the mandatory-brace recovery boundary"
    );
    assert!(matches!(following, Meaning::CharToken { ch: 'x', .. }));

    // TeX82's intervening `\relax` case remains owned by umber2-johp.209;
    // do not make the current rejection into conformance evidence here.
}

#[test]
fn scanner_syntax_optional_equals_catcode_and_relax_boundaries() {
    let mut universe = Universe::new();
    assert!(scan_with(
        &mut universe,
        vec![char_token(' '), char_token(' '), char_token('=')],
        |processor| processor
            .scan_optional_equals()
            .expect("equals scans")
            .value,
    ));

    for cat in [
        Catcode::BeginGroup,
        Catcode::EndGroup,
        Catcode::MathShift,
        Catcode::AlignmentTab,
        Catcode::EndLine,
        Catcode::Parameter,
        Catcode::Superscript,
        Catcode::Subscript,
        Catcode::Space,
        Catcode::Letter,
    ] {
        let mut universe = Universe::new();
        let mut command = CommandState::default();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                },
                Token::Char { ch: '=', cat },
                char_token('x'),
            ],
        );
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let (accepted, replayed, following) = {
            let mut processor = CommandProcessor::new(
                &mut command,
                &mut runtime,
                universe.command_context(),
                CommandHostContext::new(&mut capabilities),
            )
            .with_observer(&mut recorder);
            let accepted = processor
                .scan_optional_equals()
                .expect("optional-equals probe scans")
                .value;
            let replayed = processor
                .get_x_token()
                .expect("rejected equals replays")
                .expect("rejected equals remains")
                .meaning();
            let following = processor
                .get_x_token()
                .expect("following token delivers")
                .expect("following token remains")
                .meaning();
            (accepted, replayed, following)
        };
        assert!(!accepted, "{cat:?} equals is not §405 syntax");
        assert_eq!(replayed, Meaning::CharToken { ch: '=', cat });
        assert_eq!(
            following,
            Meaning::CharToken {
                ch: 'x',
                cat: Catcode::Letter
            }
        );
        assert_eq!(
            recorder
                .0
                .iter()
                .filter(|record| matches!(record, CommandObservation::Input(input)
                    if input.transition == InputTransition::Backup))
                .count(),
            1,
            "{cat:?} equals is canonically backed up once"
        );
    }

    // §341 turns an active character into its active control sequence before
    // §405 compares `cur_tok`; its category must not make it an equals sign.
    let mut universe = Universe::new();
    let active = universe.intern_active_character('=').symbol();
    universe.set_meaning(active, Meaning::Relax);
    let (accepted, replayed) = scan_with(
        &mut universe,
        vec![Token::Char {
            ch: '=',
            cat: Catcode::Active,
        }],
        |processor| {
            let accepted = processor
                .scan_optional_equals()
                .expect("active optional-equals probe scans")
                .value;
            let replayed = processor
                .get_x_token()
                .expect("active equals replays")
                .expect("active equals remains")
                .meaning();
            (accepted, replayed)
        },
    );
    assert!(!accepted, "active equals is not §405 syntax");
    assert_eq!(replayed, Meaning::Relax);

    let relax = universe.intern("relax").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    let (accepted, following) = scan_with(&mut universe, vec![Token::Cs(relax)], |processor| {
        let accepted = processor
            .scan_optional_equals()
            .expect("relax probe scans")
            .value;
        let following = processor
            .get_x_token()
            .expect("relax delivers")
            .expect("relax remains")
            .meaning();
        (accepted, following)
    });
    assert!(!accepted);
    assert_eq!(following, Meaning::Relax);
}

#[test]
fn scanner_syntax_keyword_character_catcode_matrix_and_mu_error() {
    for cat in [
        Catcode::Letter,
        Catcode::Other,
        Catcode::Space,
        Catcode::BeginGroup,
    ] {
        let mut universe = Universe::new();
        let tokens = ['p', 'T']
            .into_iter()
            .map(|ch| Token::Char { ch, cat })
            .collect();
        assert!(
            scan_with(&mut universe, tokens, |processor| processor
                .scan_keyword("pt")
                .expect("keyword scans")
                .value),
            "character catcode {cat:?}"
        );
    }

    let mut universe = Universe::new();
    let alias = universe.intern("p-alias").symbol();
    universe.set_meaning(alias, Meaning::CharGiven('p'));
    let (accepted, following) = scan_with(
        &mut universe,
        vec![Token::Cs(alias), char_token('t')],
        |processor| {
            let accepted = processor
                .scan_keyword("pt")
                .expect("alias probe scans")
                .value;
            let following = processor
                .get_x_token()
                .expect("alias delivers")
                .expect("alias remains")
                .control_sequence();
            (accepted, following)
        },
    );
    assert!(!accepted);
    assert_eq!(following, Some(alias));

    let mut universe = Universe::new();
    let (mu_value, ordinary_suffix) = scan_with(
        &mut universe,
        "2pt".chars().map(char_token).collect(),
        |processor| {
            let value = processor
                .scan_mu_dimension()
                .expect("non-mu unit recovers")
                .value
                .raw();
            let suffix = processor.scan_keyword("pt").expect("suffix scans").value;
            (value, suffix)
        },
    );
    assert_eq!(mu_value, 2 * Scaled::UNITY);
    assert!(ordinary_suffix);
}

#[test]
fn restricted_integer_all_five_classes_min_max_and_recovery_matrix() {
    use crate::scanners::RestrictedIntegerClass as Class;

    for (class, maximum) in [
        (Class::EightBit, 255),
        (Class::CharacterCode, 255),
        (Class::FourBit, 15),
        (Class::FifteenBit, 32_767),
        (Class::TwentySevenBit, 134_217_727),
    ] {
        for (source, value, recovered) in [
            ("0".to_owned(), 0, false),
            (maximum.to_string(), maximum, false),
            ("-1".to_owned(), 0, true),
            ((maximum + 1).to_string(), 0, true),
        ] {
            let mut universe = Universe::new();
            let scanned = scan_with(
                &mut universe,
                source.chars().map(char_token).collect(),
                |processor| {
                    processor
                        .scan_restricted_integer(class)
                        .expect("restricted scan")
                },
            );
            assert_eq!(
                (scanned.value, scanned.recovered),
                (value, recovered),
                "{class:?} with {source}"
            );
        }
    }
}

#[test]
fn integer_optional_space_sign_and_terminator_matrix() {
    let mut universe = Universe::new();
    let values = scan_with(
        &mut universe,
        scanner_tokens("12   - + - 34  +--5 "),
        |processor| {
            (0..3)
                .map(|_| {
                    processor
                        .scan_integer()
                        .expect("signed integer scans")
                        .value
                })
                .collect::<Vec<_>>()
        },
    );
    assert_eq!(values, vec![12, 34, 5]);

    let mut universe = Universe::new();
    let (value, terminator) = scan_with(
        &mut universe,
        "77x".chars().map(char_token).collect(),
        |processor| {
            let value = processor.scan_integer().expect("integer scans").value;
            let terminator = processor
                .get_x_token()
                .expect("terminator delivers")
                .expect("terminator remains")
                .meaning();
            (value, terminator)
        },
    );
    assert_eq!(value, 77);
    assert!(matches!(terminator, Meaning::CharToken { ch: 'x', .. }));
}

#[test]
fn integer_character_constant_raw_token_and_optional_space_matrix() {
    for (token, expected) in [
        (char_token('A'), 65),
        (
            Token::Char {
                ch: '~',
                cat: Catcode::Active,
            },
            126,
        ),
    ] {
        let mut universe = Universe::new();
        assert_eq!(
            scan_with(
                &mut universe,
                vec![char_token('`'), token, char_token(' '), char_token('7')],
                |processor| {
                    let value = processor
                        .scan_integer()
                        .expect("character constant scans")
                        .value;
                    let following = processor
                        .scan_integer()
                        .expect("optional space consumed")
                        .value;
                    (value, following)
                },
            ),
            (expected, 7)
        );
    }

    for (name, expected) in [("!", 33), ("A", 65)] {
        let mut universe = Universe::new();
        let symbol = universe.intern(name).symbol();
        assert_eq!(
            scan_with(
                &mut universe,
                vec![char_token('`'), Token::Cs(symbol)],
                |processor| processor
                    .scan_integer()
                    .expect("control symbol constant scans")
                    .value,
            ),
            expected
        );
    }

    let mut universe = Universe::new();
    let word = universe.intern("word").symbol();
    let (value, recovery, following) = scan_with(
        &mut universe,
        vec![char_token('`'), Token::Cs(word)],
        |processor| {
            let scanned = processor
                .scan_integer()
                .expect("improper control word recovers");
            let following = processor
                .get_x_token()
                .expect("control word delivers")
                .expect("control word remains")
                .control_sequence();
            (scanned.value, scanned.recovery, following)
        },
    );
    assert_eq!(
        (value, recovery, following),
        (0, ScalarRecovery::None, Some(word))
    );

    for (character, expected) in [('\0', 0), ('ÿ', 255)] {
        let mut universe = Universe::new();
        assert_eq!(
            scan_with(
                &mut universe,
                vec![char_token('`'), char_token(character)],
                |processor| processor
                    .scan_integer()
                    .expect("eight-bit constant scans")
                    .value,
            ),
            expected
        );
    }
}

#[test]
fn integer_character_constants_recover_values_above_255_in_exact_profile() {
    for token_is_control_sequence in [false, true] {
        let mut universe = Universe::new();
        let lambda = if token_is_control_sequence {
            Token::Cs(universe.intern("λ").symbol())
        } else {
            char_token('λ')
        };
        let mut command = CommandState::new(CommandProfile::TEX82);
        push(
            &mut command,
            vec![char_token('`'), lambda, char_token(' '), char_token('7')],
        );
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let (scanned, replayed, spacer, continuation) = {
            let mut processor = CommandProcessor::new(
                &mut command,
                &mut runtime,
                universe.command_context(),
                CommandHostContext::new(&mut capabilities),
            );
            let scanned = processor.scan_integer().expect("high constant recovers");
            let replayed = processor
                .get_next()
                .expect("offender replays")
                .expect("offender remains")
                .spelling()
                .semantic_token();
            let spacer = processor
                .get_next()
                .expect("space replays")
                .expect("space remains")
                .spelling()
                .semantic_token();
            let continuation = processor.scan_integer().expect("scanner continues").value;
            (scanned, replayed, spacer, continuation)
        };
        assert_eq!(scanned.value, 0);
        assert_eq!(scanned.recovery, ScalarRecovery::InsertedZero);
        assert_eq!(replayed, lambda);
        assert!(matches!(
            spacer,
            Token::Char {
                ch: ' ',
                cat: Catcode::Space
            }
        ));
        assert_eq!(continuation, 7);
        let text = diagnostic_text(&universe);
        assert_eq!(text.matches("! Improper alphabetic constant.").count(), 1);
        assert_eq!(
            text.matches("A one-character control sequence belongs after a ` mark.")
                .count(),
            1
        );
        assert_eq!(
            text.matches("So I'm essentially inserting \\0 here.")
                .count(),
            1
        );
    }

    let mut universe = Universe::new();
    let values = scan_with_profile(
        &mut universe,
        CommandProfile::unicode_extended(CommandDialect::Tex82),
        vec![
            char_token('`'),
            char_token('λ'),
            char_token(' '),
            char_token('7'),
        ],
        |processor| {
            (
                processor
                    .scan_integer()
                    .expect("Unicode constant scans")
                    .value,
                processor
                    .scan_integer()
                    .expect("Unicode optional space consumes")
                    .value,
            )
        },
    );
    assert_eq!(values, (955, 7));
    assert!(!diagnostic_text(&universe).contains("Improper alphabetic constant"));
}

#[test]
fn integer_all_radices_invalid_digits_missing_number_and_overflow_boundaries() {
    for (source, expected) in [
        ("42", 42),
        ("'52", 42),
        ("\"2A", 42),
        ("\"2a", 42),
        ("2147483647", i32::MAX),
        ("999999999999999999999", i32::MAX),
        ("-999999999999999999999", -i32::MAX),
    ] {
        let mut universe = Universe::new();
        assert_eq!(
            scan_with(
                &mut universe,
                source.chars().map(char_token).collect(),
                |processor| {
                    processor
                        .scan_integer()
                        .expect("radix or boundary integer scans")
                        .value
                }
            ),
            expected,
            "source {source}"
        );
    }

    let mut universe = Universe::new();
    let (value, following) = scan_with(
        &mut universe,
        "'8".chars().map(char_token).collect(),
        |processor| {
            let value = processor
                .scan_integer()
                .expect("invalid octal digit terminates")
                .value;
            let following = processor
                .get_x_token()
                .expect("invalid digit delivers")
                .expect("invalid digit remains")
                .meaning();
            (value, following)
        },
    );
    assert_eq!(value, 0);
    assert!(matches!(following, Meaning::CharToken { ch: '8', .. }));

    let mut universe = Universe::new();
    let (value, recovery, following) =
        scan_with(&mut universe, vec![char_token('x')], |processor| {
            let scanned = processor.scan_integer().expect("missing number recovers");
            let following = processor
                .get_x_token()
                .expect("offender delivers")
                .expect("offender remains")
                .meaning();
            (scanned.value, scanned.recovery, following)
        });
    assert_eq!((value, recovery), (0, ScalarRecovery::InsertedZero));
    assert!(matches!(following, Meaning::CharToken { ch: 'x', .. }));
}

#[test]
fn glue_numeric_internal_width_plus_minus_order_and_keyword_matrix() {
    use tex_state::glue::Order;

    let mut universe = Universe::new();
    let numeric = scan_with(
        &mut universe,
        scanner_tokens("1pt plus 2fill minus 3fil"),
        |processor| {
            processor
                .scan_glue(false)
                .expect("numeric glue scans")
                .value
        },
    );
    assert_eq!(numeric.width.raw(), Scaled::UNITY);
    assert_eq!(
        (numeric.stretch.raw(), numeric.stretch_order),
        (2 * Scaled::UNITY, Order::Fill)
    );
    assert_eq!(
        (numeric.shrink.raw(), numeric.shrink_order),
        (3 * Scaled::UNITY, Order::Fil)
    );

    let mut universe = Universe::new();
    universe.set_count(1, 2);
    universe.set_dimen(1, Scaled::from_raw(3 * Scaled::UNITY));
    let count = universe.intern("count-width").symbol();
    let dimen = universe.intern("dimen-width").symbol();
    universe.set_meaning(count, Meaning::CountRegister(1));
    universe.set_meaning(dimen, Meaning::DimenRegister(1));
    let count_glue = scan_with(
        &mut universe,
        [vec![Token::Cs(count)], scanner_tokens("pt plus 1pt")].concat(),
        |processor| {
            processor
                .scan_glue(false)
                .expect("internal integer width scans")
                .value
        },
    );
    assert_eq!(
        (count_glue.width.raw(), count_glue.stretch.raw()),
        (2 * Scaled::UNITY, Scaled::UNITY)
    );
    let dimen_glue = scan_with(
        &mut universe,
        [
            vec![Token::Cs(dimen)],
            scanner_tokens(" plus 1pt minus 2pt"),
        ]
        .concat(),
        |processor| {
            processor
                .scan_glue(false)
                .expect("internal dimension width scans")
                .value
        },
    );
    assert_eq!(
        (
            dimen_glue.width.raw(),
            dimen_glue.stretch.raw(),
            dimen_glue.shrink.raw()
        ),
        (3 * Scaled::UNITY, Scaled::UNITY, 2 * Scaled::UNITY)
    );

    let stored = universe.intern_glue(glue(7, 8, 9));
    universe.set_skip(4, stored);
    let skip = universe.intern("complete-glue").symbol();
    universe.set_meaning(skip, Meaning::SkipRegister(4));
    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(skip)], |processor| {
            processor
                .scan_glue(false)
                .expect("complete glue scans")
                .value
        }),
        glue(7, 8, 9)
    );

    let mut universe = Universe::new();
    let (value, replayed) = scan_with(&mut universe, scanner_tokens("0pt plux"), |processor| {
        let value = processor
            .scan_glue(false)
            .expect("failed optional keyword scans")
            .value;
        let replayed = processor
            .scan_keyword("plux")
            .expect("keyword replay scans")
            .value;
        (value, replayed)
    });
    assert_eq!(value, GlueSpec::ZERO);
    assert!(replayed);
}

#[test]
fn muglue_complete_internal_and_mixed_unit_recovery_matrix() {
    use tex_state::glue::Order;

    let mut universe = Universe::new();
    let numeric = scan_with(
        &mut universe,
        scanner_tokens("2mu plus 1filll minus 3mu"),
        |processor| {
            processor
                .scan_glue(true)
                .expect("numeric muglue scans")
                .value
        },
    );
    assert_eq!(numeric.width.raw(), 2 * Scaled::UNITY);
    assert_eq!(
        (numeric.stretch.raw(), numeric.stretch_order),
        (Scaled::UNITY, Order::Filll)
    );
    assert_eq!(numeric.shrink.raw(), 3 * Scaled::UNITY);

    let mu_id = universe.intern_glue(glue(4, 5, 6));
    universe.set_muskip(2, mu_id);
    let mu = universe.intern("complete-muglue").symbol();
    universe.set_meaning(mu, Meaning::MuskipRegister(2));
    let negative = scan_with(
        &mut universe,
        vec![char_token('-'), Token::Cs(mu)],
        |processor| {
            processor
                .scan_glue(true)
                .expect("signed muglue scans")
                .value
        },
    );
    assert_eq!(
        (
            negative.width.raw(),
            negative.stretch.raw(),
            negative.shrink.raw()
        ),
        (-4, -5, -6)
    );

    let ordinary_id = universe.intern_glue(glue(7, 8, 9));
    universe.set_skip(3, ordinary_id);
    let ordinary = universe.intern("ordinary-glue").symbol();
    universe.set_meaning(ordinary, Meaning::SkipRegister(3));
    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(ordinary)], |processor| {
            processor
                .scan_glue(true)
                .expect("mixed glue kind recovers")
                .value
        }),
        glue(7, 8, 9)
    );
}

#[test]
fn scanner_recoveries_emit_tex82_error_reports_without_changing_values() {
    let mut universe = Universe::new();

    assert_eq!(
        scan_with(&mut universe, scanner_tokens("1wat"), |processor| {
            processor.scan_dimension().expect("pt recovery scans").value
        })
        .raw(),
        Scaled::UNITY
    );
    assert_eq!(
        scan_with(&mut universe, scanner_tokens("1wat"), |processor| {
            processor
                .scan_mu_dimension()
                .expect("mu recovery scans")
                .value
        })
        .raw(),
        Scaled::UNITY
    );
    assert_eq!(
        scan_with(&mut universe, scanner_tokens("20000pt"), |processor| {
            processor
                .scan_dimension()
                .expect("overflow recovery scans")
                .value
        }),
        Scaled::MAX_DIMEN
    );

    let tokens = universe.intern("tokens").symbol();
    universe.set_meaning(tokens, Meaning::TokParam(0));
    let missing = scan_with(&mut universe, vec![Token::Cs(tokens)], |processor| {
        processor.scan_integer().expect("token list recovers")
    });
    assert_eq!(missing.value, 0);
    assert_eq!(missing.recovery, ScalarRecovery::None);

    let mu = universe.intern_glue(glue(Scaled::UNITY, 0, 0));
    universe.set_muskip(0, mu);
    let muskip = universe.intern("muskip").symbol();
    universe.set_meaning(muskip, Meaning::MuskipRegister(0));
    assert_eq!(
        scan_with(&mut universe, vec![Token::Cs(muskip)], |processor| {
            processor.scan_integer().expect("mu glue recovers").value
        }),
        Scaled::UNITY
    );

    let text = diagnostic_text(&universe);
    for message in [
        "! Illegal unit of measure (pt inserted).",
        "! Illegal unit of measure (mu inserted).",
        "! Dimension too large.",
        "! Missing number, treated as zero.",
        "! Incompatible glue units.",
        "Dimensions can be in units of em, ex, in, pt, pc,",
        "The unit of measurement in math glue must be mu.",
        "I can't work with sizes bigger than about 19 feet.",
        "A number should have been here; I inserted `0'.",
        "I'm going to assume that 1mu=1pt when they're mixed.",
    ] {
        assert!(text.contains(message), "missing {message:?} in {text}");
    }
}

#[test]
fn true_dimension_scanner_reports_prepare_mag_recoveries() {
    let mut illegal = Universe::new();
    illegal.set_mag_global(40_000);
    assert_eq!(
        scan_with(&mut illegal, scanner_tokens("1truept"), |processor| {
            processor
                .scan_dimension()
                .expect("illegal mag recovers")
                .value
        })
        .raw(),
        Scaled::UNITY
    );
    let illegal_text = diagnostic_text(&illegal);
    assert!(illegal_text.contains("! Illegal magnification has been changed to 1000 (40000)."));
    assert!(illegal_text.contains("The magnification ratio must be between 1 and 32768."));

    let mut incompatible = Universe::new();
    incompatible.set_mag_global(1200);
    let _ = scan_with(&mut incompatible, scanner_tokens("1truept"), |processor| {
        processor.scan_dimension().expect("first mag prepares")
    });
    incompatible.set_mag_global(2000);
    assert_eq!(
        scan_with(&mut incompatible, scanner_tokens("1truept"), |processor| {
            processor
                .scan_dimension()
                .expect("incompatible mag recovers")
                .value
        })
        .raw(),
        54_613
    );
    let incompatible_text = diagnostic_text(&incompatible);
    assert!(incompatible_text.contains(
        "! Incompatible magnification (2000); the previous value will be retained (1200)."
    ));
    assert!(
        incompatible_text.contains("reverted to the magnification you used earlier on this run.")
    );
}

#[test]
fn tex82_scanner_conditionals_observes_glue_and_muglue_results() {
    use tex_state::glue::Order;

    let mut command = CommandState::default();
    push(
        &mut command,
        scanner_tokens("1pt plus 2pt minus 3pt 4pt PLUS 5filll 2mu plus 1fil minus 3mu"),
    );
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let values = {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);
        [
            processor.scan_glue(false).expect("finite glue scans").value,
            processor
                .scan_glue(false)
                .expect("infinite glue scans")
                .value,
            processor.scan_glue(true).expect("muglue scans").value,
        ]
    };

    assert_eq!(
        (
            values[0].width.raw(),
            values[0].stretch.raw(),
            values[0].stretch_order,
            values[0].shrink.raw(),
            values[0].shrink_order,
        ),
        (
            Scaled::UNITY,
            2 * Scaled::UNITY,
            Order::Normal,
            3 * Scaled::UNITY,
            Order::Normal,
        )
    );
    assert_eq!(
        (
            values[1].width.raw(),
            values[1].stretch.raw(),
            values[1].stretch_order
        ),
        (4 * Scaled::UNITY, 5 * Scaled::UNITY, Order::Filll)
    );
    assert_eq!(
        (
            values[2].width.raw(),
            values[2].stretch.raw(),
            values[2].stretch_order,
            values[2].shrink.raw(),
        ),
        (
            2 * Scaled::UNITY,
            Scaled::UNITY,
            Order::Fil,
            3 * Scaled::UNITY,
        )
    );
    let observed_glue = recorder
        .0
        .iter()
        .filter_map(|record| match record {
            CommandObservation::Scanner(scanner) if scanner.kind == "glue" => {
                Some(scanner.value.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        observed_glue,
        [
            "width=65536;stretch=131072;stretch_order=normal;shrink=196608;shrink_order=normal",
            "width=262144;stretch=327680;stretch_order=filll;shrink=0;shrink_order=normal",
            "width=131072;stretch=65536;stretch_order=fil;shrink=196608;shrink_order=normal",
        ]
    );
    assert!(recorder.0.iter().any(|record| matches!(
        record,
        CommandObservation::Command(command)
            if command.spelling == ObservedToken::Character {
                character: 'P',
                catcode: Catcode::Letter,
            }
    )));
}
