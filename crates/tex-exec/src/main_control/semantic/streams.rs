//! Final read definitions publish into the actual assignment owner.
use super::super::*;

pub(in crate::main_control) fn execute_read_definition<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    receipts: &mut Option<Vec<MutationRecord>>,
    target: Symbol,
    definition: tex_state::PublishedDefinition<G>,
    global: bool,
) -> Result<MainControlStep, ExecError> {
    // TeX82 §1225 installs `read_toks`'s freshly allocated
    // macro through `define(p,call,cur_val)`, so e-TeX
    // [17.687-750] traces the same pre/post eqtb write as a
    // `\def`, immediately after collection and before the
    // next command is fetched.
    let record = receipts.is_some().then(|| MutationRecord {
        target: MutationTarget::Meaning,
        key: ObservationValue::Name(stores.resolve(target).to_owned()),
        value: ObservationValue::Tokens(observed_read_body(&definition.reference(), stores)),
        global,
    });
    AssignmentCommitter::new(stores, effects).unscoped_with_effects(
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
        receipts
            .as_mut()
            .expect("read mutation observation requested")
            .push(record);
    }
    Ok(MainControlStep::Continue)
}
