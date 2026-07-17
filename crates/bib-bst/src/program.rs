//! Immutable program and symbol values shared by compilation and later VM work.

use std::collections::BTreeMap;

use crate::{ClassicStringPool, StringPoolLimits, StringPoolUsage};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FunctionId(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SymbolId(pub u32);
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StringId(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Builtin {
    Equals,
    GreaterThan,
    LessThan,
    Add,
    Subtract,
    Concatenate,
    Assign,
    AddPeriod,
    CallType,
    ChangeCase,
    ChrToInt,
    Cite,
    Duplicate,
    Empty,
    FormatName,
    If,
    IntToChr,
    IntToStr,
    Missing,
    Newline,
    NumNames,
    Pop,
    Preamble,
    Purify,
    Quote,
    Skip,
    Stack,
    Substring,
    Swap,
    TextLength,
    TextPrefix,
    Top,
    Type,
    Warning,
    While,
    Width,
    Write,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpecialSymbol {
    Crossref,
    SortKey,
    EntryMax,
    GlobalMax,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SymbolKind {
    Builtin(Builtin),
    UserFunction(FunctionId),
    EntryField(u32),
    EntryInteger(u32),
    EntryString(u32),
    GlobalInteger(u32),
    GlobalString(u32),
    StringMacro(StringId),
    Special(SpecialSymbol),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Symbol {
    name: String,
    kind: SymbolKind,
}
impl Symbol {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub const fn kind(&self) -> &SymbolKind {
        &self.kind
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Declarations {
    symbols: Vec<Symbol>,
    names: BTreeMap<String, SymbolId>,
    entry_fields: Vec<SymbolId>,
    entry_integers: Vec<SymbolId>,
    entry_strings: Vec<SymbolId>,
    global_integers: Vec<SymbolId>,
    global_strings: Vec<SymbolId>,
    strings: Vec<String>,
}
impl Declarations {
    #[must_use]
    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }
    #[must_use]
    pub fn symbol(&self, id: SymbolId) -> Option<&Symbol> {
        self.symbols.get(id.0 as usize)
    }
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<SymbolId> {
        self.names.get(&fold(name)).copied()
    }
    #[must_use]
    pub fn entry_fields(&self) -> &[SymbolId] {
        &self.entry_fields
    }
    #[must_use]
    pub fn entry_integers(&self) -> &[SymbolId] {
        &self.entry_integers
    }
    #[must_use]
    pub fn entry_strings(&self) -> &[SymbolId] {
        &self.entry_strings
    }
    #[must_use]
    pub fn global_integers(&self) -> &[SymbolId] {
        &self.global_integers
    }
    #[must_use]
    pub fn global_strings(&self) -> &[SymbolId] {
        &self.global_strings
    }
    #[must_use]
    pub fn strings(&self) -> &[String] {
        &self.strings
    }
    pub(crate) fn insert(&mut self, name: &str, kind: SymbolKind) -> Result<SymbolId, ()> {
        let name = fold(name);
        if self.names.contains_key(&name) {
            return Err(());
        }
        let id = SymbolId(self.symbols.len() as u32);
        self.names.insert(name.clone(), id);
        self.symbols.push(Symbol { name, kind });
        Ok(id)
    }
    pub(crate) fn add_string(&mut self, value: String) -> StringId {
        let id = StringId(self.strings.len() as u32);
        self.strings.push(value);
        id
    }
    pub(crate) fn add_entry_field(&mut self, id: SymbolId) {
        self.entry_fields.push(id);
    }
    pub(crate) fn add_entry_integer(&mut self, id: SymbolId) {
        self.entry_integers.push(id);
    }
    pub(crate) fn add_entry_string(&mut self, id: SymbolId) {
        self.entry_strings.push(id);
    }
    pub(crate) fn add_global_integer(&mut self, id: SymbolId) {
        self.global_integers.push(id);
    }
    pub(crate) fn add_global_string(&mut self, id: SymbolId) {
        self.global_strings.push(id);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Instruction {
    PushInteger(i64),
    PushString(StringId),
    PushFunction(FunctionId),
    Call(FunctionId),
    Read(SymbolId),
    Assign(SymbolId),
    Builtin(Builtin),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledFunction {
    name: String,
    instructions: Vec<Instruction>,
}
impl CompiledFunction {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }
    pub(crate) fn new(name: String, instructions: Vec<Instruction>) -> Self {
        Self { name, instructions }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompiledCommand {
    Read,
    Execute(FunctionId),
    Iterate(FunctionId),
    Reverse(FunctionId),
    Sort,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProgramCharge {
    pub source_bytes: usize,
    pub tokens: usize,
    pub nesting: usize,
    pub symbols: usize,
    pub functions: usize,
    pub instructions: usize,
    pub work: usize,
    pub retained_bytes: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledStyle {
    declarations: Declarations,
    functions: Vec<CompiledFunction>,
    commands: Vec<CompiledCommand>,
    charge: ProgramCharge,
    pool_trace: Vec<String>,
}
impl CompiledStyle {
    #[must_use]
    pub fn declarations(&self) -> &Declarations {
        &self.declarations
    }
    #[must_use]
    pub fn functions(&self) -> &[CompiledFunction] {
        &self.functions
    }
    #[must_use]
    pub fn commands(&self) -> &[CompiledCommand] {
        &self.commands
    }
    #[must_use]
    pub const fn charge(&self) -> ProgramCharge {
        self.charge
    }
    /// Replays compiler-owned declarations and literal values into the
    /// job-lifetime pool. The caller owns ordering with AUX and database
    /// ingestion; repeated values retain the first pool identity.
    pub fn apply_pool_trace(&self, pool: &mut ClassicStringPool) {
        for value in &self.pool_trace {
            let _ = pool.intern(value);
        }
    }
    /// Compiler-owned pool charge in isolation, useful for cache accounting
    /// and focused tests. Job summaries should replay into their shared pool.
    #[must_use]
    pub fn compiler_pool_usage(&self) -> StringPoolUsage {
        let mut pool = ClassicStringPool::new(StringPoolLimits::unlimited());
        self.apply_pool_trace(&mut pool);
        pool.usage()
    }
    pub(crate) fn new(
        declarations: Declarations,
        functions: Vec<CompiledFunction>,
        commands: Vec<CompiledCommand>,
        charge: ProgramCharge,
        pool_trace: Vec<String>,
    ) -> Self {
        Self {
            declarations,
            functions,
            commands,
            charge,
            pool_trace,
        }
    }
}
pub(crate) fn fold(name: &str) -> String {
    name.to_ascii_lowercase()
}
pub(crate) fn builtin(name: &str) -> Option<Builtin> {
    Some(match fold(name).as_str() {
        "=" => Builtin::Equals,
        ">" => Builtin::GreaterThan,
        "<" => Builtin::LessThan,
        "+" => Builtin::Add,
        "-" => Builtin::Subtract,
        "*" => Builtin::Concatenate,
        ":=" => Builtin::Assign,
        "add.period$" => Builtin::AddPeriod,
        "call.type$" => Builtin::CallType,
        "change.case$" => Builtin::ChangeCase,
        "chr.to.int$" => Builtin::ChrToInt,
        "cite$" => Builtin::Cite,
        "duplicate$" => Builtin::Duplicate,
        "empty$" => Builtin::Empty,
        "format.name$" => Builtin::FormatName,
        "if$" => Builtin::If,
        "int.to.chr$" => Builtin::IntToChr,
        "int.to.str$" => Builtin::IntToStr,
        "missing$" => Builtin::Missing,
        "newline$" => Builtin::Newline,
        "num.names$" => Builtin::NumNames,
        "pop$" => Builtin::Pop,
        "preamble$" => Builtin::Preamble,
        "purify$" => Builtin::Purify,
        "quote$" => Builtin::Quote,
        "skip$" => Builtin::Skip,
        "stack$" => Builtin::Stack,
        "substring$" => Builtin::Substring,
        "swap$" => Builtin::Swap,
        "text.length$" => Builtin::TextLength,
        "text.prefix$" => Builtin::TextPrefix,
        "top$" => Builtin::Top,
        "type$" => Builtin::Type,
        "warning$" => Builtin::Warning,
        "while$" => Builtin::While,
        "width$" => Builtin::Width,
        "write$" => Builtin::Write,
        _ => return None,
    })
}
