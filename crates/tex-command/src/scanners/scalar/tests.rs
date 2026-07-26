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
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(processor.scan_integer().expect("chardef scans").value, 13);
}

#[test]
fn integer_scanner_accepts_mathchardef_values() {
    // TeX82 §424 groups `char_given` and `math_given` under one
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
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(
        processor.scan_integer().expect("mathchardef scans").value,
        10_000
    );
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
    // TeX82 §449/§450: with `mu` set, `scan_dimen` fetches at `mu_val` and
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
