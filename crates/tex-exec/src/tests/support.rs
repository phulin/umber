use super::*;
use std::path::PathBuf;
use tex_expand::ReadRecorder;
use tex_state::interner::Symbol;

pub(super) fn install_expandable(
    stores: &mut Universe,
    name: &str,
    primitive: ExpandablePrimitive,
) {
    let symbol = stores.intern(name);
    stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
}

pub(super) fn terminal_effect_text(stores: &Universe) -> String {
    let mut output = String::new();
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
    output
}

pub(super) fn stores_with_fonts() -> Universe {
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    const CMMI10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmmi10.tfm");
    const CMTT10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmtt10.tfm");
    const CMSY10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
    const CMEX10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmex10.tfm");

    let mut stores = Universe::with_world(tex_state::World::memory());
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

pub(crate) struct TestHooks {
    sources: AHashMap<String, String>,
    font_root: Option<PathBuf>,
}

impl TestHooks {
    pub(crate) fn new() -> Self {
        Self {
            sources: AHashMap::new(),
            font_root: None,
        }
    }

    pub(crate) fn with_source(mut self, name: &str, source: &str) -> Self {
        self.sources.insert(name.to_owned(), source.to_owned());
        self
    }

    pub(crate) fn with_font_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.font_root = Some(root.into());
        self
    }
}

impl ExpansionHooks<MemoryInput> for TestHooks {
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        _input: &mut C,
        name: &str,
    ) -> Result<MemoryInput, String> {
        self.sources
            .get(name)
            .map(|source| MemoryInput::new(source.clone()))
            .ok_or_else(|| format!("unexpected input {name}"))
    }

    fn open_font<C: tex_state::InputReadState>(
        &mut self,
        input: &mut C,
        path: &std::path::Path,
    ) -> Result<tex_state::FileContent, String> {
        let path = self
            .font_root
            .as_ref()
            .map_or_else(|| path.to_owned(), |root| root.join(path));
        input.read_input_file(&path).map_err(|err| err.to_string())
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
