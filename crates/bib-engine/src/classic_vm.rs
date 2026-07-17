//! Bounded execution of an immutable classic BibTeX style program.
//!
//! The VM owns all mutable state and never calls a style function through the
//! Rust stack.  It deliberately stops at the phase-six core built-ins; text,
//! name, and layout compatibility algorithms are owned by the next phase.

use bib_bst::{
    Builtin, CompiledCommand, CompiledStyle, FunctionId, Instruction, SymbolId, SymbolKind,
};

use crate::ClassicDatabase;

/// Values which can occur on the classic BibTeX operand stack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmValue {
    Integer(i64),
    String(String),
    Function(FunctionId),
    /// An assignment target, produced only by a quoted mutable symbol.
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
    Underflow,
    WrongType,
    NoCurrentEntry,
    InvalidFunction,
    Limit,
    Arithmetic,
    UnsupportedBuiltin,
}

/// One ordered VM diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicVmDiagnostic {
    kind: ClassicVmDiagnosticKind,
    message: String,
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
    entry_order: Vec<String>,
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
    #[must_use]
    pub fn entry_order(&self) -> &[String] {
        &self.entry_order
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
struct Frame {
    function: FunctionId,
    pc: usize,
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
    work: usize,
    fatal: bool,
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
            work: 0,
            fatal: false,
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
            entry_order,
            work: self.work,
        }
    }

    fn run(&mut self) {
        for command in self.style.commands() {
            if self.fatal {
                break;
            }
            if !self.charge() {
                break;
            }
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
        if function.0 as usize >= self.style.functions().len() {
            self.fail(
                ClassicVmDiagnosticKind::InvalidFunction,
                "invalid BST function",
            );
            return;
        }
        if self.frames.len() >= self.limits.call_depth {
            self.fail(
                ClassicVmDiagnosticKind::Limit,
                "BST call-depth limit exceeded",
            );
            return;
        }
        self.frames.push(Frame { function, pc: 0 });
        while !self.frames.is_empty() {
            if !self.charge() {
                return;
            }
            let frame = *self.frames.last().expect("non-empty loop condition");
            let Some(instruction) = self.style.functions()[frame.function.0 as usize]
                .instructions()
                .get(frame.pc)
                .cloned()
            else {
                self.frames.pop();
                continue;
            };
            self.frames.last_mut().expect("frame remains present").pc += 1;
            self.instruction(instruction);
            if self.fatal {
                return;
            }
        }
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
                if self.frames.len() >= self.limits.call_depth {
                    self.fail(
                        ClassicVmDiagnosticKind::Limit,
                        "BST call-depth limit exceeded",
                    );
                } else {
                    self.frames.push(Frame { function, pc: 0 });
                }
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
            Builtin::Assign => self.assign_from_stack(),
            Builtin::If => self.if_builtin(),
            Builtin::While => self.while_builtin(),
            Builtin::Write => {
                if let Some(value) = self.pop_string() {
                    self.effect(false, &value)
                }
            }
            Builtin::Newline => self.effect(false, "\n"),
            Builtin::Warning => {
                if let Some(value) = self.pop_string() {
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
                    self.push(VmValue::Integer(i64::from(value.is_empty())))
                }
                Some(_) => self.wrong_type(),
                None => {}
            },
            Builtin::IntToStr => {
                if let Some(value) = self.pop_integer() {
                    self.push(VmValue::String(value.to_string()));
                }
            }
            Builtin::Quote => self.push(VmValue::String("\"".to_owned())),
            Builtin::Preamble => self.push(VmValue::String(self.database.preamble())),
            Builtin::Stack | Builtin::Top => {
                if let Some(value) = self.stack.last() {
                    self.effect(true, &format!("{value:?}\n"));
                } else {
                    self.underflow();
                }
            }
            _ => self.fail(
                ClassicVmDiagnosticKind::UnsupportedBuiltin,
                "BST builtin is owned by the built-in compatibility phase",
            ),
        }
    }

    fn if_builtin(&mut self) {
        let else_function = self.pop_function();
        let then_function = self.pop_function();
        let condition = self.pop_integer();
        if let (Some(else_function), Some(then_function), Some(condition)) =
            (else_function, then_function, condition)
        {
            self.call(if condition > 0 {
                then_function
            } else {
                else_function
            });
        }
    }

    fn while_builtin(&mut self) {
        let body = self.pop_function();
        let condition = self.pop_function();
        let (Some(body), Some(condition)) = (body, condition) else {
            return;
        };
        loop {
            self.call(condition);
            if self.fatal {
                return;
            }
            let Some(value) = self.pop_integer() else {
                return;
            };
            if value <= 0 {
                return;
            }
            self.call(body);
            if self.fatal {
                return;
            }
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
    fn pop_function(&mut self) -> Option<FunctionId> {
        match self.pop() {
            Some(VmValue::Function(value)) => Some(value),
            Some(_) => {
                self.wrong_type();
                None
            }
            None => None,
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
        self.fail(
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
        if self.diagnostics.len() < self.limits.diagnostics {
            self.diagnostics.push(ClassicVmDiagnostic {
                kind,
                message: message.to_owned(),
            });
        }
        self.fatal = true;
    }
}

impl VmValue {
    fn into_sort_string(self) -> Option<String> {
        match self {
            Self::String(value) => Some(value),
            Self::Missing => Some(String::new()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
