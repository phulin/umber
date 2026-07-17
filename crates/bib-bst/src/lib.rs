//! Pure, bounded compilation of classic BibTeX `.bst` programs.
//!
//! The compiler intentionally has no VFS or database dependency. A successful
//! result is immutable and can be safely retained by a classic-session cache.

mod cache;
mod compiler;
mod lexer;
mod pool;
mod program;

pub use cache::CompilationCache;
pub use compiler::{CompileResult, compile};
pub use pool::{
    ClassicStringPool, PoolStringId, StringPoolLimit, StringPoolLimits, StringPoolUsage,
};
pub use program::{
    Builtin, CompiledCommand, CompiledFunction, CompiledStyle, Declarations, FunctionId,
    Instruction, ProgramCharge, SpecialSymbol, StringId, Symbol, SymbolId, SymbolKind,
    Web2cReallocation,
};

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

/// The stable classic-0.99d compiler identity used by caches.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ClassicCompatibility;

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
    pub const fn byte(self) -> usize {
        self.byte
    }
    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }
    #[must_use]
    pub const fn column(self) -> usize {
        self.column
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
    pub const fn kind(&self) -> DiagnosticKind {
        self.kind
    }
    #[must_use]
    pub const fn location(&self) -> SourceLocation {
        self.location
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
