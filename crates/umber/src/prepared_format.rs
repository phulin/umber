//! Persistent preparation and fresh loaded-job execution for native fixtures.

use std::sync::Arc;

use tex_command::{CommandObserver, RegisteredSourceKind};
use tex_state::{InteractionMode, JobClock, ProvenanceDemand, World};

use crate::format_fixture::LoadedRunConfiguration;
use crate::{
    EngineMode, FormatCacheStore, FormatFixture, FormatFixtureError, FormatGenerationGuards,
    FormatRecipe, FormatWorkerLauncher, LoadedFormatResource, LoadedFormatRun, OutputCapability,
    ensure_format,
};

/// One explicit, job-local execution episode for an authenticated format.
pub struct PreparedFormatJob<'a> {
    pub engine: EngineMode,
    /// Engine binary identity for startup framing, independent of the
    /// semantic command profile exercised by the job.
    pub engine_binary: tex_exec::EngineBinaryIdentity,
    pub backend: OutputCapability,
    pub clock: JobClock,
    pub interaction: InteractionMode,
    pub error_context_widths: tex_state::print::ErrorContextWidths,
    /// Optional provenance consumers selected once for this loaded job.
    ///
    /// This is job-local operational configuration and never participates in
    /// prepared-format identity or serialization.
    pub provenance_demand: ProvenanceDemand,
    pub guards: FormatGenerationGuards,
    /// Complete TeX82 §534 invocation text, including any driver selector.
    pub startup_line: String,
    pub source_name: String,
    pub source_kind: RegisteredSourceKind,
    /// Job-owned source bytes materialized into the freshly loaded session.
    pub source: Vec<u8>,
    pub resources: Vec<LoadedFormatResource>,
    pub terminal_input: Vec<String>,
    pub observer: &'a mut dyn CommandObserver,
}

/// Native provider for persistent format preparation and isolated loaded jobs.
#[derive(Clone, Debug)]
pub struct PreparedFormatProvider {
    cache: FormatCacheStore,
    launcher: FormatWorkerLauncher,
}

impl PreparedFormatProvider {
    /// Resolves the persistent platform cache through its established environment contract.
    pub fn from_environment(launcher: FormatWorkerLauncher) -> Result<Self, FormatFixtureError> {
        Ok(Self::with_store(
            FormatCacheStore::from_environment()?,
            launcher,
        ))
    }

    /// Injects an explicit store while retaining the production provider behavior.
    ///
    /// This constructor supports hermetic tests and callers with an already-resolved
    /// persistent store; it does not introduce a fallback cache.
    #[must_use]
    pub fn with_store(cache: FormatCacheStore, launcher: FormatWorkerLauncher) -> Self {
        Self { cache, launcher }
    }

    /// Authenticates or constructs the complete recipe in the persistent store.
    pub fn prepare(&self, recipe: &FormatRecipe) -> Result<FormatFixture, FormatFixtureError> {
        ensure_format(&self.cache, recipe, &self.launcher)
    }

    /// Runs one request in a provider-created fresh memory world.
    pub fn run(
        &self,
        fixture: &FormatFixture,
        job: PreparedFormatJob<'_>,
    ) -> Result<LoadedFormatRun, FormatFixtureError> {
        self.run_with_completion(fixture, job, tex_exec::RootCompletionPolicy::RequireTeXEnd)
    }

    /// Runs one authored fragment whose root EOF is the host completion boundary.
    ///
    /// Unlike [`Self::run`], this does not enter TeX82 §360's terminal retry
    /// when the registered root is exhausted. Nested sources still retire back
    /// into their parent normally, and an explicit `\end` still performs TeX's
    /// ordinary final cleanup.
    pub fn run_fragment(
        &self,
        fixture: &FormatFixture,
        job: PreparedFormatJob<'_>,
    ) -> Result<LoadedFormatRun, FormatFixtureError> {
        self.run_with_completion(fixture, job, tex_exec::RootCompletionPolicy::StopAtRootEof)
    }

    fn run_with_completion(
        &self,
        fixture: &FormatFixture,
        job: PreparedFormatJob<'_>,
        completion: tex_exec::RootCompletionPolicy,
    ) -> Result<LoadedFormatRun, FormatFixtureError> {
        job.guards.validate()?;
        let actual = fixture.engine_mode();
        if job.engine != actual {
            return Err(FormatFixtureError::ProviderProfileMismatch {
                expected: actual,
                actual: job.engine,
            });
        }
        if !job.engine_binary.supports(job.engine.command_profile()) {
            return Err(FormatFixtureError::ProviderBinaryMismatch {
                engine: job.engine,
                binary: job.engine_binary,
            });
        }
        if job.backend == OutputCapability::Pdf && !job.engine.supports_pdf_output() {
            return Err(FormatFixtureError::ProviderBackendMismatch {
                engine: job.engine,
                backend: job.backend,
            });
        }
        if job.backend == OutputCapability::Html {
            return Err(FormatFixtureError::ProviderBackendMismatch {
                engine: job.engine,
                backend: job.backend,
            });
        }

        let mut world = World::memory_with_clock(job.clock);
        for line in job.terminal_input {
            world
                .push_memory_terminal_line(line)
                .map_err(|error| FormatFixtureError::World(error.to_string()))?;
        }
        let mut loaded = fixture
            .load(world)?
            .with_provenance_demand(job.provenance_demand);
        loaded.set_interaction_mode(job.interaction);
        loaded.set_error_context_widths(job.error_context_widths);
        loaded.run_configured(
            &job.source_name,
            job.source_kind,
            Arc::from(job.source),
            &job.resources,
            LoadedRunConfiguration {
                guards: job.guards,
                engine_binary: job.engine_binary,
                startup_line: job.startup_line,
                completion,
            },
            job.observer,
        )
    }
}

#[cfg(test)]
mod tests;
