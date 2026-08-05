//! Immutable program and symbol values shared by compilation and later VM work.

use std::collections::BTreeMap;

#[cfg(test)]
use super::pool::{StringPoolLimits, StringPoolUsage};
use super::{ClassicStringPool, SourceLocation};

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

pub(crate) const BUILTIN_REGISTRY: [(Builtin, &str); 37] = [
    (Builtin::Equals, "="),
    (Builtin::GreaterThan, ">"),
    (Builtin::LessThan, "<"),
    (Builtin::Add, "+"),
    (Builtin::Subtract, "-"),
    (Builtin::Concatenate, "*"),
    (Builtin::Assign, ":="),
    (Builtin::AddPeriod, "add.period$"),
    (Builtin::CallType, "call.type$"),
    (Builtin::ChangeCase, "change.case$"),
    (Builtin::ChrToInt, "chr.to.int$"),
    (Builtin::Cite, "cite$"),
    (Builtin::Duplicate, "duplicate$"),
    (Builtin::Empty, "empty$"),
    (Builtin::FormatName, "format.name$"),
    (Builtin::If, "if$"),
    (Builtin::IntToChr, "int.to.chr$"),
    (Builtin::IntToStr, "int.to.str$"),
    (Builtin::Missing, "missing$"),
    (Builtin::Newline, "newline$"),
    (Builtin::NumNames, "num.names$"),
    (Builtin::Pop, "pop$"),
    (Builtin::Preamble, "preamble$"),
    (Builtin::Purify, "purify$"),
    (Builtin::Quote, "quote$"),
    (Builtin::Skip, "skip$"),
    (Builtin::Stack, "stack$"),
    (Builtin::Substring, "substring$"),
    (Builtin::Swap, "swap$"),
    (Builtin::TextLength, "text.length$"),
    (Builtin::TextPrefix, "text.prefix$"),
    (Builtin::Top, "top$"),
    (Builtin::Type, "type$"),
    (Builtin::Warning, "warning$"),
    (Builtin::While, "while$"),
    (Builtin::Width, "width$"),
    (Builtin::Write, "write$"),
];

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
    Call(Callable),
    Read(SymbolId),
    Assign(SymbolId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Callable {
    Function(FunctionId),
    Builtin(Builtin),
    Variable(SymbolId),
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

/// One observable Web2C dynamic-allocation transition while compiling a
/// classic style. The reference writes these records to the `.blg` stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Web2cReallocation {
    array: &'static str,
    element_size: usize,
    old_capacity: usize,
    new_capacity: usize,
}
impl Web2cReallocation {
    pub(crate) const fn new(
        array: &'static str,
        element_size: usize,
        old_capacity: usize,
        new_capacity: usize,
    ) -> Self {
        Self {
            array,
            element_size,
            old_capacity,
            new_capacity,
        }
    }
    #[must_use]
    pub const fn array(self) -> &'static str {
        self.array
    }
    #[must_use]
    pub const fn element_size(self) -> usize {
        self.element_size
    }
    #[must_use]
    pub const fn old_capacity(self) -> usize {
        self.old_capacity
    }
    #[must_use]
    pub const fn new_capacity(self) -> usize {
        self.new_capacity
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledStyle {
    declarations: Declarations,
    functions: Vec<CompiledFunction>,
    commands: Vec<CompiledCommand>,
    command_locations: Vec<SourceLocation>,
    charge: ProgramCharge,
    pool_trace: Vec<String>,
    web2c_reallocations: Vec<Web2cReallocation>,
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
    pub fn command_location(&self, index: usize) -> Option<SourceLocation> {
        self.command_locations.get(index).copied()
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
    #[cfg(test)]
    pub fn compiler_pool_usage(&self) -> StringPoolUsage {
        let mut pool = ClassicStringPool::new(StringPoolLimits::unlimited());
        self.apply_pool_trace(&mut pool);
        pool.usage()
    }
    /// Web2C allocation records in the order the reference emits while
    /// scanning this style. Cache hits replay these immutable effects.
    #[must_use]
    pub fn web2c_reallocations(&self) -> &[Web2cReallocation] {
        &self.web2c_reallocations
    }
    pub(crate) fn with_command_locations(
        declarations: Declarations,
        functions: Vec<CompiledFunction>,
        commands: Vec<CompiledCommand>,
        command_locations: Vec<SourceLocation>,
        charge: ProgramCharge,
        pool_trace: Vec<String>,
        web2c_reallocations: Vec<Web2cReallocation>,
    ) -> Self {
        debug_assert_eq!(commands.len(), command_locations.len());
        Self {
            declarations,
            functions,
            commands,
            command_locations,
            charge,
            pool_trace,
            web2c_reallocations,
        }
    }
}
pub(crate) fn fold(name: &str) -> String {
    name.to_ascii_lowercase()
}
pub(crate) fn builtin(name: &str) -> Option<Builtin> {
    let name = fold(name);
    BUILTIN_REGISTRY
        .iter()
        .find_map(|(builtin, candidate)| (*candidate == name).then_some(*builtin))
}
