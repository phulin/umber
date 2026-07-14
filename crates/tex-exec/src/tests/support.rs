use super::*;
use std::path::{Path, PathBuf};
use tex_expand::{InputResolver, ReadRecorder};
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

pub(crate) struct MemoryResolvers {
    input: WorldMemoryInputResolver,
    font: WorldFontResolver,
}

impl MemoryResolvers {
    pub(crate) fn new() -> Self {
        Self {
            input: WorldMemoryInputResolver,
            font: WorldFontResolver { root: None },
        }
    }

    pub(crate) fn with_font_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.font.root = Some(root.into());
        self
    }

    pub(crate) fn context(&mut self) -> crate::ExecutionContext<'_> {
        crate::ExecutionContext::with_resolvers("texput", &mut self.input, &mut self.font)
    }
}

struct WorldMemoryInputResolver;

impl InputResolver for WorldMemoryInputResolver {
    fn open_input(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        name: &str,
        _request_index: u64,
    ) -> Result<Box<dyn tex_lex::InputSource>, String> {
        let content = input
            .read_input_file(Path::new(name))
            .map_err(|error| error.to_string())?;
        Ok(Box::new(MemoryInput::new(
            String::from_utf8_lossy(content.bytes()).into_owned(),
        )))
    }
}

struct WorldFontResolver {
    root: Option<PathBuf>,
}

impl crate::FontResolver for WorldFontResolver {
    fn open_font(
        &mut self,
        input: &mut dyn tex_state::InputReadState,
        path: &std::path::Path,
        _request_index: u64,
    ) -> Result<tex_state::FileContent, String> {
        let path = self
            .root
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
