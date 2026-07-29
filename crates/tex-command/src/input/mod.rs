//! Private input state machines.

mod backup;
mod levels;
mod lines;
mod source;
mod stack;
mod summary;
mod tokenizer;

pub(crate) use levels::{
    BackedUpToken, BackupTreatment, InputLevel, InputLevelId, ReplayTrace, RetirementBehavior,
    SharedBackedUpBuffer, SharedTokenBuffer, SourceLevel, SourceRetirement, StoredReplayReason,
    TokenBehavior, TokenCursor, TokenPayload, TransientReplayReason,
};
pub(crate) use source::{LineBackingRegistry, RegisteredSource, SourceCursor};
#[allow(unused_imports)] // consumed by the ordered raw-delivery implementation issues
pub(crate) use stack::{
    InputRetirement, InputRetirementAction, InputRetirementError, InputRetirementReason,
    OutParameterReplay, ParameterReplayError, input_level_identity,
};

pub use lines::{
    LineTerminator, PhysicalLine, SourceCharacter, SourceLocation, SourceProvenance, SourceRange,
    SourceScalarRange,
};
pub use source::{
    MalformedUnicodeRange, RegisteredSourceKind, SourceNameClass, SourceRegistration,
    SourceRegistrationError,
};
pub use tokenizer::{
    CatcodeQueries, InvalidSourceCharacter, LexerState, SourceControlSequenceKind,
    SourceStepQueries, SourceToken, SourceTokenizationStep,
};

/// Persistent input-stack ownership.
///
/// This state owns only future deliveries and semantic identity allocation.
/// Conditions, scanner policy, meanings, and host capabilities belong to
/// other ownership classes.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct InputState {
    pub(crate) levels: Vec<InputLevel>,
    pub(crate) registered_sources: Vec<RegisteredSource>,
    pub(crate) next_level_identity: u64,
    pub(crate) next_source_identity: u64,
    /// TeX82 §362's process-global `force_eof`.
    pub(crate) force_eof: bool,
}

impl InputState {
    pub(crate) fn output_open_context(&self) -> String {
        let Some(source) = self.levels.iter().rev().find_map(|level| {
            let InputLevel::Source(source) = level else {
                return None;
            };
            Some(source)
        }) else {
            return String::new();
        };
        let Some(line) = source.cursor.line.as_ref() else {
            return String::new();
        };
        let bytes = &source.cursor.current_backing().bytes;
        let start = line.physical.content_range().start();
        let end = line.retained_end;
        let cursor = line.byte_cursor.clamp(start, end);
        let Ok(start) = usize::try_from(start) else {
            return String::new();
        };
        let Ok(end) = usize::try_from(end) else {
            return String::new();
        };
        let Ok(cursor) = usize::try_from(cursor) else {
            return String::new();
        };
        let read = String::from_utf8_lossy(&bytes[start..cursor]);
        let remaining = String::from_utf8_lossy(&bytes[cursor..end]);
        let label = format!("l.{} ", line.physical.number());
        format!(
            "\n{label}{read}\n{}{remaining}",
            " ".repeat(label.chars().count())
        )
    }

    /// TeX82's current `line` value for e-TeX's `\inputlineno`.
    ///
    /// Token-list levels retain the source line they interrupted; terminal and
    /// `\read` levels have no file line number and therefore expose zero.
    pub(crate) fn current_file_line_number(&self) -> i32 {
        self.levels
            .iter()
            .rev()
            .find_map(|level| match level {
                InputLevel::Source(source)
                    if matches!(source.name_class, SourceNameClass::File) =>
                {
                    source
                        .cursor
                        .line
                        .as_ref()
                        .map(|line| line.physical.number().min(i32::MAX as u64) as i32)
                }
                InputLevel::Source(_) => Some(0),
                InputLevel::Tokens(_) => None,
            })
            .unwrap_or(0)
    }
}
