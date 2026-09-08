use tex_command::{CharacterRunAdmission, MainCharacterConsumer, MainCharacterInput};
use tex_state::token::Catcode;

#[path = "command_consumer_chain_mixed.rs"]
mod chain_mixed;
#[path = "command_consumer_fixtures.rs"]
mod fixtures;
#[path = "command_consumer_receipts.rs"]
mod receipts;
#[path = "command_consumer_support.rs"]
mod support;
#[path = "command_consumer_text_definition.rs"]
mod text_definition;

use fixtures::{
    DEFAULT_BODY_WORDS, DEFAULT_TEXT_CHARS, DEFAULT_WARMUPS, Denominators, Evidence, Storage,
    WorkloadKind, install_benchmark_catcodes, install_chain, install_definition_symbol,
    with_universe,
};
use receipts::print_record;
#[derive(Clone, Copy, Debug, Default)]
pub struct StructuralSnapshot {
    pub macro_expansions: u64,
    pub definition_direct_stores: u64,
    pub definition_chunk_transitions: u64,
    pub definition_episode_admissions: u64,
}

#[derive(Clone, Copy)]
pub struct ProfileHooks {
    pub name: &'static str,
    pub instrumented: bool,
    pub snapshot: fn() -> StructuralSnapshot,
}

impl ProfileHooks {
    #[allow(dead_code)]
    pub const fn release() -> Self {
        Self {
            name: "release",
            instrumented: false,
            snapshot: || StructuralSnapshot {
                macro_expansions: 0,
                definition_direct_stores: 0,
                definition_chunk_transitions: 0,
                definition_episode_admissions: 0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct StructuralDelta {
    macro_expansions: Option<u64>,
    definition_direct_stores: Option<u64>,
    definition_chunk_transitions: Option<u64>,
    definition_episode_admissions: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Receipt {
    elapsed_ns: u128,
    semantic_hash: u64,
    text: Option<Evidence>,
    macro_output: Option<Evidence>,
    definition_count: u64,
    definition_body_words: usize,
    definition_body_checksum: Option<u64>,
    denominators: Denominators,
    work: tex_command::CommandWorkCounters,
    structural: StructuralDelta,
}

#[derive(Clone, Copy, Debug)]
struct Options {
    workload: Option<WorkloadKind>,
    storage: StorageSelection,
    iterations: Option<usize>,
    warmups: usize,
    text_chars: usize,
    body_words: usize,
}

#[derive(Clone, Copy, Debug, Default)]
enum StorageSelection {
    #[default]
    Both,
    Source,
    Stored,
}

impl StorageSelection {
    fn values(self) -> &'static [Storage] {
        match self {
            Self::Both => &Storage::ALL,
            Self::Source => &[Storage::Source],
            Self::Stored => &[Storage::Stored],
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            workload: None,
            storage: StorageSelection::Both,
            iterations: None,
            warmups: DEFAULT_WARMUPS,
            text_chars: DEFAULT_TEXT_CHARS,
            body_words: DEFAULT_BODY_WORDS,
        }
    }
}

pub fn run(hooks: ProfileHooks) {
    let options = parse_options();
    println!(
        "{{\"schema\":\"consumer-core-v1\",\"profile\":\"{}\",\"warmups\":{},\"text_chars\":{},\"body_words\":{},\"timed_evidence\":\"checksum_sink\"}}",
        hooks.name, options.warmups, options.text_chars, options.body_words,
    );
    let workloads = options
        .workload
        .map_or_else(|| WorkloadKind::ALL.to_vec(), |kind| vec![kind]);
    for kind in workloads {
        let iterations = options.iterations.unwrap_or(kind.default_iterations());
        for &storage in options.storage.values() {
            let receipt = run_workload(kind, storage, iterations, options, hooks);
            print_record(kind, storage, iterations, options.warmups, hooks, receipt);
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
            "--workload" => {
                let value = value_or_next(inline_value, &mut args, "--workload");
                options.workload = if value == "all" {
                    None
                } else {
                    Some(WorkloadKind::parse(&value).unwrap_or_else(|| usage("unknown workload")))
                };
            }
            "--storage" => {
                let value = value_or_next(inline_value, &mut args, "--storage");
                options.storage = match value.as_str() {
                    "source" => StorageSelection::Source,
                    "stored" => StorageSelection::Stored,
                    "both" => StorageSelection::Both,
                    _ => usage("--storage requires source, stored, or both"),
                };
            }
            "--iterations" => {
                options.iterations = Some(parse_positive(
                    value_or_next(inline_value, &mut args, "--iterations"),
                    "--iterations",
                ));
            }
            "--warmups" => {
                options.warmups = parse_nonnegative(
                    value_or_next(inline_value, &mut args, "--warmups"),
                    "--warmups",
                );
            }
            "--text-chars" => {
                options.text_chars = parse_positive(
                    value_or_next(inline_value, &mut args, "--text-chars"),
                    "--text-chars",
                );
            }
            "--body-words" => {
                options.body_words = parse_positive(
                    value_or_next(inline_value, &mut args, "--body-words"),
                    "--body-words",
                );
            }
            "--help" => usage(""),
            _ => usage("unknown argument"),
        }
    }
    options
}

fn value_or_next(
    inline: Option<&str>,
    args: &mut impl Iterator<Item = String>,
    option: &str,
) -> String {
    inline
        .map(str::to_owned)
        .or_else(|| args.next())
        .unwrap_or_else(|| usage(&format!("{option} requires a value")))
}

fn parse_positive(value: String, option: &str) -> usize {
    value
        .parse::<usize>()
        .ok()
        .filter(|count| *count > 0)
        .unwrap_or_else(|| usage(&format!("{option} must be a positive integer")))
}

fn parse_nonnegative(value: String, option: &str) -> usize {
    value
        .parse::<usize>()
        .unwrap_or_else(|_| usage(&format!("{option} must be a nonnegative integer")))
}

fn usage(error: &str) -> ! {
    if !error.is_empty() {
        eprintln!("error: {error}");
    }
    eprintln!(
        "usage: command-consumer-core [--workload NAME|all] [--storage source|stored|both] [--iterations N] [--warmups N] [--text-chars N] [--body-words N]"
    );
    std::process::exit(2);
}

fn run_workload(
    kind: WorkloadKind,
    storage: Storage,
    iterations: usize,
    options: Options,
    hooks: ProfileHooks,
) -> Receipt {
    with_universe(|universe| {
        install_benchmark_catcodes(universe);
        let definition = install_definition_symbol(universe);
        let chain = match kind {
            WorkloadKind::ParameterizedChain | WorkloadKind::MixedPipeline => {
                Some(install_chain(universe))
            }
            WorkloadKind::LongText | WorkloadKind::DefinitionBody => None,
        };
        match kind {
            WorkloadKind::LongText => text_definition::run_long_text(
                universe,
                storage,
                iterations,
                options.warmups,
                options.text_chars,
                hooks,
            ),
            WorkloadKind::DefinitionBody => text_definition::run_definition_body(
                universe,
                storage,
                iterations,
                options.warmups,
                options.body_words,
                definition,
                hooks,
            ),
            WorkloadKind::ParameterizedChain => chain_mixed::run_chain(
                universe,
                storage,
                iterations,
                options.warmups,
                chain.expect("chain fixture"),
                options.text_chars,
                options.body_words,
                hooks,
            ),
            WorkloadKind::MixedPipeline => chain_mixed::run_mixed(
                universe,
                storage,
                iterations,
                options.warmups,
                chain.expect("chain fixture"),
                definition,
                options.text_chars,
                options.body_words,
                hooks,
            ),
        }
    })
}

struct TextConsumer {
    chunk_size: usize,
    remaining: usize,
    evidence: Evidence,
}

impl TextConsumer {
    fn new(chunk_size: usize) -> Self {
        Self {
            chunk_size,
            remaining: chunk_size,
            evidence: Evidence::seeded(),
        }
    }

    fn reset_operation(&mut self) {
        self.remaining = self.chunk_size;
    }

    fn reset_evidence(&mut self) {
        self.remaining = self.chunk_size;
        self.evidence = Evidence::seeded();
    }

    fn absorb(&mut self, ch: char) {
        self.evidence.absorb(ch);
        self.remaining = self.remaining.saturating_sub(1);
    }
}

impl<G> MainCharacterConsumer<G> for TextConsumer {
    #[inline(always)]
    fn admit<'state, 'admission, 'fuel, 'effects, 'run>(
        &mut self,
        state: &'state mut tex_state::CommandContext<'admission, G>,
        _fuel: &'fuel mut tex_command::CommandFuel,
        _diagnostic_effects: &'effects mut tex_state::diagnostic::DiagnosticEffects,
        input: MainCharacterInput<'run>,
    ) -> CharacterRunAdmission {
        match input {
            MainCharacterInput::Borrowed(run) => {
                if self.remaining == 0 {
                    return CharacterRunAdmission::tokenizer_fallback();
                }
                let take = run.bytes().len().min(self.remaining);
                for &byte in &run.bytes()[..take] {
                    let ch = char::from(byte);
                    if !matches!(state.catcode(ch), Catcode::Letter | Catcode::Other) {
                        return CharacterRunAdmission::new(0, false);
                    }
                    self.absorb(ch);
                }
                CharacterRunAdmission::new(
                    u32::try_from(take).expect("text fixture run fits u32"),
                    take == run.bytes().len() && self.remaining != 0,
                )
            }
            MainCharacterInput::Scalar { ch, .. } => {
                if !matches!(state.catcode(ch), Catcode::Letter | Catcode::Other) {
                    return CharacterRunAdmission::new(0, false);
                }
                self.absorb(ch);
                CharacterRunAdmission::new(1, self.remaining != 0)
            }
        }
    }
}
