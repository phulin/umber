use std::fmt;

use tex_command::{RegisteredSourceKind, SourceRegistration};
use tex_exec::{MainControl, MainControlStep, StepResult};
use tex_state::interner::InternerBudget;
use tex_state::{AssignmentScope, Universe};

use crate::{BatchResult, Workload, benchmark_font, prepare_plain_catcodes, serialize_dvi};

#[derive(Debug)]
pub enum ProductionError {
    Register(tex_command::SourceRegistrationError),
    Execute(tex_exec::ExecError),
    UnexpectedResource(tex_exec::ResourceNeed),
    MissingArtifact,
    ExtraArtifacts(usize),
    Artifact(tex_out::ParseError),
    MissingDvi,
    ExtraDvi(usize),
    Dvi(tex_out::dvi::DviError),
    State(String),
}

impl fmt::Display for ProductionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "production batch failed: {self:?}")
    }
}

impl std::error::Error for ProductionError {}

pub fn run_production(workload: &Workload) -> Result<BatchResult, ProductionError> {
    let budget =
        InternerBudget::new(65_536, 131_072, 16 * 1024 * 1024).expect("benchmark interner budget");
    tex_state::with_universe(budget, |stores| run_generation(stores, workload))
        .map_err(|error| ProductionError::State(format!("{error:?}")))?
}

fn run_generation<G>(
    stores: &mut Universe<G>,
    workload: &Workload,
) -> Result<BatchResult, ProductionError> {
    prepare_plain_catcodes(stores);
    let font = {
        let mut context = stores
            .command_context()
            .map_err(|error| ProductionError::State(format!("{error:?}")))?;
        let font = context.intern_font(benchmark_font());
        context
            .assign_current_font(font, AssignmentScope::Global)
            .map_err(|error| ProductionError::State(format!("{error:?}")))?;
        font
    };
    std::hint::black_box(font);
    let mut control = MainControl::tex82_initex(stores);
    control.set_dvi_output(true);
    let source = SourceRegistration::new(RegisteredSourceKind::Generated, workload.source());
    control
        .register_root_source(source)
        .map_err(ProductionError::Register)?;

    loop {
        match control
            .advance_episode(stores)
            .map_err(ProductionError::Execute)?
        {
            StepResult::Progress(MainControlStep::Continue) => {}
            StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
            StepResult::Suspended(need) => {
                return Err(ProductionError::UnexpectedResource(need));
            }
        }
    }

    let artifacts = stores.world().committed_artifacts();
    let (artifact, artifact_bytes) = match artifacts {
        [] => return Err(ProductionError::MissingArtifact),
        [artifact] => (
            tex_out::PageArtifact::from_bytes(artifact.bytes())
                .map_err(ProductionError::Artifact)?,
            artifact.bytes().to_vec(),
        ),
        artifacts => return Err(ProductionError::ExtraArtifacts(artifacts.len())),
    };
    let plans = control.take_prepared_dvi_pages();
    let plan = match plans.len() {
        0 => return Err(ProductionError::MissingDvi),
        1 => plans.into_iter().next().expect("one DVI plan").into_plan(),
        count => return Err(ProductionError::ExtraDvi(count)),
    };
    let dvi = serialize_dvi(plan).map_err(ProductionError::Dvi)?;
    let effects = stores.world().effect_records().to_vec();
    let terminal = stores
        .world()
        .memory_terminal_output()
        .unwrap_or_default()
        .to_vec();
    let log = stores
        .world()
        .memory_log_output()
        .unwrap_or_default()
        .to_vec();
    Ok(BatchResult {
        counts: [
            stores
                .count(0)
                .map_err(|error| ProductionError::State(format!("{error:?}")))?,
            stores
                .count(1)
                .map_err(|error| ProductionError::State(format!("{error:?}")))?,
            stores
                .count(2)
                .map_err(|error| ProductionError::State(format!("{error:?}")))?,
        ],
        artifact,
        artifact_bytes,
        dvi,
        effects,
        terminal,
        log,
        calls: workload.calls(),
        command_work: Some(control.command_work()),
    })
}
