use std::env;

use proptest::prelude::*;
use proptest::test_runner::Config;
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
    Shipout {
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
            fn $name((program, mask) in (commit_program_strategy(), prop::collection::vec(any::<bool>(), 0..18))) {
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
fn terminal_read_chunk_survives_a_prior_retained_run() {
    with_universe(|universe| {
        run_tex_chunk(universe, r"\write0{before} ");
        assert_eq!(universe.world().stream_bufs().terminal_input_next(), 0);
        run_tex_chunk(universe, r"\read15 to\RA \message{t:\RA} ");
    });
}

#[test]
fn shipout_commit_cursor_survives_a_prior_effect_commit() {
    with_universe(|universe| {
        run_tex_chunk(universe, r"\message{before} ");
        commit_all(universe);
        run_tex_chunk(universe, r"\shipout\hbox{\write16{page}} ");
    });
}

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
    with_universe(|universe| {
        let checkpoint = universe.runtime_checkpoint().expect("runtime checkpoint");
        run_steps(universe, &program.steps);
        assert_no_committed_outputs(universe, program);
        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("effectful rollback");
        assert_no_committed_outputs(universe, program);
    });
}

fn assert_commit_path_matches_straight_line(program: &Program, mask: &[bool]) {
    with_universe(|universe| {
        for (index, step) in program.steps.iter().enumerate() {
            run_step(universe, step);
            if should_commit(index, mask) {
                commit_all(universe);
                assert_eq!(
                    committed_outputs(universe),
                    committed_prefix_outputs(program, index),
                    "committed prefix mismatch at step {index} for program:\n{}",
                    program.render()
                );
            }
        }
        commit_all(universe);
        assert_eq!(
            committed_outputs(universe),
            committed_prefix_outputs(program, program.steps.len() - 1),
            "final committed output mismatch for program:\n{}",
            program.render()
        );
    });
}

fn committed_prefix_outputs(program: &Program, end_index: usize) -> CommittedOutputs {
    with_universe(|universe| {
        run_steps(universe, &program.steps[..=end_index]);
        commit_all(universe);
        committed_outputs(universe)
    })
}

fn should_commit(index: usize, mask: &[bool]) -> bool {
    !mask.is_empty() && mask[index % mask.len()]
}

fn run_steps<G>(universe: &mut Universe<G>, steps: &[Step]) {
    for step in steps {
        run_step(universe, step);
    }
}

fn run_step<G>(universe: &mut Universe<G>, step: &Step) {
    match step {
        Step::Tex(step) => run_tex_chunk(universe, &step.render()),
        Step::RngTick { register } => {
            let random = universe.world_mut().next_random_u64();
            universe
                .assign_count(
                    *register,
                    (random % 10_000) as i32,
                    tex_state::AssignmentScope::Global,
                )
                .expect("assign random count");
        }
    }
}

fn run_tex_chunk<G>(universe: &mut Universe<G>, source: &str) {
    umber::run_memory_with_stores(source, universe)
        .unwrap_or_else(|err| panic!("effectful chunk failed: {err}\n{source}"));
}

fn commit_all<G>(universe: &mut Universe<G>) {
    let effect_pos = universe.world().effect_pos();
    universe
        .publish_effect_prefix(effect_pos)
        .expect("memory world commit succeeds");
}

fn committed_outputs<G>(universe: &Universe<G>) -> CommittedOutputs {
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

fn assert_no_committed_outputs<G>(universe: &Universe<G>, program: &Program) {
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

fn with_universe<R>(
    use_universe: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>) -> R,
) -> R {
    let mut world = World::memory();
    seed_world(&mut world);
    umber::with_engine_world(world, |universe| {
        umber::prepare_run_stores(universe);
        // These are retained fragments, not interactive error-dialogue tests.
        universe.set_interaction_mode(tex_state::InteractionMode::Scroll);
        use_universe(universe)
    })
    .expect("fresh effectful replay universe")
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
    // Two independent consumers draw from this pool, and the harness must
    // stay at `errorstop`: §484 makes `\read` from the terminal a fatal
    // error in the nonstop modes, so the generator's `TerminalRead` step
    // needs the interactive one. Besides those reads, §82's `error` reads a
    // line for §83's advice on every recoverable error -- and the generator
    // deliberately emits out-of-range registers (`300..308` against TeX82's
    // 255 maximum), so §436's `Bad register code` fires on ordinary runs.
    // The pool therefore has to cover prompts as well as reads.
    for index in 0..512 {
        world
            .push_memory_terminal_line(format!("terminal{index}"))
            .expect("seed terminal line");
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
            Self::Shipout { seed, register } => {
                format!(r"\shipout\hbox{{\write16{{s{seed}:\the\count{register}}}}} ")
            }
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

fn commit_program_strategy() -> impl Strategy<Value = Program> {
    prop::collection::vec(commit_step_strategy(), 1..18).prop_map(|steps| Program { steps })
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        10 => tex_step_strategy().prop_map(Step::Tex),
        2 => register_strategy().prop_map(|register| Step::RngTick { register }),
    ]
}

fn commit_step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        10 => tex_step_with_shipout_strategy().prop_map(Step::Tex),
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

fn tex_step_with_shipout_strategy() -> impl Strategy<Value = TexStep> {
    prop_oneof![
        12 => tex_step_strategy(),
        3 => (0_u8..32, register_strategy()).prop_map(|(seed, register)| {
            TexStep::Shipout { seed, register }
        }),
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
