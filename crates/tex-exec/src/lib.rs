//! TeX execution engine scaffold.
//!
//! This crate owns the stomach's mode nest and main-control dispatch. It pulls
//! only fully expanded tokens from `tex_expand::get_x_token*`; raw token reads
//! stay in the lexer/gullet pipeline.

#![forbid(unsafe_code)]

use std::fmt;

use tex_expand::{
    EngineMode, ExpandError, ExpansionHooks, NoopRecorder, ReadRecorder,
    get_x_token_with_recorder_and_hooks,
};
use tex_lex::{InputSource, InputStack};
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::stores::Stores;
use tex_state::token::Token;

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
        let mut stats = ExecutionStats::default();
        loop {
            let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, self)?
            else {
                return Ok(stats);
            };
            stats.delivered_tokens += 1;
            match dispatch_delivered_token(self.nest.current_mode(), token, stores)? {
                DispatchAction::Continue => {}
            }
        }
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
}

/// Dispatches one gullet-delivered token in the current mode.
pub fn dispatch_delivered_token(
    mode: Mode,
    token: Token,
    stores: &Stores,
) -> Result<DispatchAction, ExecError> {
    let meaning = match token {
        Token::Cs(symbol) => stores.meaning(symbol),
        Token::Char { .. } | Token::Param(_) => {
            return unimplemented_typesetting(mode, token, "character material");
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
    UnsupportedCommand {
        token: Token,
        opcode: u8,
    },
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
            Self::UnsupportedCommand { token, opcode } => {
                write!(
                    f,
                    "unsupported unexpandable opcode {opcode} for token {token:?}"
                )
            }
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
            Self::EmptyModeNestSummary
            | Self::CannotPopBaseMode
            | Self::UndefinedControlSequence { .. }
            | Self::UnexpectedMacroDelivery { .. }
            | Self::UnexpectedExpandableDelivery { .. }
            | Self::ExtraConditionalControl(_)
            | Self::ExtraEndCsName
            | Self::UnsupportedCommand { .. }
            | Self::UnimplementedTypesetting { .. } => None,
        }
    }
}

impl From<ExpandError> for ExecError {
    fn from(value: ExpandError) -> Self {
        Self::Expand(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_lex::MemoryInput;
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

        assert_eq!(
            dispatch_delivered_token(Mode::Vertical, Token::Cs(relax), &stores)
                .expect("relax dispatch"),
            DispatchAction::Continue
        );
    }

    #[test]
    fn dispatch_character_hits_loud_typesetting_stub() {
        let stores = Stores::new();
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };

        let err = dispatch_delivered_token(Mode::Horizontal, token, &stores)
            .expect_err("characters need typesetting");
        assert!(matches!(
            err,
            ExecError::UnimplementedTypesetting {
                mode: Mode::Horizontal,
                token: Token::Char { ch: 'x', .. },
                operation: "character material",
            }
        ));
        assert!(
            err.to_string()
                .contains("typesetting path is not implemented yet")
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
}
