use std::alloc::System;
use std::hint::black_box;
use std::sync::Arc;

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};
use tex_command::{
    AlignmentIdentity, CommandFuelLedger, CommandHostCapabilities, CommandHostContext,
    CommandObservation, CommandObserver, CommandProcessor, CommandState, PrintCommand,
    RegisteredSourceKind, SourceRegistration, append_print_cmd_chr_text,
    install_tex82_expandable_primitives, install_tex82_unexpandable_primitives,
};
use tex_state::env::AssignmentScope;
use tex_state::interner::InternerBudget;
use tex_state::meaning::{Meaning, MeaningFlags, MeaningWord, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token, TokenWord};
use tex_state::{TokenListId, Universe};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const OPERATIONS: usize = 64;
const PERTURBATION_BYTES: usize = 64;
const INLINE_CONTROL_SEQUENCE: &str = "allocationbaseline";
const SPILLED_CONTROL_SEQUENCE: &str = "pathologicalcontrolsequencenameoverinlinebound";

#[derive(Clone, Copy)]
enum Configuration {
    Unobserved,
    ExternalObserver,
}

impl Configuration {
    const ALL: [Self; 2] = [Self::Unobserved, Self::ExternalObserver];

    const fn name(self) -> &'static str {
        match self {
            Self::Unobserved => "unobserved",
            Self::ExternalObserver => "external_observer",
        }
    }
}

#[derive(Clone, Copy)]
enum Workload {
    SingleTokenBackup,
    MacroArgumentMatching,
    ScanToksAbsorption,
    KeywordScanning,
    DimensionScanning,
    AlignmentPreambleScanning,
    TwoTokenOffSaveRecovery,
    RenderedTokenInstallation,
    CommandTextRendering,
    TokenListIteration,
    ShiftCase,
    MacroDefinition,
    ReadTokenCollection,
    OutputReplayExpansion,
    InlineControlSequenceTokenization,
    SpilledControlSequenceTokenization,
}

impl Workload {
    const ALL: [Self; 16] = [
        Self::SingleTokenBackup,
        Self::MacroArgumentMatching,
        Self::ScanToksAbsorption,
        Self::KeywordScanning,
        Self::DimensionScanning,
        Self::AlignmentPreambleScanning,
        Self::TwoTokenOffSaveRecovery,
        Self::RenderedTokenInstallation,
        Self::CommandTextRendering,
        Self::TokenListIteration,
        Self::ShiftCase,
        Self::MacroDefinition,
        Self::ReadTokenCollection,
        Self::OutputReplayExpansion,
        Self::InlineControlSequenceTokenization,
        Self::SpilledControlSequenceTokenization,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::SingleTokenBackup => "single_token_backup",
            Self::MacroArgumentMatching => "macro_argument_matching",
            Self::ScanToksAbsorption => "scan_toks_absorption",
            Self::KeywordScanning => "keyword_scanning",
            Self::DimensionScanning => "dimension_scanning",
            Self::AlignmentPreambleScanning => "alignment_preamble_scanning",
            Self::TwoTokenOffSaveRecovery => "two_token_off_save_recovery",
            Self::RenderedTokenInstallation => "rendered_token_installation",
            Self::CommandTextRendering => "command_text_rendering",
            Self::TokenListIteration => "token_list_iteration",
            Self::ShiftCase => "shift_case",
            Self::MacroDefinition => "macro_definition",
            Self::ReadTokenCollection => "read_token_collection",
            Self::OutputReplayExpansion => "output_replay_expansion",
            Self::InlineControlSequenceTokenization => "inline_control_sequence_tokenization",
            Self::SpilledControlSequenceTokenization => "spilled_control_sequence_tokenization",
        }
    }

    const fn supports_configuration(self, configuration: Configuration) -> bool {
        !matches!(self, Self::CommandTextRendering)
            || matches!(configuration, Configuration::Unobserved)
    }
}

#[derive(Default)]
struct CountingObserver(usize);

impl CommandObserver for CountingObserver {
    fn committed(&mut self, observation: CommandObservation) {
        black_box(observation);
        self.0 += 1;
    }
}

struct ProcessorCase<G> {
    command: CommandState<G>,
    operation: tex_command::CommandAttemptOperation,
    capabilities: CommandHostCapabilities,
    diagnostic_effects: tex_state::diagnostic::DiagnosticEffects,
    replay: Option<TokenListId<G>>,
}

struct RenderingCase<G> {
    command: PrintCommand<G>,
    text: String,
}

fn main() {
    let perturb = std::env::args().any(|argument| argument == "--perturb");
    println!(
        "command allocation baseline: operations={OPERATIONS} perturbation_bytes={}",
        if perturb { PERTURBATION_BYTES } else { 0 }
    );
    for workload in Workload::ALL {
        for configuration in Configuration::ALL {
            if workload.supports_configuration(configuration) {
                let stats = measure(workload, configuration, perturb);
                print_stats(workload, configuration, stats);
            }
        }
    }
}

fn measure(workload: Workload, configuration: Configuration, perturb: bool) -> Stats {
    with_universe(|universe| {
        install_tex82_expandable_primitives(universe);
        install_tex82_unexpandable_primitives(universe);
        for _ in 0..3 {
            run_one(universe, workload, configuration, false);
        }
        let mut measured = None;
        for _ in 0..OPERATIONS {
            let stats = run_one(universe, workload, configuration, perturb);
            measured = Some((stats.allocations, stats.bytes_allocated));
        }
        let (allocations, bytes_allocated) = measured.expect("at least one measured operation");
        Stats {
            allocations,
            bytes_allocated,
            ..Stats::default()
        }
    })
}

fn run_one<G>(
    universe: &mut Universe<G>,
    workload: Workload,
    configuration: Configuration,
    perturb: bool,
) -> Stats {
    if matches!(workload, Workload::CommandTextRendering) {
        let mut case = rendering_case(universe);
        let region = Region::new(GLOBAL);
        perturb_if_requested(perturb);
        case.text.clear();
        let context = universe.command_context().expect("command context");
        append_print_cmd_chr_text(&context, case.command, &mut case.text);
        black_box(&case.text);
        return region.change();
    }

    let mut case = processor_case(universe, workload);
    let mut fuel = CommandFuelLedger::default();
    let region = Region::new(GLOBAL);
    perturb_if_requested(perturb);
    let mut observer = CountingObserver::default();
    let mut context = universe.command_context().expect("command context");
    let processor = CommandProcessor::new(
        &mut case.command,
        &mut context,
        CommandHostContext::new(&mut case.capabilities),
        fuel.fuel_mut(),
        None,
        &mut case.diagnostic_effects,
    );
    let mut processor = match configuration {
        Configuration::ExternalObserver => processor.with_observer(&mut observer),
        Configuration::Unobserved => processor,
    };
    match workload {
        Workload::SingleTokenBackup => {
            let command = processor
                .get_next()
                .expect("backup token delivers")
                .expect("backup token is present");
            processor
                .back_input(command)
                .expect("single token backs up");
        }
        Workload::MacroArgumentMatching => {
            black_box(
                processor
                    .get_x_token()
                    .expect("macro arguments match")
                    .expect("macro replacement is present"),
            );
        }
        Workload::ScanToksAbsorption => {
            black_box(
                processor
                    .scan_balanced_text(false)
                    .expect("scan_toks succeeds"),
            );
        }
        Workload::KeywordScanning => {
            let tex_command::RetainedScalarScan::Complete(scanned) =
                processor.scan_keyword_retained("dimension")
            else {
                panic!("preloaded keyword scan must complete synchronously")
            };
            assert!(scanned.value);
            black_box(scanned);
        }
        Workload::DimensionScanning => {
            let tex_command::RetainedScalarScan::Complete(scanned) =
                processor.scan_dimension_retained()
            else {
                panic!("preloaded dimension scan must complete synchronously")
            };
            black_box(scanned);
        }
        Workload::AlignmentPreambleScanning => {
            black_box(
                processor
                    .scan_alignment_preamble_opening()
                    .expect("alignment opener scans"),
            );
            processor
                .begin_alignment_preamble_scan(None)
                .expect("alignment preamble scans");
        }
        Workload::TwoTokenOffSaveRecovery => {
            let command = processor
                .get_next()
                .expect("off-save command delivers")
                .expect("off-save command is present");
            processor
                .recover_off_save(
                    command,
                    &[
                        Token::Char {
                            ch: 'R',
                            cat: Catcode::Other,
                        },
                        Token::Char {
                            ch: '.',
                            cat: Catcode::Other,
                        },
                    ],
                )
                .expect("two-token off-save recovery installs");
        }
        Workload::RenderedTokenInstallation => {
            black_box(
                processor
                    .get_x_token()
                    .expect("rendered expansion succeeds")
                    .expect("rendered expansion produces a token"),
            );
        }
        Workload::TokenListIteration => {
            while let Some(command) = processor.get_token().expect("token list iterates") {
                black_box(command);
            }
        }
        Workload::ShiftCase => processor.shift_case(true).expect("case shift completes"),
        Workload::MacroDefinition => {
            black_box(
                processor
                    .scan_macro_definition(false, false)
                    .expect("macro definition scans"),
            );
        }
        Workload::ReadTokenCollection => {
            black_box(
                processor
                    .scan_input_stream_request(UnexpandablePrimitive::Read, false)
                    .expect("read token collection succeeds"),
            );
        }
        Workload::OutputReplayExpansion => {
            black_box(
                processor
                    .expand_output_replay(case.replay.expect("replay fixture is present"))
                    .expect("output replay expands"),
            );
        }
        Workload::InlineControlSequenceTokenization
        | Workload::SpilledControlSequenceTokenization => {
            black_box(
                processor
                    .get_token()
                    .expect("control sequence tokenizes")
                    .expect("control sequence is present"),
            );
        }
        Workload::CommandTextRendering => unreachable!("rendering has its own case"),
    }
    drop(processor);
    black_box(observer.0);
    case.command
        .commit_attempt_operation(case.operation)
        .expect("benchmark operation commits");
    region.change()
}

fn perturb_if_requested(perturb: bool) {
    if perturb {
        black_box(vec![0_u8; PERTURBATION_BYTES]);
    }
}

fn print_stats(workload: Workload, configuration: Configuration, stats: Stats) {
    println!(
        "{} configuration={} scratch_pool=warm allocations_per_op={} requested_bytes_per_op={}",
        workload.name(),
        configuration.name(),
        stats.allocations,
        stats.bytes_allocated,
    );
}

fn with_universe<R>(
    benchmark: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>) -> R,
) -> R {
    let budget = InternerBudget::new(65_536, 65_536, 8 << 20).expect("benchmark interner budget");
    tex_state::with_universe(budget, benchmark).expect("benchmark universe")
}

fn processor_case<G>(universe: &mut Universe<G>, workload: Workload) -> ProcessorCase<G> {
    install_benchmark_catcodes(universe);
    let source = match workload {
        Workload::SingleTokenBackup => "x",
        Workload::MacroArgumentMatching => {
            r"\m{abcdefghijklmnop}\m{abcdefghijklmnop}\m{abcdefghijklmnop}\m{abcdefghijklmnop}\m{abcdefghijklmnop}\m{abcdefghijklmnop}"
        }
        Workload::ScanToksAbsorption => "{abcdefghijklmnop}",
        Workload::KeywordScanning => "dimension ",
        Workload::DimensionScanning => "123.5pt ",
        Workload::AlignmentPreambleScanning => r"{#&#\cr",
        Workload::TwoTokenOffSaveRecovery => "x",
        Workload::RenderedTokenInstallation => r"\number12345 ",
        Workload::TokenListIteration => "",
        Workload::ShiftCase => "{abcdefghijklmnop}",
        Workload::MacroDefinition => r"\m#1{abcdefghijklmnop}",
        Workload::ReadTokenCollection => r"99 to \line",
        Workload::OutputReplayExpansion => "",
        Workload::InlineControlSequenceTokenization => r"\allocationbaseline ",
        Workload::SpilledControlSequenceTokenization => {
            r"\pathologicalcontrolsequencenameoverinlinebound "
        }
        Workload::CommandTextRendering => unreachable!(),
    };
    let mut command = CommandState::default();
    if matches!(workload, Workload::MacroArgumentMatching) {
        install_macro(universe);
    }
    if matches!(workload, Workload::AlignmentPreambleScanning) {
        command.begin_alignment(AlignmentIdentity::new(1));
    }
    if matches!(workload, Workload::ReadTokenCollection) {
        universe.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
        universe
            .world_mut()
            .push_memory_terminal_line("abcdefghijklmnop")
            .expect("terminal line registers");
    }
    if matches!(
        workload,
        Workload::InlineControlSequenceTokenization | Workload::SpilledControlSequenceTokenization
    ) {
        universe
            .intern(match workload {
                Workload::InlineControlSequenceTokenization => INLINE_CONTROL_SEQUENCE,
                Workload::SpilledControlSequenceTokenization => SPILLED_CONTROL_SEQUENCE,
                _ => unreachable!(),
            })
            .expect("control sequence is interned");
    }

    let replay = matches!(workload, Workload::OutputReplayExpansion).then(|| {
        let words = "abcdefghijklmnop"
            .chars()
            .map(|ch| {
                TokenWord::pack(Token::Char {
                    ch,
                    cat: Catcode::Letter,
                })
            })
            .collect::<Vec<_>>();
        universe.allocate_token_list(&words).expect("replay tokens")
    });

    if matches!(workload, Workload::TokenListIteration) {
        let words = (0..16)
            .map(|index| {
                TokenWord::pack(Token::Char {
                    ch: char::from(b'a' + index),
                    cat: Catcode::Letter,
                })
            })
            .collect::<Vec<_>>();
        let tokens = universe.allocate_token_list(&words).expect("stored tokens");
        let context = universe.command_context().expect("command context");
        command.push_everyjob(&context, tokens);
    } else {
        let registered = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source.as_bytes()),
            ))
            .expect("benchmark source registers");
        command
            .open_registered_source(registered)
            .expect("benchmark source opens");
    }

    let operation = command.begin_attempt_operation();
    let mut case = ProcessorCase {
        command,
        operation,
        capabilities: CommandHostCapabilities::default(),
        diagnostic_effects: tex_state::diagnostic::DiagnosticEffects::new(),
        replay,
    };
    if matches!(workload, Workload::MacroArgumentMatching) {
        let mut fuel = CommandFuelLedger::default();
        let mut context = universe.command_context().expect("command context");
        let mut processor = CommandProcessor::new(
            &mut case.command,
            &mut context,
            CommandHostContext::new(&mut case.capabilities),
            fuel.fuel_mut(),
            None,
            &mut case.diagnostic_effects,
        );
        for _ in 0..48 {
            black_box(
                processor
                    .get_x_token()
                    .expect("macro warmup succeeds")
                    .expect("macro warmup token is present"),
            );
        }
        for _ in 0..2 {
            let replay_warmup = processor
                .get_next()
                .expect("macro replay warmup delivers")
                .expect("macro replay warmup is present");
            processor
                .back_input(replay_warmup)
                .expect("macro replay warmup backs up");
            for _ in 0..16 {
                black_box(
                    processor
                        .get_x_token()
                        .expect("macro replay warmup succeeds")
                        .expect("macro replay warmup token is present"),
                );
            }
        }
        let pending = processor
            .get_next()
            .expect("warmed macro call delivers")
            .expect("warmed macro call is present");
        processor
            .back_input(pending)
            .expect("warmed macro call backs up");
    }
    case
}

fn install_benchmark_catcodes<G>(universe: &mut Universe<G>) {
    let mut context = universe.command_context().expect("command context");
    for (character, catcode) in [
        ('{', Catcode::BeginGroup),
        ('}', Catcode::EndGroup),
        ('&', Catcode::AlignmentTab),
        ('#', Catcode::Parameter),
    ] {
        context
            .assign_code(
                tex_state::CodeTableKind::Catcode,
                character,
                i64::from(catcode as u8),
                AssignmentScope::Global,
            )
            .expect("command benchmark category code");
    }
}

fn install_macro<G>(universe: &mut Universe<G>) {
    let name = universe.intern("m").expect("macro name");
    let definition = universe
        .allocate_definition(
            &[TokenWord::pack(Token::param(1))],
            &[TokenWord::pack(Token::param(1))],
        )
        .expect("macro definition");
    universe
        .assign_meaning(
            name,
            MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
            AssignmentScope::Global,
        )
        .expect("macro meaning");
}

fn rendering_case<G>(universe: &mut Universe<G>) -> RenderingCase<G> {
    let symbol = universe
        .intern("allocationbaseline")
        .expect("render symbol");
    universe
        .assign_meaning(
            symbol,
            MeaningWord::from_static(Meaning::Relax),
            AssignmentScope::Global,
        )
        .expect("render meaning");
    let mut command = CommandState::default();
    let registered = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(r"\allocationbaseline ".as_bytes()),
        ))
        .expect("rendering source registers");
    command
        .open_registered_source(registered)
        .expect("rendering source opens");
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = CommandFuelLedger::default();
    let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("command context");
    let current = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut diagnostic_effects,
    )
    .get_next()
    .expect("rendering command delivers")
    .expect("rendering command is present");
    RenderingCase {
        command: PrintCommand::from_current(&current),
        text: String::with_capacity(32),
    }
}
