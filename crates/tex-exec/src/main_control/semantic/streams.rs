//! Direct streams semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_input_stream<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    request: tex_command::InputStreamRequest<tex_state::PublishedDefinition<G>>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        match request {
            tex_command::InputStreamRequest::Open { .. }
            | tex_command::InputStreamRequest::Close { .. } => {
                unreachable!("input open/close executes at the MainControl outer barrier")
            }
            // TeX82 §482 has already collected the list inside the
            // command core, which also reported §1225's missing-`to`
            // recovery at the point tex.web reports it; the definition is
            // all that is left.
            tex_command::InputStreamRequest::Read {
                target,
                global,
                definition,
                ..
            } => {
                // TeX82 §1225 installs `read_toks`'s freshly allocated
                // macro through `define(p,call,cur_val)`, so e-TeX
                // [17.687-750] traces the same pre/post eqtb write as a
                // `\def`, immediately after collection and before the
                // next command is fetched.
                let record = command.observes_mutations().then(|| MutationRecord {
                    target: MutationTarget::Meaning,
                    key: ObservationValue::Name(stores.resolve(target).to_owned()),
                    value: ObservationValue::Tokens(observed_read_body(
                        &definition.reference(),
                        stores,
                    )),
                    global,
                });
                let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                    .unscoped_with_effects(record, |stores, diagnostic_effects| {
                        assignment_tracing::trace_meaning_write(
                            stores,
                            diagnostic_effects,
                            Token::Cs(target),
                            true,
                            global,
                            |stores| {
                                stores
                                    .assign_published_definition(
                                        target,
                                        definition,
                                        MeaningFlags::EMPTY,
                                        assignment_scope(global),
                                    )
                                    .expect("prepared read definition belongs to admitted state")
                            },
                        );
                    });
                command.retain_assignment_receipt(receipt);
            }
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_deferred_open_out<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    stream: &mut u8,
    file_name: &mut String,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        crate::box_runtime::append_whatsit(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
            Whatsit::OpenOut {
                slot: StreamSlot::new(*stream),
                path: std::mem::take(file_name),
            },
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_deferred_close_out<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    stream: &mut tex_command::WriteStreamSelector,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        crate::box_runtime::append_whatsit(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
            Whatsit::CloseOut {
                slot: stream.stream_slot(),
            },
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_deferred_write<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    stream: &mut tex_command::WriteStreamSelector,
    tokens: &mut tex_state::TokenListId<G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let tokens = stores.node_token_list(tokens);
        crate::box_runtime::append_whatsit(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
            Whatsit::DeferredWrite {
                sink: replay_write_sink(*stream),
                tokens,
            },
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_deferred_special<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    deferred: &mut bool,
    tokens: &mut tex_state::TokenListId<G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    match deferred {
        true => {
            let tokens = stores.node_token_list(tokens);
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::DeferredSpecial {
                    class: "dvi".to_owned(),
                    tokens,
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        false => {
            let mut text = String::new();
            for word in stores.token_list(tokens.clone()) {
                tex_state::token_show::append_token_string_text(
                    &**stores,
                    word.semantic_token(),
                    &mut text,
                );
            }
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::Special {
                    class: "dvi".to_owned(),
                    payload: tex_byte_text(&text),
                },
            )?;
            Ok(ReplayStep::Continue)
        }
    }
}
