use std::alloc::System;
use std::hint::black_box;
use std::sync::Arc;

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};
use tex_command::{
    AlignmentIdentity, CommandHostCapabilities, CommandHostContext, CommandObservation,
    CommandObserver, CommandProcessor, CommandRuntime, CommandState, PrintCommand,
    RegisteredSourceKind, SourceRegistration, append_print_cmd_chr_text,
};
use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{
    ExpandablePrimitive, Meaning, MeaningFlags, UnexpandablePrimitive,
};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

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
    InlineControlSequenceTokenization,
    SpilledControlSequenceTokenization,
}

impl Workload {
    const ALL: [Self; 13] = [
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

struct ProcessorCase {
    universe: Universe,
    command: CommandState,
    runtime: CommandRuntime,
    capabilities: CommandHostCapabilities,
}

enum Case {
    Processor(Box<ProcessorCase>),
    Rendering(Box<RenderingCase>),
}

struct RenderingCase {
    universe: Universe,
    command: PrintCommand,
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
    let mut warm = build_case(workload);
    run_case(workload, configuration, &mut warm, false);
    drop(warm);

    let mut cases = (0..OPERATIONS)
        .map(|_| build_case(workload))
        .collect::<Vec<_>>();
    let region = Region::new(GLOBAL);
    for case in &mut cases {
        run_case(workload, configuration, case, perturb);
    }
    region.change()
}

fn print_stats(workload: Workload, configuration: Configuration, stats: Stats) {
    assert_eq!(
        stats.allocations % OPERATIONS,
        0,
        "allocation count must be stable per operation"
    );
    assert_eq!(
        stats.bytes_allocated % OPERATIONS,
        0,
        "requested bytes must be stable per operation"
    );
    println!(
        "{} configuration={} allocations_per_op={} requested_bytes_per_op={}",
        workload.name(),
        configuration.name(),
        stats.allocations / OPERATIONS,
        stats.bytes_allocated / OPERATIONS,
    );
}

fn run_case(workload: Workload, configuration: Configuration, case: &mut Case, perturb: bool) {
    if perturb {
        black_box(vec![0_u8; PERTURBATION_BYTES]);
    }
    match case {
        Case::Rendering(case) => {
            case.text.clear();
            append_print_cmd_chr_text(
                &case.universe.command_context(),
                case.command,
                &mut case.text,
            );
            black_box(&case.text);
        }
        Case::Processor(case) => {
            let mut observer = CountingObserver::default();
            let processor = CommandProcessor::new(
                &mut case.command,
                &mut case.runtime,
                case.universe.command_context(),
                CommandHostContext::new(&mut case.capabilities),
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
                    let scanned = processor.scan_keyword("dimension").expect("keyword scans");
                    assert!(scanned.value);
                    black_box(scanned);
                }
                Workload::DimensionScanning => {
                    black_box(processor.scan_dimension().expect("dimension scans"));
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
                Workload::ShiftCase => {
                    processor.shift_case(true).expect("case shift completes");
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
            black_box(observer.0);
        }
    }
}

fn build_case(workload: Workload) -> Case {
    if matches!(workload, Workload::CommandTextRendering) {
        return rendering_case();
    }

    let mut universe = Universe::new_with_plain_catcodes();
    let source = match workload {
        Workload::SingleTokenBackup => "x",
        Workload::MacroArgumentMatching => r"\m{abcdefghijklmnop}",
        Workload::ScanToksAbsorption => "{abcdefghijklmnop}",
        Workload::KeywordScanning => "dimension ",
        Workload::DimensionScanning => "123.5pt ",
        Workload::AlignmentPreambleScanning => r"{#&#\cr",
        Workload::TwoTokenOffSaveRecovery => "x",
        Workload::RenderedTokenInstallation => r"\number12345 ",
        Workload::TokenListIteration => "",
        Workload::ShiftCase => "{abcdefghijklmnop}",
        Workload::InlineControlSequenceTokenization => r"\allocationbaseline ",
        Workload::SpilledControlSequenceTokenization => {
            r"\pathologicalcontrolsequencenameoverinlinebound "
        }
        Workload::CommandTextRendering => unreachable!(),
    };

    let mut command = CommandState::default();
    let runtime = CommandRuntime::default();
    let capabilities = CommandHostCapabilities::default();

    if matches!(workload, Workload::MacroArgumentMatching) {
        install_macro(&mut universe);
    }
    if matches!(workload, Workload::AlignmentPreambleScanning) {
        let cr = universe.intern("cr").symbol();
        universe.set_meaning(
            cr,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
        );
        command.begin_alignment(AlignmentIdentity::new(1));
    }
    if matches!(workload, Workload::RenderedTokenInstallation) {
        let number = universe.intern("number").symbol();
        universe.set_meaning(
            number,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );
    }
    if matches!(
        workload,
        Workload::InlineControlSequenceTokenization | Workload::SpilledControlSequenceTokenization
    ) {
        universe.intern(match workload {
            Workload::InlineControlSequenceTokenization => INLINE_CONTROL_SEQUENCE,
            Workload::SpilledControlSequenceTokenization => SPILLED_CONTROL_SEQUENCE,
            _ => unreachable!(),
        });
    }

    if matches!(workload, Workload::TokenListIteration) {
        let tokens = (0..16)
            .map(|index| Token::Char {
                ch: char::from(b'a' + index),
                cat: Catcode::Letter,
            })
            .collect::<Vec<_>>();
        let traced = tokens
            .into_iter()
            .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN))
            .collect::<Vec<_>>();
        command.push_everyjob(universe.finish_traced_token_list(&traced));
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

    Case::Processor(Box::new(ProcessorCase {
        universe,
        command,
        runtime,
        capabilities,
    }))
}

fn install_macro(universe: &mut Universe) {
    let name = universe.intern("m").symbol();
    let parameters = universe.intern_token_list(&[Token::param(1)]);
    let replacement = universe.intern_token_list(&[Token::param(1)]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        parameters,
        replacement,
    ));
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
}

fn rendering_case() -> Case {
    let mut universe = Universe::new_with_plain_catcodes();
    let symbol = universe.intern("allocationbaseline").symbol();
    universe.set_meaning(symbol, Meaning::Relax);
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
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    let current = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .get_next()
    .expect("rendering command delivers")
    .expect("rendering command is present");
    Case::Rendering(Box::new(RenderingCase {
        universe,
        command: PrintCommand::from_current(&current),
        text: String::with_capacity(32),
    }))
}
