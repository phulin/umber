//! Retired `InputStack` editor restart adapter.
//!
//! Canonical editor sessions restore through
//! [`crate::EngineCheckpoint::fork_canonical_editor`]. This module exists only
//! for the synchronous compatibility path in `tex-incr`; keeping it separate
//! prevents lexer reconstruction from becoming part of the checkpoint schema.

use std::time::Duration;

pub(crate) use tex_lex::{InputSource, InputStack, LayoutCursor, MemoryInput, WorldInput};
use tex_state::{ContentHash, FragmentStore, GenerationSubstrate, Universe};

use crate::checkpoint::CheckpointContinuation;
use crate::timing::TelemetryTimer;
use crate::{EditorRestoreError, EngineCheckpoint, ModeNest};

impl crate::Executor {
    /// Binds an editor layout to the retired executor's root delivery state.
    pub fn install_editor_root_layout(
        &mut self,
        input: &mut InputStack,
        layout: &tex_state::EditorLayout,
        fragments: &FragmentStore,
    ) -> Result<(), EditorRestoreError> {
        let cursor =
            LayoutCursor::new(layout, fragments).map_err(EditorRestoreError::LayoutCursor)?;
        input
            .install_root_layout_cursor(cursor)
            .ok_or(EditorRestoreError::RootRevisionMismatch)?;
        Ok(())
    }

    /// Restores the retired editor path. Canonical sessions use command-owned
    /// checkpoint restoration and never enter this adapter.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::disallowed_methods)] // Diagnostic latency; no engine fact observes it.
    pub fn restore_editor_checkpoint(
        &mut self,
        input: &mut InputStack,
        universe: &mut Universe,
        substrate: &GenerationSubstrate,
        checkpoint: &EngineCheckpoint,
        old_source: &str,
        source: &str,
        fragments: &FragmentStore,
        layout: &tex_state::EditorLayout,
    ) -> Result<Duration, EditorRestoreError> {
        if checkpoint.root_content_hash
            != Some(tex_state::ContentHash::from_bytes(old_source.as_bytes()))
        {
            return Err(EditorRestoreError::RootRevisionMismatch);
        }
        if checkpoint.root_anchor > old_source.len()
            || checkpoint.root_anchor > source.len()
            || old_source.as_bytes()[..checkpoint.root_anchor]
                != source.as_bytes()[..checkpoint.root_anchor]
        {
            return Err(EditorRestoreError::ChangedRootPrefix);
        }
        let fork_started = TelemetryTimer::start();
        let mut restored_universe = substrate
            .fork_at(&checkpoint.universe)
            .map_err(EditorRestoreError::Fork)?;
        let fork_latency = fork_started.elapsed();
        restored_universe
            .install_editor_fragments(fragments, layout)
            .map_err(EditorRestoreError::Layout)?;
        let CheckpointContinuation::LegacyInput(input_summary) = &checkpoint.continuation else {
            return Err(EditorRestoreError::CanonicalContinuation);
        };
        let (summary, root_source) = restored_universe
            .rebind_root_editor_layout(input_summary, source.as_bytes(), checkpoint.root_anchor)
            .map_err(EditorRestoreError::RootRebind)?;
        let restored_modes =
            ModeNest::from_summary(checkpoint.modes.clone()).map_err(EditorRestoreError::Mode)?;
        let mut restored_input = InputStack::from_summary(&summary, |source_id, record, frame| {
            if source_id == root_source {
                let source = if frame.byte_projection() {
                    MemoryInput::byte_projection_from_offset(source, checkpoint.root_anchor)
                } else {
                    MemoryInput::from_offset(source, checkpoint.root_anchor)
                }
                .with_logical_path(layout.path());
                return Ok::<Box<dyn InputSource>, EditorRestoreError>(Box::new(source));
            }
            let Some(record) = record else {
                return Err(EditorRestoreError::IncludedInputUnavailable(source_id));
            };
            let content = restored_universe
                .world()
                .recorded_input_content(record)
                .ok_or(EditorRestoreError::IncludedInputUnavailable(source_id))?;
            Ok(Box::new(WorldInput::from_content_at_offset(
                content,
                frame.next_source_offset(),
            )))
        })?;
        let installed_root = restored_input
            .install_root_layout_cursor(
                LayoutCursor::new(layout, fragments).map_err(EditorRestoreError::LayoutCursor)?,
            )
            .ok_or(EditorRestoreError::RootRevisionMismatch)?;
        debug_assert_eq!(installed_root, root_source);
        restored_universe.set_root_editor_content_hash(ContentHash::from_bytes(source.as_bytes()));
        restored_universe.set_input_summary(restored_input.summary());
        *universe = restored_universe;
        *input = restored_input;
        self.nest = restored_modes;
        self.budget_counters = checkpoint.budget_counters;
        Ok(fork_latency)
    }
}
