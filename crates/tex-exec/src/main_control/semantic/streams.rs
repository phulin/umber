//! Direct streams semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_input_stream<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    request: tex_command::InputStreamRequest<G>,
) -> Result<MainControlStep, ExecError> {
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
                AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped_with_effects(
                    |stores, diagnostic_effects| {
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
                    },
                );
                if let Some(record) = record {
                    command.retain_assignment_mutation(record);
                }
            }
        }
        Ok(MainControlStep::Continue)
    }
}
