use std::fmt;

use tex_command::{RegisteredSourceKind, SourceRegistration};
use tex_exec::{MainControl, MainControlStep};
use tex_state::Universe;

use crate::{BatchResult, Workload, benchmark_font, serialize_dvi};

#[derive(Debug)]
pub enum ProductionError {
    Register(tex_command::SourceRegistrationError),
    Execute(tex_exec::ExecError),
    MissingArtifact,
    ExtraArtifacts(usize),
    Artifact(tex_out::ParseError),
    MissingDvi,
    ExtraDvi(usize),
    Dvi(tex_out::dvi::DviError),
}

impl fmt::Display for ProductionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "production batch failed: {self:?}")
    }
}

impl std::error::Error for ProductionError {}

pub fn run_production(workload: &Workload) -> Result<BatchResult, ProductionError> {
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(benchmark_font());
    let mut control = MainControl::tex82_initex(&mut stores);
    stores.set_current_font_global(font);
    control.set_dvi_output(true);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            workload.source(),
        ))
        .map_err(ProductionError::Register)?;

    while let MainControlStep::Continue = control
        .step(&mut stores)
        .map_err(ProductionError::Execute)?
    {}

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
        counts: [stores.count(0), stores.count(1), stores.count(2)],
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
