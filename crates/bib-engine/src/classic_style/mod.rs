//! Pure, bounded compilation of classic BibTeX `.bst` programs.
//!
//! The compiler intentionally has no VFS or database dependency. A successful
//! result is immutable and can be safely retained by a classic-session cache.

mod cache;
mod compiler;
mod pool;
mod program;
mod read;
mod vm;

pub(crate) use cache::ClassicRuntimeCache;
pub(crate) use compiler::{CompileResult, compile};
pub(crate) use pool::ClassicStringPool;
pub(crate) use program::{
    BUILTIN_REGISTRY, Builtin, Callable, CompiledCommand, CompiledStyle, FunctionId, Instruction,
    SpecialSymbol, SymbolId, SymbolKind,
};
pub(crate) use read::{
    ClassicDatabase, ClassicDatabaseDiagnostic, ClassicDatabaseEntry, ClassicDatabaseSource,
};
pub(crate) use vm::{
    ClassicVmDiagnostic, ClassicVmDiagnosticKind, ClassicVmLimits, ClassicVmLogEvent,
    ClassicVmResult, execute_classic_style,
};

#[cfg(test)]
pub(crate) use pool::StringPoolLimit;
#[cfg(test)]
pub(crate) use read::prepare_classic_database;

/// Hard limits for one style compilation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CompileLimits {
    pub bytes: usize,
    pub tokens: usize,
    pub nesting: usize,
    pub symbols: usize,
    pub functions: usize,
    pub instructions: usize,
    pub diagnostics: usize,
    pub work: usize,
    pub retained_cache_bytes: usize,
}

impl Default for CompileLimits {
    fn default() -> Self {
        Self {
            bytes: 8 * 1024 * 1024,
            tokens: 1_000_000,
            nesting: 256,
            symbols: 100_000,
            functions: 100_000,
            instructions: 1_000_000,
            diagnostics: 1_000,
            work: 16 * 1024 * 1024,
            retained_cache_bytes: 64 * 1024 * 1024,
        }
    }
}

/// A byte/line source coordinate. Byte offsets are zero based; lines and
/// columns are one based and count source bytes, not Unicode scalar values.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceLocation {
    byte: usize,
    line: usize,
    column: usize,
}
impl SourceLocation {
    #[must_use]
    pub const fn new(byte: usize, line: usize, column: usize) -> Self {
        Self { byte, line, column }
    }
    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticKind {
    Syntax,
    UnknownCommand,
    Phase,
    DuplicateSymbol,
    Shadowing,
    UnknownSymbol,
    IllegalRecursion,
    Limit,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    kind: DiagnosticKind,
    location: SourceLocation,
    message: String,
}
impl Diagnostic {
    pub(crate) fn new(
        kind: DiagnosticKind,
        location: SourceLocation,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            location,
            message: message.into(),
        }
    }
    #[must_use]
    #[cfg(test)]
    pub const fn kind(&self) -> DiagnosticKind {
        self.kind
    }
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompileStats {
    pub cache_hit: bool,
    pub tokens: usize,
    pub nesting: usize,
    pub work: usize,
}

#[cfg(test)]
mod tests;
