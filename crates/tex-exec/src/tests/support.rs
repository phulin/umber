use super::*;

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

pub(super) struct EdefInputHooks;

impl ExpansionHooks<MemoryInput> for EdefInputHooks {
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        _input: &mut C,
        name: &str,
    ) -> Result<MemoryInput, String> {
        if name == "inc" {
            Ok(MemoryInput::new("OK"))
        } else {
            Err(format!("unexpected input {name}"))
        }
    }
}

pub(super) struct MemoryInputHooks {
    sources: HashMap<String, String>,
}

impl MemoryInputHooks {
    pub(super) fn new() -> Self {
        Self {
            sources: HashMap::new(),
        }
    }

    pub(super) fn with_source(mut self, name: &str, source: &str) -> Self {
        self.sources.insert(name.to_owned(), source.to_owned());
        self
    }
}

impl ExpansionHooks<MemoryInput> for MemoryInputHooks {
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
}
