//! Apply scanned streams values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_input_stream(
        &mut self,
        mut request: tex_command::InputStreamRequest<G>,
        mut resource: Option<SourceRegistration>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        self.control.resolve_input_stream_request(
            &mut request,
            &mut resource,
            self.stores,
            self.provider,
        )?;
        if let tex_command::InputStreamRequest::Open {
            stream, file_name, ..
        } = &request
        {
            let slot = replay_stream_slot(*stream);
            let packed_name = file_name.packed();
            let applied = (|| -> Result<ReplayStep, ExecError> {
                self.stores.world_mut().close_in(slot);
                if let Some(resource) = &resource {
                    if let Some(record) = resource.active_world_record() {
                        self.stores.world_mut().open_in_record(slot, record)?;
                    } else {
                        self.stores
                            .world_mut()
                            .set_memory_file(&packed_name, resource.bytes().to_vec())?;
                        let content = InputReadState::read_input_file(
                            &mut self.stores.input_open_context(),
                            std::path::Path::new(&packed_name),
                        )?;
                        self.stores.world_mut().open_in_content(slot, &content)?;
                    }
                }
                Ok(ReplayStep::Continue)
            })();
            return self.finish_host(applied);
        }
        if let tex_command::InputStreamRequest::Close { stream, .. } = &request {
            self.stores
                .world_mut()
                .close_in(replay_stream_slot(*stream));
            return self.finish_host(Ok(ReplayStep::Continue));
        }
        self.control.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_input_stream(stores, command, request)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_deferred_open_out(
        &mut self,
        mut stream: u8,
        mut file_name: String,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_deferred_open_out(
                    stores,
                    modes,
                    command,
                    &mut stream,
                    &mut file_name,
                )
            },
        )
    }

    pub(super) fn execute_deferred_close_out(
        &mut self,
        mut stream: tex_command::WriteStreamSelector,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_deferred_close_out(stores, modes, command, &mut stream)
            },
        )
    }

    pub(super) fn execute_deferred_write(
        &mut self,
        mut stream: tex_command::WriteStreamSelector,
        tokens: tex_command::AttemptTokenListId,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut tokens = publication::publish_tokens(
            self.control.command.state(),
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            tokens,
        )?;
        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_deferred_write(stores, modes, command, &mut stream, &mut tokens)
            },
        )
    }

    pub(super) fn execute_deferred_special(
        &mut self,
        mut deferred: bool,
        tokens: tex_command::AttemptTokenListId,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut tokens = publication::publish_tokens(
            self.control.command.state(),
            &mut self
                .stores
                .command_context()
                .expect("token publication admission"),
            tokens,
        )?;
        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_deferred_special(
                    stores,
                    modes,
                    command,
                    &mut deferred,
                    &mut tokens,
                )
            },
        )
    }
}
