use std::env;
use std::path::PathBuf;

use proptest::prelude::*;
use proptest::test_runner::Config;
use tex_expand::ExpansionHooks;
use tex_lex::{InputStack, MemoryInput};
use tex_state::{Universe, World};

const REPLAY_SHARDS: u32 = 8;
const OUTPUT_PATHS: [&str; 3] = ["out0.aux", "out1.aux", "out2.aux"];

#[derive(Clone, Debug)]
struct Program {
    steps: Vec<Step>,
}

#[derive(Clone, Debug)]
enum Step {
    Tex(TexStep),
    RngTick { register: u16 },
}

#[derive(Clone, Debug)]
enum TexStep {
    CountAssign {
        register: u16,
        value: i32,
    },
    OpenOut {
        slot: u8,
    },
    CloseOut {
        slot: u8,
    },
    Message {
        seed: u8,
        register: u16,
    },
    Write {
        slot: u8,
        seed: u8,
        register: u16,
    },
    Input {
        file: InputFile,
    },
    OpenInRead {
        slot: u8,
        file: ReadFile,
        target: ReadTarget,
    },
    TerminalRead {
        target: ReadTarget,
    },
}

#[derive(Clone, Copy, Debug)]
enum InputFile {
    IncA,
    IncB,
}

#[derive(Clone, Copy, Debug)]
enum ReadFile {
    ReadA,
    ReadB,
}

#[derive(Clone, Copy, Debug)]
enum ReadTarget {
    RA,
    RB,
    RC,
}

#[derive(Debug, Eq, PartialEq)]
struct CommittedOutputs {
    terminal: Vec<u8>,
    log: Vec<u8>,
    streams: Vec<Option<Vec<u8>>>,
}

macro_rules! replay_identity_shard {
    ($name:ident, $shard:expr) => {
        proptest! {
            #![proptest_config(Config {
                cases: prop_cases_for_shard($shard),
                failure_persistence: None,
                ..Config::default()
            })]

            #[test]
            fn $name(program in program_strategy()) {
                assert_effectful_replay_identity(&program);
            }
        }
    };
}

macro_rules! commit_path_shard {
    ($name:ident, $shard:expr) => {
        proptest! {
            #![proptest_config(Config {
                cases: prop_cases_for_shard($shard),
                failure_persistence: None,
                ..Config::default()
            })]

            #[test]
            fn $name((program, mask) in (program_strategy(), prop::collection::vec(any::<bool>(), 0..18))) {
                assert_commit_path_matches_straight_line(&program, &mask);
            }
        }
    };
}

replay_identity_shard!(effectful_replay_identity_0, 0);
replay_identity_shard!(effectful_replay_identity_1, 1);
replay_identity_shard!(effectful_replay_identity_2, 2);
replay_identity_shard!(effectful_replay_identity_3, 3);
replay_identity_shard!(effectful_replay_identity_4, 4);
replay_identity_shard!(effectful_replay_identity_5, 5);
replay_identity_shard!(effectful_replay_identity_6, 6);
replay_identity_shard!(effectful_replay_identity_7, 7);

commit_path_shard!(effectful_commit_path_0, 0);
commit_path_shard!(effectful_commit_path_1, 1);
commit_path_shard!(effectful_commit_path_2, 2);
commit_path_shard!(effectful_commit_path_3, 3);
commit_path_shard!(effectful_commit_path_4, 4);
commit_path_shard!(effectful_commit_path_5, 5);
commit_path_shard!(effectful_commit_path_6, 6);
commit_path_shard!(effectful_commit_path_7, 7);

#[test]
fn fixed_effectful_program_does_not_leak_before_commit() {
    let program = Program {
        steps: vec![
            Step::Tex(TexStep::OpenOut { slot: 0 }),
            Step::Tex(TexStep::Message {
                seed: 1,
                register: 0,
            }),
            Step::Tex(TexStep::Write {
                slot: 0,
                seed: 2,
                register: 0,
            }),
            Step::Tex(TexStep::Input {
                file: InputFile::IncA,
            }),
            Step::RngTick { register: 41 },
            Step::Tex(TexStep::OpenInRead {
                slot: 1,
                file: ReadFile::ReadA,
                target: ReadTarget::RA,
            }),
            Step::Tex(TexStep::CloseOut { slot: 0 }),
        ],
    };

    assert_effectful_replay_identity(&program);
    assert_commit_path_matches_straight_line(&program, &[true, false, true]);
}

fn assert_effectful_replay_identity(program: &Program) {
    let mut universe = setup_universe();
    let before = universe.testing_state_hash();
    let checkpoint = universe.snapshot();

    run_steps(&mut universe, &program.steps);

    assert_no_committed_outputs(&universe, program);
    universe.rollback(&checkpoint);
    assert_eq!(
        universe.testing_state_hash(),
        before,
        "effectful rollback hash diverged after program:\n{}",
        program.render()
    );
    assert_no_committed_outputs(&universe, program);
}

fn assert_commit_path_matches_straight_line(program: &Program, mask: &[bool]) {
    let mut universe = setup_universe();
    for (index, step) in program.steps.iter().enumerate() {
        run_step(&mut universe, step);
        if should_commit(index, mask) {
            commit_all(&mut universe);
            assert_eq!(
                committed_outputs(&universe),
                committed_prefix_outputs(program, index),
                "committed prefix mismatch at step {index} for program:\n{}",
                program.render()
            );
        }
    }

    commit_all(&mut universe);
    assert_eq!(
        committed_outputs(&universe),
        committed_prefix_outputs(program, program.steps.len() - 1),
        "final committed output mismatch for program:\n{}",
        program.render()
    );
}

fn committed_prefix_outputs(program: &Program, end_index: usize) -> CommittedOutputs {
    let mut universe = setup_universe();
    run_steps(&mut universe, &program.steps[..=end_index]);
    commit_all(&mut universe);
    committed_outputs(&universe)
}

fn should_commit(index: usize, mask: &[bool]) -> bool {
    !mask.is_empty() && mask[index % mask.len()]
}

fn run_steps(universe: &mut Universe, steps: &[Step]) {
    for step in steps {
        run_step(universe, step);
    }
}

fn run_step(universe: &mut Universe, step: &Step) {
    match step {
        Step::Tex(step) => run_tex_chunk(universe, &step.render()),
        Step::RngTick { register } => {
            let random = universe.world_mut().next_random_u64();
            universe.set_count(*register, (random % 10_000) as i32);
        }
    }
}

fn run_tex_chunk(universe: &mut Universe, source: &str) {
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut hooks = FuzzHooks;
    umber::run_input_with_hooks(&mut input, universe, &mut hooks)
        .unwrap_or_else(|err| panic!("effectful chunk failed: {err}\n{source}"));
}

fn commit_all(universe: &mut Universe) {
    let effect_pos = universe.world().effect_pos();
    universe
        .world_mut()
        .commit_effects(effect_pos)
        .expect("memory world commit succeeds");
}

fn committed_outputs(universe: &Universe) -> CommittedOutputs {
    CommittedOutputs {
        terminal: universe
            .world()
            .memory_terminal_output()
            .expect("memory world terminal output")
            .to_vec(),
        log: universe
            .world()
            .memory_log_output()
            .expect("memory world log output")
            .to_vec(),
        streams: OUTPUT_PATHS
            .iter()
            .map(|path| universe.world().memory_output(path).map(<[u8]>::to_vec))
            .collect(),
    }
}

fn assert_no_committed_outputs(universe: &Universe, program: &Program) {
    let outputs = committed_outputs(universe);
    assert!(
        outputs.terminal.is_empty(),
        "terminal bytes leaked before commit for program:\n{}",
        program.render()
    );
    assert!(
        outputs.log.is_empty(),
        "log bytes leaked before commit for program:\n{}",
        program.render()
    );
    assert!(
        outputs.streams.iter().all(Option::is_none),
        "stream bytes leaked before commit for program:\n{}",
        program.render()
    );
}

fn setup_universe() -> Universe {
    let mut world = World::memory();
    seed_world(&mut world);
    let mut universe = Universe::with_world(world);
    umber::prepare_run_stores(&mut universe);
    universe
}

fn seed_world(world: &mut World) {
    for (path, bytes) in [
        (
            "inc0.tex",
            br"\count10=1 \message{incA:\the\count10} "[..].to_vec(),
        ),
        (
            "inc1.tex",
            br"\advance\count10 by 2 \message{incB:\the\count10} "[..].to_vec(),
        ),
        ("read0.txt", b"alpha\n".to_vec()),
        ("read1.txt", b"beta\n".to_vec()),
    ] {
        world
            .set_memory_file(path, bytes)
            .expect("seed memory file");
    }
    for index in 0..64 {
        world
            .push_memory_terminal_line(format!("terminal{index}"))
            .expect("seed terminal line");
    }
}

#[derive(Clone, Copy, Debug)]
struct FuzzHooks;

impl ExpansionHooks<MemoryInput> for FuzzHooks {
    fn open_input(&mut self, stores: &mut Universe, name: &str) -> Result<MemoryInput, String> {
        let mut path = PathBuf::from(name);
        if path.extension().is_none() {
            path.set_extension("tex");
        }
        let content = stores
            .world_mut()
            .read_file(&path)
            .map_err(|err| format!("{} ({err})", path.display()))?;
        Ok(MemoryInput::new(
            String::from_utf8_lossy(content.bytes()).into_owned(),
        ))
    }

    fn job_name(&self) -> &str {
        "effect-fuzz"
    }
}

impl Program {
    fn render(&self) -> String {
        let mut source = String::new();
        for step in &self.steps {
            match step {
                Step::Tex(step) => source.push_str(&step.render()),
                Step::RngTick { register } => {
                    source.push_str(&format!("<rng->\\count{register}> "));
                }
            }
        }
        source
    }
}

impl TexStep {
    fn render(&self) -> String {
        match *self {
            Self::CountAssign { register, value } => format!(r"\count{register}={value} "),
            Self::OpenOut { slot } => format!(r"\openout{slot}={} ", output_path(slot)),
            Self::CloseOut { slot } => format!(r"\closeout{slot} "),
            Self::Message { seed, register } => {
                format!(r"\message{{m{seed}:\the\count{register}}} ")
            }
            Self::Write {
                slot,
                seed,
                register,
            } => format!(r"\write{slot}{{w{seed}:\the\count{register}}} "),
            Self::Input { file } => format!(r"\input{{{}}} ", file.name()),
            Self::OpenInRead { slot, file, target } => {
                format!(
                    r"\openin{slot}={}.txt \read{slot} to\{} \message{{r:\{}}} ",
                    file.name(),
                    target.name(),
                    target.name()
                )
            }
            Self::TerminalRead { target } => {
                format!(
                    r"\read15 to\{} \message{{t:\{}}} ",
                    target.name(),
                    target.name()
                )
            }
        }
    }
}

impl InputFile {
    fn name(self) -> &'static str {
        match self {
            Self::IncA => "inc0",
            Self::IncB => "inc1",
        }
    }
}

impl ReadFile {
    fn name(self) -> &'static str {
        match self {
            Self::ReadA => "read0",
            Self::ReadB => "read1",
        }
    }
}

impl ReadTarget {
    fn name(self) -> &'static str {
        match self {
            Self::RA => "RA",
            Self::RB => "RB",
            Self::RC => "RC",
        }
    }
}

fn output_path(slot: u8) -> &'static str {
    OUTPUT_PATHS[usize::from(slot) % OUTPUT_PATHS.len()]
}

fn program_strategy() -> impl Strategy<Value = Program> {
    prop::collection::vec(step_strategy(), 1..18).prop_map(|steps| Program { steps })
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        10 => tex_step_strategy().prop_map(Step::Tex),
        2 => register_strategy().prop_map(|register| Step::RngTick { register }),
    ]
}

fn tex_step_strategy() -> impl Strategy<Value = TexStep> {
    prop_oneof![
        4 => (register_strategy(), value_strategy()).prop_map(|(register, value)| {
            TexStep::CountAssign { register, value }
        }),
        2 => stream_slot_strategy().prop_map(|slot| TexStep::OpenOut { slot }),
        1 => stream_slot_strategy().prop_map(|slot| TexStep::CloseOut { slot }),
        4 => (0_u8..32, register_strategy()).prop_map(|(seed, register)| {
            TexStep::Message { seed, register }
        }),
        4 => (stream_slot_strategy(), 0_u8..32, register_strategy()).prop_map(
            |(slot, seed, register)| TexStep::Write {
                slot,
                seed,
                register,
            },
        ),
        3 => input_file_strategy().prop_map(|file| TexStep::Input { file }),
        3 => (1_u8..3, read_file_strategy(), read_target_strategy()).prop_map(
            |(slot, file, target)| TexStep::OpenInRead { slot, file, target },
        ),
        2 => read_target_strategy().prop_map(|target| TexStep::TerminalRead { target }),
    ]
}

fn stream_slot_strategy() -> impl Strategy<Value = u8> {
    0_u8..3
}

fn register_strategy() -> impl Strategy<Value = u16> {
    prop_oneof![0_u16..8, 40_u16..48, 300_u16..308]
}

fn value_strategy() -> impl Strategy<Value = i32> {
    -5_i32..20
}

fn input_file_strategy() -> impl Strategy<Value = InputFile> {
    prop_oneof![Just(InputFile::IncA), Just(InputFile::IncB)]
}

fn read_file_strategy() -> impl Strategy<Value = ReadFile> {
    prop_oneof![Just(ReadFile::ReadA), Just(ReadFile::ReadB)]
}

fn read_target_strategy() -> impl Strategy<Value = ReadTarget> {
    prop_oneof![
        Just(ReadTarget::RA),
        Just(ReadTarget::RB),
        Just(ReadTarget::RC),
    ]
}

#[allow(clippy::disallowed_methods)]
fn prop_cases() -> u32 {
    env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(32)
}

fn prop_cases_for_shard(shard: u32) -> u32 {
    let cases = prop_cases();
    let base = cases / REPLAY_SHARDS;
    let remainder = cases % REPLAY_SHARDS;
    base + u32::from(shard < remainder)
}
