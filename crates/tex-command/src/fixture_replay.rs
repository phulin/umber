//! Hermetic replay of registered canonical command-fixture sources.
//!
//! This module is test-only on purpose. It turns the committed fixture
//! contract into complete, immutable source registrations and drives those
//! registrations through the one canonical raw-delivery path. It neither
//! interprets fixture expectations nor provides an alternate source reader,
//! lexer, expander, or executor.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tex_oracle::{CommittedFixture, EngineDialect, validate_tex82_command_trace_suite};
use tex_state::Universe;

use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandProcessor, CommandProfile, CommandRuntime, CommandState, RegisteredSourceKind,
    SourceRegistration,
};

const TEX82_FIXTURE_ROOT: &str = "tests/corpus/command/tex82";
const MAX_DELIVERIES_OVERHEAD: usize = 64;

/// Registered source inputs for one committed TeX82 fixture.
///
/// The selector, profile, and source bytes are all derived from a fully
/// validated [`CommittedFixture`]. `sources` is bytewise ordered by the
/// fixture's logical source name, making registration and replay order
/// independent of host directory iteration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RegisteredTex82Fixture {
    pub(crate) selector: String,
    pub(crate) profile: CommandProfile,
    sources: BTreeMap<String, Arc<[u8]>>,
}

/// Complete deterministic inventory of canonical TeX82 fixture sources.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Tex82FixtureRegistry {
    fixtures: BTreeMap<String, RegisteredTex82Fixture>,
}

/// Observations captured while one registered source exhausts its input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceReplay {
    pub(crate) source: String,
    pub(crate) observations: Vec<CommandObservation>,
    pub(crate) deliveries: usize,
}

/// Captured observer streams for every source of one registered fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FixtureReplay {
    pub(crate) selector: String,
    pub(crate) profile: CommandProfile,
    pub(crate) sources: Vec<SourceReplay>,
}

/// A bounded registration or replay failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FixtureReplayError(String);

impl std::fmt::Display for FixtureReplayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for FixtureReplayError {}

impl Tex82FixtureRegistry {
    /// Loads the complete committed TeX82 command suite after validating its
    /// pinned regeneration contract, fixture identities, and coverage audit.
    pub(crate) fn load(repository: impl AsRef<Path>) -> Result<Self, FixtureReplayError> {
        let repository = repository.as_ref();
        let suite = validate_tex82_command_trace_suite(repository)
            .map_err(|error| FixtureReplayError(format!("invalid TeX82 fixture suite: {error}")))?;
        let mut fixtures = BTreeMap::new();
        for entry in suite.fixtures {
            let directory = repository.join(TEX82_FIXTURE_ROOT).join(
                entry.selector.strip_prefix("tex82/").ok_or_else(|| {
                    FixtureReplayError(format!("unsafe TeX82 fixture selector {}", entry.selector))
                })?,
            );
            let fixture = CommittedFixture::load(&directory).map_err(|error| {
                FixtureReplayError(format!("{} could not be loaded: {error}", entry.selector))
            })?;
            let registered = RegisteredTex82Fixture::from_committed(&directory, fixture)?;
            if fixtures
                .insert(entry.selector.clone(), registered)
                .is_some()
            {
                return Err(FixtureReplayError(format!(
                    "TeX82 fixture selector {} is registered more than once",
                    entry.selector
                )));
            }
        }
        if fixtures.is_empty() {
            return Err(FixtureReplayError(
                "TeX82 fixture registry has no committed fixtures".into(),
            ));
        }
        Ok(Self { fixtures })
    }

    pub(crate) fn fixture(&self, selector: &str) -> Option<&RegisteredTex82Fixture> {
        self.fixtures.get(selector)
    }

    pub(crate) fn fixtures(&self) -> impl Iterator<Item = &RegisteredTex82Fixture> {
        self.fixtures.values()
    }
}

impl RegisteredTex82Fixture {
    #[allow(
        clippy::disallowed_methods,
        reason = "this test-only registry loads already validated committed fixture bytes, never engine input"
    )]
    fn from_committed(
        directory: &Path,
        fixture: CommittedFixture,
    ) -> Result<Self, FixtureReplayError> {
        if fixture.manifest.oracle.engine.dialect != EngineDialect::Tex82
            || fixture.manifest.profile.invocation != "initex"
            || fixture.manifest.profile.characters != "eight_bit_exact"
        {
            return Err(FixtureReplayError(format!(
                "{} is not pinned to the TeX82 INITEX eight-bit profile",
                fixture.manifest.name
            )));
        }

        let mut sources = BTreeMap::new();
        for (name, artifact) in &fixture.manifest.sources {
            let path = directory.join(&artifact.path);
            let bytes = std::fs::read(&path).map_err(|error| {
                FixtureReplayError(format!(
                    "{} source {name} is unavailable at {}: {error}",
                    fixture.manifest.name,
                    path.display()
                ))
            })?;
            if u64::try_from(bytes.len()).ok() != Some(artifact.bytes) {
                return Err(FixtureReplayError(format!(
                    "{} source {name} length changed after fixture validation",
                    fixture.manifest.name
                )));
            }
            sources.insert(name.clone(), Arc::from(bytes));
        }
        if sources.is_empty() {
            return Err(FixtureReplayError(format!(
                "{} registers no executable sources",
                fixture.manifest.name
            )));
        }

        Ok(Self {
            selector: fixture.manifest.name,
            profile: CommandProfile::TEX82,
            sources,
        })
    }

    /// Replays every registered source from a fresh explicit INITEX command
    /// state. Each source is an independently bounded command episode, so a
    /// malformed or stale registration cannot consume a later source.
    pub(crate) fn replay(&self) -> Result<FixtureReplay, FixtureReplayError> {
        let mut sources = Vec::with_capacity(self.sources.len());
        for (name, bytes) in &self.sources {
            sources.push(replay_source(self.profile, name, Arc::clone(bytes))?);
        }
        Ok(FixtureReplay {
            selector: self.selector.clone(),
            profile: self.profile,
            sources,
        })
    }
}

fn replay_source(
    profile: CommandProfile,
    name: &str,
    bytes: Arc<[u8]>,
) -> Result<SourceReplay, FixtureReplayError> {
    let limit = bytes
        .len()
        .checked_mul(2)
        .and_then(|value| value.checked_add(MAX_DELIVERIES_OVERHEAD))
        .ok_or_else(|| FixtureReplayError(format!("source {name} replay bound overflows")))?;
    let mut command = CommandState::new(profile);
    let source = command
        .register_source(SourceRegistration::new(RegisteredSourceKind::World, bytes))
        .map_err(|error| FixtureReplayError(format!("source {name} cannot register: {error}")))?;
    command
        .open_registered_source(source)
        .map_err(|error| FixtureReplayError(format!("source {name} cannot open: {error}")))?;

    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut deliveries = 0;
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);
        loop {
            if deliveries == limit {
                return Err(FixtureReplayError(format!(
                    "source {name} exceeded its bounded replay limit of {limit} deliveries"
                )));
            }
            match processor.get_next() {
                Ok(Some(_)) => deliveries += 1,
                Ok(None) => break,
                Err(error) => {
                    return Err(FixtureReplayError(format!(
                        "source {name} replay failed after {deliveries} deliveries: {error}"
                    )));
                }
            }
        }
    }

    Ok(SourceReplay {
        source: name.into(),
        observations: recorder.0,
        deliveries,
    })
}

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn repository_root() -> PathBuf {
    test_support::repository_root()
}

#[test]
fn every_committed_tex82_fixture_registers_and_replays_deterministically() {
    let registry = Tex82FixtureRegistry::load(repository_root()).expect("committed registry");
    // The registry enumerates the corpus rather than a list, so this asserts
    // only that it found something to replay. Pinning an exact count made the
    // test a second, silent registry of how many fixtures exist, which
    // `umber2-alfh.2`'s split then had to come back and edit.
    assert!(registry.fixtures().count() > 0);
    for fixture in registry.fixtures() {
        assert_eq!(fixture.profile, CommandProfile::TEX82);
        let first = fixture.replay().expect("first replay");
        let second = fixture.replay().expect("second replay");
        assert_eq!(first, second, "{} replay drifted", fixture.selector);
        assert!(
            first
                .sources
                .iter()
                .all(|source| !source.observations.is_empty())
        );
    }
}

#[test]
fn fixture_lookup_rejects_unknown_selectors_without_replay() {
    let registry = Tex82FixtureRegistry::load(repository_root()).expect("committed registry");
    assert!(registry.fixture("tex82/missing").is_none());
}

#[test]
fn replay_bound_is_derived_from_registered_source_bytes() {
    let fixture = RegisteredTex82Fixture {
        selector: "tex82/bounded".into(),
        profile: CommandProfile::TEX82,
        sources: BTreeMap::from([("empty.tex".into(), Arc::from(&b""[..]))]),
    };
    let replay = fixture.replay().expect("empty source is bounded");
    assert_eq!(replay.sources[0].deliveries, 0);
}
