//! Shared state constructors for active crate-internal executor tests.

/// A universe in `\nonstopmode`, so recovery tests do not enter TeX's
/// interactive terminal dialog at the first recoverable error.
#[must_use]
pub(crate) fn universe() -> tex_state::Universe {
    with_nonstop_mode(tex_state::Universe::new())
}

/// [`universe`] with plain TeX category codes installed.
#[must_use]
pub(crate) fn universe_with_plain_catcodes() -> tex_state::Universe {
    with_nonstop_mode(tex_state::Universe::new_with_plain_catcodes())
}

fn with_nonstop_mode(mut universe: tex_state::Universe) -> tex_state::Universe {
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    universe
}

/// Publishes one detached source frame for fallback-only diagnostic tests.
pub(crate) fn publish_input_context(
    universe: &mut tex_state::Universe,
    line_number: usize,
    line: &str,
    cursor: usize,
) {
    let source_id = tex_state::SourceId::new(0);
    let registration = universe
        .register_input_source(
            source_id,
            tex_state::source_map::SourceDescriptor::generated(std::sync::Arc::from(
                line.as_bytes(),
            )),
        )
        .expect("fallback context source registers");
    let source = tex_state::SourceFrameSummary::new(
        0,
        line.len(),
        line_number,
        cursor + 1,
        tex_state::LexerState::MidLine,
        line.to_owned(),
        cursor,
        Vec::new(),
        false,
    )
    .with_registration(Some(registration));
    universe.set_input_summary(tex_state::InputSummary::new(
        vec![tex_state::InputFrameSummary::Source {
            source_id,
            input_record: None,
            source,
        }],
        None,
        None,
    ));
}
