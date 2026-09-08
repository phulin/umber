//! Host-neutral immutable resource protocol for retained canonical execution.

use std::path::Path;
use std::sync::Arc;

use tex_command::{
    ResourceNeed, ResourceOutcome, ResourceProvider, ResourceReplayEffect, ResourceResolution,
};
use tex_state::{
    FileContent, FileModificationDate, InputDependency, InputDependencyAccess,
    InputDependencyOutcome, InputOrigin, InputReadState, SharedBytes, Universe, WorldError,
};

trait ResourceWorldBackend {
    fn with_input_read_state(&mut self, operation: &mut dyn FnMut(&mut dyn InputReadState));
    fn read_file(&mut self, path: &Path) -> Result<FileContent, WorldError>;
    fn read_same_run_output_file(&mut self, path: &Path)
    -> Result<Option<FileContent>, WorldError>;
    fn register_selected_file(
        &mut self,
        path: &Path,
        bytes: Arc<[u8]>,
    ) -> Result<FileContent, WorldError>;
}

impl<G> ResourceWorldBackend for Universe<G> {
    fn with_input_read_state(&mut self, operation: &mut dyn FnMut(&mut dyn InputReadState)) {
        operation(&mut self.input_open_context());
    }

    fn read_file(&mut self, path: &Path) -> Result<FileContent, WorldError> {
        self.world_mut().read_file(path)
    }

    fn read_same_run_output_file(
        &mut self,
        path: &Path,
    ) -> Result<Option<FileContent>, WorldError> {
        self.world_mut().read_same_run_output_file(path)
    }

    fn register_selected_file(
        &mut self,
        path: &Path,
        bytes: Arc<[u8]>,
    ) -> Result<FileContent, WorldError> {
        self.input_open_context()
            .read_supplied_input_file(path, bytes.into())
    }
}

impl<G> ResourceWorldBackend for &mut Universe<G> {
    fn with_input_read_state(&mut self, operation: &mut dyn FnMut(&mut dyn InputReadState)) {
        operation(&mut self.input_open_context());
    }

    fn read_file(&mut self, path: &Path) -> Result<FileContent, WorldError> {
        self.world_mut().read_file(path)
    }

    fn read_same_run_output_file(
        &mut self,
        path: &Path,
    ) -> Result<Option<FileContent>, WorldError> {
        self.world_mut().read_same_run_output_file(path)
    }

    fn register_selected_file(
        &mut self,
        path: &Path,
        bytes: Arc<[u8]>,
    ) -> Result<FileContent, WorldError> {
        self.input_open_context()
            .read_supplied_input_file(path, bytes.into())
    }
}

struct InputReadStateBackend<'a> {
    input: &'a mut dyn InputReadState,
}

impl ResourceWorldBackend for InputReadStateBackend<'_> {
    fn with_input_read_state(&mut self, operation: &mut dyn FnMut(&mut dyn InputReadState)) {
        operation(self.input);
    }

    fn read_file(&mut self, path: &Path) -> Result<FileContent, WorldError> {
        self.input.read_input_file(path)
    }

    fn read_same_run_output_file(
        &mut self,
        path: &Path,
    ) -> Result<Option<FileContent>, WorldError> {
        self.input.read_same_run_output_file(path)
    }

    fn register_selected_file(
        &mut self,
        path: &Path,
        bytes: Arc<[u8]>,
    ) -> Result<FileContent, WorldError> {
        self.input.read_supplied_input_file(path, bytes.into())
    }
}

pub struct ResourceWorld<'a> {
    backend: Box<dyn ResourceWorldBackend + 'a>,
    replay_effects: Vec<ResourceReplayEffect>,
}

impl<'a> ResourceWorld<'a> {
    #[must_use]
    pub fn new<G>(stores: &'a mut Universe<G>) -> Self {
        Self {
            backend: Box::new(stores),
            replay_effects: Vec::new(),
        }
    }

    /// Borrows only the admitted command episode's input World view. The
    /// resulting resource world cannot reach the surrounding Universe or any
    /// parser owner.
    #[must_use]
    pub fn from_input_read_state(input: &'a mut dyn InputReadState) -> Self {
        Self {
            backend: Box::new(InputReadStateBackend { input }),
            replay_effects: Vec::new(),
        }
    }

    /// Borrows the candidate's private world for resource bookkeeping.
    pub fn with_input_read_state<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn InputReadState) -> T,
    ) -> T {
        let mut operation = Some(operation);
        let mut result = None;
        let replay_effects = &mut self.replay_effects;
        self.backend.with_input_read_state(&mut |input| {
            let mut recording = RecordingInputReadState {
                input,
                replay_effects,
            };
            result = Some(operation
                .take()
                .expect("resource input operation runs once")(
                &mut recording
            ));
        });
        result.expect("resource input operation ran")
    }

    /// Takes semantic effects recorded while the host answered this request.
    /// The resource host owns the returned list until the answer is either
    /// applied or discarded by its surrounding transaction.
    pub fn take_replay_effects(&mut self) -> Vec<ResourceReplayEffect> {
        std::mem::take(&mut self.replay_effects)
    }

    pub fn read_file(&mut self, path: impl AsRef<Path>) -> Result<FileContent, WorldError> {
        self.backend.read_file(path.as_ref())
    }

    /// Reads an exact output path produced by this run, preserving output
    /// precedence without allowing a declined host search to read external
    /// input accidentally.
    pub fn read_same_run_output_file(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Option<FileContent>, WorldError> {
        self.backend.read_same_run_output_file(path.as_ref())
    }

    pub fn register_selected_file(
        &mut self,
        path: impl AsRef<Path>,
        bytes: Arc<[u8]>,
    ) -> Result<FileContent, WorldError> {
        self.backend.register_selected_file(path.as_ref(), bytes)
    }

    /// Records one resolver observation in the admitted World and retains the
    /// same fact in this provider call's replay-effect list.
    pub fn record_input_dependency(
        &mut self,
        path: impl AsRef<Path>,
        outcome: InputDependencyOutcome,
        access: InputDependencyAccess,
    ) -> Result<(), WorldError> {
        let path = path.as_ref().to_owned();
        self.with_input_read_state(|input| input.record_input_dependency(&path, outcome, access))
    }
}

struct RecordingInputReadState<'a> {
    input: &'a mut dyn InputReadState,
    replay_effects: &'a mut Vec<ResourceReplayEffect>,
}

impl InputReadState for RecordingInputReadState<'_> {
    fn read_input_file(&mut self, path: &Path) -> Result<FileContent, WorldError> {
        self.input.read_input_file(path)
    }

    fn read_pending_output_file(&mut self, path: &Path) -> Result<Option<FileContent>, WorldError> {
        self.input.read_pending_output_file(path)
    }

    fn read_same_run_output_file(
        &mut self,
        path: &Path,
    ) -> Result<Option<FileContent>, WorldError> {
        self.input.read_same_run_output_file(path)
    }

    fn read_supplied_input_file(
        &mut self,
        path: &Path,
        bytes: SharedBytes,
    ) -> Result<FileContent, WorldError> {
        self.input.read_supplied_input_file(path, bytes)
    }

    fn read_selected_input_file(
        &mut self,
        path: &Path,
        bytes: SharedBytes,
        modification_date: Option<FileModificationDate>,
        origin: InputOrigin,
        dependencies: &[InputDependency],
    ) -> Result<Option<FileContent>, WorldError> {
        self.input
            .read_selected_input_file(path, bytes, modification_date, origin, dependencies)
    }

    fn read_selected_input_record(
        &mut self,
        path: &Path,
        record_hint: Option<tex_state::InputRecordId>,
        bytes: SharedBytes,
        modification_date: Option<FileModificationDate>,
        origin: InputOrigin,
        dependencies: &[InputDependency],
    ) -> Result<Option<FileContent>, WorldError> {
        self.input.read_selected_input_record(
            path,
            record_hint,
            bytes,
            modification_date,
            origin,
            dependencies,
        )
    }

    fn record_input_dependency(
        &mut self,
        path: &Path,
        outcome: InputDependencyOutcome,
        access: InputDependencyAccess,
    ) -> Result<(), WorldError> {
        self.input.record_input_dependency(path, outcome, access)?;
        self.replay_effects
            .push(ResourceReplayEffect::InputDependency {
                path: path.to_owned(),
                outcome,
                access,
            });
        Ok(())
    }
}

pub trait ResourceHost {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome;
}

/// Adapter used by ordinary candidate execution. It gives the legacy host
/// resolver exactly one call through the command episode's narrow input view;
/// all payloads and dependency facts are owned before returning to command
/// processing. A declined or unavailable required input still checks the
/// exact same-run output namespace, whose precedence is owned by the engine
/// World rather than the host search policy.
pub struct ResourceHostProvider<'a> {
    host: &'a mut dyn ResourceHost,
}

impl<'a> ResourceHostProvider<'a> {
    #[must_use]
    pub fn new(host: &'a mut dyn ResourceHost) -> Self {
        Self { host }
    }
}

impl<G> ResourceProvider<G> for ResourceHostProvider<'_> {
    fn resolve(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        need: &ResourceNeed,
    ) -> ResourceResolution {
        let mut effects = Vec::new();
        let outcome = state.with_input_read_state(|input| {
            let mut world = ResourceWorld::from_input_read_state(input);
            let outcome = self.host.fulfill(&mut world, need);
            let outcome = if matches!(
                (&outcome, need),
                (
                    ResourceOutcome::Unavailable | ResourceOutcome::Declined,
                    ResourceNeed::Input { .. },
                )
            ) {
                let ResourceNeed::Input { name, .. } = need else {
                    unreachable!("same-run fallback only handles input needs")
                };
                match world.read_same_run_output_file(name) {
                    Ok(Some(content)) => ResourceOutcome::Fulfilled(
                        tex_command::ResourceFulfillment::world_input(name, content),
                    ),
                    Ok(None) => outcome,
                    Err(error) => ResourceOutcome::Failed(error.into()),
                }
            } else {
                outcome
            };
            effects = world.take_replay_effects();
            outcome
        });
        let dependencies = effects
            .iter()
            .map(ResourceReplayEffect::input_dependency)
            .collect();
        ResourceResolution::new(outcome, dependencies)
    }
}
