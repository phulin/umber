use super::*;
use tex_expand::ReadRecorder;
use tex_state::interner::Symbol;

pub(super) fn terminal_effect_text(stores: &Universe) -> String {
    // Shipout (and job completion) materializes prior terminal *and* log
    // effects in the memory backend and removes them from the
    // rollback-capable live effect suffix. This helper already merges
    // `PrintSink::Log` writes from the live suffix -- §245's
    // `begin_diagnostic` redirects a `\tracingonline<=0` dump there, e.g.
    // `\showbox`/`\showlists`/`\showifs`/`\showgroups` -- so it must merge
    // the committed log transcript across that same boundary, or a dump that
    // survives to job completion (a `\showifs` before `\end`, for instance)
    // silently disappears from every test that reads this function instead
    // of `memory_log_output` directly.
    let mut output = stores
        .world()
        .memory_terminal_output()
        .map_or_else(String::new, |bytes| {
            String::from_utf8_lossy(bytes).into_owned()
        });
    for record in stores.world().effect_records() {
        if let EffectRecord::StreamWrite { sink, text } = record
            && matches!(
                sink,
                PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log
            )
        {
            output.push_str(text);
        }
    }
    if let Some(bytes) = stores.world().memory_log_output() {
        output.push_str(&String::from_utf8_lossy(bytes));
    }
    output
}

/// [`terminal_effect_text`] with tex.web §58's `max_print_line` breaks
/// removed, for a test whose subject is a message's content rather than its
/// layout. See [`tex_state::print::without_line_breaks`].
pub(super) fn terminal_effect_text_unbroken(stores: &Universe) -> String {
    tex_state::print::without_line_breaks(&terminal_effect_text(stores))
}

pub(super) fn stores_with_fonts() -> Universe {
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    const CMMI10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmmi10.tfm");
    const CMTT10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmtt10.tfm");
    const CMSY10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
    const CMEX10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmex10.tfm");

    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    // See `crate::test_harness`: these run non-interactive jobs.
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    stores
        .world_mut()
        .set_memory_file("cmmi10.tfm", CMMI10.to_vec())
        .expect("seed cmmi10");
    stores
        .world_mut()
        .set_memory_file("cmtt10.tfm", CMTT10.to_vec())
        .expect("seed cmtt10");
    stores
        .world_mut()
        .set_memory_file("cmsy10.tfm", CMSY10.to_vec())
        .expect("seed cmsy10");
    stores
        .world_mut()
        .set_memory_file("cmex10.tfm", CMEX10.to_vec())
        .expect("seed cmex10");
    stores
}

pub(super) fn font_meaning(stores: &Universe, name: &str) -> tex_state::ids::FontId {
    let symbol = stores.symbol(name).expect("font control sequence");
    match stores.meaning(symbol) {
        Meaning::Font(id) => id,
        meaning => panic!("expected font meaning, got {meaning:?}"),
    }
}

#[derive(Default)]
pub(crate) struct TestRecorder {
    pub(crate) meanings: Vec<Meaning>,
}

impl ReadRecorder for TestRecorder {
    fn record_meaning(&mut self, _symbol: Symbol, meaning: Meaning) {
        self.meanings.push(meaning);
    }
}
