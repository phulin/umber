use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Profile {
    pub(crate) libs: Vec<Library>,
    pub(crate) threads: Vec<Thread>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Library {
    pub(crate) name: String,
    pub(crate) debug_name: String,
    pub(crate) code_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Thread {
    pub(crate) name: String,
    pub(crate) process_name: String,
    pub(crate) string_array: Vec<String>,
    pub(crate) resource_table: ResourceTable,
    pub(crate) func_table: FuncTable,
    pub(crate) frame_table: FrameTable,
    pub(crate) stack_table: StackTable,
    pub(crate) samples: Samples,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResourceTable {
    pub(crate) lib: Vec<Option<usize>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FuncTable {
    pub(crate) name: Vec<usize>,
    pub(crate) resource: Vec<Option<usize>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FrameTable {
    pub(crate) address: Vec<Option<u64>>,
    pub(crate) func: Vec<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StackTable {
    pub(crate) frame: Vec<usize>,
    pub(crate) prefix: Vec<Option<usize>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Samples {
    pub(crate) stack: Vec<Option<usize>>,
    pub(crate) weight: Option<Vec<Option<f64>>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SymbolSidecar {
    pub(crate) string_table: Vec<String>,
    pub(crate) data: Vec<LibrarySymbols>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LibrarySymbols {
    pub(crate) debug_name: String,
    pub(crate) code_id: Option<String>,
    pub(crate) symbol_table: Vec<SymbolEntry>,
    #[serde(default)]
    pub(crate) known_addresses: Vec<(u64, usize)>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SymbolEntry {
    pub(crate) rva: u64,
    pub(crate) size: u64,
    pub(crate) symbol: usize,
    pub(crate) frames: Option<Vec<SymbolFrame>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SymbolFrame {
    pub(crate) function: usize,
}
