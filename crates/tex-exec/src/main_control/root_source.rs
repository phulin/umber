//! Root source identity and framing for the existing main-control owner.

use super::*;

impl<G> MainControl<G> {
    /// Registers and opens the one root source selected by the host before
    /// main control starts.  Source acquisition is deliberately
    /// complete before this call: the command state retains only immutable
    /// bytes and never reaches back into a host input stack.
    pub fn register_root_source(
        &mut self,
        source: SourceRegistration,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        debug_assert!(
            self.root_main_source.is_none(),
            "one MainControl has exactly one root main source"
        );
        let source = if source.role().is_some() {
            source
        } else {
            source.with_role(tex_command::SourceRole::RootDocument)
        };
        let id = self.command.register_source(source)?;
        // `id` was just allocated by this command state, so this can fail
        // only if the state implementation has violated its own invariant.
        self.command
            .open_registered_source(id)
            .expect("freshly registered source must be openable");
        self.root_main_source = Some(id);
        Ok(id)
    }

    /// Returns the physical root row owned by command input and therefore
    /// restorable by a named checkpoint. Compact source context can outlive
    /// that row for diagnostics, so it is not sufficient for restart.
    pub(super) fn restartable_root_source_identity(&self) -> Option<tex_state::SourceId> {
        self.command.live_physical_root_source_id()
    }

    /// Substitutes the edited root buffer after an aggregate checkpoint fork.
    /// The command input owner journals the old backing; this scalar root id
    /// changes only in the candidate's restored MainControl.
    #[doc(hidden)]
    pub fn rebind_root_source_for_editor(
        &mut self,
        bytes: std::sync::Arc<[u8]>,
        unchanged_prefix: usize,
    ) -> Result<(), SourceRegistrationError> {
        let accepted = self
            .restartable_root_source_identity()
            .expect("a rooted checkpoint retains its main source identity");
        let current = self
            .command
            .rebind_generated_source(accepted, bytes, unchanged_prefix)?;
        self.root_main_source = Some(current);
        Ok(())
    }

    /// Renders the registered root's §537 opening at the driver's startup
    /// boundary without advancing input.
    pub fn open_registered_root_framing(&mut self, stores: &mut Universe<G>) {
        let source = self
            .root_main_source
            .expect("root framing requires a registered root source");
        let Some(name) = self.command.live_file_framing_name(source) else {
            return;
        };
        stores
            .command_context()
            .expect("live generation")
            .print_file_open(name);
    }

    /// Selects whether exhaustion of the registered root ends an authored
    /// fragment or enters TeX82 §360's missing-`\end` handling.
    pub fn set_root_completion_policy(&mut self, policy: RootCompletionPolicy) {
        self.root_completion = policy;
    }

    /// Performs one §360 terminal-input episode after a complete job reaches
    /// root EOF without `\end` or `\dump`.
    ///
    /// One accepted line is installed as a real terminal source and returned
    /// to main control for ordinary tokenization and fuel accounting. Fatal
    /// EOF is latched as §93's terminal `End`, so no driver can advance the
    /// exhausted source again.
    pub(super) fn handle_root_end_of_input(&mut self, stores: &mut Universe<G>) -> ReplayStep {
        let previous_line_was_empty = self
            .terminal_line_was_empty
            .unwrap_or(self.startup_terminal_line.is_empty());
        let action = {
            let mut context = stores.command_context().expect("live generation");
            crate::job::prompt_for_more_input(
                &mut context,
                &self.startup_terminal_line,
                previous_line_was_empty,
            )
        };
        match action {
            crate::job::EndOfInputAction::Line(line) => {
                self.terminal_line_was_empty = Some(line.is_empty());
                self.command.set_terminal_context_line(&line);
                let source =
                    SourceRegistration::new(RegisteredSourceKind::Generated, line.into_bytes());
                let id = self
                    .command
                    .register_source(source)
                    .expect("finite command fuel bounds terminal source identities");
                self.command
                    .open_registered_source_as(id, tex_command::SourceNameClass::Terminal)
                    .expect("fresh terminal source must be openable");
                ReplayStep::Continue
            }
            crate::job::EndOfInputAction::Fatal(fatal) => self.succumb(fatal),
        }
    }

    /// Registers the startup root and immediately renders its §537 opening
    /// after the driver has opened the transcript.
    pub fn register_startup_root_source(
        &mut self,
        stores: &mut Universe<G>,
        source: SourceRegistration,
        startup_name: &str,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        let has_resolved_name = source.name().is_some();
        // The host supplies the already-acquired startup source, but TeX82
        // reached it through §§516--520's `end_name`. The following §537
        // `a_make_name_string` result is immediately flushed when it is last.
        let components = tex_command::FileNameComponents::from_tex_name(startup_name);
        let mut context = stores
            .command_context()
            .expect("startup accounting requires a live generation");
        for component in [&components.area, &components.name, &components.extension] {
            if !component.is_empty() {
                context.slow_make_string_pool_string(component);
            }
        }
        drop(context);
        let id = self.register_root_source(source)?;
        if has_resolved_name {
            self.open_registered_root_framing(stores);
        } else {
            crate::job::open_startup_input_after_log(stores, startup_name);
        }
        Ok(id)
    }

    /// Accounts for a host-retained root that bypassed §526's live filename
    /// scan but still crossed §537's opened-name boundary.
    pub fn record_retained_startup_strings(
        &mut self,
        stores: &mut Universe<G>,
        requested_name: &str,
        resolved_name: Option<&str>,
    ) {
        if self.initex {
            let components = tex_command::FileNameComponents::from_tex_name(requested_name);
            let stem = components.name;
            // §§534--536 retain the startup name component as `job_name`
            // and the transcript's opened name before §537 retains the
            // requested and host-resolved input names below.
            let mut context = stores
                .command_context()
                .expect("startup accounting requires a live generation");
            if !stem.is_empty() {
                context.make_string_pool_string(&stem);
                context.make_string_pool_string(&format!("{stem}.log"));
            }
        }
        stores
            .command_context()
            .expect("startup accounting requires a live generation")
            .make_string_pool_string(requested_name);
        if let Some(resolved_name) = resolved_name
            && resolved_name != requested_name
        {
            stores
                .command_context()
                .expect("startup accounting requires a live generation")
                .make_string_pool_string(resolved_name);
        }
    }
}
