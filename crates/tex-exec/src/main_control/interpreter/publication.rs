//! Publication consumes genuine scanner requests into native token owners.
use super::*;
use tex_command::TokenRequest;

pub(super) fn publish_tokens<G, T: TokenRequest<tex_command::AttemptTokenListId>>(
    command: &mut CommandState<G>,
    stores: &mut CommandContext<'_, G>,
    request: T,
) -> Result<T::Mapped<tex_state::TokenListId<G>>, ExecError> {
    let mut rows = 0usize;
    let mut words = 0usize;
    let mut invalid = false;
    request.visit_tokens(&mut |source| match command.attempt_token_words(source) {
        Ok(tokens) => {
            if let (Some(next_rows), Some(next_words)) =
                (rows.checked_add(1), words.checked_add(tokens.len()))
            {
                rows = next_rows;
                words = next_words;
            } else {
                invalid = true;
            }
        }
        Err(_) => invalid = true,
    });
    if invalid {
        return Err(ExecError::MissingToken {
            context: "validated scanner token roots",
        });
    }
    stores
        .reserve_token_lists(rows, words)
        .map_err(|_| ExecError::MissingToken {
            context: "scanner token destination capacity",
        })?;
    Ok(request.map_tokens(&mut |source| {
        let words = command
            .attempt_token_words(&source)
            .expect("all scanner roots validated before publication");
        let published = stores
            .allocate_reserved_token_list(words.iter().map(|word| word.token_word()))
            .expect("all native destinations reserved before publication");
        command
            .discard_attempt_tokens(source)
            .expect("validated source consumed");
        published
    }))
}
pub(super) fn publish_alignment_preamble<G>(
    command: &mut CommandState<G>,
    stores: &mut CommandContext<'_, G>,
    active: &mut ActiveReplayAlignment<G>,
) -> Result<(), ExecError> {
    let preamble = command
        .take_completed_alignment_preamble(active.identity)
        .map_err(|_| ExecError::MissingToken {
            context: "completed alignment preamble",
        })?;
    let mut invalid = false;
    let mut rows = 0usize;
    let mut words = 0usize;
    for column in &preamble.columns {
        for source in column
            .u_template
            .as_ref()
            .into_iter()
            .chain(std::iter::once(&column.v_template))
        {
            match command.attempt_token_words(source) {
                Ok(tokens) => {
                    if let (Some(r), Some(w)) =
                        (rows.checked_add(1), words.checked_add(tokens.len()))
                    {
                        rows = r;
                        words = w;
                    } else {
                        invalid = true;
                    }
                }
                Err(_) => invalid = true,
            }
        }
    }
    if invalid {
        return Err(ExecError::MissingToken {
            context: "validated alignment template roots",
        });
    }
    stores
        .reserve_token_lists(rows, words)
        .map_err(|_| ExecError::MissingToken {
            context: "scanner token destination capacity",
        })?;
    let mut publish = |source| {
        let words = command
            .attempt_token_words(&source)
            .expect("validated alignment source");
        let published = stores
            .allocate_reserved_token_list(words.iter().map(|word| word.token_word()))
            .expect("reserved alignment destination");
        command
            .discard_attempt_tokens(source)
            .expect("validated template consumed");
        published
    };
    active.columns = preamble
        .columns
        .into_iter()
        .map(|column| PreparedAlignmentCellTemplates {
            u_template: column.u_template.map(&mut publish),
            v_template: publish(column.v_template),
        })
        .collect();
    active.tabskips = preamble.tabskips;
    active.default_tabskip = preamble.default_tabskip;
    active.repeat_start = preamble.repeat_start;
    active.column = 0;
    if active.columns.is_empty() {
        return Err(ExecError::MissingToken {
            context: "first alignment preamble column",
        });
    }
    Ok(())
}
