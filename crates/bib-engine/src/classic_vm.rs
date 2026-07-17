//! Bounded execution of an immutable classic BibTeX style program.
//!
//! The VM owns all mutable state and never calls a style function through the
//! Rust stack. Text and name operations use the classic byte-oriented text
//! model: braces are structural, a top-level braced control sequence is one
//! text unit, and ordinary strings retain their original TeX spelling.

use bib_bst::{
    Builtin, CompiledCommand, CompiledStyle, FunctionId, Instruction, SourceLocation, SymbolId,
    SymbolKind,
};

use crate::ClassicDatabase;

pub(crate) const CLASSIC_BUILTINS: [(Builtin, &str); 37] = [
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

/// Values which can occur on the classic BibTeX operand stack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmValue {
    Integer(i64),
    String(String),
    Function(FunctionId),
    /// A quoted mutable symbol. It is an assignment target for `:=` and a
    /// deferred getter when consumed by a control-flow builtin.
    Variable(SymbolId),
    Missing,
}

/// Limits enforced independently for every VM run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClassicVmLimits {
    pub stack_values: usize,
    pub call_depth: usize,
    pub string_bytes: usize,
    pub bbl_bytes: usize,
    pub blg_bytes: usize,
    pub diagnostics: usize,
    pub work: usize,
}

impl Default for ClassicVmLimits {
    fn default() -> Self {
        Self {
            stack_values: 1_000,
            call_depth: 256,
            string_bytes: 8 * 1024 * 1024,
            bbl_bytes: 8 * 1024 * 1024,
            blg_bytes: 8 * 1024 * 1024,
            diagnostics: 1_000,
            work: 64 * 1024 * 1024,
        }
    }
}

/// Stable classes of VM diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassicVmDiagnosticKind {
    Warning,
    Underflow,
    WrongType,
    NoCurrentEntry,
    InvalidFunction,
    Limit,
    Arithmetic,
}

/// One ordered VM diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicVmDiagnostic {
    kind: ClassicVmDiagnosticKind,
    message: String,
    source: Option<SourceLocation>,
    entry: Option<String>,
}

impl ClassicVmDiagnostic {
    #[must_use]
    pub const fn kind(&self) -> ClassicVmDiagnosticKind {
        self.kind
    }
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
    pub(crate) const fn source(&self) -> Option<SourceLocation> {
        self.source
    }
    pub(crate) fn entry(&self) -> Option<&str> {
        self.entry.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClassicVmLogEvent {
    Stack(String),
    Diagnostic(ClassicVmDiagnostic),
}

/// Detached effects and audit state produced by a VM attempt.
///
/// A fatal execution retains partial effects for inspection, but `bbl` and
/// `blg` withhold them so callers cannot accidentally publish an artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicVmResult {
    fatal: bool,
    bbl: String,
    blg: String,
    diagnostics: Vec<ClassicVmDiagnostic>,
    log_events: Vec<ClassicVmLogEvent>,
    entry_order: Vec<String>,
    builtin_calls: [usize; CLASSIC_BUILTINS.len()],
    work: usize,
}

impl ClassicVmResult {
    #[must_use]
    pub const fn is_fatal(&self) -> bool {
        self.fatal
    }
    #[must_use]
    pub fn bbl(&self) -> Option<&str> {
        (!self.fatal).then_some(&self.bbl)
    }
    #[must_use]
    pub fn blg(&self) -> Option<&str> {
        (!self.fatal).then_some(&self.blg)
    }
    #[must_use]
    pub fn partial_bbl(&self) -> &str {
        &self.bbl
    }
    #[must_use]
    pub fn partial_blg(&self) -> &str {
        &self.blg
    }
    #[must_use]
    pub fn diagnostics(&self) -> &[ClassicVmDiagnostic] {
        &self.diagnostics
    }
    pub(crate) fn log_events(&self) -> &[ClassicVmLogEvent] {
        &self.log_events
    }
    #[must_use]
    pub fn entry_order(&self) -> &[String] {
        &self.entry_order
    }
    pub(crate) const fn builtin_calls(&self) -> &[usize; CLASSIC_BUILTINS.len()] {
        &self.builtin_calls
    }
    #[must_use]
    pub const fn work(&self) -> usize {
        self.work
    }
}

/// Executes the compiled command stream against already-prepared `READ` data.
#[must_use]
pub fn execute_classic_style(
    style: &CompiledStyle,
    database: &ClassicDatabase,
    limits: ClassicVmLimits,
) -> ClassicVmResult {
    let mut vm = Vm::new(style, database, limits);
    vm.run();
    vm.result()
}

#[derive(Clone, Copy)]
enum Frame {
    Function {
        function: FunctionId,
        pc: usize,
    },
    While {
        condition: Callable,
        body: Callable,
        state: WhileState,
    },
}

#[derive(Clone, Copy)]
enum Callable {
    Function(FunctionId),
    /// A quoted mutable symbol, invoked by reading its current value.
    Variable(SymbolId),
}

#[derive(Clone, Copy)]
enum WhileState {
    RunCondition,
    CheckCondition,
    RunBody,
}

struct EntryState {
    integers: Vec<i64>,
    strings: Vec<String>,
    sort_key: String,
}

struct Vm<'a> {
    style: &'a CompiledStyle,
    database: &'a ClassicDatabase,
    limits: ClassicVmLimits,
    stack: Vec<VmValue>,
    frames: Vec<Frame>,
    globals_i: Vec<i64>,
    globals_s: Vec<String>,
    entries: Vec<EntryState>,
    order: Vec<usize>,
    current: Option<usize>,
    bbl: String,
    blg: String,
    diagnostics: Vec<ClassicVmDiagnostic>,
    log_events: Vec<ClassicVmLogEvent>,
    builtin_calls: [usize; CLASSIC_BUILTINS.len()],
    work: usize,
    fatal: bool,
    instruction_source: Option<SourceLocation>,
}

impl<'a> Vm<'a> {
    fn new(
        style: &'a CompiledStyle,
        database: &'a ClassicDatabase,
        limits: ClassicVmLimits,
    ) -> Self {
        let declarations = style.declarations();
        let entry_count = database.entries().len();
        Self {
            style,
            database,
            limits,
            stack: Vec::new(),
            frames: Vec::new(),
            globals_i: vec![0; declarations.global_integers().len()],
            globals_s: vec![String::new(); declarations.global_strings().len()],
            entries: (0..entry_count)
                .map(|_| EntryState {
                    integers: vec![0; declarations.entry_integers().len()],
                    strings: vec![String::new(); declarations.entry_strings().len()],
                    sort_key: String::new(),
                })
                .collect(),
            order: (0..entry_count).collect(),
            current: None,
            bbl: String::new(),
            blg: String::new(),
            diagnostics: Vec::new(),
            log_events: Vec::new(),
            builtin_calls: [0; CLASSIC_BUILTINS.len()],
            work: 0,
            fatal: false,
            instruction_source: None,
        }
    }

    fn result(self) -> ClassicVmResult {
        let entry_order = self
            .order
            .iter()
            .map(|&index| {
                self.database
                    .entries()
                    .nth(index)
                    .expect("VM order is initialized from database entries")
                    .key()
                    .to_owned()
            })
            .collect();
        ClassicVmResult {
            fatal: self.fatal,
            bbl: self.bbl,
            blg: self.blg,
            diagnostics: self.diagnostics,
            log_events: self.log_events,
            entry_order,
            builtin_calls: self.builtin_calls,
            work: self.work,
        }
    }

    fn run(&mut self) {
        for (command_index, command) in self.style.commands().iter().enumerate() {
            if self.fatal {
                break;
            }
            if !self.charge() {
                break;
            }
            self.instruction_source = self.style.command_location(command_index);
            match *command {
                CompiledCommand::Read => {}
                CompiledCommand::Execute(function) => {
                    self.current = None;
                    self.call(function);
                }
                CompiledCommand::Iterate(function) => self.iterate(function, false),
                CompiledCommand::Reverse(function) => self.iterate(function, true),
                CompiledCommand::Sort => self.sort(),
            }
        }
        self.current = None;
    }

    fn iterate(&mut self, function: FunctionId, reverse: bool) {
        let order = self.order.clone();
        let iter: Box<dyn Iterator<Item = usize>> = if reverse {
            Box::new(order.into_iter().rev())
        } else {
            Box::new(order.into_iter())
        };
        for entry in iter {
            if self.fatal {
                break;
            }
            if !self.charge() {
                break;
            }
            self.current = Some(entry);
            self.call(function);
        }
        self.current = None;
    }

    fn sort(&mut self) {
        let Some(sort_symbol) = self.style.declarations().lookup("sort.key$") else {
            return;
        };
        let mut keyed = Vec::with_capacity(self.order.len());
        let order = self.order.clone();
        for entry in order {
            if !self.charge() {
                return;
            }
            self.current = Some(entry);
            let value = self.read(sort_symbol);
            keyed.push((value.into_sort_string().unwrap_or_default(), entry));
        }
        self.current = None;
        // Stable sorting preserves database order for equivalent keys.
        keyed.sort_by(|left, right| {
            let _ = self.charge();
            left.0.cmp(&right.0)
        });
        if !self.fatal {
            self.order = keyed.into_iter().map(|(_, entry)| entry).collect();
        }
    }

    fn call(&mut self, function: FunctionId) {
        if self.fatal {
            return;
        }
        if !self.push_function(function) {
            return;
        }
        while !self.frames.is_empty() {
            if !self.charge() {
                return;
            }
            let frame = *self.frames.last().expect("non-empty loop condition");
            match frame {
                Frame::Function { function, pc } => {
                    let Some(instruction) = self.style.functions()[function.0 as usize]
                        .instructions()
                        .get(pc)
                        .cloned()
                    else {
                        self.frames.pop();
                        continue;
                    };
                    let Frame::Function { pc, .. } = self
                        .frames
                        .last_mut()
                        .expect("function frame remains present")
                    else {
                        unreachable!("current frame is a function frame");
                    };
                    *pc += 1;
                    self.instruction(instruction);
                }
                Frame::While {
                    condition,
                    body,
                    state,
                } => self.while_frame(condition, body, state),
            }
            if self.fatal {
                return;
            }
        }
    }

    fn push_function(&mut self, function: FunctionId) -> bool {
        if function.0 as usize >= self.style.functions().len() {
            self.fail(
                ClassicVmDiagnosticKind::InvalidFunction,
                "invalid BST function",
            );
            return false;
        }
        if self.frames.len() >= self.limits.call_depth {
            self.fail(
                ClassicVmDiagnosticKind::Limit,
                "BST call-depth limit exceeded",
            );
            return false;
        }
        self.frames.push(Frame::Function { function, pc: 0 });
        true
    }

    fn while_frame(&mut self, condition: Callable, body: Callable, state: WhileState) {
        match state {
            WhileState::RunCondition => {
                self.set_while_state(WhileState::CheckCondition);
                self.call_callable(condition);
            }
            WhileState::CheckCondition => {
                let Some(condition) = self.pop_integer() else {
                    return;
                };
                if condition <= 0 {
                    self.frames.pop();
                } else {
                    self.set_while_state(WhileState::RunBody);
                    self.call_callable(body);
                }
            }
            WhileState::RunBody => self.set_while_state(WhileState::RunCondition),
        }
    }

    fn set_while_state(&mut self, state: WhileState) {
        let Some(Frame::While { state: current, .. }) = self.frames.last_mut() else {
            unreachable!("while continuation is the active frame");
        };
        *current = state;
    }

    fn instruction(&mut self, instruction: Instruction) {
        match instruction {
            Instruction::PushInteger(value) => self.push(VmValue::Integer(value)),
            Instruction::PushString(id) => self.push(VmValue::String(
                self.style
                    .declarations()
                    .strings()
                    .get(id.0 as usize)
                    .cloned()
                    .unwrap_or_default(),
            )),
            Instruction::PushFunction(function) => self.push(VmValue::Function(function)),
            Instruction::Call(function) => {
                self.push_function(function);
            }
            Instruction::Read(symbol) => {
                let value = self.read(symbol);
                self.push(value);
            }
            Instruction::Assign(symbol) => self.push(VmValue::Variable(symbol)),
            Instruction::Builtin(builtin) => self.builtin(builtin),
        }
    }

    fn builtin(&mut self, builtin: Builtin) {
        self.builtin_calls[builtin as usize] += 1;
        match builtin {
            Builtin::Duplicate => {
                if let Some(value) = self.stack.last().cloned() {
                    self.push(value)
                } else {
                    self.underflow()
                }
            }
            Builtin::Pop => {
                self.pop();
            }
            Builtin::Swap => {
                if self.stack.len() < 2 {
                    self.underflow()
                } else {
                    let at = self.stack.len();
                    self.stack.swap(at - 1, at - 2);
                }
            }
            Builtin::Skip => {}
            Builtin::Add | Builtin::Subtract => self.arithmetic(builtin),
            Builtin::Equals | Builtin::GreaterThan | Builtin::LessThan => self.compare(builtin),
            Builtin::Concatenate => self.concatenate(),
            Builtin::Assign => self.assign_from_stack(),
            Builtin::If => self.if_builtin(),
            Builtin::While => self.while_builtin(),
            Builtin::AddPeriod => self.add_period(),
            Builtin::CallType => self.call_type(),
            Builtin::ChangeCase => self.change_case(),
            Builtin::ChrToInt => self.chr_to_int(),
            Builtin::Write => {
                if let Some(value) = self.pop_string() {
                    self.effect(false, &value)
                }
            }
            Builtin::Newline => self.effect(false, "\n"),
            Builtin::Warning => {
                if let Some(value) = self.pop_string() {
                    self.report(ClassicVmDiagnosticKind::Warning, &value);
                    self.effect(true, &value);
                    self.effect(true, "\n");
                }
            }
            Builtin::Cite => match self.current_entry() {
                Some(entry) => self.push(VmValue::String(entry.key().to_owned())),
                None => self.no_entry(),
            },
            Builtin::Type => match self.current_entry() {
                Some(entry) => self.push(VmValue::String(entry.entry_type().to_owned())),
                None => self.no_entry(),
            },
            Builtin::Missing => {
                let value = self.pop();
                self.push(VmValue::Integer(i64::from(matches!(
                    value,
                    Some(VmValue::Missing)
                ))));
            }
            Builtin::Empty => match self.pop() {
                Some(VmValue::Missing) => self.push(VmValue::Integer(1)),
                Some(VmValue::String(value)) => {
                    self.push(VmValue::Integer(i64::from(is_classic_whitespace(&value))))
                }
                Some(_) => self.wrong_type(),
                None => {}
            },
            Builtin::IntToStr => {
                if let Some(value) = self.pop_integer() {
                    self.push(VmValue::String(value.to_string()));
                }
            }
            Builtin::IntToChr => self.int_to_chr(),
            Builtin::FormatName => self.format_name(),
            Builtin::NumNames => self.num_names(),
            Builtin::Quote => self.push(VmValue::String("\"".to_owned())),
            Builtin::Preamble => self.push(VmValue::String(self.database.preamble())),
            Builtin::Purify => self.purify(),
            Builtin::Substring => self.substring(),
            Builtin::TextLength => self.text_length(),
            Builtin::TextPrefix => self.text_prefix(),
            Builtin::Width => self.width(),
            Builtin::Stack => self.print_stack(),
            Builtin::Top => self.print_top(),
        }
    }

    fn concatenate(&mut self) {
        let right = self.pop_string();
        let left = self.pop_string();
        if let (Some(left), Some(right)) = (left, right) {
            self.push(VmValue::String(format!("{left}{right}")));
        }
    }

    fn add_period(&mut self) {
        let Some(value) = self.pop_string() else {
            return;
        };
        let last = value.trim_end_matches('}').chars().next_back();
        if value.is_empty() || matches!(last, Some('.' | '?' | '!')) {
            self.push(VmValue::String(value));
        } else {
            self.push(VmValue::String(format!("{value}.")));
        }
    }

    fn call_type(&mut self) {
        let Some(entry) = self.current_entry() else {
            self.no_entry();
            return;
        };
        let name = entry.entry_type();
        let function = self
            .function_named(name)
            .or_else(|| self.function_named("default.type"));
        if let Some(function) = function {
            self.push_function(function);
        }
    }

    fn function_named(&self, name: &str) -> Option<FunctionId> {
        let symbol = self.style.declarations().lookup(name)?;
        match self.style.declarations().symbol(symbol)?.kind() {
            SymbolKind::UserFunction(function) => Some(*function),
            _ => None,
        }
    }

    fn change_case(&mut self) {
        let spec = self.pop_string();
        let text = self.pop_string();
        let (Some(spec), Some(text)) = (spec, text) else {
            return;
        };
        let Some(mode) = CaseMode::parse(&spec) else {
            self.push(VmValue::String(text));
            return;
        };
        self.push(VmValue::String(change_case(&text, mode)));
    }

    fn chr_to_int(&mut self) {
        let Some(value) = self.pop_string() else {
            return;
        };
        let mut chars = value.chars();
        let value = match (chars.next(), chars.next()) {
            (Some(character), None) if (character as u32) <= 0x7f => character as i64,
            _ => 0,
        };
        self.push(VmValue::Integer(value));
    }

    fn int_to_chr(&mut self) {
        let Some(value) = self.pop_integer() else {
            return;
        };
        let value = u8::try_from(value).ok().filter(|value| *value <= 0x7f);
        self.push(VmValue::String(
            value.map(char::from).map_or_else(String::new, String::from),
        ));
    }

    fn num_names(&mut self) {
        let Some(value) = self.pop_string() else {
            return;
        };
        self.push(VmValue::Integer(split_names(&value).len() as i64));
    }

    fn format_name(&mut self) {
        let format = self.pop_string();
        let ordinal = self.pop_integer();
        let names = self.pop_string();
        let (Some(format), Some(ordinal), Some(names)) = (format, ordinal, names) else {
            return;
        };
        let value = usize::try_from(ordinal)
            .ok()
            .and_then(|ordinal| ordinal.checked_sub(1))
            .and_then(|index| {
                split_names(&names)
                    .get(index)
                    .map(|name| format_bib_name(name, &format))
            })
            .unwrap_or_default();
        self.push(VmValue::String(value));
    }

    fn purify(&mut self) {
        let Some(value) = self.pop_string() else {
            return;
        };
        self.push(VmValue::String(purify(&value)));
    }

    fn substring(&mut self) {
        let count = self.pop_integer();
        let start = self.pop_integer();
        let value = self.pop_string();
        let (Some(count), Some(start), Some(value)) = (count, start, value) else {
            return;
        };
        self.push(VmValue::String(classic_substring(&value, start, count)));
    }

    fn text_length(&mut self) {
        let Some(value) = self.pop_string() else {
            return;
        };
        self.push(VmValue::Integer(classic_text_units(&value).len() as i64));
    }

    fn text_prefix(&mut self) {
        let count = self.pop_integer();
        let value = self.pop_string();
        let (Some(count), Some(value)) = (count, value) else {
            return;
        };
        self.push(VmValue::String(classic_text_prefix(&value, count)));
    }

    fn width(&mut self) {
        let Some(value) = self.pop_string() else {
            return;
        };
        self.push(VmValue::Integer(classic_width(&value)));
    }

    fn print_stack(&mut self) {
        while let Some(value) = self.stack.pop() {
            let text = value.stack_text();
            self.effect(true, &format!("{text}\n"));
            self.log_events.push(ClassicVmLogEvent::Stack(text));
            if self.fatal {
                return;
            }
        }
    }

    fn print_top(&mut self) {
        if let Some(value) = self.pop() {
            self.effect(true, &format!("{value:?}\n"));
        }
    }

    fn if_builtin(&mut self) {
        let else_function = self.pop_callable();
        let then_function = self.pop_callable();
        let condition = self.pop_integer();
        if let (Some(else_function), Some(then_function), Some(condition)) =
            (else_function, then_function, condition)
        {
            self.call_callable(if condition > 0 {
                then_function
            } else {
                else_function
            });
        }
    }

    fn while_builtin(&mut self) {
        let body = self.pop_callable();
        let condition = self.pop_callable();
        let (Some(body), Some(condition)) = (body, condition) else {
            return;
        };
        if self.frames.len() >= self.limits.call_depth {
            self.fail(
                ClassicVmDiagnosticKind::Limit,
                "BST call-depth limit exceeded",
            );
        } else {
            self.frames.push(Frame::While {
                condition,
                body,
                state: WhileState::RunCondition,
            });
        }
    }

    fn arithmetic(&mut self, builtin: Builtin) {
        let right = self.pop_integer();
        let left = self.pop_integer();
        if let (Some(left), Some(right)) = (left, right) {
            let value = if builtin == Builtin::Add {
                left.checked_add(right)
            } else {
                left.checked_sub(right)
            };
            match value {
                Some(value) => self.push(VmValue::Integer(value)),
                None => self.fail(ClassicVmDiagnosticKind::Arithmetic, "BST integer overflow"),
            }
        }
    }

    fn compare(&mut self, builtin: Builtin) {
        let right = self.pop();
        let left = self.pop();
        let Some(right) = right else {
            return;
        };
        let Some(left) = left else {
            return;
        };
        let ordering = match (&left, &right) {
            (VmValue::Integer(left), VmValue::Integer(right)) => Some(left.cmp(right)),
            (VmValue::String(left), VmValue::String(right)) => Some(left.cmp(right)),
            _ if builtin == Builtin::Equals => {
                Some(std::cmp::Ordering::Equal).filter(|_| left == right)
            }
            _ => None,
        };
        match ordering {
            Some(ordering) => self.push(VmValue::Integer(i64::from(match builtin {
                Builtin::Equals => ordering == std::cmp::Ordering::Equal,
                Builtin::GreaterThan => ordering == std::cmp::Ordering::Greater,
                _ => ordering == std::cmp::Ordering::Less,
            }))),
            None => self.wrong_type(),
        }
    }

    fn assign_from_stack(&mut self) {
        let target = self.pop();
        let value = self.pop();
        let (Some(VmValue::Variable(target)), Some(value)) = (target, value) else {
            self.wrong_type();
            return;
        };
        self.assign(target, value);
    }

    fn assign(&mut self, symbol: SymbolId, value: VmValue) {
        let kind = self
            .style
            .declarations()
            .symbol(symbol)
            .map(|symbol| symbol.kind().clone());
        match kind {
            Some(SymbolKind::GlobalInteger(index)) => {
                self.set_integer_global(index as usize, value)
            }
            Some(SymbolKind::GlobalString(index)) => self.set_string_global(index as usize, value),
            Some(SymbolKind::EntryInteger(index)) => self.set_integer_entry(index as usize, value),
            Some(SymbolKind::EntryString(index)) => self.set_string_entry(index as usize, value),
            Some(SymbolKind::Special(bib_bst::SpecialSymbol::SortKey)) => self.set_sort_key(value),
            _ => self.wrong_type(),
        }
    }

    fn read(&mut self, symbol: SymbolId) -> VmValue {
        let Some(kind) = self
            .style
            .declarations()
            .symbol(symbol)
            .map(|symbol| symbol.kind().clone())
        else {
            self.fail(
                ClassicVmDiagnosticKind::InvalidFunction,
                "invalid BST symbol",
            );
            return VmValue::Missing;
        };
        match kind {
            SymbolKind::EntryField(_) => self
                .current_entry()
                .and_then(|entry| {
                    entry
                        .field(symbol)
                        .map(|value| VmValue::String(value.to_owned()))
                })
                .unwrap_or_else(|| {
                    if self.current.is_none() {
                        self.no_entry();
                    }
                    VmValue::Missing
                }),
            SymbolKind::EntryInteger(index) => self
                .current
                .map(|entry| VmValue::Integer(self.entries[entry].integers[index as usize]))
                .unwrap_or_else(|| {
                    self.no_entry();
                    VmValue::Missing
                }),
            SymbolKind::EntryString(index) => self
                .current
                .map(|entry| VmValue::String(self.entries[entry].strings[index as usize].clone()))
                .unwrap_or_else(|| {
                    self.no_entry();
                    VmValue::Missing
                }),
            SymbolKind::GlobalInteger(index) => VmValue::Integer(self.globals_i[index as usize]),
            SymbolKind::GlobalString(index) => {
                VmValue::String(self.globals_s[index as usize].clone())
            }
            SymbolKind::StringMacro(index) => {
                VmValue::String(self.style.declarations().strings()[index.0 as usize].clone())
            }
            SymbolKind::Special(bib_bst::SpecialSymbol::SortKey) => self
                .current
                .map(|entry| VmValue::String(self.entries[entry].sort_key.clone()))
                .unwrap_or(VmValue::Missing),
            SymbolKind::Special(bib_bst::SpecialSymbol::Crossref) => self
                .current_entry()
                .and_then(|entry| entry.crossref().map(str::to_owned))
                .map(VmValue::String)
                .unwrap_or(VmValue::Missing),
            // These are fixed by the pinned classic Web2C configuration, not
            // by Umber's separate safety limits.
            SymbolKind::Special(bib_bst::SpecialSymbol::EntryMax) => VmValue::Integer(100),
            SymbolKind::Special(bib_bst::SpecialSymbol::GlobalMax) => VmValue::Integer(1_000),
            _ => VmValue::Missing,
        }
    }

    fn set_integer_global(&mut self, index: usize, value: VmValue) {
        if let VmValue::Integer(value) = value {
            self.globals_i[index] = value;
        } else {
            self.wrong_type();
        }
    }
    fn set_string_global(&mut self, index: usize, value: VmValue) {
        if let VmValue::String(value) = value {
            self.globals_s[index] = value;
        } else {
            self.wrong_type();
        }
    }
    fn set_integer_entry(&mut self, index: usize, value: VmValue) {
        match (self.current, value) {
            (Some(entry), VmValue::Integer(value)) => self.entries[entry].integers[index] = value,
            (None, _) => self.no_entry(),
            _ => self.wrong_type(),
        }
    }
    fn set_string_entry(&mut self, index: usize, value: VmValue) {
        match (self.current, value) {
            (Some(entry), VmValue::String(value)) => self.entries[entry].strings[index] = value,
            (None, _) => self.no_entry(),
            _ => self.wrong_type(),
        }
    }
    fn set_sort_key(&mut self, value: VmValue) {
        match (self.current, value) {
            (Some(entry), VmValue::String(value)) => self.entries[entry].sort_key = value,
            (None, _) => self.no_entry(),
            _ => self.wrong_type(),
        }
    }

    fn current_entry(&self) -> Option<&crate::ClassicDatabaseEntry> {
        self.current
            .and_then(|index| self.database.entries().nth(index))
    }
    fn push(&mut self, value: VmValue) {
        if self.stack.len() >= self.limits.stack_values {
            self.fail(
                ClassicVmDiagnosticKind::Limit,
                "BST operand-stack limit exceeded",
            );
        } else if self.value_bytes(&value) > self.limits.string_bytes {
            self.fail(ClassicVmDiagnosticKind::Limit, "BST string limit exceeded");
        } else {
            self.stack.push(value);
        }
    }
    fn pop(&mut self) -> Option<VmValue> {
        let value = self.stack.pop();
        if value.is_none() {
            self.underflow();
        }
        value
    }
    fn pop_integer(&mut self) -> Option<i64> {
        match self.pop() {
            Some(VmValue::Integer(value)) => Some(value),
            Some(_) => {
                self.wrong_type();
                None
            }
            None => None,
        }
    }
    fn pop_string(&mut self) -> Option<String> {
        match self.pop() {
            Some(VmValue::String(value)) => Some(value),
            Some(_) => {
                self.wrong_type();
                None
            }
            None => None,
        }
    }
    fn pop_callable(&mut self) -> Option<Callable> {
        match self.pop() {
            Some(VmValue::Function(value)) => Some(Callable::Function(value)),
            Some(VmValue::Variable(value)) => Some(Callable::Variable(value)),
            Some(_) => {
                self.wrong_type();
                None
            }
            None => None,
        }
    }
    fn call_callable(&mut self, callable: Callable) {
        match callable {
            Callable::Function(function) => {
                self.push_function(function);
            }
            Callable::Variable(symbol) => {
                let value = self.read(symbol);
                self.push(value);
            }
        }
    }
    fn value_bytes(&self, value: &VmValue) -> usize {
        match value {
            VmValue::String(value) => value.len(),
            _ => 0,
        }
    }
    fn effect(&mut self, log: bool, text: &str) {
        let sink = if log { &mut self.blg } else { &mut self.bbl };
        let limit = if log {
            self.limits.blg_bytes
        } else {
            self.limits.bbl_bytes
        };
        if sink.len().saturating_add(text.len()) > limit {
            self.fail(ClassicVmDiagnosticKind::Limit, "BST output limit exceeded");
        } else {
            sink.push_str(text);
        }
    }
    fn charge(&mut self) -> bool {
        self.work = self.work.saturating_add(1);
        if self.work > self.limits.work {
            self.fail(ClassicVmDiagnosticKind::Limit, "BST work limit exceeded");
            false
        } else {
            true
        }
    }
    fn underflow(&mut self) {
        self.report(
            ClassicVmDiagnosticKind::Underflow,
            "BST operand stack underflow",
        );
    }
    fn wrong_type(&mut self) {
        self.fail(
            ClassicVmDiagnosticKind::WrongType,
            "BST operand stack type mismatch",
        );
    }
    fn no_entry(&mut self) {
        self.fail(
            ClassicVmDiagnosticKind::NoCurrentEntry,
            "BST operation requires a current entry",
        );
    }
    fn fail(&mut self, kind: ClassicVmDiagnosticKind, message: &str) {
        self.report(kind, message);
        self.fatal = true;
    }
    fn report(&mut self, kind: ClassicVmDiagnosticKind, message: &str) {
        if self.diagnostics.len() < self.limits.diagnostics {
            let diagnostic = ClassicVmDiagnostic {
                kind,
                message: message.to_owned(),
                source: self.instruction_source,
                entry: self.current_entry().map(|entry| entry.key().to_owned()),
            };
            self.log_events
                .push(ClassicVmLogEvent::Diagnostic(diagnostic.clone()));
            self.diagnostics.push(diagnostic);
        }
    }
}

impl VmValue {
    fn stack_text(&self) -> String {
        match self {
            Self::Integer(value) => value.to_string(),
            Self::String(value) => value.clone(),
            Self::Function(_) | Self::Variable(_) => "function".to_owned(),
            Self::Missing => "missing".to_owned(),
        }
    }
    fn into_sort_string(self) -> Option<String> {
        match self {
            Self::String(value) => Some(value),
            Self::Missing => Some(String::new()),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum CaseMode {
    Title,
    Lower,
    Upper,
}

impl CaseMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "t" | "T" => Some(Self::Title),
            "l" | "L" => Some(Self::Lower),
            "u" | "U" => Some(Self::Upper),
            _ => None,
        }
    }
}

/// A classic text character is an ordinary character or a complete top-level
/// braced control-sequence group.  Keeping the original range lets prefixing
/// preserve TeX spelling and close ordinary unbalanced groups deterministically.
#[derive(Clone, Copy)]
struct TextUnit {
    end: usize,
}

fn classic_text_units(value: &str) -> Vec<TextUnit> {
    let mut units = Vec::new();
    let mut at = 0;
    let bytes = value.as_bytes();
    while at < bytes.len() {
        if bytes[at] == b'{' && bytes.get(at + 1) == Some(&b'\\') {
            let mut depth = 1_usize;
            let mut end = at + 2;
            while end < bytes.len() && depth != 0 {
                match bytes[end] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                end += 1;
            }
            units.push(TextUnit { end });
            at = end;
        } else if bytes[at] == b'{' || bytes[at] == b'}' {
            at += 1;
        } else {
            let width = value[at..].chars().next().map_or(1, char::len_utf8);
            at += width;
            units.push(TextUnit { end: at });
        }
    }
    units
}

fn classic_text_prefix(value: &str, count: i64) -> String {
    if count <= 0 {
        return String::new();
    }
    let units = classic_text_units(value);
    let end = units
        .get(count as usize - 1)
        .map_or(value.len(), |unit| unit.end);
    let mut prefix = value[..end].to_owned();
    let mut depth = 0_usize;
    for character in prefix.chars() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    prefix.extend(std::iter::repeat_n('}', depth));
    prefix
}

fn classic_substring(value: &str, start: i64, count: i64) -> String {
    if count <= 0 || start == 0 {
        return String::new();
    }
    let chars: Vec<char> = value.chars().collect();
    let start = if start > 0 {
        i128::from(start - 1)
    } else {
        // A negative start identifies the right edge of the requested slice.
        // Thus `#-1 #4 substring$` retains the final four characters.
        chars.len() as i128 + i128::from(start) - i128::from(count) + 1
    };
    if start < 0 || start >= chars.len() as i128 {
        return String::new();
    }
    chars
        .into_iter()
        .skip(start as usize)
        .take(usize::try_from(count).unwrap_or(usize::MAX))
        .collect()
}

fn is_classic_whitespace(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
}

fn change_case(value: &str, mode: CaseMode) -> String {
    let mut result = String::with_capacity(value.len());
    let mut depth = 0_usize;
    let mut first = true;
    let mut after_colon = false;
    for character in value.chars() {
        match character {
            '{' => {
                depth += 1;
                result.push(character);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                result.push(character);
            }
            _ if depth != 0 => result.push(character),
            _ => {
                let preserve = matches!(mode, CaseMode::Title) && (first || after_colon);
                let converted = match mode {
                    CaseMode::Lower => character.to_ascii_lowercase(),
                    CaseMode::Upper => character.to_ascii_uppercase(),
                    CaseMode::Title if !preserve => character.to_ascii_lowercase(),
                    CaseMode::Title => character,
                };
                result.push(converted);
                if character == ':' {
                    after_colon = true;
                } else if !character.is_ascii_whitespace() {
                    after_colon = false;
                }
                first = false;
            }
        }
    }
    result
}

fn purify(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character.is_ascii_alphanumeric() {
            result.push(character);
        } else if character.is_ascii_whitespace() || matches!(character, '-' | '~') {
            result.push(' ');
        } else if character == '\\' {
            let mut control = String::new();
            while characters
                .peek()
                .is_some_and(|next| next.is_ascii_alphabetic())
            {
                control.push(characters.next().expect("peeked character"));
            }
            match control.as_str() {
                "i" => result.push('i'),
                "j" => result.push('j'),
                "oe" | "OE" | "ae" | "AE" | "ss" => result.push_str(&control),
                _ => {}
            }
        }
    }
    result
}

fn split_names(value: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    let bytes = value.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && bytes[at..].starts_with(b" and ") => {
                names.push(value[start..at].trim().to_owned());
                at += 4;
                start = at;
                continue;
            }
            _ => {}
        }
        at += 1;
    }
    names.push(value[start..].trim().to_owned());
    names
}

fn format_bib_name(name: &str, format: &str) -> String {
    let parts = BibName::parse(name);
    let mut result = String::new();
    let mut at = 0;
    let bytes = format.as_bytes();
    while at < bytes.len() {
        if bytes[at] != b'{' {
            result.push(bytes[at] as char);
            at += 1;
            continue;
        }
        let Some(close) = matching_close_brace(format, at) else {
            result.push('{');
            at += 1;
            continue;
        };
        let pattern = &format[at + 1..close];
        let key = pattern
            .chars()
            .find(|character| matches!(character, 'f' | 'v' | 'l' | 'j'));
        if let Some(key) = key {
            let words = match key {
                'f' => &parts.first,
                'v' => &parts.von,
                'l' => &parts.last,
                _ => &parts.jr,
            };
            if !words.is_empty() {
                let abbreviated = pattern.matches(key).count() == 1;
                result.push_str(&format_name_words(
                    words,
                    abbreviated,
                    pattern,
                    has_following_name_part(&format[close + 1..], &parts),
                ));
            }
        } else {
            result.push_str(pattern);
        }
        at = close + 1;
    }
    result
}

fn format_name_words(
    words: &[String],
    abbreviated: bool,
    pattern: &str,
    has_following_part: bool,
) -> String {
    let key_at = pattern
        .find(['f', 'v', 'l', 'j'])
        .expect("name pattern has a part key");
    let key = pattern[key_at..]
        .chars()
        .next()
        .expect("part key starts at character boundary");
    let key_end = pattern
        .rfind(key)
        .expect("name pattern contains its part key")
        + key.len_utf8();
    let before = &pattern[..key_at];
    let raw_after = &pattern[key_end..];
    let (word_separator, after) = if raw_after.starts_with('{') {
        matching_close_brace(raw_after, 0).map_or((None, raw_after), |close| {
            (Some(&raw_after[1..close]), &raw_after[close + 1..])
        })
    } else {
        (None, raw_after)
    };
    // A trailing tie on a multi-word part is its inter-word separator, not
    // punctuation around the entire rendered part.  If another name part
    // follows, the reference leaves the ordinary boundary space after it.
    let consume_tie_as_word_separator = after == "~" && words.len() > 1;
    let mut result = String::from(before);
    for (index, word) in words.iter().enumerate() {
        if abbreviated {
            result.push_str(&abbreviated_name_word(word));
        } else {
            result.push_str(word);
        }
        if index + 1 < words.len() {
            if let Some(separator) = word_separator {
                result.push_str(separator);
            } else {
                if abbreviated {
                    result.push('.');
                }
                if consume_tie_as_word_separator || abbreviated {
                    result.push('~');
                } else {
                    result.push(' ');
                }
            }
        }
    }
    if !consume_tie_as_word_separator {
        result.push_str(after);
    } else if has_following_part {
        result.push(' ');
    }
    result
}

fn abbreviated_name_word(word: &str) -> String {
    let mut result = String::new();
    for (index, segment) in split_name_word_segments(word).into_iter().enumerate() {
        if index != 0 {
            result.push_str(".-");
        }
        if let Some(braced_initial) = initial_braced_group(segment) {
            result.push_str(braced_initial);
        } else if let Some(initial) = segment.chars().next() {
            result.push(initial);
        }
    }
    result
}

fn initial_braced_group(segment: &str) -> Option<&str> {
    if !segment.starts_with('{') {
        return None;
    }
    let mut depth = 0_usize;
    for (at, character) in segment.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&segment[..at + character.len_utf8()]);
                }
            }
            _ => {}
        }
    }
    Some(segment)
}

fn split_name_word_segments(word: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    for (at, character) in word.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            '-' if depth == 0 => {
                segments.push(&word[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    segments.push(&word[start..]);
    segments
}

fn has_following_name_part(format: &str, parts: &BibName) -> bool {
    let mut at = 0;
    let bytes = format.as_bytes();
    while at < bytes.len() {
        if bytes[at] != b'{' {
            at += 1;
            continue;
        }
        let Some(close) = matching_close_brace(format, at) else {
            return false;
        };
        let key = format[at + 1..close]
            .chars()
            .find(|character| matches!(character, 'f' | 'v' | 'l' | 'j'));
        if let Some(key) = key {
            let words = match key {
                'f' => &parts.first,
                'v' => &parts.von,
                'l' => &parts.last,
                _ => &parts.jr,
            };
            if !words.is_empty() {
                return true;
            }
        }
        at = close + 1;
    }
    false
}

fn matching_close_brace(value: &str, open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (offset, character) in value[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

#[derive(Default)]
struct BibName {
    first: Vec<String>,
    von: Vec<String>,
    last: Vec<String>,
    jr: Vec<String>,
}

impl BibName {
    fn parse(value: &str) -> Self {
        let commas: Vec<Vec<String>> = value.split(',').map(name_words).collect();
        match commas.as_slice() {
            [last, first] => Self {
                last: last.clone(),
                first: first.clone(),
                ..Self::default()
            },
            [last, jr, first, ..] => Self {
                last: last.clone(),
                jr: jr.clone(),
                first: first.clone(),
                ..Self::default()
            },
            _ => {
                let words = name_words(value);
                let split = words
                    .iter()
                    .position(|word| starts_lower(word))
                    .unwrap_or(words.len());
                let last_start = if split == words.len() {
                    split.saturating_sub(1)
                } else {
                    words[split..]
                        .iter()
                        .position(|word| !starts_lower(word))
                        .map_or_else(|| words.len().saturating_sub(1), |offset| split + offset)
                };
                let von_start = split.min(last_start);
                Self {
                    first: words[..von_start].to_vec(),
                    von: words[von_start..last_start].to_vec(),
                    last: words[last_start..].to_vec(),
                    jr: Vec::new(),
                }
            }
        }
    }
}

fn name_words(value: &str) -> Vec<String> {
    value.split_whitespace().map(str::to_owned).collect()
}
fn starts_lower(value: &str) -> bool {
    value
        .chars()
        .find(|character| character.is_ascii_alphabetic())
        .is_some_and(|character| character.is_ascii_lowercase())
}

fn classic_width(value: &str) -> i64 {
    let mut width = 0_i64;
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '{' && characters.peek() == Some(&'\\') {
            characters.next();
            let mut control = String::new();
            while characters
                .peek()
                .is_some_and(|next| next.is_ascii_alphabetic())
            {
                control.push(characters.next().expect("peeked character"));
            }
            width += match control.as_str() {
                "ss" => 500,
                "ae" | "oe" => 722,
                "AE" | "OE" => 903,
                _ => control.chars().next().map_or(0, classic_char_width),
            };
        } else {
            width += classic_char_width(character);
        }
    }
    width
}

fn classic_char_width(character: char) -> i64 {
    match character {
        ' ' => 278,
        '!' => 278,
        '"' => 500,
        '#' => 833,
        '$' => 500,
        '%' => 833,
        '&' => 778,
        '\'' => 278,
        '(' | ')' => 389,
        '*' => 500,
        '+' => 778,
        ',' => 278,
        '-' => 333,
        '.' => 278,
        '/' => 500,
        '0'..='9' => 500,
        ':' | ';' => 278,
        '<' | '>' => 778,
        '=' => 778,
        '?' => 500,
        '@' => 778,
        'A' => 750,
        'B' => 708,
        'C' => 722,
        'D' => 764,
        'E' => 681,
        'F' => 653,
        'G' => 785,
        'H' => 750,
        'I' => 361,
        'J' => 514,
        'K' => 778,
        'L' => 625,
        'M' => 917,
        'N' => 750,
        'O' => 778,
        'P' => 681,
        'Q' => 778,
        'R' => 736,
        'S' => 556,
        'T' => 722,
        'U' => 750,
        'V' => 750,
        'W' => 1028,
        'X' => 750,
        'Y' => 750,
        'Z' => 611,
        '[' | ']' => 278,
        '\\' => 500,
        '^' => 278,
        '_' => 500,
        '`' => 278,
        'a' => 500,
        'b' => 556,
        'c' => 444,
        'd' => 556,
        'e' => 444,
        'f' => 306,
        'g' => 500,
        'h' => 556,
        'i' => 278,
        'j' => 306,
        'k' => 528,
        'l' => 278,
        'm' => 833,
        'n' => 556,
        'o' => 500,
        'p' => 556,
        'q' => 528,
        'r' => 392,
        's' => 394,
        't' => 389,
        'u' => 556,
        'v' => 528,
        'w' => 722,
        'x' => 528,
        'y' => 528,
        'z' => 444,
        '{' | '}' => 500,
        '~' => 500,
        _ => 0,
    }
}

#[cfg(test)]
mod tests;
