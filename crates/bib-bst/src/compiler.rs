//! Stateful top-level parsing, validation, and lowering.

use std::sync::Arc;

use crate::lexer::{Token, TokenKind, lex};
use crate::program::{
    Builtin, CompiledCommand, CompiledFunction, CompiledStyle, Declarations, FunctionId,
    Instruction, ProgramCharge, SpecialSymbol, SymbolId, SymbolKind, builtin, fold,
};
use crate::{CompileLimits, CompileStats, Diagnostic, DiagnosticKind, SourceLocation};

#[derive(Clone, Debug)]
pub struct CompileResult {
    program: Option<Arc<CompiledStyle>>,
    diagnostics: Vec<Diagnostic>,
    stats: CompileStats,
}
impl CompileResult {
    #[must_use]
    pub fn program(&self) -> Option<&Arc<CompiledStyle>> {
        self.program.as_ref()
    }
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    #[must_use]
    pub const fn stats(&self) -> CompileStats {
        self.stats
    }
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.program.is_some()
    }
    pub(crate) fn cached(program: Arc<CompiledStyle>) -> Self {
        let charge = program.charge();
        Self {
            program: Some(program),
            diagnostics: Vec::new(),
            stats: CompileStats {
                cache_hit: true,
                tokens: charge.tokens,
                nesting: charge.nesting,
                work: charge.work,
            },
        }
    }
}

#[must_use]
pub fn compile(bytes: &[u8], limits: CompileLimits) -> CompileResult {
    if bytes.len() > limits.bytes {
        return CompileResult {
            program: None,
            diagnostics: vec![Diagnostic::new(
                DiagnosticKind::Limit,
                SourceLocation::new(0, 1, 1),
                "BST byte limit exceeded",
            )],
            stats: CompileStats::default(),
        };
    }
    let lexed = lex(bytes, limits);
    let mut compiler = Compiler::new(
        lexed.tokens,
        lexed.diagnostics,
        limits,
        bytes.len(),
        lexed.nesting,
        lexed.work,
    );
    compiler.parse();
    compiler.finish()
}

struct Compiler {
    tokens: Vec<Token>,
    at: usize,
    limits: CompileLimits,
    diagnostics: Vec<Diagnostic>,
    declarations: Declarations,
    functions: Vec<CompiledFunction>,
    commands: Vec<CompiledCommand>,
    entry_seen: bool,
    read_seen: bool,
    source_bytes: usize,
    nesting: usize,
    work: usize,
    pool_trace: Vec<String>,
}
impl Compiler {
    fn new(
        tokens: Vec<Token>,
        diagnostics: Vec<Diagnostic>,
        limits: CompileLimits,
        source_bytes: usize,
        nesting: usize,
        work: usize,
    ) -> Self {
        let mut declarations = Declarations::default();
        for (name, builtin) in builtin_names() {
            let _ = declarations.insert(name, SymbolKind::Builtin(builtin));
        }
        for (name, special) in [
            ("crossref", SpecialSymbol::Crossref),
            ("sort.key$", SpecialSymbol::SortKey),
            ("entry.max$", SpecialSymbol::EntryMax),
            ("global.max$", SpecialSymbol::GlobalMax),
        ] {
            let _ = declarations.insert(name, SymbolKind::Special(special));
        }
        Self {
            tokens,
            at: 0,
            limits,
            diagnostics,
            declarations,
            functions: Vec::new(),
            commands: Vec::new(),
            entry_seen: false,
            read_seen: false,
            source_bytes,
            nesting,
            work,
            pool_trace: Vec::new(),
        }
    }
    fn parse(&mut self) {
        while self.at < self.tokens.len() && !self.at_limit() {
            let Some((name, location)) = self.identifier() else {
                self.error_here(DiagnosticKind::UnknownCommand, "expected BST command");
                self.recover();
                continue;
            };
            match fold(&name).as_str() {
                "entry" => self.entry(location),
                "integers" => self.variables(location, true),
                "strings" => self.variables(location, false),
                "macro" => self.macro_command(location),
                "function" => self.function(location),
                "read" => self.read(location),
                "execute" => self.invoke(location, Invoke::Execute),
                "iterate" => self.invoke(location, Invoke::Iterate),
                "reverse" => self.invoke(location, Invoke::Reverse),
                "sort" => self.sort(location),
                _ => {
                    self.error(
                        DiagnosticKind::UnknownCommand,
                        location,
                        "unknown BST command",
                    );
                    self.recover();
                }
            }
        }
    }
    fn entry(&mut self, location: SourceLocation) {
        if self.entry_seen || self.read_seen {
            self.error(
                DiagnosticKind::Phase,
                location,
                "ENTRY must occur once before READ",
            );
            return;
        }
        self.entry_seen = true;
        let fields = self.identifiers_group();
        let integers = self.identifiers_group();
        let strings = self.identifiers_group();
        for name in fields {
            self.declare(
                &name,
                SymbolKind::EntryField(self.declarations.entry_fields().len() as u32),
                location,
                |d, id| d.add_entry_field(id),
            );
        }
        for name in integers {
            self.declare(
                &name,
                SymbolKind::EntryInteger(self.declarations.entry_integers().len() as u32),
                location,
                |d, id| d.add_entry_integer(id),
            );
        }
        for name in strings {
            self.declare(
                &name,
                SymbolKind::EntryString(self.declarations.entry_strings().len() as u32),
                location,
                |d, id| d.add_entry_string(id),
            );
        }
    }
    fn variables(&mut self, location: SourceLocation, integer: bool) {
        for name in self.identifiers_group() {
            if integer {
                self.declare(
                    &name,
                    SymbolKind::GlobalInteger(self.declarations.global_integers().len() as u32),
                    location,
                    |d, id| d.add_global_integer(id),
                );
            } else {
                self.declare(
                    &name,
                    SymbolKind::GlobalString(self.declarations.global_strings().len() as u32),
                    location,
                    |d, id| d.add_global_string(id),
                );
            }
        }
    }
    fn macro_command(&mut self, location: SourceLocation) {
        if self.read_seen {
            self.error(DiagnosticKind::Phase, location, "MACRO must precede READ");
            return;
        }
        let Some(name) = self.one_identifier_group() else {
            self.recover();
            return;
        };
        let Some(value) = self.one_string_group() else {
            self.recover();
            return;
        };
        self.pool_trace.push(value.clone());
        let id = self.declarations.add_string(value);
        self.declare(&name, SymbolKind::StringMacro(id), location, |_, _| {});
    }
    fn function(&mut self, location: SourceLocation) {
        let Some(name) = self.one_identifier_group() else {
            self.recover();
            return;
        };
        if self.declarations.lookup(&name).is_some() {
            self.error(
                DiagnosticKind::Shadowing,
                location,
                "function shadows an existing BST symbol",
            );
            self.skip_group();
            return;
        }
        if self.functions.len() >= self.limits.functions {
            self.error(
                DiagnosticKind::Limit,
                location,
                "BST function limit exceeded",
            );
            self.skip_group();
            return;
        }
        let id = FunctionId(self.functions.len() as u32);
        if self
            .declarations
            .insert(&name, SymbolKind::UserFunction(id))
            .is_err()
        {
            self.error(
                DiagnosticKind::DuplicateSymbol,
                location,
                "duplicate BST function",
            );
            self.skip_group();
            return;
        }
        self.pool_trace.push(fold(&name));
        let body = self.body_group();
        // Reserve the stable function ID before lowering nested anonymous
        // bodies. They are ordinary functions but must not displace the named
        // declaration that introduced them.
        self.functions
            .push(CompiledFunction::new(name.clone(), Vec::new()));
        let instructions = self.lower_body(body, id);
        self.functions[id.0 as usize] = CompiledFunction::new(name, instructions);
    }
    fn read(&mut self, location: SourceLocation) {
        if self.read_seen {
            self.error(DiagnosticKind::Phase, location, "READ may occur only once");
            return;
        }
        if !self.entry_seen {
            self.error(DiagnosticKind::Phase, location, "READ requires ENTRY first");
            return;
        }
        self.read_seen = true;
        self.commands.push(CompiledCommand::Read);
    }
    fn invoke(&mut self, location: SourceLocation, kind: Invoke) {
        if !self.read_seen {
            self.error(
                DiagnosticKind::Phase,
                location,
                "execution commands require READ first",
            );
            return;
        }
        let Some(name) = self.one_identifier_group() else {
            self.recover();
            return;
        };
        let Some(id) = self.function_id(&name, location) else {
            return;
        };
        self.commands.push(match kind {
            Invoke::Execute => CompiledCommand::Execute(id),
            Invoke::Iterate => CompiledCommand::Iterate(id),
            Invoke::Reverse => CompiledCommand::Reverse(id),
        });
    }
    fn sort(&mut self, location: SourceLocation) {
        if !self.read_seen {
            self.error(DiagnosticKind::Phase, location, "SORT requires READ first");
        } else {
            self.commands.push(CompiledCommand::Sort);
        }
    }
    fn lower_body(&mut self, body: Vec<Token>, current: FunctionId) -> Vec<Instruction> {
        let mut instructions = Vec::new();
        let mut at = 0;
        while at < body.len() {
            self.work += 1;
            if self.work > self.limits.work || instructions.len() >= self.limits.instructions {
                self.error(
                    DiagnosticKind::Limit,
                    body[at].location,
                    "BST instruction or work limit exceeded",
                );
                break;
            }
            match &body[at].kind {
                TokenKind::Integer(value) => instructions.push(Instruction::PushInteger(*value)),
                TokenKind::String(value) => {
                    self.pool_trace.push(value.clone());
                    let id = self.declarations.add_string(value.clone());
                    instructions.push(Instruction::PushString(id));
                }
                TokenKind::Quote => {
                    at += 1;
                    let Some(token) = body.get(at) else {
                        self.error(
                            DiagnosticKind::Syntax,
                            body[at - 1].location,
                            "quote requires a symbol",
                        );
                        break;
                    };
                    let TokenKind::Identifier(name) = &token.kind else {
                        self.error(
                            DiagnosticKind::Syntax,
                            token.location,
                            "quote requires an identifier",
                        );
                        at += 1;
                        continue;
                    };
                    match self.declarations.lookup(name).and_then(|id| {
                        self.declarations
                            .symbol(id)
                            .map(|symbol| (id, symbol.kind().clone()))
                    }) {
                        Some((_, SymbolKind::UserFunction(id))) => {
                            instructions.push(Instruction::PushFunction(id))
                        }
                        Some((_, SymbolKind::Builtin(builtin))) => {
                            if let Some(id) = self.builtin_function(builtin, name, token.location) {
                                instructions.push(Instruction::PushFunction(id));
                            }
                        }
                        Some((
                            id,
                            SymbolKind::EntryField(_)
                            | SymbolKind::StringMacro(_)
                            | SymbolKind::Special(SpecialSymbol::Crossref)
                            | SymbolKind::Special(SpecialSymbol::EntryMax)
                            | SymbolKind::Special(SpecialSymbol::GlobalMax),
                        )) => {
                            if let Some(function) = self.read_function(id, name, token.location) {
                                instructions.push(Instruction::PushFunction(function));
                            }
                        }
                        Some((
                            id,
                            SymbolKind::GlobalInteger(_)
                            | SymbolKind::GlobalString(_)
                            | SymbolKind::EntryInteger(_)
                            | SymbolKind::EntryString(_)
                            | SymbolKind::Special(SpecialSymbol::SortKey),
                        )) => instructions.push(Instruction::Assign(id)),
                        None => self.error(
                            DiagnosticKind::UnknownSymbol,
                            token.location,
                            "unknown quoted BST symbol",
                        ),
                    };
                }
                TokenKind::Identifier(name) => {
                    self.lower_identifier(name, body[at].location, current, &mut instructions)
                }
                TokenKind::OpenBrace => {
                    let (inner, end) = nested_body(&body, at);
                    if end.is_none() {
                        self.error(
                            DiagnosticKind::Syntax,
                            body[at].location,
                            "unterminated anonymous function",
                        );
                        break;
                    }
                    let id = self.anonymous(inner, current);
                    instructions.push(Instruction::PushFunction(id));
                    at = end.unwrap_or(at);
                }
                TokenKind::CloseBrace => self.error(
                    DiagnosticKind::Syntax,
                    body[at].location,
                    "unexpected close brace in function",
                ),
            }
            at += 1;
        }
        instructions
    }
    fn anonymous(&mut self, body: Vec<Token>, current: FunctionId) -> FunctionId {
        let id = FunctionId(self.functions.len() as u32);
        if self.functions.len() >= self.limits.functions {
            self.error(
                DiagnosticKind::Limit,
                SourceLocation::new(0, 1, 1),
                "BST function limit exceeded",
            );
            return current;
        }
        self.functions.push(CompiledFunction::new(
            format!("<anonymous:{}>", id.0),
            Vec::new(),
        ));
        let instructions = self.lower_body(body, current);
        self.functions[id.0 as usize] =
            CompiledFunction::new(format!("<anonymous:{}>", id.0), instructions);
        id
    }
    fn lower_identifier(
        &mut self,
        name: &str,
        location: SourceLocation,
        current: FunctionId,
        instructions: &mut Vec<Instruction>,
    ) {
        let Some(id) = self.declarations.lookup(name) else {
            self.error(
                DiagnosticKind::UnknownSymbol,
                location,
                "unknown BST symbol",
            );
            return;
        };
        let Some(kind) = self
            .declarations
            .symbol(id)
            .map(|symbol| symbol.kind().clone())
        else {
            return;
        };
        match kind {
            SymbolKind::Builtin(builtin) => instructions.push(Instruction::Builtin(builtin)),
            SymbolKind::UserFunction(function) if function == current => self.error(
                DiagnosticKind::IllegalRecursion,
                location,
                "recursive BST function definition",
            ),
            SymbolKind::UserFunction(function) => instructions.push(Instruction::Call(function)),
            _ => instructions.push(Instruction::Read(id)),
        }
    }
    fn identifiers_group(&mut self) -> Vec<String> {
        self.group()
            .into_iter()
            .filter_map(|token| match token.kind {
                TokenKind::Identifier(name) => Some(name),
                _ => {
                    self.error(
                        DiagnosticKind::Syntax,
                        token.location,
                        "declaration group requires identifiers",
                    );
                    None
                }
            })
            .collect()
    }
    fn one_identifier_group(&mut self) -> Option<String> {
        let values = self.identifiers_group();
        if values.len() == 1 {
            values.into_iter().next()
        } else {
            self.error_here(DiagnosticKind::Syntax, "expected exactly one identifier");
            None
        }
    }
    fn one_string_group(&mut self) -> Option<String> {
        let values = self.group();
        if values.len() == 1
            && let TokenKind::String(value) = &values[0].kind
        {
            return Some(value.clone());
        }
        self.error_here(DiagnosticKind::Syntax, "expected exactly one string");
        None
    }
    fn group(&mut self) -> Vec<Token> {
        if !self.open() {
            self.error_here(DiagnosticKind::Syntax, "expected BST brace group");
            return Vec::new();
        }
        let start = self.at;
        let mut depth = 1;
        while self.at < self.tokens.len() {
            match &self.tokens[self.at].kind {
                TokenKind::OpenBrace => depth += 1,
                TokenKind::CloseBrace => {
                    depth -= 1;
                    if depth == 0 {
                        let result = self.tokens[start..self.at].to_vec();
                        self.at += 1;
                        return result;
                    }
                }
                _ => {}
            }
            self.at += 1;
        }
        self.error_here(DiagnosticKind::Syntax, "unterminated BST brace group");
        Vec::new()
    }
    fn body_group(&mut self) -> Vec<Token> {
        self.group()
    }
    fn skip_group(&mut self) {
        let _ = self.group();
    }
    fn open(&mut self) -> bool {
        if self
            .tokens
            .get(self.at)
            .is_some_and(|token| matches!(token.kind, TokenKind::OpenBrace))
        {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn identifier(&mut self) -> Option<(String, SourceLocation)> {
        let token = self.tokens.get(self.at)?;
        let TokenKind::Identifier(name) = &token.kind else {
            return None;
        };
        self.at += 1;
        Some((name.clone(), token.location))
    }
    fn function_id(&mut self, name: &str, location: SourceLocation) -> Option<FunctionId> {
        let symbol = self
            .declarations
            .lookup(name)
            .and_then(|id| self.declarations.symbol(id))
            .map(|symbol| (symbol.name().to_owned(), symbol.kind().clone()));
        match symbol {
            Some((symbol_name, kind)) => match kind {
                SymbolKind::UserFunction(id) => Some(id),
                SymbolKind::Builtin(builtin) => {
                    self.builtin_function(builtin, &symbol_name, location)
                }
                _ => {
                    self.error(
                        DiagnosticKind::Syntax,
                        location,
                        "command requires a callable BST function",
                    );
                    None
                }
            },
            None => {
                self.error(
                    DiagnosticKind::UnknownSymbol,
                    location,
                    "unknown BST function",
                );
                None
            }
        }
    }
    fn builtin_function(
        &mut self,
        builtin: Builtin,
        name: &str,
        location: SourceLocation,
    ) -> Option<FunctionId> {
        if self.functions.len() >= self.limits.functions {
            self.error(
                DiagnosticKind::Limit,
                location,
                "BST function limit exceeded",
            );
            return None;
        }
        let id = FunctionId(self.functions.len() as u32);
        self.functions.push(CompiledFunction::new(
            format!("<builtin:{name}>"),
            vec![Instruction::Builtin(builtin)],
        ));
        Some(id)
    }
    fn read_function(
        &mut self,
        symbol: SymbolId,
        name: &str,
        location: SourceLocation,
    ) -> Option<FunctionId> {
        if self.functions.len() >= self.limits.functions {
            self.error(
                DiagnosticKind::Limit,
                location,
                "BST function limit exceeded",
            );
            return None;
        }
        let id = FunctionId(self.functions.len() as u32);
        self.functions.push(CompiledFunction::new(
            format!("<read:{name}>"),
            vec![Instruction::Read(symbol)],
        ));
        Some(id)
    }
    fn declare(
        &mut self,
        name: &str,
        kind: SymbolKind,
        location: SourceLocation,
        add: impl FnOnce(&mut Declarations, SymbolId),
    ) {
        if self.declarations.symbols().len() >= self.limits.symbols {
            self.error(DiagnosticKind::Limit, location, "BST symbol limit exceeded");
            return;
        }
        match self.declarations.insert(name, kind) {
            Ok(id) => {
                self.pool_trace.push(fold(name));
                add(&mut self.declarations, id)
            }
            Err(()) => self.error(
                DiagnosticKind::DuplicateSymbol,
                location,
                "duplicate or shadowing BST symbol",
            ),
        }
    }
    fn recover(&mut self) {
        // The command token was already consumed. Prefer a balanced argument
        // group as the recovery boundary; otherwise leave the next token for
        // the top-level loop so an adjacent valid command is not discarded.
        if self
            .tokens
            .get(self.at)
            .is_some_and(|token| matches!(token.kind, TokenKind::OpenBrace))
        {
            self.skip_group();
        }
    }
    fn error_here(&mut self, kind: DiagnosticKind, message: &str) {
        let location = self
            .tokens
            .get(self.at)
            .map_or(SourceLocation::new(self.source_bytes, 1, 1), |token| {
                token.location
            });
        self.error(kind, location, message);
    }
    fn error(&mut self, kind: DiagnosticKind, location: SourceLocation, message: &str) {
        if self.diagnostics.len() < self.limits.diagnostics {
            self.diagnostics
                .push(Diagnostic::new(kind, location, message));
        }
    }
    fn at_limit(&mut self) -> bool {
        self.work += 1;
        if self.work > self.limits.work {
            self.error_here(DiagnosticKind::Limit, "BST parser work limit exceeded");
            true
        } else {
            false
        }
    }
    fn finish(mut self) -> CompileResult {
        let instructions = self
            .functions
            .iter()
            .map(|function| function.instructions().len())
            .sum();
        let charge = ProgramCharge {
            source_bytes: self.source_bytes,
            tokens: self.tokens.len(),
            nesting: self.nesting,
            symbols: self.declarations.symbols().len(),
            functions: self.functions.len(),
            instructions,
            work: self.work,
            retained_bytes: self
                .source_bytes
                .saturating_add(instructions * std::mem::size_of::<Instruction>())
                .saturating_add(
                    self.declarations
                        .strings()
                        .iter()
                        .map(String::len)
                        .sum::<usize>(),
                ),
        };
        let success = self.diagnostics.is_empty() && charge.fits(self.limits);
        if !charge.fits(self.limits) {
            self.error(
                DiagnosticKind::Limit,
                SourceLocation::new(self.source_bytes, 1, 1),
                "compiled BST exceeds active limits",
            );
        }
        CompileResult {
            program: success.then(|| {
                Arc::new(CompiledStyle::new(
                    self.declarations,
                    self.functions,
                    self.commands,
                    charge,
                    self.pool_trace,
                ))
            }),
            diagnostics: self.diagnostics,
            stats: CompileStats {
                cache_hit: false,
                tokens: charge.tokens,
                nesting: charge.nesting,
                work: charge.work,
            },
        }
    }
}
#[derive(Clone, Copy)]
enum Invoke {
    Execute,
    Iterate,
    Reverse,
}
fn nested_body(tokens: &[Token], start: usize) -> (Vec<Token>, Option<usize>) {
    let mut depth = 1;
    let inner_start = start + 1;
    for (at, token) in tokens.iter().enumerate().skip(inner_start) {
        match &token.kind {
            TokenKind::OpenBrace => depth += 1,
            TokenKind::CloseBrace => {
                depth -= 1;
                if depth == 0 {
                    return (tokens[inner_start..at].to_vec(), Some(at));
                }
            }
            _ => {}
        }
    }
    (Vec::new(), None)
}
fn builtin_names() -> impl Iterator<Item = (&'static str, Builtin)> {
    [
        "=",
        ">",
        "<",
        "+",
        "-",
        "*",
        ":=",
        "add.period$",
        "call.type$",
        "change.case$",
        "chr.to.int$",
        "cite$",
        "duplicate$",
        "empty$",
        "format.name$",
        "if$",
        "int.to.chr$",
        "int.to.str$",
        "missing$",
        "newline$",
        "num.names$",
        "pop$",
        "preamble$",
        "purify$",
        "quote$",
        "skip$",
        "stack$",
        "substring$",
        "swap$",
        "text.length$",
        "text.prefix$",
        "top$",
        "type$",
        "warning$",
        "while$",
        "width$",
        "write$",
    ]
    .into_iter()
    .filter_map(|name| builtin(name).map(|builtin| (name, builtin)))
}
impl ProgramCharge {
    pub(crate) fn fits(self, limits: CompileLimits) -> bool {
        self.source_bytes <= limits.bytes
            && self.tokens <= limits.tokens
            && self.nesting <= limits.nesting
            && self.symbols <= limits.symbols
            && self.functions <= limits.functions
            && self.instructions <= limits.instructions
            && self.work <= limits.work
            && self.retained_bytes <= limits.retained_cache_bytes
    }
}
