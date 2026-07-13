use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use tex_exec::{Executor, try_execute_assignment};
use tex_expand::{ExpansionHooks, NoopRecorder, get_x_token_with_hooks};
use tex_lex::{InputSource, InputStack, MemoryInput};
use tex_out::PageArtifact;
use tex_out::dvi::{DviError, DviStreamWriter};
use tex_state::env::banks::IntParam;
use tex_state::token::TracedTokenWord;
use tex_state::{
    ContentHash, EffectPos, EffectRecord, ExpansionContext, PrintSink, Universe, WorldError,
};

mod input_search;

pub use input_search::{TexFontSearchPath, TexInputSearchPath};

/// The only checkpoint policy supported by composed engine sessions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointPolicy {
    NamedExecutorBoundaries,
}

/// Exclusive composition boundary for input, hooks, state, diagnostics, and artifacts.
pub struct EngineSession<'a, S, H> {
    input: &'a mut InputStack<S>,
    stores: &'a mut Universe,
    hooks: &'a mut H,
    artifact_cursor: usize,
    checkpoint_policy: CheckpointPolicy,
}

impl<'a, S, H> EngineSession<'a, S, H>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    pub fn new(input: &'a mut InputStack<S>, stores: &'a mut Universe, hooks: &'a mut H) -> Self {
        let artifact_cursor = stores.world().artifact_commits().len();
        Self {
            input,
            stores,
            hooks,
            artifact_cursor,
            checkpoint_policy: CheckpointPolicy::NamedExecutorBoundaries,
        }
    }

    #[must_use]
    pub const fn checkpoint_policy(&self) -> CheckpointPolicy {
        self.checkpoint_policy
    }

    #[must_use]
    pub fn stores(&self) -> &Universe {
        self.stores
    }

    pub fn stores_mut(&mut self) -> &mut Universe {
        self.stores
    }

    pub fn execute(&mut self) -> Result<RunResult, tex_exec::ExecError> {
        let mut recorder = NoopRecorder;
        let stats = Executor::new().run_with_recorder_and_hooks(
            self.input,
            self.stores,
            &mut recorder,
            self.hooks,
        )?;
        let committed = self.stores.world().artifact_commits();
        debug_assert_eq!(
            &committed[self.artifact_cursor..],
            stats.shipped_artifacts.as_slice()
        );
        self.artifact_cursor = committed.len();
        Ok(RunResult {
            terminal_text: uncommitted_terminal_text(self.stores),
            artifacts: stats.shipped_artifacts,
            dumped_format: stats.dumped_format,
        })
    }

    pub fn next_expanded_token(
        &mut self,
    ) -> Result<Option<TracedTokenWord>, tex_expand::ExpandError> {
        let mut expansion = ExpansionContext::new(self.stores);
        get_x_token_with_hooks(self.input, &mut expansion, self.hooks)
    }

    pub fn try_execute_assignment(
        &mut self,
        token: TracedTokenWord,
    ) -> Result<bool, tex_exec::ExecError> {
        try_execute_assignment(token, self.input, self.stores, self.hooks)
    }

    pub fn publish_input_summary(&mut self) {
        let summary = self.input.publication_summary(self.stores);
        self.stores.set_input_summary(summary);
    }
}

/// Shared file search and job identity policy for run-like commands.
pub struct FileSessionHooks {
    input_search: TexInputSearchPath,
    font_search: TexFontSearchPath,
    job_name: String,
}

impl FileSessionHooks {
    #[must_use]
    pub fn from_environment(path: &Path) -> Self {
        let areas = |name| {
            std::env::var_os(name)
                .map(|value| {
                    std::env::split_paths(&value)
                        .filter(|path| !path.as_os_str().is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        Self::new(path, areas("TEXINPUTS"), areas("TEXFONTS"))
    }

    #[must_use]
    pub fn new(path: &Path, tex_input_areas: Vec<PathBuf>, tex_font_areas: Vec<PathBuf>) -> Self {
        let base_dir = path.parent().unwrap_or_else(|| Path::new(".")).to_owned();
        let job_name = path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("texput")
            .to_owned();
        Self {
            input_search: TexInputSearchPath::new(&base_dir, tex_input_areas),
            font_search: TexFontSearchPath::new(base_dir, tex_font_areas),
            job_name,
        }
    }
}

impl ExpansionHooks<tex_lex::WorldInput> for FileSessionHooks {
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        input: &mut C,
        name: &str,
    ) -> Result<tex_lex::WorldInput, String> {
        self.input_search
            .read(input, name)
            .map(tex_lex::WorldInput::from_content)
    }

    fn open_font<C: tex_state::InputReadState>(
        &mut self,
        input: &mut C,
        path: &Path,
    ) -> Result<tex_state::FileContent, String> {
        self.font_search.read(input, path)
    }

    fn job_name(&self) -> &str {
        &self.job_name
    }
}

/// Result of running TeX through the batch executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunResult {
    pub terminal_text: String,
    pub artifacts: Vec<ContentHash>,
    pub dumped_format: bool,
}

/// A fully prepared downstream file that has not been materialized.
pub struct DriverFile {
    path: PathBuf,
    bytes: Vec<u8>,
}

impl DriverFile {
    #[must_use]
    pub fn new(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self { path, bytes }
    }
}

/// Finalization state before the engine's World effects have committed.
pub struct PlannedFinalization {
    effect_pos: EffectPos,
    files: Vec<DriverFile>,
}

impl PlannedFinalization {
    pub fn new(effect_pos: EffectPos, files: Vec<DriverFile>) -> Result<Self, FinalizationError> {
        let mut paths = BTreeSet::new();
        for file in &files {
            if !paths.insert(file.path.clone()) {
                return Err(FinalizationError::ConflictingDriverPath(file.path.clone()));
            }
        }
        Ok(Self { effect_pos, files })
    }

    pub fn commit_effects(
        self,
        stores: &mut Universe,
    ) -> Result<CommittedFinalization, FinalizationError> {
        stores.commit_effects(self.effect_pos)?;
        Ok(CommittedFinalization { files: self.files })
    }

    /// Explicit fixture policy: retain effect records and materialize nothing.
    pub fn discard_uncommitted(self) {}
}

/// Finalization state that may materialize downstream files safely.
pub struct CommittedFinalization {
    files: Vec<DriverFile>,
}

impl CommittedFinalization {
    pub fn materialize(self, stores: &mut Universe) -> Result<(), FinalizationError> {
        for file in self.files {
            stores.world_mut().write_file(file.path, file.bytes)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum FinalizationError {
    ConflictingDriverPath(PathBuf),
    World(WorldError),
}

impl std::fmt::Display for FinalizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConflictingDriverPath(path) => write!(
                f,
                "multiple downstream outputs resolve to {}",
                path.display()
            ),
            Self::World(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for FinalizationError {}

impl From<WorldError> for FinalizationError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

/// Installs the primitive/state setup used by `umber run`.
pub fn prepare_run_stores(stores: &mut Universe) {
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    tex_expand::install_expandable_primitives(stores);
    tex_exec::install_unexpandable_primitives(stores);
    stores.intern("par");
}

/// Installs the primitive/state setup used by `umber run --etex`.
pub fn prepare_etex_run_stores(stores: &mut Universe) {
    prepare_run_stores(stores);
    tex_expand::install_etex_expandable_primitives(stores);
    tex_exec::install_etex_unexpandable_primitives(stores);
}

#[cfg(test)]
mod primitive_mode_tests {
    use super::*;
    use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};

    #[test]
    fn protected_is_hidden_in_tex82_compatibility_mode() {
        let mut stores = Universe::default();
        prepare_run_stores(&mut stores);
        let protected = stores.intern("protected");
        assert_eq!(stores.meaning(protected), Meaning::Undefined);
        let readline = stores.intern("readline");
        assert_eq!(stores.meaning(readline), Meaning::Undefined);
        let everyeof = stores.intern("everyeof");
        assert_eq!(stores.meaning(everyeof), Meaning::Undefined);
    }

    #[test]
    fn protected_is_installed_in_etex_extended_mode() {
        let mut stores = Universe::default();
        prepare_etex_run_stores(&mut stores);
        let protected = stores.intern("protected");
        assert_eq!(
            stores.meaning(protected),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Protected)
        );
        let readline = stores.intern("readline");
        assert_eq!(
            stores.meaning(readline),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ReadLine)
        );
        let everyeof = stores.intern("everyeof");
        assert!(matches!(stores.meaning(everyeof), Meaning::TokParam(_)));
    }

    #[test]
    fn etex_expandable_primitives_follow_driver_mode() {
        let mut compatibility = Universe::default();
        prepare_run_stores(&mut compatibility);
        let unexpanded = compatibility.intern("unexpanded");
        let detokenize = compatibility.intern("detokenize");
        let unless = compatibility.intern("unless");
        let scantokens = compatibility.intern("scantokens");
        let etex_version = compatibility.intern("eTeXversion");
        let etex_revision = compatibility.intern("eTeXrevision");
        let ifdefined = compatibility.intern("ifdefined");
        let ifcsname = compatibility.intern("ifcsname");
        let currentgrouplevel = compatibility.intern("currentgrouplevel");
        let currentgrouptype = compatibility.intern("currentgrouptype");
        assert_eq!(compatibility.meaning(unexpanded), Meaning::Undefined);
        assert_eq!(compatibility.meaning(detokenize), Meaning::Undefined);
        assert_eq!(compatibility.meaning(unless), Meaning::Undefined);
        assert_eq!(compatibility.meaning(scantokens), Meaning::Undefined);
        for symbol in [
            etex_version,
            etex_revision,
            ifdefined,
            ifcsname,
            currentgrouplevel,
            currentgrouptype,
        ] {
            assert_eq!(compatibility.meaning(symbol), Meaning::Undefined);
        }

        let mut extended = Universe::default();
        prepare_etex_run_stores(&mut extended);
        let unexpanded = extended.intern("unexpanded");
        let detokenize = extended.intern("detokenize");
        let unless = extended.intern("unless");
        let scantokens = extended.intern("scantokens");
        assert_eq!(
            extended.meaning(unexpanded),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unexpanded)
        );
        assert_eq!(
            extended.meaning(detokenize),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Detokenize)
        );
        assert_eq!(
            extended.meaning(unless),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unless)
        );
        assert_eq!(
            extended.meaning(scantokens),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Scantokens)
        );
        let version = extended.intern("eTeXversion");
        assert_eq!(
            extended.meaning(version),
            Meaning::InternalInteger(tex_state::meaning::InternalInteger::ETeXVersion)
        );
        for (name, value) in [
            (
                "currentgrouplevel",
                tex_state::meaning::InternalInteger::CurrentGroupLevel,
            ),
            (
                "currentgrouptype",
                tex_state::meaning::InternalInteger::CurrentGroupType,
            ),
        ] {
            let symbol = extended.intern(name);
            assert_eq!(extended.meaning(symbol), Meaning::InternalInteger(value));
        }
        for (name, primitive) in [
            ("eTeXrevision", ExpandablePrimitive::ETeXRevision),
            ("ifdefined", ExpandablePrimitive::IfDefined),
            ("ifcsname", ExpandablePrimitive::IfCsName),
        ] {
            let symbol = extended.intern(name);
            assert_eq!(
                extended.meaning(symbol),
                Meaning::ExpandablePrimitive(primitive)
            );
        }
    }
}

/// Runs an already-open input stack through the same executor path as `umber run`.
pub fn run_input_with_hooks<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<String, tex_exec::ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    run_input_collecting_artifacts(input, stores, hooks).map(|result| result.terminal_text)
}

/// Runs input and returns the artifact ids emitted by `\shipout` in order.
pub fn run_input_collecting_artifacts<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<RunResult, tex_exec::ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    EngineSession::new(input, stores, hooks).execute()
}

/// Reads committed page artifacts from `World` and writes a complete DVI file.
pub fn dvi_from_artifacts(
    stores: &Universe,
    artifacts: &[ContentHash],
) -> Result<Vec<u8>, DviBuildError> {
    write_dvi_from_artifacts(stores, artifacts, Vec::new())
}

/// Decodes, validates, emits, and drops each artifact before loading the next.
pub fn write_dvi_from_artifacts<W: std::io::Write>(
    stores: &Universe,
    artifacts: &[ContentHash],
    sink: W,
) -> Result<W, DviBuildError> {
    let mut writer = DviStreamWriter::new(sink);
    for &hash in artifacts {
        let bytes = stores
            .world()
            .read_artifact(hash)?
            .ok_or(DviBuildError::MissingArtifact(hash))?;
        let page = PageArtifact::from_bytes(&bytes)?;
        writer.write_page(&page)?;
    }
    Ok(writer.finish()?)
}

/// Runs in-memory TeX through the `umber run` executor setup.
pub fn run_memory_with_stores(
    source: &str,
    stores: &mut Universe,
) -> Result<String, tex_exec::ExecError> {
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut hooks = MemoryRunHooks;
    run_input_with_hooks(&mut input, stores, &mut hooks)
}

#[derive(Clone, Copy, Debug, Default)]
struct MemoryRunHooks;

impl ExpansionHooks<MemoryInput> for MemoryRunHooks {
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        _input: &mut C,
        name: &str,
    ) -> Result<MemoryInput, String> {
        Err(format!("memory run cannot open input {name}"))
    }

    fn job_name(&self) -> &str {
        "texput"
    }
}

fn uncommitted_terminal_text(stores: &Universe) -> String {
    let mut text = String::new();
    for record in stores.world().effect_records() {
        let EffectRecord::StreamWrite { sink, text: chunk } = record else {
            continue;
        };
        match sink {
            PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log => {
                text.push_str(chunk);
            }
            PrintSink::Stream(_) => {}
        }
    }
    text
}

#[derive(Debug)]
pub enum DviBuildError {
    MissingArtifact(ContentHash),
    World(WorldError),
    Parse(tex_out::ParseError),
    Dvi(DviError),
}

impl std::fmt::Display for DviBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingArtifact(hash) => {
                write!(f, "shipped page artifact {} is missing", hash.hex())
            }
            Self::World(err) => write!(f, "{err}"),
            Self::Parse(err) => write!(f, "{err}"),
            Self::Dvi(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for DviBuildError {}

impl From<WorldError> for DviBuildError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

impl From<tex_out::ParseError> for DviBuildError {
    fn from(value: tex_out::ParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<DviError> for DviBuildError {
    fn from(value: DviError) -> Self {
        Self::Dvi(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{DriverFile, FinalizationError, PlannedFinalization};
    use std::path::PathBuf;
    use tex_state::{PrintSink, StreamSlot, Universe, World};

    #[test]
    #[allow(clippy::disallowed_methods)] // Verifies real host ordering at the World boundary.
    fn driver_materialization_follows_engine_effect_commit() {
        let temp = tempfile::tempdir().expect("temp dir");
        let output = temp.path().join("shared.out");
        let mut stores = Universe::with_world(World::real());
        let slot = StreamSlot::new(1);
        stores.world_mut().open_out(slot, &output);
        stores
            .world_mut()
            .write_text(PrintSink::Stream(slot), "engine");
        let plan = PlannedFinalization::new(
            stores.world().effect_pos(),
            vec![DriverFile::new(output.clone(), b"driver".to_vec())],
        )
        .expect("paths are distinct");

        plan.commit_effects(&mut stores)
            .expect("effects commit")
            .materialize(&mut stores)
            .expect("driver materializes");

        assert_eq!(std::fs::read(output).expect("read output"), b"driver");
    }

    #[test]
    fn failed_effect_commit_cannot_materialize_driver_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut stores = Universe::with_world(World::real());
        let slot = StreamSlot::new(1);
        stores.world_mut().open_out(slot, temp.path());
        stores
            .world_mut()
            .write_text(PrintSink::Stream(slot), "cannot write a directory");
        let driver_path = temp.path().join("driver.dvi");
        let plan = PlannedFinalization::new(
            stores.world().effect_pos(),
            vec![DriverFile::new(driver_path.clone(), b"driver".to_vec())],
        )
        .expect("paths are distinct");

        assert!(plan.commit_effects(&mut stores).is_err());
        assert!(!driver_path.exists());
    }

    #[test]
    fn duplicate_driver_paths_are_rejected_before_finalization() {
        let stores = Universe::with_world(World::memory());
        let result = PlannedFinalization::new(
            stores.world().effect_pos(),
            vec![
                DriverFile::new(PathBuf::from("same.out"), vec![1]),
                DriverFile::new(PathBuf::from("same.out"), vec![2]),
            ],
        );
        assert!(matches!(
            result,
            Err(FinalizationError::ConflictingDriverPath(path)) if path == std::path::Path::new("same.out")
        ));
    }

    #[test]
    fn fixture_policy_preserves_effects_without_materializing_files() {
        let mut stores = Universe::with_world(World::memory());
        stores
            .world_mut()
            .write_text(PrintSink::Terminal, "fixture");
        let plan = PlannedFinalization::new(
            stores.world().effect_pos(),
            vec![DriverFile::new(PathBuf::from("fixture.dvi"), vec![1])],
        )
        .expect("path is unique");

        plan.discard_uncommitted();

        assert_eq!(stores.world().effect_records().len(), 1);
        assert_eq!(stores.world().memory_output("fixture.dvi"), None);
    }
}
