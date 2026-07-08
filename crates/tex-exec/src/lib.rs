//! TeX execution engine scaffold.
//!
//! This crate owns the stomach's mode nest and main-control dispatch. It pulls
//! only fully expanded tokens from `tex_expand::get_x_token*`; raw token reads
//! stay in the lexer/gullet pipeline.

#![forbid(unsafe_code)]

use std::fmt;

use tex_expand::scan::ScanToksError;
use tex_expand::{
    EngineMode, ExpandError, ExpansionHooks, NoopRecorder, ReadRecorder,
    get_x_token_with_recorder_and_hooks,
};
use tex_lex::{InputSource, InputStack, LexError};
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::stores::{GroupKind, GroupMismatch, Stores};
use tex_state::token::{Catcode, Token};

mod assignments;
mod diagnostics;

pub use assignments::{install_unexpandable_primitives, try_execute_assignment};
pub use diagnostics::{LogSink, NoopLogSink, StringLogSink};

/// One of TeX's six semantic modes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Mode {
    Vertical,
    InternalVertical,
    Horizontal,
    RestrictedHorizontal,
    Math,
    DisplayMath,
}

impl Mode {
    /// The three-way mode family used by `\ifvmode`, `\ifhmode`, `\ifmmode`.
    #[must_use]
    pub const fn engine_mode(self) -> EngineMode {
        match self {
            Self::Vertical | Self::InternalVertical => EngineMode::Vertical,
            Self::Horizontal | Self::RestrictedHorizontal => EngineMode::Horizontal,
            Self::Math | Self::DisplayMath => EngineMode::Math,
        }
    }

    /// Whether TeX's `\ifinner` predicate is true in this mode.
    #[must_use]
    pub const fn is_inner(self) -> bool {
        matches!(
            self,
            Self::InternalVertical | Self::RestrictedHorizontal | Self::Math
        )
    }
}

/// Placeholder for the list-under-construction owned by each nest level.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ListBuilderSummary {
    #[default]
    Empty,
}

/// Snapshot-summary state for one mode level.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ModeLevelSummary {
    mode: Mode,
    list: ListBuilderSummary,
}

impl ModeLevelSummary {
    #[must_use]
    pub const fn new(mode: Mode) -> Self {
        Self {
            mode,
            list: ListBuilderSummary::Empty,
        }
    }

    #[must_use]
    pub const fn mode(self) -> Mode {
        self.mode
    }

    #[must_use]
    pub const fn list(self) -> ListBuilderSummary {
        self.list
    }
}

/// Snapshot-coverable summary of the whole mode nest.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ModeNestSummary {
    levels: Vec<ModeLevelSummary>,
}

impl ModeNestSummary {
    #[must_use]
    pub fn levels(&self) -> &[ModeLevelSummary] {
        &self.levels
    }
}

/// Explicit stack of TeX mode levels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModeNest {
    levels: Vec<ModeLevelSummary>,
}

impl Default for ModeNest {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeNest {
    /// Creates the outer main vertical nest level.
    #[must_use]
    pub fn new() -> Self {
        Self {
            levels: vec![ModeLevelSummary::new(Mode::Vertical)],
        }
    }

    /// Rehydrates a nest from snapshot summary state.
    pub fn from_summary(summary: ModeNestSummary) -> Result<Self, ExecError> {
        if summary.levels.is_empty() {
            return Err(ExecError::EmptyModeNestSummary);
        }
        Ok(Self {
            levels: summary.levels,
        })
    }

    #[must_use]
    pub fn summary(&self) -> ModeNestSummary {
        ModeNestSummary {
            levels: self.levels.clone(),
        }
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    #[must_use]
    pub fn current_mode(&self) -> Mode {
        self.levels
            .last()
            .expect("ModeNest always has at least one level")
            .mode()
    }

    pub fn push(&mut self, mode: Mode) {
        self.levels.push(ModeLevelSummary::new(mode));
    }

    pub fn pop(&mut self) -> Result<ModeLevelSummary, ExecError> {
        if self.levels.len() == 1 {
            return Err(ExecError::CannotPopBaseMode);
        }
        Ok(self
            .levels
            .pop()
            .expect("length checked before popping mode level"))
    }
}

/// Stomach interpreter state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Executor {
    nest: ModeNest,
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nest: ModeNest::new(),
        }
    }

    pub fn from_nest(nest: ModeNest) -> Self {
        Self { nest }
    }

    #[must_use]
    pub fn nest(&self) -> &ModeNest {
        &self.nest
    }

    pub fn nest_mut(&mut self) -> &mut ModeNest {
        &mut self.nest
    }

    /// Runs main control until the gullet has no more delivered tokens.
    pub fn run<S>(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut Stores,
    ) -> Result<ExecutionStats, ExecError>
    where
        S: InputSource,
    {
        self.run_with_recorder(input, stores, &mut NoopRecorder)
    }

    /// Runs main control while recording expansion meaning reads.
    pub fn run_with_recorder<S, R>(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut Stores,
        recorder: &mut R,
    ) -> Result<ExecutionStats, ExecError>
    where
        S: InputSource,
        R: ReadRecorder,
    {
        let mut hooks = NoopExecHooks;
        self.run_with_recorder_and_hooks(input, stores, recorder, &mut hooks)
    }

    /// Runs main control while recording reads and using driver expansion hooks.
    pub fn run_with_recorder_and_hooks<S, R, H>(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut Stores,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<ExecutionStats, ExecError>
    where
        S: InputSource,
        R: ReadRecorder,
        H: ExpansionHooks<S>,
    {
        self.run_with_recorder_and_hooks_and_log_sink(
            input,
            stores,
            recorder,
            hooks,
            &mut NoopLogSink,
        )
    }

    /// Runs main control with expansion hooks and a diagnostic log sink.
    pub fn run_with_recorder_and_hooks_and_log_sink<S, R, H, L>(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut Stores,
        recorder: &mut R,
        hooks: &mut H,
        log: &mut L,
    ) -> Result<ExecutionStats, ExecError>
    where
        S: InputSource,
        R: ReadRecorder,
        H: ExpansionHooks<S>,
        L: LogSink,
    {
        let mut stats = ExecutionStats::default();
        loop {
            let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
            else {
                return Ok(stats);
            };
            stats.delivered_tokens += 1;
            match dispatch_delivered_token_with_log_sink(
                self.nest.current_mode(),
                token,
                input,
                stores,
                hooks,
                log,
            )? {
                DispatchAction::Continue => {}
                DispatchAction::End => return Ok(stats),
                DispatchAction::NotConsumed => {
                    return Err(unimplemented_typesetting(
                        self.nest.current_mode(),
                        token,
                        "non-assignment command",
                    )
                    .expect_err("unimplemented_typesetting always returns Err"));
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct NoopExecHooks;

impl<S> ExpansionHooks<S> for NoopExecHooks
where
    S: InputSource,
{
    fn open_input(&mut self, _name: &str) -> Result<S, String> {
        Err("execution input hook is not installed".to_owned())
    }
}

impl<S> ExpansionHooks<S> for Executor
where
    S: InputSource,
{
    fn open_input(&mut self, _name: &str) -> Result<S, String> {
        Err("execution input hook is not installed".to_owned())
    }

    fn mode(&self) -> EngineMode {
        self.nest.current_mode().engine_mode()
    }

    fn is_inner_mode(&self) -> bool {
        self.nest.current_mode().is_inner()
    }
}

/// Main-control progress counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecutionStats {
    pub delivered_tokens: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchAction {
    Continue,
    End,
    NotConsumed,
}

/// Dispatches one gullet-delivered token in the current mode.
pub fn dispatch_delivered_token<S, H>(
    mode: Mode,
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    dispatch_delivered_token_with_log_sink(mode, token, input, stores, hooks, &mut NoopLogSink)
}

/// Dispatches one delivered token while writing diagnostics to `log`.
pub fn dispatch_delivered_token_with_log_sink<S, H, L>(
    mode: Mode,
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
    log: &mut L,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
    L: LogSink,
{
    let meaning = match token {
        Token::Cs(symbol) => stores.meaning(symbol),
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        } => {
            stores.enter_group_with_kind(GroupKind::Simple);
            return Ok(DispatchAction::Continue);
        }
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        } => {
            leave_group(input, stores, GroupKind::Simple)?;
            return Ok(DispatchAction::Continue);
        }
        Token::Char {
            cat: Catcode::Space,
            ..
        } => {
            return Ok(DispatchAction::Continue);
        }
        Token::Char { .. } | Token::Param(_) => {
            return Ok(DispatchAction::NotConsumed);
        }
    };

    match meaning {
        Meaning::Relax => Ok(DispatchAction::Continue),
        Meaning::Undefined => Err(ExecError::UndefinedControlSequence {
            name: stores.resolve_cs_name(token),
        }),
        Meaning::CharGiven(_) => unimplemented_typesetting(mode, token, "character token command"),
        Meaning::Macro { .. } => Err(ExecError::UnexpectedMacroDelivery {
            name: stores.resolve_cs_name(token),
        }),
        Meaning::ExpandablePrimitive(primitive) => dispatch_delivered_expandable(token, primitive),
        Meaning::UnexpandablePrimitive(primitive) => {
            assignments::execute_unexpandable(primitive, input, stores, hooks, log)
        }
        meaning @ (Meaning::CountRegister(_)
        | Meaning::DimenRegister(_)
        | Meaning::SkipRegister(_)
        | Meaning::MuskipRegister(_)
        | Meaning::ToksRegister(_)
        | Meaning::IntParam(_)
        | Meaning::DimenParam(_)
        | Meaning::GlueParam(_)
        | Meaning::TokParam(_)) => {
            assignments::execute_assignment_meaning(meaning, input, stores, hooks)
        }
        Meaning::MathCharGiven(_) => {
            unimplemented_typesetting(mode, token, "math character command")
        }
        Meaning::Unknown(raw) => Err(ExecError::UnsupportedCommand {
            token,
            opcode: raw.op(),
        }),
    }
}

fn dispatch_delivered_expandable(
    token: Token,
    primitive: ExpandablePrimitive,
) -> Result<DispatchAction, ExecError> {
    match primitive {
        ExpandablePrimitive::EndCsName => Err(ExecError::ExtraEndCsName),
        ExpandablePrimitive::Fi | ExpandablePrimitive::Else | ExpandablePrimitive::Or => {
            Err(ExecError::ExtraConditionalControl(primitive))
        }
        _ => Err(ExecError::UnexpectedExpandableDelivery { token, primitive }),
    }
}

fn leave_group<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    expected: GroupKind,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    match stores.leave_group_with_kind(expected) {
        Ok(tokens) => {
            push_tokens(input, stores, tokens);
            Ok(())
        }
        Err(mismatch) => Err(group_mismatch_error(expected, mismatch)),
    }
}

fn group_mismatch_error(expected: GroupKind, mismatch: GroupMismatch) -> ExecError {
    let no_open_group = mismatch.actual() == expected;
    match (expected, mismatch.actual(), no_open_group) {
        (GroupKind::Simple, _, true) => ExecError::TooManyRightBraces,
        (GroupKind::Simple, GroupKind::SemiSimple, false) => {
            ExecError::ExtraRightBraceOrForgottenEndgroup
        }
        (GroupKind::SemiSimple, _, true) => ExecError::ExtraEndGroup,
        (GroupKind::SemiSimple, GroupKind::Simple, false) => ExecError::EndGroupMismatch {
            started_by: mismatch.actual().start_text(),
        },
        (GroupKind::Simple, GroupKind::Simple, false)
        | (GroupKind::SemiSimple, GroupKind::SemiSimple, false) => {
            unreachable!("matching group kinds are returned as successful leaves, not mismatches")
        }
    }
}

pub(crate) fn push_tokens<S, I>(input: &mut InputStack<S>, stores: &mut Stores, tokens: I)
where
    S: InputSource,
    I: IntoIterator<Item = Token>,
{
    let tokens: Vec<_> = tokens.into_iter().collect();
    if tokens.is_empty() {
        return;
    }
    let token_list = stores.intern_token_list(&tokens);
    input.push_token_list(token_list, tex_lex::TokenListReplayKind::Inserted);
}

fn unimplemented_typesetting(
    mode: Mode,
    token: Token,
    operation: &'static str,
) -> Result<DispatchAction, ExecError> {
    Err(ExecError::UnimplementedTypesetting {
        mode,
        token,
        operation,
    })
}

trait ResolveTokenName {
    fn resolve_cs_name(&self, token: Token) -> String;
}

impl ResolveTokenName for Stores {
    fn resolve_cs_name(&self, token: Token) -> String {
        match token {
            Token::Cs(symbol) => self.resolve(symbol).to_owned(),
            Token::Char { ch, cat } => format!("{ch:?}/{cat:?}"),
            Token::Param(slot) => format!("#{slot}"),
        }
    }
}

#[derive(Debug)]
pub enum ExecError {
    Expand(ExpandError),
    Lex(LexError),
    ScanToks(ScanToksError),
    ScanGlue(tex_expand::scan_glue::ScanGlueError),
    EmptyModeNestSummary,
    CannotPopBaseMode,
    UndefinedControlSequence {
        name: String,
    },
    UnexpectedMacroDelivery {
        name: String,
    },
    UnexpectedExpandableDelivery {
        token: Token,
        primitive: ExpandablePrimitive,
    },
    ExtraConditionalControl(ExpandablePrimitive),
    ExtraEndCsName,
    TooManyRightBraces,
    ExtraRightBraceOrForgottenEndgroup,
    ExtraEndGroup,
    EndGroupMismatch {
        started_by: &'static str,
    },
    UnsupportedCommand {
        token: Token,
        opcode: u8,
    },
    MissingPrefixedCommand,
    PrefixWithNonAssignment {
        token: Token,
    },
    PrefixWithNonDefinition,
    MissingControlSequence {
        context: &'static str,
    },
    ExpectedControlSequence {
        context: &'static str,
        token: Token,
    },
    MissingToken {
        context: &'static str,
    },
    InvalidLetRhs {
        token: Token,
    },
    UnsupportedAssignmentTarget,
    RegisterNumberOutOfRange(i32),
    ArithmeticOverflow,
    InvalidCode {
        context: &'static str,
        value: i32,
    },
    ReadNeedsTo,
    ReadNotImplemented,
    UnimplementedTypesetting {
        mode: Mode,
        token: Token,
        operation: &'static str,
    },
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expand(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::ScanToks(err) => write!(f, "{err}"),
            Self::ScanGlue(err) => write!(f, "{err}"),
            Self::EmptyModeNestSummary => write!(f, "mode nest summary has no levels"),
            Self::CannotPopBaseMode => write!(f, "cannot pop the base vertical mode level"),
            Self::UndefinedControlSequence { name } => {
                write!(f, "undefined control sequence \\{name}")
            }
            Self::UnexpectedMacroDelivery { name } => {
                write!(f, "macro \\{name} reached execution without expansion")
            }
            Self::UnexpectedExpandableDelivery { token, primitive } => write!(
                f,
                "expandable primitive {primitive:?} reached execution as delivered token {token:?}"
            ),
            Self::ExtraConditionalControl(primitive) => {
                write!(f, "extra conditional control {primitive:?}")
            }
            Self::ExtraEndCsName => write!(f, "extra \\endcsname"),
            Self::TooManyRightBraces => write!(f, "Too many }}'s."),
            Self::ExtraRightBraceOrForgottenEndgroup => {
                write!(f, "Extra }}, or forgotten \\endgroup.")
            }
            Self::ExtraEndGroup => write!(f, "Extra \\endgroup."),
            Self::EndGroupMismatch { started_by } => {
                write!(f, "\\endgroup ended a group started by {started_by}")
            }
            Self::UnsupportedCommand { token, opcode } => {
                write!(
                    f,
                    "unsupported unexpandable opcode {opcode} for token {token:?}"
                )
            }
            Self::MissingPrefixedCommand => write!(f, "You can't use a prefix with `end of input'"),
            Self::PrefixWithNonAssignment { token } => {
                write!(f, "You can't use a prefix with `{token:?}'")
            }
            Self::PrefixWithNonDefinition => write!(f, "You can't use a prefix with `\\let'"),
            Self::MissingControlSequence { context } => {
                write!(f, "missing control sequence after {context}")
            }
            Self::ExpectedControlSequence { context, token } => {
                write!(
                    f,
                    "expected control sequence after {context}, got {token:?}"
                )
            }
            Self::MissingToken { context } => write!(f, "missing token while scanning {context}"),
            Self::InvalidLetRhs { token } => {
                write!(f, "\\let cannot assign macro parameter token {token:?}")
            }
            Self::UnsupportedAssignmentTarget => write!(f, "unsupported assignment target"),
            Self::RegisterNumberOutOfRange(value) => {
                write!(f, "register number {value} is out of range")
            }
            Self::ArithmeticOverflow => write!(f, "Arithmetic overflow"),
            Self::InvalidCode { context, value } => {
                write!(f, "Invalid code ({value}) while scanning {context}")
            }
            Self::ReadNeedsTo => write!(f, "Missing `to' inserted for \\read"),
            Self::ReadNotImplemented => write!(f, "I can't \\read from terminal in nonstop modes"),
            Self::UnimplementedTypesetting {
                mode,
                token,
                operation,
            } => write!(
                f,
                "typesetting path is not implemented yet: {operation} in {mode:?} for token {token:?}"
            ),
        }
    }
}

impl std::error::Error for ExecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Expand(err) => Some(err),
            Self::Lex(err) => Some(err),
            Self::ScanToks(err) => Some(err),
            Self::ScanGlue(err) => Some(err),
            Self::EmptyModeNestSummary
            | Self::CannotPopBaseMode
            | Self::UndefinedControlSequence { .. }
            | Self::UnexpectedMacroDelivery { .. }
            | Self::UnexpectedExpandableDelivery { .. }
            | Self::ExtraConditionalControl(_)
            | Self::ExtraEndCsName
            | Self::TooManyRightBraces
            | Self::ExtraRightBraceOrForgottenEndgroup
            | Self::ExtraEndGroup
            | Self::EndGroupMismatch { .. }
            | Self::UnsupportedCommand { .. }
            | Self::MissingPrefixedCommand
            | Self::PrefixWithNonAssignment { .. }
            | Self::PrefixWithNonDefinition
            | Self::MissingControlSequence { .. }
            | Self::ExpectedControlSequence { .. }
            | Self::MissingToken { .. }
            | Self::InvalidLetRhs { .. }
            | Self::UnsupportedAssignmentTarget
            | Self::RegisterNumberOutOfRange(_)
            | Self::ArithmeticOverflow
            | Self::InvalidCode { .. }
            | Self::ReadNeedsTo
            | Self::ReadNotImplemented
            | Self::UnimplementedTypesetting { .. } => None,
        }
    }
}

impl From<ExpandError> for ExecError {
    fn from(value: ExpandError) -> Self {
        Self::Expand(value)
    }
}

impl From<LexError> for ExecError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<ScanToksError> for ExecError {
    fn from(value: ScanToksError) -> Self {
        Self::ScanToks(value)
    }
}

impl From<tex_expand::scan_glue::ScanGlueError> for ExecError {
    fn from(value: tex_expand::scan_glue::ScanGlueError) -> Self {
        Self::ScanGlue(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_lex::MemoryInput;
    use tex_state::env::banks::IntParam;
    use tex_state::meaning::ExpandablePrimitive;
    use tex_state::token::Catcode;

    #[test]
    fn nest_push_pop_and_summary_cover_all_modes() {
        let mut nest = ModeNest::new();
        for mode in [
            Mode::InternalVertical,
            Mode::Horizontal,
            Mode::RestrictedHorizontal,
            Mode::Math,
            Mode::DisplayMath,
        ] {
            nest.push(mode);
        }

        assert_eq!(nest.depth(), 6);
        assert_eq!(nest.current_mode(), Mode::DisplayMath);

        let summary = nest.summary();
        let restored = ModeNest::from_summary(summary.clone()).expect("valid summary");
        assert_eq!(restored.summary(), summary);

        assert_eq!(nest.pop().expect("display math").mode(), Mode::DisplayMath);
        assert_eq!(nest.pop().expect("math").mode(), Mode::Math);
        assert_eq!(
            nest.pop().expect("restricted h").mode(),
            Mode::RestrictedHorizontal
        );
        assert_eq!(nest.pop().expect("h").mode(), Mode::Horizontal);
        assert_eq!(
            nest.pop().expect("internal v").mode(),
            Mode::InternalVertical
        );
        assert_eq!(
            nest.pop().expect_err("base cannot pop").to_string(),
            "cannot pop the base vertical mode level"
        );
    }

    #[test]
    fn mode_queries_are_backed_by_current_nest_level() {
        let mut executor = Executor::new();
        assert_eq!(
            <Executor as ExpansionHooks<MemoryInput>>::mode(&executor),
            EngineMode::Vertical
        );
        assert!(!<Executor as ExpansionHooks<MemoryInput>>::is_inner_mode(
            &executor
        ));

        executor.nest_mut().push(Mode::RestrictedHorizontal);
        assert_eq!(
            <Executor as ExpansionHooks<MemoryInput>>::mode(&executor),
            EngineMode::Horizontal
        );
        assert!(<Executor as ExpansionHooks<MemoryInput>>::is_inner_mode(
            &executor
        ));

        executor.nest_mut().push(Mode::DisplayMath);
        assert_eq!(
            <Executor as ExpansionHooks<MemoryInput>>::mode(&executor),
            EngineMode::Math
        );
        assert!(!<Executor as ExpansionHooks<MemoryInput>>::is_inner_mode(
            &executor
        ));
    }

    #[test]
    fn dispatch_relax_continues_without_state_mutation() {
        let mut stores = Stores::new();
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let mut input = InputStack::new(MemoryInput::new(""));
        let mut hooks = NoopExecHooks;

        assert_eq!(
            dispatch_delivered_token(
                Mode::Vertical,
                Token::Cs(relax),
                &mut input,
                &mut stores,
                &mut hooks
            )
            .expect("relax dispatch"),
            DispatchAction::Continue
        );
    }

    #[test]
    fn dispatch_character_hits_loud_typesetting_stub() {
        let mut stores = Stores::new();
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let mut input = InputStack::new(MemoryInput::new(""));
        let mut hooks = NoopExecHooks;

        assert_eq!(
            dispatch_delivered_token(Mode::Horizontal, token, &mut input, &mut stores, &mut hooks)
                .expect("character dispatch"),
            DispatchAction::NotConsumed
        );
    }

    #[test]
    fn main_control_uses_get_x_token_and_expands_macros_before_dispatch() {
        let mut stores = Stores::new();
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let mut input = InputStack::new(MemoryInput::new("\\relax"));

        let stats = Executor::new()
            .run(&mut input, &mut stores)
            .expect("execution succeeds");
        assert_eq!(stats.delivered_tokens, 1);
    }

    #[test]
    fn def_and_gdef_assign_macro_meanings_through_group_barrier() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("\\def\\a{A}\\gdef\\b{B}"));
        stores.enter_group();

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("definitions execute");
        let a = stores.symbol("a").expect("a was interned");
        let b = stores.symbol("b").expect("b was interned");
        assert!(matches!(stores.meaning(a), Meaning::Macro { .. }));
        assert!(matches!(stores.meaning(b), Meaning::Macro { .. }));

        let _ = stores.leave_group();
        assert_eq!(stores.meaning(a), Meaning::Undefined);
        assert!(matches!(stores.meaning(b), Meaning::Macro { .. }));
    }

    #[test]
    fn edef_omits_noexpand_command_and_freezes_the_output() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        install_expandable(&mut stores, "noexpand", ExpandablePrimitive::NoExpand);
        install_expandable(&mut stores, "the", ExpandablePrimitive::The);
        stores.intern("toks");
        stores.set_int_param(IntParam::END_LINE_CHAR, -1);
        let a = stores.intern("a");
        let b = stores.intern("b");
        let toks_body = stores.intern_token_list(&[Token::Cs(b)]);
        stores.set_toks(0, toks_body);
        let mut input = InputStack::new(MemoryInput::new("\\edef\\e{\\noexpand\\a\\the\\toks0}"));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("edef executes");
        let e = stores.symbol("e").expect("e was interned");
        let meaning = stores.macro_meaning(e).expect("e is a macro");

        assert_eq!(
            stores.tokens(meaning.replacement_text()),
            &[Token::Cs(a), Token::Cs(b)]
        );
    }

    #[test]
    fn edef_expansion_uses_active_input_hooks() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        install_expandable(&mut stores, "input", ExpandablePrimitive::Input);
        stores.set_int_param(IntParam::END_LINE_CHAR, -1);
        let mut input = InputStack::new(MemoryInput::new("\\edef\\e{\\input{inc}}"));
        let mut hooks = EdefInputHooks;

        Executor::new()
            .run_with_recorder_and_hooks(&mut input, &mut stores, &mut NoopRecorder, &mut hooks)
            .expect("edef executes through input hook");
        let e = stores.symbol("e").expect("e was interned");
        let meaning = stores.macro_meaning(e).expect("e is a macro");

        assert_eq!(
            stores.tokens(meaning.replacement_text()),
            &[
                Token::Char {
                    ch: 'O',
                    cat: Catcode::Letter
                },
                Token::Char {
                    ch: 'K',
                    cat: Catcode::Letter
                },
            ]
        );
    }

    #[test]
    fn let_assigns_control_sequence_and_implicit_character_meanings() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let a = stores.intern("a");
        stores.set_meaning(a, Meaning::CharGiven('Q'));
        let mut input = InputStack::new(MemoryInput::new("\\let\\b=\\a\\let\\c = Z"));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("let assignments execute");
        assert_eq!(
            stores.meaning(stores.symbol("b").expect("b was interned")),
            Meaning::CharGiven('Q')
        );
        assert_eq!(
            stores.meaning(stores.symbol("c").expect("c was interned")),
            Meaning::CharGiven('Z')
        );
    }

    #[test]
    fn futurelet_assigns_second_token_meaning_and_preserves_order() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let futurelet = stores.symbol("futurelet").expect("futurelet");
        let mut input = InputStack::new(MemoryInput::new("\\n\\first x"));
        let mut hooks = NoopExecHooks;

        dispatch_delivered_token(
            Mode::Vertical,
            Token::Cs(futurelet),
            &mut input,
            &mut stores,
            &mut hooks,
        )
        .expect("futurelet executes");

        let n = stores.symbol("n").expect("n was interned");
        assert_eq!(stores.meaning(n), Meaning::CharGiven('x'));
        assert_eq!(
            input.next_token(&mut stores).expect("first replayed"),
            Some(Token::Cs(stores.symbol("first").expect("first")))
        );
        assert_eq!(
            input.next_token(&mut stores).expect("second replayed"),
            Some(Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            })
        );
    }

    #[test]
    fn long_prefix_on_let_reports_tex_prefix_error() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("\\long\\let\\a=b"));

        let err = Executor::new()
            .run(&mut input, &mut stores)
            .expect_err("prefix is illegal");
        assert!(err.to_string().contains("You can't use a prefix with"));
    }

    #[test]
    fn globaldefs_forces_and_suppresses_global_assignments() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        stores.enter_group();
        let mut input = InputStack::new(MemoryInput::new(
            "\\globaldefs=1 \\def\\a{A}\\globaldefs=-1 \\gdef\\b{B}",
        ));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("globaldefs assignments execute");
        let a = stores.symbol("a").expect("a");
        let b = stores.symbol("b").expect("b");
        assert!(matches!(stores.meaning(a), Meaning::Macro { .. }));
        assert!(matches!(stores.meaning(b), Meaning::Macro { .. }));

        let _ = stores.leave_group();
        assert!(matches!(stores.meaning(a), Meaning::Macro { .. }));
        assert_eq!(stores.meaning(b), Meaning::Undefined);
    }

    #[test]
    fn brace_and_begingroup_groups_restore_local_assignments() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(
            "{\\count0=1\\global\\count1=2}\\begingroup\\count2=3\\endgroup",
        ));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("grouping primitives execute");

        assert_eq!(stores.count(0), 0);
        assert_eq!(stores.count(1), 2);
        assert_eq!(stores.count(2), 0);
    }

    #[test]
    fn aftergroup_replays_tokens_fifo_on_group_exit() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(
            "\\def\\A{\\count0=1}\\def\\B{\\count0=2}{\\aftergroup\\A\\aftergroup\\B}",
        ));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("aftergroup executes");

        assert_eq!(stores.count(0), 2);
    }

    #[test]
    fn afterassignment_fires_before_aftergroup_tokens() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(
            "\\def\\A{\\global\\count0=1}\\def\\B{\\global\\count0=2}{\\aftergroup\\B\\afterassignment\\A\\count1=7}",
        ));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("afterassignment and aftergroup execute");

        assert_eq!(stores.count(0), 2);
        assert_eq!(stores.count(1), 0);
    }

    #[test]
    fn afterassignment_slot_is_single_token_and_overwrites_previous() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(
            "\\def\\A{\\count0=1}\\def\\B{\\count0=2}\\afterassignment\\A\\afterassignment\\B\\count1=7",
        ));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("afterassignment executes");

        assert_eq!(stores.count(0), 2);
        assert_eq!(stores.count(1), 7);
    }

    #[test]
    fn group_mismatch_errors_use_tex_primary_text() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("}"));

        let err = Executor::new()
            .run(&mut input, &mut stores)
            .expect_err("extra right brace is an error");
        assert_eq!(err.to_string(), "Too many }'s.");

        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("\\begingroup}"));
        let err = Executor::new()
            .run(&mut input, &mut stores)
            .expect_err("right brace cannot close begingroup");
        assert_eq!(err.to_string(), "Extra }, or forgotten \\endgroup.");

        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("\\endgroup"));
        let err = Executor::new()
            .run(&mut input, &mut stores)
            .expect_err("extra endgroup is an error");
        assert_eq!(err.to_string(), "Extra \\endgroup.");

        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new("{\\endgroup"));
        let err = Executor::new()
            .run(&mut input, &mut stores)
            .expect_err("endgroup cannot close brace group");
        assert_eq!(err.to_string(), "\\endgroup ended a group started by {");
    }

    #[test]
    fn register_assignments_cover_sparse_aliases_and_arithmetic() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(
            "\\count300 = 7 \\countdef\\foo=300 \\advance\\foo by 5 \\multiply\\foo 3 \\divide\\foo by 2",
        ));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("register assignments execute");

        assert_eq!(stores.count(300), 18);
    }

    #[test]
    fn chardef_and_mathchardef_are_internal_integers() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(
            "\\chardef\\A=65 \\mathchardef\\M=\"7132 \\count0=\\A \\count1=\\M",
        ));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("character definitions execute");

        assert_eq!(stores.count(0), 65);
        assert_eq!(stores.count(1), 0x7132);
    }

    #[test]
    fn token_register_assignments_scan_balanced_text_and_copy_variables() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(
            "\\toks0={a{b}c}\\toksdef\\T=1 \\T=\\toks0",
        ));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("token assignments execute");

        assert_eq!(stores.tokens(stores.toks(0)), stores.tokens(stores.toks(1)));
        assert_eq!(stores.tokens(stores.toks(0)).len(), 5);
    }

    #[test]
    fn glue_arithmetic_preserves_fil_order_rules() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(
            "\\skip0=1pt plus 2fil minus 6pt \\advance\\skip0 by 3pt plus 4fill minus 1pt \\divide\\skip0 by 2",
        ));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("glue arithmetic executes");
        let spec = stores.glue(stores.skip(0));

        assert_eq!(spec.width.raw(), 2 * tex_state::scaled::Scaled::UNITY);
        assert_eq!(spec.stretch.raw(), 2 * tex_state::scaled::Scaled::UNITY);
        assert_eq!(spec.stretch_order, tex_state::glue::Order::Fill);
        assert_eq!(spec.shrink.raw(), 7 * tex_state::scaled::Scaled::UNITY / 2);
        assert_eq!(spec.shrink_order, tex_state::glue::Order::Normal);
    }

    #[test]
    fn arithmetic_overflow_reports_tex_error_text() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(
            "\\count0=2147483647 \\advance\\count0 by 1",
        ));

        let err = Executor::new()
            .run(&mut input, &mut stores)
            .expect_err("advance should overflow");

        assert_eq!(err.to_string(), "Arithmetic overflow");
    }

    #[test]
    fn code_table_assignment_validates_and_bumps_generation_on_same_value() {
        let mut stores = Stores::new();
        install_unexpandable_primitives(&mut stores);
        let before = stores.code_table_generations();
        let mut input = InputStack::new(MemoryInput::new("\\catcode`\\@=12 \\catcode`\\@=12"));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("catcode assignments execute");
        let after = stores.code_table_generations();

        assert_eq!(stores.catcode('@'), Catcode::Other);
        assert_eq!(after.catcode, before.catcode + 2);
    }

    fn install_expandable(stores: &mut Stores, name: &str, primitive: ExpandablePrimitive) {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
    }

    struct EdefInputHooks;

    impl ExpansionHooks<MemoryInput> for EdefInputHooks {
        fn open_input(&mut self, name: &str) -> Result<MemoryInput, String> {
            if name == "inc" {
                Ok(MemoryInput::new("OK"))
            } else {
                Err(format!("unexpected input {name}"))
            }
        }
    }
}
