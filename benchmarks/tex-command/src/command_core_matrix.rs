use std::hint::black_box;
use std::time::Instant;

use tex_command::{
    CommandFuelLedger, CommandHostCapabilities, CommandHostContext, CommandObservation,
    CommandObserver, CommandProcessor, CommandState, DeliveryStatus,
};
use tex_state::Universe;
use tex_state::interner::Symbol;
use tex_state::token::{Catcode, Token};

#[path = "command_core_fixtures.rs"]
mod fixtures;
#[path = "command_core_receipts.rs"]
mod receipts;

use fixtures::{
    OutputToken, Workload, build_source_text, build_stored_words, install_benchmark_catcodes,
    install_macro, open_source, with_universe, workload,
};

use receipts::{print_record, subtract_work, validate_receipt};

const DEFAULT_ITERATIONS: usize = 100_000;
const DEFAULT_WARMUPS: usize = 64;
const SEMANTIC_SEED: u64 = 0xCBF2_9CE4_8422_2325;
const MISSING_DELIVERY_CHECKSUM: u64 = 0xDEAD_0001;
const MISSING_DELIVERY_SEMANTIC: u64 = 0xDEAD_0002;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Storage {
    Source,
    Stored,
}

impl Storage {
    const ALL: [Self; 2] = [Self::Source, Self::Stored];

    const fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Stored => "stored",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Delivery {
    RawNext,
    RawToken,
    Expanded,
}

impl Delivery {
    const ALL: [Self; 3] = [Self::RawNext, Self::RawToken, Self::Expanded];

    const fn name(self) -> &'static str {
        match self {
            Self::RawNext => "raw_next",
            Self::RawToken => "raw_token",
            Self::Expanded => "expanded",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ObserverMode {
    Disabled,
    Enabled,
}

impl ObserverMode {
    const ALL: [Self; 2] = [Self::Disabled, Self::Enabled];

    const fn name(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Case {
    Plain,
    EmptyMacro,
    NonemptyMacro,
    ParameterizedEmpty,
    ParameterizedIdentity,
    BalancedArgument,
    DelimitedNestedArgument,
}

impl Case {
    const ALL: [Self; 7] = [
        Self::Plain,
        Self::EmptyMacro,
        Self::NonemptyMacro,
        Self::ParameterizedEmpty,
        Self::ParameterizedIdentity,
        Self::BalancedArgument,
        Self::DelimitedNestedArgument,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::EmptyMacro => "empty_macro",
            Self::NonemptyMacro => "nonempty_macro",
            Self::ParameterizedEmpty => "parameterized_empty",
            Self::ParameterizedIdentity => "parameterized_identity",
            Self::BalancedArgument => "balanced_argument",
            Self::DelimitedNestedArgument => "delimited_nested_argument",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|case| case.name() == name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ObserverSelection {
    Both,
    Disabled,
    Enabled,
}

#[derive(Clone, Copy, Debug)]
struct Options {
    case: Option<Case>,
    iterations: usize,
    warmups: usize,
    observers: ObserverSelection,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            case: None,
            iterations: DEFAULT_ITERATIONS,
            warmups: DEFAULT_WARMUPS,
            observers: ObserverSelection::Both,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Receipt {
    elapsed_ns: u128,
    checksum: u64,
    semantic_hash: u64,
    observer_records: u64,
    work: tex_command::CommandWorkCounters,
    macro_expansions: u64,
}

#[derive(Clone, Copy, Debug)]
struct DeliveryEvidence {
    checksum: u64,
    semantic: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StructuralSnapshot {
    pub macro_expansions: u64,
}

#[derive(Clone, Copy)]
pub struct ProfileHooks {
    pub name: &'static str,
    pub snapshot: fn() -> StructuralSnapshot,
}

impl ProfileHooks {
    #[allow(dead_code)]
    pub const fn release() -> Self {
        Self {
            name: "release",
            snapshot: || StructuralSnapshot {
                macro_expansions: 0,
            },
        }
    }
}

#[derive(Default)]
struct CountingObserver {
    records: u64,
}

impl CommandObserver for CountingObserver {
    fn committed(&mut self, observation: CommandObservation) {
        black_box(observation);
        self.records = self.records.saturating_add(1);
    }
}

pub fn run(hooks: ProfileHooks) {
    let options = parse_options();
    let cases = options
        .case
        .map_or_else(|| Case::ALL.to_vec(), |case| vec![case]);
    let observers: &[ObserverMode] = match options.observers {
        ObserverSelection::Both => &ObserverMode::ALL,
        ObserverSelection::Disabled => &[ObserverMode::Disabled],
        ObserverSelection::Enabled => &[ObserverMode::Enabled],
    };

    println!(
        "{{\"schema\":\"command-core-matrix-v1\",\"profile\":\"{}\",\"iterations\":{},\"warmups\":{}}}",
        hooks.name, options.iterations, options.warmups,
    );

    for case in cases {
        for storage in Storage::ALL {
            for delivery in Delivery::ALL {
                for &observer in observers {
                    let receipt = run_case(case, storage, delivery, observer, options, hooks);
                    print_record(
                        case, storage, delivery, observer, options, hooks.name, receipt,
                    );
                }
            }
        }
    }
}

fn parse_options() -> Options {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        let (key, inline_value) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(key, value)| (key, Some(value)));
        match key {
            "--case" => {
                let value = inline_value
                    .map(str::to_owned)
                    .or_else(|| args.next())
                    .unwrap_or_else(|| usage("--case requires a value"));
                options.case = if value == "all" {
                    None
                } else {
                    Some(Case::parse(&value).unwrap_or_else(|| usage("unknown --case value")))
                };
            }
            "--iterations" => {
                options.iterations = parse_positive_count(
                    inline_value
                        .map(str::to_owned)
                        .or_else(|| args.next())
                        .unwrap_or_else(|| usage("--iterations requires a value")),
                    "--iterations",
                );
            }
            "--warmups" => {
                options.warmups = parse_nonnegative_count(
                    inline_value
                        .map(str::to_owned)
                        .or_else(|| args.next())
                        .unwrap_or_else(|| usage("--warmups requires a value")),
                    "--warmups",
                );
            }
            "--observer" => {
                let value = inline_value
                    .map(str::to_owned)
                    .or_else(|| args.next())
                    .unwrap_or_else(|| usage("--observer requires disabled, enabled, or both"));
                options.observers = match value.as_str() {
                    "disabled" => ObserverSelection::Disabled,
                    "enabled" => ObserverSelection::Enabled,
                    "both" => ObserverSelection::Both,
                    _ => usage("--observer requires disabled, enabled, or both"),
                };
            }
            "--help" => usage(""),
            _ => usage("unknown argument"),
        }
    }
    options
}

fn parse_positive_count(value: String, option: &str) -> usize {
    value
        .parse::<usize>()
        .ok()
        .filter(|count| *count > 0)
        .unwrap_or_else(|| usage(&format!("{option} must be a positive integer")))
}

fn parse_nonnegative_count(value: String, option: &str) -> usize {
    value
        .parse::<usize>()
        .unwrap_or_else(|_| usage(&format!("{option} must be a nonnegative integer")))
}

fn usage(error: &str) -> ! {
    if !error.is_empty() {
        eprintln!("error: {error}");
    }
    eprintln!(
        "usage: command-core-matrix [--case NAME|all] [--iterations N] [--warmups N] [--observer disabled|enabled|both]"
    );
    std::process::exit(2);
}

fn run_case(
    case: Case,
    storage: Storage,
    delivery: Delivery,
    observer_mode: ObserverMode,
    options: Options,
    hooks: ProfileHooks,
) -> Receipt {
    with_universe(|universe| {
        let workload = workload(case);
        install_benchmark_catcodes(universe);
        let macro_symbol = install_macro(universe, workload);
        validate_fixture(universe, storage, delivery, workload, macro_symbol);

        let mut command = CommandState::default();
        let operation_count = options
            .iterations
            .checked_add(options.warmups)
            .expect("matrix operation count overflow");
        let denominators = workload.denominators(storage, delivery);
        let stored_words = build_stored_words(workload, macro_symbol, delivery, operation_count);
        assert_eq!(
            stored_words.len(),
            denominators.stored_words_per_operation * operation_count,
            "stored matrix fixture word census"
        );
        match storage {
            Storage::Source => {
                let source = build_source_text(workload, delivery, operation_count);
                open_source(&mut command, &source);
            }
            Storage::Stored => {
                let token_list = universe
                    .allocate_token_list(&stored_words)
                    .expect("stored matrix input allocation");
                let context = universe.command_context().expect("stored matrix context");
                command.push_everyjob(&context, token_list);
            }
        }

        let mut fuel = CommandFuelLedger::default();
        {
            let mut capabilities = CommandHostCapabilities::default();
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("warm matrix context");
            let mut warm_observer = CountingObserver::default();
            let mut processor = CommandProcessor::new(
                &mut command,
                &mut context,
                CommandHostContext::new(&mut capabilities),
                fuel.fuel_mut(),
                None,
                &mut effects,
            );
            if matches!(observer_mode, ObserverMode::Enabled) {
                processor = processor.with_observer(&mut warm_observer);
            }
            for _ in 0..options.warmups {
                black_box(deliver_one(&mut processor, delivery));
            }
        }

        let work_before = fuel.work();
        let snapshot_before = (hooks.snapshot)();
        let mut capabilities = CommandHostCapabilities::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("matrix command context");
        let mut observer = CountingObserver::default();
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut context,
            CommandHostContext::new(&mut capabilities),
            fuel.fuel_mut(),
            None,
            &mut effects,
        );
        if matches!(observer_mode, ObserverMode::Enabled) {
            processor = processor.with_observer(&mut observer);
        }
        let start = Instant::now();
        let mut checksum = 0_u64;
        let mut semantic_hash = SEMANTIC_SEED;
        for _ in 0..options.iterations {
            let evidence = black_box(deliver_one(&mut processor, delivery));
            checksum = checksum.wrapping_add(evidence.checksum);
            semantic_hash = semantic_hash.rotate_left(7) ^ evidence.semantic;
        }
        let elapsed_ns = start.elapsed().as_nanos();
        drop(processor);
        let work_after = fuel.work();
        let measured_observer_records = observer.records;
        drop(context);

        // The sentinel uses a fresh processor episode so its fuel and observer
        // work cannot enter the measured receipt. It proves the fixture ended
        // exactly after the requested warmups and iterations.
        let mut sentinel_capabilities = CommandHostCapabilities::default();
        let mut sentinel_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut sentinel_context = universe.command_context().expect("matrix sentinel context");
        let mut sentinel_processor = CommandProcessor::new(
            &mut command,
            &mut sentinel_context,
            CommandHostContext::new(&mut sentinel_capabilities),
            fuel.fuel_mut(),
            None,
            &mut sentinel_effects,
        );
        assert_end(&mut sentinel_processor, delivery);
        drop(sentinel_processor);
        assert_eq!(observer.records, measured_observer_records);

        let macro_expansions = (hooks.snapshot)()
            .macro_expansions
            .saturating_sub(snapshot_before.macro_expansions);
        let work = subtract_work(work_after, work_before);
        let receipt = Receipt {
            elapsed_ns,
            checksum,
            semantic_hash,
            observer_records: measured_observer_records,
            work,
            macro_expansions,
        };
        if hooks.name == "profiling" && matches!(delivery, Delivery::Expanded) {
            assert_eq!(
                receipt.macro_expansions,
                denominators
                    .macro_calls_per_operation
                    .saturating_mul(options.iterations) as u64,
                "profiling macro census disagrees with the known call denominator"
            );
        }
        validate_receipt(
            case,
            storage,
            delivery,
            observer_mode,
            options,
            workload,
            denominators,
            receipt,
        );
        receipt
    })
}

fn deliver_one<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    delivery: Delivery,
) -> DeliveryEvidence {
    let mut destination = None;
    let status = match delivery {
        Delivery::RawNext => processor.get_next_into(&mut destination),
        Delivery::RawToken => processor.get_token_into(&mut destination),
        Delivery::Expanded => processor.get_x_token_into(&mut destination),
    }
    .expect("command-core matrix delivery");
    if !matches!(status, DeliveryStatus::Command) {
        return DeliveryEvidence {
            checksum: MISSING_DELIVERY_CHECKSUM,
            semantic: MISSING_DELIVERY_SEMANTIC,
        };
    }
    let Some(command) = destination.take() else {
        return DeliveryEvidence {
            checksum: MISSING_DELIVERY_CHECKSUM,
            semantic: MISSING_DELIVERY_SEMANTIC,
        };
    };
    DeliveryEvidence {
        checksum: observed_checksum(&command),
        semantic: observed_semantic(&command),
    }
}

fn observed_checksum<G>(command: &tex_command::CurrentCommand<G>) -> u64 {
    match command.spelling().semantic_token() {
        Token::Char { ch, .. } => ch as u64,
        Token::Cs(_) if command.control_sequence().is_some() => {
            OutputToken::ControlSequence.checksum()
        }
        _ => MISSING_DELIVERY_CHECKSUM,
    }
}

fn observed_semantic<G>(command: &tex_command::CurrentCommand<G>) -> u64 {
    match command.spelling().semantic_token() {
        Token::Char { ch, cat } => 0x1000_0000 ^ (ch as u64) ^ u64::from(cat as u8),
        Token::Cs(_) => 0x2000_0000 ^ u64::from(command.control_sequence().is_some()),
        Token::Param(_) => 0x3000_0000,
        Token::Frozen(_) => 0x4000_0000,
    }
}

fn validate_fixture<G>(
    universe: &mut Universe<G>,
    storage: Storage,
    delivery: Delivery,
    workload: Workload,
    macro_symbol: Option<Symbol>,
) {
    let mut command = CommandState::default();
    let stored_words = build_stored_words(workload, macro_symbol, delivery, 1);
    match storage {
        Storage::Source => {
            let source = build_source_text(workload, delivery, 1);
            open_source(&mut command, &source);
        }
        Storage::Stored => {
            let token_list = universe
                .allocate_token_list(&stored_words)
                .expect("matrix validation input allocation");
            let context = universe
                .command_context()
                .expect("matrix validation context");
            command.push_everyjob(&context, token_list);
        }
    }
    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe
        .command_context()
        .expect("matrix validation command context");
    let mut fuel = CommandFuelLedger::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    let delivered = fetch_required(&mut processor, delivery);
    validate_semantics(&delivered, workload.output(delivery), macro_symbol);
    assert_end(&mut processor, delivery);
}

fn fetch_required<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    delivery: Delivery,
) -> tex_command::CurrentCommand<G> {
    let mut destination = None;
    let status = match delivery {
        Delivery::RawNext => processor.get_next_into(&mut destination),
        Delivery::RawToken => processor.get_token_into(&mut destination),
        Delivery::Expanded => processor.get_x_token_into(&mut destination),
    }
    .expect("matrix validation delivery");
    assert_eq!(
        status,
        DeliveryStatus::Command,
        "matrix validation ended early"
    );
    destination.expect("matrix validation command destination")
}

fn assert_end<G>(processor: &mut CommandProcessor<'_, '_, G>, delivery: Delivery) {
    let mut destination = None;
    let status = match delivery {
        Delivery::RawNext => processor.get_next_into(&mut destination),
        Delivery::RawToken => processor.get_token_into(&mut destination),
        Delivery::Expanded => processor.get_x_token_into(&mut destination),
    }
    .expect("matrix sentinel delivery");
    assert_eq!(
        status,
        DeliveryStatus::End,
        "matrix input has an unexpected suffix"
    );
    assert!(destination.is_none(), "matrix sentinel returned a command");
}

fn validate_semantics<G>(
    command: &tex_command::CurrentCommand<G>,
    output: OutputToken,
    macro_symbol: Option<Symbol>,
) {
    match output {
        OutputToken::Char(expected) => {
            assert_eq!(
                command.spelling().semantic_token(),
                Token::Char {
                    ch: expected,
                    cat: Catcode::Letter,
                }
            );
            assert_eq!(command.control_sequence(), None);
        }
        OutputToken::ControlSequence => {
            let symbol = macro_symbol.expect("control-sequence output has a macro symbol");
            assert_eq!(command.spelling().semantic_token(), Token::Cs(symbol));
            assert_eq!(command.control_sequence(), Some(symbol));
        }
    }
}
