//! Fused resident-input advancement and raw/expanded command delivery.

use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, ResolvedMeaning};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use crate::command::{
    CommandClass, DeliveryStamp, HotCommand, HotPrimitiveInvocation, MacroMatchDelivery,
};
use crate::execution_scratch::ArgumentSetId;
use crate::expansion_work::ActiveControlTag;
use crate::input::{
    InputLevel, InputLevelId, PackedInputFrame, ResidentBoundary, ResidentSourceAdvance,
    ResidentSourceCharacterRun, ResidentSourceTop, ResidentTokenStorage, SourceLocation,
    SourceNameClass, TokenBehavior,
};
use crate::{CommandError, CommandReplayDelivery, CurrentCommand};

use super::end_input::{RetirementHandoff, SourceExhaustionStatus};
use super::expand_render::format_pdf_date;
use super::{
    AlignmentLookahead, CommandProcessor, DeliveryStatus, MainCharacterConsumer, MainCharacterInput,
};

use crate::observation::{
    CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation, CommandProvenance,
    InputReason, InputRecord, InputTransition,
};

/// TeX82 §345's invalid source-character report.
const INVALID_SOURCE_CHARACTER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0345;

enum ResidentColdOutcome {
    Retry,
    Finished(DeliveryStatus),
    Synthetic { literal_catcode: Option<Catcode> },
}
#[derive(Clone, Copy)]
enum ExpandedUntilMode {
    Protected,
    PreserveUndefined,
}
#[cfg(test)]
#[derive(Clone, Copy)]
enum ResidentStorageKind {
    Stored,
    MacroBody,
    MacroArgument,
}
enum InputFrameTransition<G> {
    Boundary(ResidentBoundary),
    Source {
        resident_index: usize,
    },
    ResidentExhausted {
        resident_index: usize,
        identity: InputLevelId,
    },
    Parameter {
        slot: u8,
        arguments: Option<ArgumentSetId<G>>,
        active_source: Option<tex_state::packed_input::SourceContext>,
    },
}

/// One packed word selected from a resident token row. The selection keeps
/// the row's already-admitted coordinates beside the word so the delivery
/// loops can either resolve it into the hot command or consume an ordinary
/// character directly in the main-control run.
enum ResidentWordRead<G> {
    NoResident,
    Source {
        resident_index: usize,
    },
    Parameter {
        slot: u8,
        arguments: Option<ArgumentSetId<G>>,
        active_source: Option<tex_state::packed_input::SourceContext>,
    },
    Exhausted {
        resident_index: usize,
        identity: InputLevelId,
    },
    Word {
        word: TokenWord,
        origin: OriginId,
        identity: u64,
        position: u64,
        active_source: Option<tex_state::packed_input::SourceContext>,
        suppress_expandable: bool,
        #[cfg(test)]
        storage_kind: ResidentStorageKind,
        #[cfg(feature = "profiling")]
        raw_kind: crate::fuel::RawDeliveryKind,
    },
}

/// Reads one packed word from an already-selected resident storage domain.
///
/// Stack mutation, exhaustion, substitution, diagnostics, and recovery must
/// remain outside this instruction body. The loader is specific to the
/// selected lifetime domain and the packed frame remains the sole logical
/// cursor shared by all of them.
#[inline(always)]
fn next_word_from_current_frame(
    frame: &mut PackedInputFrame,
    load: impl FnOnce(u32) -> Option<(TokenWord, OriginId)>,
) -> Option<(TokenWord, OriginId, u32)> {
    let position = frame.position();
    if position >= frame.limit() {
        return None;
    }
    let (word, origin) = load(position)?;
    let consumed = frame.advance_resident();
    debug_assert_eq!(consumed, position);
    Some((word, origin, position))
}

/// Reads one word from an admitted macro replacement cursor.
///
/// The hot path checks the packed frame bound, loads the retained physical
/// slot, advances the frame's sole logical position, and then advances the
/// body's physical cache. A physical crossing is settled by its cold
/// directory transition only after the frame confirms that replacement words
/// remain.
#[inline(always)]
fn next_macro_body_word_from_current_frame<G>(
    frame: &mut PackedInputFrame,
    body: &mut crate::input::MacroBodyCursor<G>,
) -> Option<(TokenWord, OriginId, u32)> {
    let position = frame.position();
    if position >= frame.limit() {
        return None;
    }
    let word = body.body.load_current_word()?;
    let consumed = frame.advance_resident();
    debug_assert_eq!(consumed, position);
    let boundary = body.body.advance_current_word();
    if boundary && frame.position() < frame.limit() {
        body.body.advance_chunk_cold(frame.position());
    }
    Some((word, OriginId::UNKNOWN, position))
}

fn static_meaning<G>(meaning: &ResolvedMeaning<G>) -> Option<Meaning> {
    match meaning {
        ResolvedMeaning::Static(meaning) => Some(*meaning),
        ResolvedMeaning::Macro { .. } => None,
    }
}

/// The one decision TeX.web §380 makes after raw delivery has resolved the
/// current meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpandedCommandAction {
    Return,
    EndTemplate,
    Expand(ExpansionDispatch),
}

enum ExpandedHotDispatch {
    Continue,
    Finished(DeliveryStatus),
}

/// Exact parent capability carried between the loop and one child admission.
/// `Captured` is a live parent slot that still needs its one Await transition;
/// `Awaiting` is the same slot restored from an inline frame link. Encoding
/// that distinction in the value keeps admission/rollback from relying on a
/// parallel boolean or on rediscovering the lane top.
#[derive(Debug)]
enum ParentAdmission<G> {
    Captured(crate::expansion_work::ExpansionControlSlot<G>),
    Awaiting(crate::expansion_work::ExpansionControlSlot<G>),
}

#[derive(Debug)]
struct ResumedExpandedDelivery<G> {
    command: HotCommand<G>,
    delivery_expanded: bool,
    parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    return_capability: Option<crate::expansion_work::control::ExpansionReturnCapability<G>>,
}

impl<G> Copy for ParentAdmission<G> {}

impl<G> Clone for ParentAdmission<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> ParentAdmission<G> {
    #[inline]
    fn slot(self) -> crate::expansion_work::ExpansionControlSlot<G> {
        match self {
            Self::Captured(slot) | Self::Awaiting(slot) => slot,
        }
    }

    #[inline]
    const fn needs_await(self) -> bool {
        matches!(self, Self::Captured(_))
    }
}

enum ActiveControlSnapshot<G> {
    Return(crate::expansion_work::ExpansionControlSlot<G>),
    Expanded(
        crate::expansion_work::ExpansionControlView<
            G,
            crate::expansion_work::control::SynchronousExpandedControl,
        >,
    ),
    ExpandAfterSync(
        crate::expansion_work::ExpansionControlView<
            G,
            crate::expansion_work::control::SynchronousExpandAfterControl<G>,
        >,
    ),
    IfCompare(
        crate::expansion_work::ExpansionControlView<
            G,
            crate::expansion_work::control::SynchronousIfCompareControl,
        >,
    ),
    IfNumber(
        crate::expansion_work::ExpansionControlView<
            G,
            crate::expansion_work::control::SynchronousIfNumberControl,
        >,
    ),
    IfDimension(
        crate::expansion_work::ExpansionControlView<
            G,
            crate::expansion_work::control::SynchronousIfDimensionControl,
        >,
    ),
    Number(
        crate::expansion_work::ExpansionControlView<
            G,
            crate::expansion_work::control::SynchronousNumberControl,
        >,
    ),
    PdfXImageBBox,
    FontName,
    CsName,
    IfCsName,
    The(crate::expansion_work::ExpansionControlView<G, crate::expansion_work::control::TheControl>),
}

impl<G> Copy for ActiveControlSnapshot<G> {}

impl<G> Clone for ActiveControlSnapshot<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> ActiveControlSnapshot<G> {
    /// Returns the exact active parent capability when its phase can suspend
    /// around a nested expanded child. The snapshot already contains the
    /// typed lane slot, so dispatch does not rediscover the top control.
    fn awaitable_slot(self) -> Option<crate::expansion_work::ExpansionControlSlot<G>> {
        match self {
            Self::Return(slot) => Some(slot),
            Self::ExpandAfterSync(control)
                if control.phase
                    == crate::expansion_work::control::SynchronousExpandAfterPhase::NeedSecond =>
            {
                Some(control.slot)
            }
            Self::IfCompare(control)
                if matches!(
                    control.phase,
                    crate::expansion_work::control::SynchronousIfComparePhase::NeedFirst
                        | crate::expansion_work::control::SynchronousIfComparePhase::NeedSecond {
                            ..
                        }
                ) => Some(control.slot),
            Self::IfNumber(control)
                if matches!(
                    control.phase,
                    crate::expansion_work::control::SynchronousIfNumberPhase::NeedLeft
                        | crate::expansion_work::control::SynchronousIfNumberPhase::Left {
                            ..
                        }
                        | crate::expansion_work::control::SynchronousIfNumberPhase::NeedRelation {
                            ..
                        }
                        | crate::expansion_work::control::SynchronousIfNumberPhase::Right {
                            ..
                        }
                        | crate::expansion_work::control::SynchronousIfNumberPhase::RegisterIndex {
                            ..
                        }
                ) => Some(control.slot),
            Self::IfDimension(control)
                if matches!(
                    control.phase,
                    crate::expansion_work::control::SynchronousIfDimensionPhase::NeedLeft
                        | crate::expansion_work::control::SynchronousIfDimensionPhase::Left {
                            ..
                        }
                        | crate::expansion_work::control::SynchronousIfDimensionPhase::NeedRelation {
                            ..
                        }
                        | crate::expansion_work::control::SynchronousIfDimensionPhase::Right {
                            ..
                        }
                        | crate::expansion_work::control::SynchronousIfDimensionPhase::RegisterIndex {
                            ..
                        }
                ) => Some(control.slot),
                    Self::Number(control)
                if matches!(
                    control.phase,
                    crate::expansion_work::control::SynchronousNumberPhase::Need
                        | crate::expansion_work::control::SynchronousNumberPhase::Leading { .. }
                        | crate::expansion_work::control::SynchronousNumberPhase::Accumulating {
                            ..
                        }
                        | crate::expansion_work::control::SynchronousNumberPhase::RegisterIndex {
                            ..
                        }
                ) => Some(control.slot),
            _ => None,
        }
    }
}

/// The exact TeX.web §366 branch selected by expanded-command
/// classification. This is call-local control flow, not a retained meaning
/// representation: a resource suspension continues to own only its one
/// `CurrentCommand` and re-borrows that meaning when the operation resumes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExpansionDispatch {
    Macro,
    Primitive(ExpandablePrimitive),
    Undefined,
}

/// Primitive branches in the rich dispatcher that install a compact
/// synchronous frame before they request another expanded token. Parent
/// admission is restricted to these branches; ordinary expandable commands
/// either mutate input or finish without creating a child frame. The other
/// compact branches are cut over earlier in `dispatch_expanded_action`.
#[inline]
fn starts_synchronous_control(dispatch: ExpansionDispatch) -> bool {
    matches!(
        dispatch,
        ExpansionDispatch::Primitive(
            ExpandablePrimitive::ExpandAfter
                | ExpandablePrimitive::CsName
                | ExpandablePrimitive::IfCsName
                | ExpandablePrimitive::The
                | ExpandablePrimitive::Unless
        )
    )
}

/// Primitive families that can consume their opener from the occupied hot
/// command.  These are either copy-small control-lane starters or operand-free
/// conversions; rich scanner/observer/resource families stay on the cold arm.
#[inline(always)]
fn is_hot_synchronous_primitive(primitive: ExpandablePrimitive) -> bool {
    matches!(
        primitive,
        ExpandablePrimitive::Expanded
            | ExpandablePrimitive::ExpandAfter
            | ExpandablePrimitive::CsName
            | ExpandablePrimitive::IfCsName
            | ExpandablePrimitive::The
            | ExpandablePrimitive::If
            | ExpandablePrimitive::IfCat
            | ExpandablePrimitive::IfNum
            | ExpandablePrimitive::IfPdfAbsNum
            | ExpandablePrimitive::IfDim
            | ExpandablePrimitive::IfPdfAbsDim
            | ExpandablePrimitive::IfOdd
            | ExpandablePrimitive::IfCase
            | ExpandablePrimitive::IfVoid
            | ExpandablePrimitive::IfHBox
            | ExpandablePrimitive::IfVBox
            | ExpandablePrimitive::IfEof
            | ExpandablePrimitive::IfFontChar
            | ExpandablePrimitive::FontName
            | ExpandablePrimitive::PdfFontSize
            | ExpandablePrimitive::PdfFontName
            | ExpandablePrimitive::PdfFontObjectNumber
            | ExpandablePrimitive::PdfInsertHeight
            | ExpandablePrimitive::PdfXFormName
            | ExpandablePrimitive::PdfPageRef
            | ExpandablePrimitive::PdfLastMatch
            | ExpandablePrimitive::PdfXImageBBox
            | ExpandablePrimitive::PdfEscapeString
            | ExpandablePrimitive::PdfEscapeHex
            | ExpandablePrimitive::PdfUnescapeHex
            | ExpandablePrimitive::StringCompare
            | ExpandablePrimitive::TopMark
            | ExpandablePrimitive::FirstMark
            | ExpandablePrimitive::BotMark
            | ExpandablePrimitive::SplitFirstMark
            | ExpandablePrimitive::SplitBotMark
            | ExpandablePrimitive::TopMarks
            | ExpandablePrimitive::FirstMarks
            | ExpandablePrimitive::BotMarks
            | ExpandablePrimitive::SplitFirstMarks
            | ExpandablePrimitive::SplitBotMarks
            | ExpandablePrimitive::Number
            | ExpandablePrimitive::RomanNumeral
            | ExpandablePrimitive::PdfUniformDeviate
            | ExpandablePrimitive::LeftMarginKern
            | ExpandablePrimitive::RightMarginKern
            | ExpandablePrimitive::EndInput
            | ExpandablePrimitive::JobName
            | ExpandablePrimitive::ETeXRevision
            | ExpandablePrimitive::PdfTeXRevision
            | ExpandablePrimitive::PdfTeXBanner
            | ExpandablePrimitive::PdfNormalDeviate
            | ExpandablePrimitive::CreationDate
            | ExpandablePrimitive::ShellEscape
    )
}

#[inline(always)]
fn hot_primitive_starts_control(primitive: ExpandablePrimitive) -> bool {
    matches!(
        primitive,
        ExpandablePrimitive::Expanded
            | ExpandablePrimitive::ExpandAfter
            | ExpandablePrimitive::CsName
            | ExpandablePrimitive::IfCsName
            | ExpandablePrimitive::The
            | ExpandablePrimitive::If
            | ExpandablePrimitive::IfCat
            | ExpandablePrimitive::IfNum
            | ExpandablePrimitive::IfPdfAbsNum
            | ExpandablePrimitive::IfDim
            | ExpandablePrimitive::IfPdfAbsDim
            | ExpandablePrimitive::IfOdd
            | ExpandablePrimitive::IfCase
            | ExpandablePrimitive::IfVoid
            | ExpandablePrimitive::IfHBox
            | ExpandablePrimitive::IfVBox
            | ExpandablePrimitive::IfEof
            | ExpandablePrimitive::IfFontChar
            | ExpandablePrimitive::FontName
            | ExpandablePrimitive::PdfFontSize
            | ExpandablePrimitive::PdfFontName
            | ExpandablePrimitive::PdfFontObjectNumber
            | ExpandablePrimitive::PdfInsertHeight
            | ExpandablePrimitive::PdfXFormName
            | ExpandablePrimitive::PdfPageRef
            | ExpandablePrimitive::PdfLastMatch
            | ExpandablePrimitive::PdfXImageBBox
            | ExpandablePrimitive::PdfEscapeString
            | ExpandablePrimitive::PdfEscapeHex
            | ExpandablePrimitive::PdfUnescapeHex
            | ExpandablePrimitive::StringCompare
            | ExpandablePrimitive::TopMarks
            | ExpandablePrimitive::FirstMarks
            | ExpandablePrimitive::BotMarks
            | ExpandablePrimitive::SplitFirstMarks
            | ExpandablePrimitive::SplitBotMarks
            | ExpandablePrimitive::Number
            | ExpandablePrimitive::RomanNumeral
            | ExpandablePrimitive::PdfUniformDeviate
            | ExpandablePrimitive::LeftMarginKern
            | ExpandablePrimitive::RightMarginKern
    )
}

#[inline(always)]
fn primitive_owns_parent(primitive: ExpandablePrimitive) -> bool {
    hot_primitive_starts_control(primitive) || matches!(primitive, ExpandablePrimitive::Unless)
}

#[cfg(test)]
thread_local! {
    static EXPANDED_CLASSIFICATIONS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Focused ownership/dispatch evidence for the expanded hot loop.  These
/// counters are test/profiling instrumentation only; the production loop has
/// no side ledger or dispatch token.
#[cfg(any(test, feature = "profiling"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExpansionHotCounters {
    pub(crate) primitive_hot_dispatches: u64,
    pub(crate) active_dispatch_calls: u64,
    pub(crate) primitive_cold_materializations: u64,
}

#[cfg(any(test, feature = "profiling"))]
thread_local! {
    static EXPANSION_HOT_COUNTERS:
        core::cell::Cell<ExpansionHotCounters> = const { core::cell::Cell::new(
            ExpansionHotCounters {
                primitive_hot_dispatches: 0,
                active_dispatch_calls: 0,
                primitive_cold_materializations: 0,
            },
        ) };
}

#[inline(always)]
fn record_primitive_hot_dispatch() {
    #[cfg(any(test, feature = "profiling"))]
    EXPANSION_HOT_COUNTERS.with(|counter| {
        let mut value = counter.get();
        value.primitive_hot_dispatches = value.primitive_hot_dispatches.saturating_add(1);
        counter.set(value);
    });
}

#[inline(always)]
fn record_active_dispatch_call() {
    #[cfg(any(test, feature = "profiling"))]
    EXPANSION_HOT_COUNTERS.with(|counter| {
        let mut value = counter.get();
        value.active_dispatch_calls = value.active_dispatch_calls.saturating_add(1);
        counter.set(value);
    });
}

#[inline(always)]
fn record_primitive_cold_materialization() {
    #[cfg(any(test, feature = "profiling"))]
    EXPANSION_HOT_COUNTERS.with(|counter| {
        let mut value = counter.get();
        value.primitive_cold_materializations =
            value.primitive_cold_materializations.saturating_add(1);
        counter.set(value);
    });
}

#[cfg(any(test, feature = "profiling"))]
#[allow(dead_code)]
pub(crate) fn expansion_hot_counters() -> ExpansionHotCounters {
    EXPANSION_HOT_COUNTERS.with(core::cell::Cell::get)
}

#[cfg(test)]
fn expanded_classifications() -> u64 {
    EXPANDED_CLASSIFICATIONS.with(core::cell::Cell::get)
}

#[inline(always)]
fn classify_expanded_command<G>(command: &CurrentCommand<G>) -> ExpandedCommandAction {
    #[cfg(test)]
    EXPANDED_CLASSIFICATIONS.with(|counter| counter.set(counter.get().saturating_add(1)));

    match command.meaning_ref() {
        ResolvedMeaning::Macro { .. } => ExpandedCommandAction::Expand(ExpansionDispatch::Macro),
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)) => {
            ExpandedCommandAction::EndTemplate
        }
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName)) => {
            ExpandedCommandAction::Return
        }
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(primitive)) => {
            ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(*primitive))
        }
        ResolvedMeaning::Static(Meaning::Undefined)
            if !matches!(command.spelling().semantic_token(), Token::Param(_)) =>
        {
            ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
        }
        ResolvedMeaning::Static(_) => ExpandedCommandAction::Return,
    }
}

#[inline(always)]
fn classify_hot_command<G>(command: &HotCommand<G>) -> ExpandedCommandAction {
    #[cfg(test)]
    EXPANDED_CLASSIFICATIONS.with(|counter| counter.set(counter.get().saturating_add(1)));

    let word = command.command_word();
    match word.class() {
        CommandClass::Macro => ExpandedCommandAction::Expand(ExpansionDispatch::Macro),
        CommandClass::Expandable => match word.expandable_primitive() {
            Some(ExpandablePrimitive::EndTemplate) => ExpandedCommandAction::EndTemplate,
            Some(ExpandablePrimitive::EndCsName) => ExpandedCommandAction::Return,
            Some(primitive) => {
                ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(primitive))
            }
            None => ExpandedCommandAction::Return,
        },
        CommandClass::Undefined if command.spelling_word().out_parameter_slot().is_none() => {
            ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
        }
        _ => ExpandedCommandAction::Return,
    }
}

/// The finite expansion set selected by the pinned structural census.
///
/// These families execute against the borrowed live command in the one
/// processor episode. Everything else remains a cold arm in this same
/// interpreter; the profiling materialization counter records only that
/// explicit fallback boundary.
#[inline(always)]
#[cfg(feature = "profiling")]
fn is_ranked_fused_expansion(dispatch: ExpansionDispatch) -> bool {
    matches!(
        dispatch,
        ExpansionDispatch::Macro
            | ExpansionDispatch::Primitive(
                ExpandablePrimitive::ExpandAfter
                    | ExpandablePrimitive::Fi
                    | ExpandablePrimitive::IfX
                    | ExpandablePrimitive::IfNum
                    | ExpandablePrimitive::If
                    | ExpandablePrimitive::CsName
                    | ExpandablePrimitive::NoExpand
                    | ExpandablePrimitive::Detokenize
                    | ExpandablePrimitive::String
                    | ExpandablePrimitive::IfFalse
                    | ExpandablePrimitive::RomanNumeral
                    | ExpandablePrimitive::Else
                    | ExpandablePrimitive::Expanded
                    | ExpandablePrimitive::IfCsName
                    | ExpandablePrimitive::Number
                    | ExpandablePrimitive::The
                    | ExpandablePrimitive::PdfUniformDeviate
                    | ExpandablePrimitive::PdfXImageBBox
            )
    )
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Selects one word from the already-admitted resident row. This is the
    /// small cursor instruction shared by scalar delivery and the main-loop
    /// character path; source rows and all exhaustion transitions stay in the
    /// cold reader below.
    #[inline(always)]
    fn next_resident_word(&mut self) -> Result<ResidentWordRead<G>, CommandError> {
        let command_state = &mut *self.command;
        let Some(resident_index) = command_state.roots.input.levels.top.checked_sub(1) else {
            return Ok(ResidentWordRead::NoResident);
        };
        #[cfg(test)]
        {
            command_state
                .roots
                .input
                .levels
                .cursor_mutations
                .typed_top_accesses = command_state
                .roots
                .input
                .levels
                .cursor_mutations
                .typed_top_accesses
                .saturating_add(1);
            command_state
                .raw_delivery_path_counters
                .resident_transitions = command_state
                .raw_delivery_path_counters
                .resident_transitions
                .saturating_add(1);
        }

        let InputLevel::Resident(row) = &mut command_state.roots.input.levels.rows[resident_index]
        else {
            return Ok(ResidentWordRead::Source { resident_index });
        };
        let exhausted_identity = row.header.identity();
        let identity = exhausted_identity.0;
        let active_source = row.header.frame.source_context();
        let suppress_expandable = row.header.frame.flags().contains(
            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
        );
        #[cfg(test)]
        let storage_kind = match &row.storage {
            ResidentTokenStorage::MacroBody(_) => ResidentStorageKind::MacroBody,
            ResidentTokenStorage::MacroArgument(_) => ResidentStorageKind::MacroArgument,
            ResidentTokenStorage::Replay { .. }
            | ResidentTokenStorage::Attempt(_)
            | ResidentTokenStorage::Durable(_) => ResidentStorageKind::Stored,
        };
        #[cfg(feature = "profiling")]
        let raw_kind = match &row.storage {
            ResidentTokenStorage::MacroArgument(_) => crate::fuel::RawDeliveryKind::MacroArgument,
            ResidentTokenStorage::Replay { .. }
            | ResidentTokenStorage::Attempt(_)
            | ResidentTokenStorage::Durable(_)
            | ResidentTokenStorage::MacroBody(_) => crate::fuel::RawDeliveryKind::StoredToken,
        };

        let current = match &mut row.storage {
            ResidentTokenStorage::Replay { replay, cursor } => {
                #[cfg(test)]
                {
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .replay_domain_dispatches = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .replay_domain_dispatches
                        .saturating_add(1);
                    command_state.stored_token_advance_counters.span_selections = command_state
                        .stored_token_advance_counters
                        .span_selections
                        .saturating_add(1);
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries
                        .saturating_add(1);
                }
                next_word_from_current_frame(&mut row.header.frame, |_position| {
                    command_state
                        .roots
                        .input
                        .replay
                        .advance_sequential(
                            *replay,
                            cursor,
                            #[cfg(test)]
                            &mut command_state
                                .stored_token_advance_counters
                                .replay_segment_inspections,
                            #[cfg(test)]
                            &mut command_state
                                .stored_token_advance_counters
                                .replay_run_transitions,
                        )
                        .map(|word| (word.token_word(), word.origin()))
                })
            }
            ResidentTokenStorage::Attempt(list) => {
                #[cfg(test)]
                {
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .attempt_domain_dispatches = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .attempt_domain_dispatches
                        .saturating_add(1);
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries
                        .saturating_add(1);
                }
                next_word_from_current_frame(&mut row.header.frame, |position| {
                    command_state
                        .attempt
                        .arena()
                        .resident_token_word(list, position as usize)
                        .map(|word| (word.token_word(), word.origin()))
                })
            }
            ResidentTokenStorage::Durable(list) => {
                #[cfg(test)]
                {
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .durable_domain_dispatches = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .durable_domain_dispatches
                        .saturating_add(1);
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries
                        .saturating_add(1);
                }
                next_word_from_current_frame(&mut row.header.frame, |position| {
                    list.word_at(position as usize)
                        .map(|word| (word, tex_state::token::OriginId::UNKNOWN))
                })
            }
            ResidentTokenStorage::MacroBody(body) => {
                #[cfg(test)]
                {
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .macro_body_domain_dispatches = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .macro_body_domain_dispatches
                        .saturating_add(1);
                }
                next_macro_body_word_from_current_frame(&mut row.header.frame, body)
            }
            ResidentTokenStorage::MacroArgument(argument) => {
                #[cfg(test)]
                {
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .macro_argument_branch_entries = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .macro_argument_branch_entries
                        .saturating_add(1);
                }
                next_word_from_current_frame(&mut row.header.frame, |position| {
                    argument.advance_delivery(position, &command_state.scratch)
                })
            }
        };

        let Some((word, origin, position)) = current else {
            return Ok(ResidentWordRead::Exhausted {
                resident_index,
                identity: exhausted_identity,
            });
        };

        #[cfg(test)]
        match storage_kind {
            ResidentStorageKind::Stored => {
                command_state.stored_token_advance_counters.packed_loads = command_state
                    .stored_token_advance_counters
                    .packed_loads
                    .saturating_add(1);
                command_state.stored_token_advance_counters.cursor_advances = command_state
                    .stored_token_advance_counters
                    .cursor_advances
                    .saturating_add(1);
            }
            ResidentStorageKind::MacroBody => {
                command_state.macro_kernel_counters.body_words = command_state
                    .macro_kernel_counters
                    .body_words
                    .saturating_add(1);
                command_state.macro_kernel_counters.body_frame_advances = command_state
                    .macro_kernel_counters
                    .body_frame_advances
                    .saturating_add(1);
            }
            ResidentStorageKind::MacroArgument => {
                command_state.macro_kernel_counters.argument_words = command_state
                    .macro_kernel_counters
                    .argument_words
                    .saturating_add(1);
                command_state.macro_kernel_counters.argument_cursor_advances = command_state
                    .macro_kernel_counters
                    .argument_cursor_advances
                    .saturating_add(1);
            }
        }

        let arguments = match &row.storage {
            ResidentTokenStorage::MacroBody(body) => Some(body.arguments),
            _ if !matches!(row.header.behavior(), TokenBehavior::Parameter) => Some(None),
            _ => None,
        };
        if let Some(arguments) = arguments
            && let Some(slot) = word.out_parameter_slot()
        {
            #[cfg(test)]
            match storage_kind {
                ResidentStorageKind::Stored => {
                    command_state
                        .stored_token_advance_counters
                        .parameter_interceptions = command_state
                        .stored_token_advance_counters
                        .parameter_interceptions
                        .saturating_add(1);
                }
                ResidentStorageKind::MacroBody => {
                    command_state.macro_kernel_counters.body_parameter_pushes = command_state
                        .macro_kernel_counters
                        .body_parameter_pushes
                        .saturating_add(1);
                }
                ResidentStorageKind::MacroArgument => {}
            }
            return Ok(ResidentWordRead::Parameter {
                slot,
                arguments,
                active_source,
            });
        }
        self.enter_resident_delivery();

        Ok(ResidentWordRead::Word {
            word,
            origin,
            identity,
            position: u64::from(position),
            active_source,
            suppress_expandable,
            #[cfg(test)]
            storage_kind,
            #[cfg(feature = "profiling")]
            raw_kind,
        })
    }

    #[inline(always)]
    fn admit_resident_word(
        &mut self,
        selected: ResidentWordRead<G>,
        destination: &mut Option<HotCommand<G>>,
    ) -> Result<Option<Catcode>, CommandError> {
        let ResidentWordRead::Word {
            word,
            origin,
            identity,
            position,
            active_source,
            suppress_expandable,
            #[cfg(test)]
            storage_kind,
            #[cfg(feature = "profiling")]
            raw_kind,
        } = selected
        else {
            return Err(CommandError::input_invariant());
        };
        #[cfg(test)]
        match storage_kind {
            ResidentStorageKind::Stored => {
                self.command.stored_token_advance_counters.command_writes = self
                    .command
                    .stored_token_advance_counters
                    .command_writes
                    .saturating_add(1);
                self.command.raw_delivery_path_counters.stored_direct = self
                    .command
                    .raw_delivery_path_counters
                    .stored_direct
                    .saturating_add(1);
            }
            ResidentStorageKind::MacroBody => {
                self.command.macro_kernel_counters.body_command_writes = self
                    .command
                    .macro_kernel_counters
                    .body_command_writes
                    .saturating_add(1);
            }
            ResidentStorageKind::MacroArgument => {
                self.command.macro_kernel_counters.argument_command_writes = self
                    .command
                    .macro_kernel_counters
                    .argument_command_writes
                    .saturating_add(1);
                self.command
                    .raw_delivery_path_counters
                    .macro_argument_direct = self
                    .command
                    .raw_delivery_path_counters
                    .macro_argument_direct
                    .saturating_add(1);
            }
        }
        let resolution = if let Some(command) = destination.as_mut() {
            command.write_resolved_delivery(
                word,
                origin,
                identity,
                position,
                active_source,
                false,
                None,
                suppress_expandable,
                self.state,
            )
        } else {
            let (command, resolution) = HotCommand::from_resolved_delivery(
                word,
                origin,
                identity,
                position,
                active_source,
                false,
                None,
                suppress_expandable,
                self.state,
            );
            destination.replace(command);
            resolution
        };
        #[cfg(test)]
        if matches!(storage_kind, ResidentStorageKind::Stored) {
            self.command.stored_token_advance_counters.meaning_lookups = self
                .command
                .stored_token_advance_counters
                .meaning_lookups
                .saturating_add(u64::from(resolution.meaning_lookup()));
        }
        #[cfg(feature = "profiling")]
        self.fuel.record_raw_delivery(
            self.command.delivery_mode.scanner_active(),
            resolution.meaning_lookup(),
            raw_kind,
        );
        Ok(resolution.literal_catcode())
    }

    #[inline(always)]
    fn settle_hot_delivery(
        &mut self,
        command: &mut HotCommand<G>,
        literal_catcode: Option<Catcode>,
    ) -> Result<(), CommandError> {
        self.command.delivery_mode.begin_token(
            command.suppresses_expandable_control_sequence(),
            command.is_outer(),
        );
        self.command.roots.alignment.account_literal_brace(
            &mut self.command.timeline,
            command,
            literal_catcode,
        );
        self.advance_delivery_sequence();
        if command.is_direct_source_delivery() {
            self.readmit_delivery_stamp(command.delivery_stamp());
        }
        if self.command.delivery_mode.requires_slow_settlement() {
            self.settle_exceptional_delivery(command)?;
        }
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn transition_resident_word(
        &mut self,
        selected: ResidentWordRead<G>,
        destination: &mut Option<HotCommand<G>>,
        expanded_eof: bool,
    ) -> Result<ResidentColdOutcome, CommandError> {
        let transition = match selected {
            ResidentWordRead::NoResident => InputFrameTransition::Boundary(ResidentBoundary::Empty),
            ResidentWordRead::Source { resident_index } => {
                InputFrameTransition::Source { resident_index }
            }
            ResidentWordRead::Parameter {
                slot,
                arguments,
                active_source,
            } => {
                self.transition_input_frame(
                    InputFrameTransition::Parameter {
                        slot,
                        arguments,
                        active_source,
                    },
                    destination,
                )?;
                return Ok(ResidentColdOutcome::Retry);
            }
            ResidentWordRead::Exhausted {
                resident_index,
                identity,
            } => InputFrameTransition::ResidentExhausted {
                resident_index,
                identity,
            },
            ResidentWordRead::Word { .. } => {
                return Err(CommandError::input_invariant());
            }
        };
        let outcome = self.transition_input_frame(transition, destination)?;
        if expanded_eof && matches!(outcome, ResidentColdOutcome::Finished(DeliveryStatus::End)) {
            if self.finish_number_continuation_at_end()? {
                return Ok(ResidentColdOutcome::Retry);
            }
            if self.finish_pdf_ximage_bbox_continuation_at_end()? {
                return Ok(ResidentColdOutcome::Retry);
            }
            if self.command.scratch.active_control_is_synchronous() {
                self.command
                    .scratch
                    .abort_synchronous_controls()
                    .map_err(crate::scan_toks::scratch_command_error)?;
            }
        }
        Ok(outcome)
    }

    /// The concrete TeX82 §341 raw-token loop. It owns one fuel charge for
    /// each semantic raw token and retries only through cold input transitions.
    #[inline(always)]
    pub(super) fn raw_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut hot_destination = None;
        let result = self.raw_next_hot(&mut hot_destination);
        self.finish_hot_delivery(destination, &mut hot_destination, result)
    }

    /// Raw delivery for TeX82 §394's macro matcher.  This is the same
    /// canonical resident/source reader, settlement, outer-validity, replay,
    /// freshness, and fuel path as [`Self::raw_next`], but it returns the
    /// compact settled token instead of crossing the rich command boundary.
    ///
    /// Alignment delimiter interception is completed in the compact slot as
    /// well; a matcher therefore never needs to materialize a
    /// `CurrentCommand` merely to hand a token to its argument cursor.
    pub(super) fn raw_next_matcher(
        &mut self,
        paragraph_token: Option<TokenWord>,
    ) -> Result<Option<MacroMatchDelivery<G>>, CommandError> {
        let mut hot = None;
        loop {
            match self.raw_next_hot(&mut hot)? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::Command => {
                    let command = hot.take().ok_or_else(CommandError::input_invariant)?;
                    // The matcher needs the category of the delivered
                    // spelling, not the category implied by its effective
                    // command.  A control sequence may resolve to a
                    // character command while remaining a control-sequence
                    // token in TeX's parameter grammar.
                    let literal_catcode = command.spelling_word().literal_catcode();
                    let delivery =
                        MacroMatchDelivery::from_hot(command, literal_catcode, paragraph_token);
                    if matches!(
                        delivery.alignment_adjustment(),
                        crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                    ) {
                        self.begin_scalar_alignment_v_template_hot(&delivery)?;
                        continue;
                    }
                    #[cfg(test)]
                    {
                        // Preserve the collector-path metric's meaning: one
                        // compact matcher classification per raw token. This
                        // is a packed fact projection, not a
                        // `ClassifiedToken` materialization.
                        self.command
                            .token_collector_path_counters
                            .raw_classifications = self
                            .command
                            .token_collector_path_counters
                            .raw_classifications
                            .saturating_add(1);
                    }
                    if matches!(
                        delivery.alignment_adjustment(),
                        crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                    ) {
                        self.begin_scalar_alignment_v_template_hot(&delivery)?;
                        continue;
                    }
                    debug_assert!(
                        !delivery.is_outer(),
                        "an outer command must be recovered before macro matching"
                    );
                    return Ok(Some(delivery));
                }
                DeliveryStatus::CharacterRun
                | DeliveryStatus::CharacterRunBoundary
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentEndTemplate
                | DeliveryStatus::AlignmentClosingBrace => {
                    return Err(CommandError::input_invariant());
                }
            }
        }
    }

    #[inline(always)]
    fn raw_next_hot(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let depth = self.command.transient.active_expansion_depth;
        let mut command = None;
        if let Err(failure) = self.charge_command_action() {
            return self.fail_hot_expanded_delivery(destination, depth, failure);
        }
        let literal_catcode = 'fetch: loop {
            let selected = match self.next_resident_word() {
                Ok(selected) => selected,
                Err(failure) => {
                    return self.fail_hot_expanded_delivery(destination, depth, failure);
                }
            };
            if matches!(selected, ResidentWordRead::Word { .. }) {
                break 'fetch self.admit_resident_word(selected, &mut command)?;
            }
            let cold = match self.transition_resident_word(selected, &mut command, false) {
                Ok(cold) => cold,
                Err(failure) => {
                    return self.fail_hot_expanded_delivery(destination, depth, failure);
                }
            };
            match cold {
                ResidentColdOutcome::Retry => continue 'fetch,
                ResidentColdOutcome::Finished(status) => {
                    destination.take();
                    return Ok(status);
                }
                ResidentColdOutcome::Synthetic { literal_catcode } => {
                    break 'fetch literal_catcode;
                }
            }
        };
        let mut command = command
            .take()
            .expect("resident admission initializes the hot command");
        if let Err(failure) = self.settle_hot_delivery(&mut command, literal_catcode) {
            return self.fail_hot_expanded_delivery(destination, depth, failure);
        }
        *destination = Some(command);
        Ok(DeliveryStatus::Command)
    }

    /// Delivers one expanded command through the compact loop and materializes
    /// only at the caller's rich-command boundary. The scanner-owned callers
    /// use the hot entry directly, so a terminal delimiter operand never
    /// crosses this boundary merely to be classified.
    pub(super) fn expanded_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next_with_action(destination, None)
    }

    fn expanded_next_with_action(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut initial_action = initial_action;
        self.expanded_next_with_boundary(destination, initial_action.take(), None)
    }

    fn expanded_next_with_boundary(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
        return_boundary: Option<(
            crate::expansion_work::ExpansionControlSlot<G>,
            crate::expansion_work::control::ExpansionReturnSink,
        )>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut initial_action = initial_action;
        loop {
            if destination.is_none() && self.canonical_expression_resume_pending()? {
                self.run_pending_canonical_expression()?;
                continue;
            }
            let mut hot_destination = destination.take().map(HotCommand::from_current);
            let result = self.expanded_next_hot_with_boundary(
                &mut hot_destination,
                initial_action.take(),
                return_boundary,
            );
            if matches!(
                result,
                Ok(DeliveryStatus::PendingExpanded)
                    if self.command.scratch.has_pending_expression_frame()
            ) {
                hot_destination.take();
                self.run_pending_canonical_expression()?;
                continue;
            }
            return self.finish_hot_delivery(destination, &mut hot_destination, result);
        }
    }

    fn finish_hot_delivery(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        hot_destination: &mut Option<HotCommand<G>>,
        result: Result<DeliveryStatus, CommandError>,
    ) -> Result<DeliveryStatus, CommandError> {
        match result {
            Ok(status) => {
                if matches!(
                    status,
                    DeliveryStatus::End
                        | DeliveryStatus::ReplayCompleted(_)
                        | DeliveryStatus::CharacterRun
                ) {
                    hot_destination.take();
                } else {
                    *destination = hot_destination.take().map(|command| command.materialize());
                }
                Ok(status)
            }
            Err(error) => {
                hot_destination.take();
                destination.take();
                Err(error)
            }
        }
    }

    /// The concrete TeX82 §380 `get_x_token` loop. Expansion remains in the
    /// continuously occupied hot command; only scanner/diagnostic/resource
    /// boundaries materialize or park it.
    fn expanded_next_hot(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next_hot_with_boundary(destination, initial_action, None)
    }

    fn expanded_next_hot_with_boundary(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
        return_boundary: Option<(
            crate::expansion_work::ExpansionControlSlot<G>,
            crate::expansion_work::control::ExpansionReturnSink,
        )>,
    ) -> Result<DeliveryStatus, CommandError> {
        let depth = self.command.transient.active_expansion_depth;
        self.command.scratch.note_delivery_entry(depth);
        let Some(active_depth) = depth.checked_add(1) else {
            return self.fail_hot_expanded_delivery(
                destination,
                depth,
                CommandError::input_invariant(),
            );
        };
        self.command.transient.active_expansion_depth = active_depth;
        let resuming = self.expansion_resume.is_some()
            || self
                .scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion);
        let (mut command, mut delivery_expanded, mut carried_parent, resumed_return_capability) =
            if resuming {
                match self.resume_expanded_delivery(destination.take()) {
                    Ok(resumed) => (
                        Some(resumed.command),
                        resumed.delivery_expanded,
                        resumed.parent.map(ParentAdmission::Awaiting),
                        resumed.return_capability,
                    ),
                    Err(failure) => {
                        return self.fail_hot_expanded_delivery(destination, depth, failure);
                    }
                }
            } else if let Some(command) = destination.take() {
                (Some(command), false, None, None)
            } else {
                (None, false, None, None)
            };
        self.resumed_return_capability = resumed_return_capability;
        let mut return_boundary = return_boundary;
        if let Some(expected_sink) = self.resumed_return_sink.take() {
            let Some(capability) = self.resumed_return_capability.take() else {
                return self.fail_hot_expanded_delivery(
                    destination,
                    depth,
                    CommandError::input_invariant(),
                );
            };
            if capability.sink() != expected_sink {
                return self.fail_hot_expanded_delivery(
                    destination,
                    depth,
                    CommandError::input_invariant(),
                );
            }
            let slot = capability.slot();
            if self.scanner_return_capability.replace(capability).is_some() {
                return self.fail_hot_expanded_delivery(
                    destination,
                    depth,
                    CommandError::input_invariant(),
                );
            }
            return_boundary = Some((slot, expected_sink));
        }
        let mut initial_action = initial_action;
        let mut suppress_first_expansion_trace = delivery_expanded;
        let status = 'delivery: loop {
            if command.is_none() {
                if return_boundary.is_some_and(|(slot, destination)| {
                    destination
                        == crate::expansion_work::control::ExpansionReturnSink::ScannerExpansion
                        && self.command.scratch.active_control_slot() == Some(slot)
                }) {
                    break 'delivery DeliveryStatus::PendingExpanded;
                }
                debug_assert!(
                    destination.is_none(),
                    "the caller-owned hot command must be empty before a resident fetch"
                );
                if let Err(failure) = self.charge_command_action() {
                    return self.fail_hot_expanded_delivery(destination, depth, failure);
                }
                let literal_catcode = 'fetch: loop {
                    let selected = match self.next_resident_word() {
                        Ok(selected) => selected,
                        Err(failure) => {
                            return self.fail_hot_expanded_delivery(destination, depth, failure);
                        }
                    };
                    if matches!(selected, ResidentWordRead::Word { .. }) {
                        break 'fetch self.admit_resident_word(selected, &mut command)?;
                    }
                    let cold = match self.transition_resident_word(selected, &mut command, true) {
                        Ok(cold) => cold,
                        Err(failure) => {
                            return self.fail_hot_expanded_delivery(destination, depth, failure);
                        }
                    };
                    match cold {
                        ResidentColdOutcome::Retry => continue 'fetch,
                        ResidentColdOutcome::Finished(status) => break 'delivery status,
                        ResidentColdOutcome::Synthetic { literal_catcode } => {
                            break 'fetch literal_catcode;
                        }
                    }
                };
                let command_ref = command
                    .as_mut()
                    .expect("resident delivery initializes the hot command");
                if let Err(failure) = self.settle_hot_delivery(command_ref, literal_catcode) {
                    return self.fail_hot_expanded_delivery(destination, depth, failure);
                }
            }

            let hot_command = command
                .as_mut()
                .expect("resident delivery initializes the hot command");
            let action = initial_action
                .take()
                .unwrap_or_else(|| classify_hot_command(hot_command));
            let active_control = self.command.scratch.active_control_tag();
            if active_control.is_none() {
                match action {
                    ExpandedCommandAction::Return => {
                        break self.finish_expanded_command(hot_command, delivery_expanded);
                    }
                    ExpandedCommandAction::EndTemplate => {
                        if matches!(
                            hot_command.alignment_adjustment(),
                            crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                        ) {
                            break DeliveryStatus::AlignmentEndTemplate;
                        }
                        hot_command.convert_end_template_to_endv(self.state.frozen_endv_token());
                        break self.finish_expanded_command(hot_command, delivery_expanded);
                    }
                    ExpandedCommandAction::Expand(ExpansionDispatch::Macro) => {
                        // Parameterless and ordinary macro chains are the
                        // common expandable path. Keep their compact owner
                        // in this loop; no continuation dispatcher or rich
                        // command bridge is needed while the macro body is installed.
                        delivery_expanded = true;
                        let _ = std::mem::take(&mut suppress_first_expansion_trace);
                        if let Err(failure) =
                            self.expand_classified_occupied(hot_command, ExpansionDispatch::Macro)
                        {
                            match failure {
                                CommandError::ParagraphInMacroArgument
                                | CommandError::OuterInMacroArgument => {}
                                failure => {
                                    return self.fail_hot_expanded_delivery(
                                        destination,
                                        depth,
                                        failure,
                                    );
                                }
                            }
                        }
                        debug_assert!(command.is_some(), "macro expansion consumes its hot owner");
                        command.take();
                        continue;
                    }
                    ExpandedCommandAction::Expand(ExpansionDispatch::Undefined) => {
                        match self.expand_undefined_hot(
                            hot_command,
                            None,
                            &mut delivery_expanded,
                            &mut suppress_first_expansion_trace,
                        )? {
                            ExpandedHotDispatch::Continue => {
                                command.take();
                                continue;
                            }
                            ExpandedHotDispatch::Finished(status) => break 'delivery status,
                        }
                    }
                    ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(primitive)) => {
                        let mut command_parked = false;
                        match self.expand_primitive_hot(
                            hot_command,
                            primitive,
                            None,
                            &mut delivery_expanded,
                            &mut suppress_first_expansion_trace,
                            &mut carried_parent,
                            destination,
                            depth,
                            &mut command_parked,
                        )? {
                            ExpandedHotDispatch::Continue => {
                                command.take();
                                continue;
                            }
                            ExpandedHotDispatch::Finished(status) => break 'delivery status,
                        }
                    }
                }
            }
            #[cfg(debug_assertions)]
            let dispatch_progress_before = (
                self.command.top_input_level_identity(),
                active_control,
                self.command.scratch.expansion_control_progress(),
            );
            let mut command_parked = false;
            match self.dispatch_expanded_action(
                hot_command,
                action,
                active_control,
                &mut delivery_expanded,
                &mut suppress_first_expansion_trace,
                &mut carried_parent,
                destination,
                depth,
                &mut command_parked,
            )? {
                ExpandedHotDispatch::Continue => {
                    #[cfg(debug_assertions)]
                    {
                        let (input_before, control_before, epoch_before) = dispatch_progress_before;
                        let progress = matches!(action, ExpandedCommandAction::Expand(_))
                            || input_before != self.command.top_input_level_identity()
                            || control_before != self.command.scratch.active_control_tag()
                            || epoch_before != self.command.scratch.expansion_control_progress()
                            || active_control.is_some()
                                && matches!(
                                    action,
                                    ExpandedCommandAction::Return
                                        | ExpandedCommandAction::EndTemplate
                                );
                        debug_assert!(
                            progress,
                            "expanded dispatcher returned Continue without consuming input, settling a control, emitting, or parking"
                        );
                    }
                    debug_assert!(
                        command.is_some(),
                        "continuation dispatch consumes its caller-owned hot command"
                    );
                    command.take();
                }
                ExpandedHotDispatch::Finished(status) => {
                    let child_still_owns_delivery = return_boundary.is_some_and(|(slot, sink)| {
                        sink
                            == crate::expansion_work::control::ExpansionReturnSink::ScannerExpansion
                            && self.command.scratch.active_control_slot() != Some(slot)
                    });
                    if child_still_owns_delivery {
                        command.take();
                        continue;
                    }
                    break status;
                }
            }
        };
        debug_assert_eq!(
            self.command.transient.active_expansion_depth, active_depth,
            "expanded delivery balances its depth"
        );
        self.command.transient.active_expansion_depth = depth;
        if matches!(
            status,
            DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_) | DeliveryStatus::CharacterRun
        ) {
            destination.take();
        } else {
            *destination = command.take();
        }
        Ok(status)
    }

    #[cold]
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn dispatch_expanded_action(
        &mut self,
        command: &mut HotCommand<G>,
        action: ExpandedCommandAction,
        active_control: Option<crate::expansion_work::ActiveControlTag>,
        delivery_expanded: &mut bool,
        suppress_first_expansion_trace: &mut bool,
        carried_parent: &mut Option<ParentAdmission<G>>,
        destination: &mut Option<HotCommand<G>>,
        depth: u32,
        command_parked: &mut bool,
    ) -> Result<ExpandedHotDispatch, CommandError> {
        record_active_dispatch_call();
        // e-TeX `\expanded` is a balanced expanded-token collector.  Its
        // body stays in the same hot delivery loop: expandable commands
        // fall through to the ordinary dispatch below, while settled
        // words are appended to the attempt-owned buffer here.  This is
        // deliberately before the other operand controls so a nested
        // `\the`/conditional can use the same LIFO lane.
        let active = match active_control {
            None => None,
            Some(ActiveControlTag::Return) => self
                .command
                .scratch
                .active_control_slot()
                .map(ActiveControlSnapshot::Return),
            Some(ActiveControlTag::Expanded) => self
                .command
                .scratch
                .top_expanded_control()
                .map_err(crate::scan_toks::scratch_command_error)?
                .map(ActiveControlSnapshot::Expanded),
            Some(ActiveControlTag::ExpandAfterSync) => self
                .command
                .scratch
                .top_expandafter_control()
                .map_err(crate::scan_toks::scratch_command_error)?
                .map(ActiveControlSnapshot::ExpandAfterSync),
            Some(ActiveControlTag::IfCompare) => self
                .command
                .scratch
                .top_if_compare_control()
                .map_err(crate::scan_toks::scratch_command_error)?
                .map(ActiveControlSnapshot::IfCompare),
            Some(ActiveControlTag::IfNumber) => self
                .command
                .scratch
                .top_if_number_control()
                .map_err(crate::scan_toks::scratch_command_error)?
                .map(ActiveControlSnapshot::IfNumber),
            Some(ActiveControlTag::IfDimension) => self
                .command
                .scratch
                .top_if_dimension_control()
                .map_err(crate::scan_toks::scratch_command_error)?
                .map(ActiveControlSnapshot::IfDimension),
            Some(ActiveControlTag::Number) => self
                .command
                .scratch
                .top_number_control()
                .map_err(crate::scan_toks::scratch_command_error)?
                .map(ActiveControlSnapshot::Number),
            Some(ActiveControlTag::PdfXImageBBox) => self
                .command
                .scratch
                .top_pdf_ximage_bbox_control()
                .map_err(crate::scan_toks::scratch_command_error)?
                .map(|_| ActiveControlSnapshot::PdfXImageBBox),
            Some(ActiveControlTag::FontName) => self
                .command
                .scratch
                .top_fontname_control()
                .map_err(crate::scan_toks::scratch_command_error)?
                .map(|_| ActiveControlSnapshot::FontName),
            Some(ActiveControlTag::CsName) => self
                .command
                .scratch
                .top_csname_control()
                .map_err(crate::scan_toks::scratch_command_error)?
                .map(|_| ActiveControlSnapshot::CsName),
            Some(ActiveControlTag::IfCsName) => self
                .command
                .scratch
                .top_ifcsname_control()
                .map_err(crate::scan_toks::scratch_command_error)?
                .map(|_| ActiveControlSnapshot::IfCsName),
            Some(ActiveControlTag::The) => self
                .command
                .scratch
                .top_the_control()
                .map_err(crate::scan_toks::scratch_command_error)?
                .map(ActiveControlSnapshot::The),
            Some(_) => None,
        };
        if matches!(active, Some(ActiveControlSnapshot::Return(_)))
            && matches!(
                action,
                ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate
            )
        {
            // The scanner-owned return capability is deliberately left in
            // the lane for its caller to consume.  Returning the settled
            // command here keeps the scanner boundary in one delivery step;
            // no ambient top-control retry is needed.
            return Ok(ExpandedHotDispatch::Finished(
                self.finish_expanded_command(command, *delivery_expanded),
            ));
        }
        if let Some(ActiveControlSnapshot::Expanded(control)) = active {
            match control.phase {
                crate::expansion_work::control::SynchronousExpandedPhase::NeedOpening => {
                    let is_space =
                        command.character_catcode() == Some(tex_state::token::Catcode::Space);
                    let is_relax = matches!(
                        command.resolved_meaning(),
                        ResolvedMeaning::Static(Meaning::Relax)
                    );
                    if is_space || is_relax {
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    if command.character_catcode() == Some(tex_state::token::Catcode::BeginGroup) {
                        self.command
                            .scratch
                            .begin_expanded_body()
                            .map_err(crate::scan_toks::scratch_command_error)?;
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    // §403's recovery backs the rejected command up,
                    // installs the synthetic opening brace in alignment
                    // state, and then continues this same collector.
                    self.recover_expanded_opening(*command)?;
                    return Ok(ExpandedHotDispatch::Continue);
                }
                crate::expansion_work::control::SynchronousExpandedPhase::Collecting => {
                    if matches!(
                        control.kind,
                        crate::expansion_work::control::SynchronousExpandedKind::Unexpanded
                            | crate::expansion_work::control::SynchronousExpandedKind::Detokenize
                    ) {
                        let _ = self.append_expanded_word(command)?;
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    if matches!(
                        action,
                        ExpandedCommandAction::Expand(ExpansionDispatch::Macro)
                    ) && command
                        .command_word()
                        .flags()
                        .contains(tex_state::meaning::MeaningFlags::PROTECTED)
                    {
                        // e-TeX's expanded collector suppresses protected
                        // macros for this delivery while retaining their
                        // original spelling in the resulting token list.
                        command.suppress_expandable();
                        let _ = self.append_expanded_word(command)?;
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    if matches!(
                        action,
                        ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate
                    ) {
                        let _ = self.append_expanded_word(command)?;
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                }
            }
        }
        // Within an expanded collector, `\unexpanded` consumes a raw
        // balanced child and splices its words into the parent's writer.
        // Keeping that child in the same control lane avoids the legacy
        // collector's recursive scan and preserves expandable spellings.
        if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
            ExpandablePrimitive::Unexpanded,
        )) = action
            && let Some(ActiveControlSnapshot::Expanded(control)) = active
            && control.kind == crate::expansion_work::control::SynchronousExpandedKind::Expanded
        {
            self.run_nested_expansion_with_parent(active, carried_parent, |this, parent| {
                this.begin_unexpanded_continuation_with_parent(
                    command.origin(),
                    control.writer,
                    parent,
                )
            })?;
            return Ok(ExpandedHotDispatch::Continue);
        }

        // `\detokenize` consumes its balanced child without expansion,
        // but writes the canonical token spelling as character tokens
        // directly into the enclosing expanded collector.
        if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
            ExpandablePrimitive::Detokenize,
        )) = action
            && let Some(ActiveControlSnapshot::Expanded(control)) = active
            && control.kind == crate::expansion_work::control::SynchronousExpandedKind::Expanded
        {
            self.run_nested_expansion_with_parent(active, carried_parent, |this, parent| {
                this.begin_detokenize_continuation_with_parent(
                    command.origin(),
                    control.writer,
                    parent,
                )
            })?;
            return Ok(ExpandedHotDispatch::Continue);
        }

        // `\expandafter` owns two raw operands but only the second one is
        // expanded. Its compact control intercepts the first command and
        // then lets every nested expansion continue through this same
        // delivery loop. Once that second stream settles on a returned
        // command, backup/replay is performed at the semantic boundary.
        if let Some(ActiveControlSnapshot::ExpandAfterSync(control)) = active {
            match control.phase {
                crate::expansion_work::control::SynchronousExpandAfterPhase::NeedFirst => {
                    self.command
                        .scratch
                        .save_expandafter_first(*command)
                        .map_err(crate::scan_toks::scratch_command_error)?;
                    return Ok(ExpandedHotDispatch::Continue);
                }
                crate::expansion_work::control::SynchronousExpandAfterPhase::NeedSecond => {
                    if matches!(action, ExpandedCommandAction::Return) {
                        self.complete_expandafter_continuation(*command)?;
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                }
                crate::expansion_work::control::SynchronousExpandAfterPhase::AwaitNested => {}
            }
        }

        // `\if` and `\ifcat` each request two expanded operands. Keep
        // only their compact scalar projection in the control lane; an
        // operand that is itself expandable is allowed to run normally
        // and returns here when its result settles.
        if let Some(ActiveControlSnapshot::IfCompare(control)) = active {
            match control.phase {
                crate::expansion_work::control::SynchronousIfComparePhase::NeedFirst => {
                    if matches!(action, ExpandedCommandAction::Return) {
                        self.command
                            .scratch
                            .save_if_compare_first(
                                command.conditional_character_code(),
                                (control.kind == crate::conditionals::ConditionalKind::IfCat)
                                    .then(|| command.conditional_category_code())
                                    .flatten(),
                            )
                            .map_err(crate::scan_toks::scratch_command_error)?;
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                }
                crate::expansion_work::control::SynchronousIfComparePhase::NeedSecond {
                    ..
                } => {
                    if matches!(action, ExpandedCommandAction::Return) {
                        self.complete_if_compare_continuation(*command)?;
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                }
                crate::expansion_work::control::SynchronousIfComparePhase::AwaitFirst
                | crate::expansion_work::control::SynchronousIfComparePhase::AwaitSecond {
                    ..
                } => {}
            }
        }

        // Numeric and dimension conditionals consume their common
        // literal form directly from the hot command.  Expandable
        // operands remain ordinary delivery actions and return to this
        // compact phase instead of retaining a scalar scanner frame on
        // the Rust stack.
        if let Some(ActiveControlSnapshot::IfNumber(control)) = active {
            let nested_delimiter = matches!(
                action,
                ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                    ExpandablePrimitive::Else | ExpandablePrimitive::Or | ExpandablePrimitive::Fi,
                ))
            ) && self
                .command
                .conditions
                .current()
                .is_some_and(|frame| frame.identity != control.condition);
            if matches!(
                action,
                ExpandedCommandAction::Return
                    | ExpandedCommandAction::EndTemplate
                    | ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                        ExpandablePrimitive::Else
                            | ExpandablePrimitive::Or
                            | ExpandablePrimitive::Fi,
                    ))
            ) && !nested_delimiter
                && !matches!(
                    control.phase,
                    crate::expansion_work::control::SynchronousIfNumberPhase::AwaitLeft { .. }
                        | crate::expansion_work::control::SynchronousIfNumberPhase::AwaitRelation { .. }
                        | crate::expansion_work::control::SynchronousIfNumberPhase::AwaitRight { .. }
                )
            {
                match self.advance_if_number_continuation(*command)? {
                    crate::conditionals::IfNumberAdvance::Continue
                    | crate::conditionals::IfNumberAdvance::Complete => {
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                }
            }
        }

        if let Some(ActiveControlSnapshot::IfDimension(control)) = active {
            let nested_delimiter = matches!(
                action,
                ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                    ExpandablePrimitive::Else | ExpandablePrimitive::Or | ExpandablePrimitive::Fi,
                ))
            ) && self
                .command
                .conditions
                .current()
                .is_some_and(|frame| frame.identity != control.condition);
            if matches!(
                    action,
                    ExpandedCommandAction::Return
                        | ExpandedCommandAction::EndTemplate
                        | ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                            ExpandablePrimitive::Else
                                | ExpandablePrimitive::Or
                                | ExpandablePrimitive::Fi,
                        ))
                ) && !nested_delimiter && !matches!(
                    control.phase,
                    crate::expansion_work::control::SynchronousIfDimensionPhase::AwaitLeft {
                        ..
                    }
                        | crate::expansion_work::control::SynchronousIfDimensionPhase::AwaitRelation {
                            ..
                        }
                        | crate::expansion_work::control::SynchronousIfDimensionPhase::AwaitRight {
                            ..
                        }
                ) {
                    match self.advance_if_dimension_continuation(*command)? {
                        crate::conditionals::IfDimensionAdvance::Continue
                        | crate::conditionals::IfDimensionAdvance::Complete => {
                            return Ok(ExpandedHotDispatch::Continue);
                        }
                    }
                }
        }

        if let Some(ActiveControlSnapshot::Number(control)) = active
            && matches!(
                action,
                ExpandedCommandAction::Return
                    | ExpandedCommandAction::EndTemplate
                    | ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                        ExpandablePrimitive::Else
                            | ExpandablePrimitive::Or
                            | ExpandablePrimitive::Fi,
                    ))
            )
            && !matches!(
                control.phase,
                crate::expansion_work::control::SynchronousNumberPhase::Await { .. }
                    | crate::expansion_work::control::SynchronousNumberPhase::RegisterIndexAwait { .. }
            )
        {
            let _complete = self.advance_number_continuation(*command)?;
            return Ok(ExpandedHotDispatch::Continue);
        }

        if matches!(active, Some(ActiveControlSnapshot::PdfXImageBBox))
            && matches!(
                action,
                ExpandedCommandAction::Return
                    | ExpandedCommandAction::EndTemplate
                    | ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                        ExpandablePrimitive::Else
                            | ExpandablePrimitive::Or
                            | ExpandablePrimitive::Fi,
                    ))
            )
        {
            let _ = self.advance_pdf_ximage_bbox_continuation(*command, false)?;
            return Ok(ExpandedHotDispatch::Continue);
        }

        // `\fontname` consumes one expanded font identifier.  Keep its
        // opener in the compact control lane so nested conversions are
        // reduced by this loop rather than by recursively re-entering a
        // font scanner.
        if matches!(active, Some(ActiveControlSnapshot::FontName)) {
            match action {
                ExpandedCommandAction::Expand(_) => {}
                ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate => {
                    self.complete_fontname_continuation(*command)?;
                    return Ok(ExpandedHotDispatch::Continue);
                }
            }
        }

        // A `\the` scalar child may cross an immutable resource barrier
        // (for example while resolving a font/register operand).  Its
        // control has already been removed before entering the scalar
        // scanner, so the resumed phase carries only the opener origin
        // and re-enters this same loop with the original target command.
        // This branch must run before ordinary classification: the
        // restored command is the target, not a new top-level expansion.
        let resumed_the = match self.resumed_expansion.take() {
            Some(crate::state::PendingExpansionResume::The { opener }) => Some(opener),
            Some(other) => {
                self.resumed_expansion = Some(other);
                None
            }
            None => None,
        };
        if let Some(opener) = resumed_the {
            let target = command.materialize();
            match self.complete_the_continuation(&target, opener) {
                Ok(()) => {
                    return Ok(ExpandedHotDispatch::Continue);
                }
                Err(error) if error.is_resource_suspension() => {
                    *command_parked = true;
                    return self
                        .park_the_continuation(
                            target,
                            opener,
                            *delivery_expanded,
                            error,
                            destination,
                            depth,
                        )
                        .map(ExpandedHotDispatch::Finished);
                }
                Err(error) => {
                    return self.fail_expanded_dispatch(destination, depth, error);
                }
            }
        }

        // `\csname` is another expanded-token consumer. Its spelling is
        // kept in the generation-owned name lane while this compact
        // control remains at the top of the same delivery stack. Nested
        // character-producing expansions therefore return here instead
        // of entering `scan_csname_characters` recursively.
        if matches!(active, Some(ActiveControlSnapshot::CsName)) {
            match action {
                ExpandedCommandAction::Expand(_) => {}
                ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate => {
                    if command.command_word().expandable_primitive()
                        == Some(ExpandablePrimitive::EndCsName)
                    {
                        self.complete_csname_continuation(None)?;
                    } else if let Some(character) = command.character_token() {
                        self.append_csname_character(character)?;
                        return Ok(ExpandedHotDispatch::Continue);
                    } else {
                        self.complete_csname_continuation(Some(command.materialize()))?;
                    }
                    return Ok(ExpandedHotDispatch::Continue);
                }
            }
        }

        // `\ifcsname` shares the expanded character stream with
        // `\csname`, but its terminator completes a conditional frame
        // instead of backing a control-sequence token. Keeping this
        // predicate in the same control lane removes the recursive
        // scanner edge while preserving the evaluating condition limit.
        if matches!(active, Some(ActiveControlSnapshot::IfCsName)) {
            match action {
                ExpandedCommandAction::Expand(_) => {}
                ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate => {
                    if command.command_word().expandable_primitive()
                        == Some(ExpandablePrimitive::EndCsName)
                    {
                        self.complete_ifcsname_continuation(None)?;
                    } else if let Some(character) = command.character_token() {
                        self.append_csname_character(character)?;
                        return Ok(ExpandedHotDispatch::Continue);
                    } else {
                        self.complete_ifcsname_continuation(Some(command.materialize()))?;
                    }
                    return Ok(ExpandedHotDispatch::Continue);
                }
            }
        }

        // A `\the` operand is itself an expanded-token request.  Keep
        // that request in the generation-owned control lane and consume
        // targets from this same hot loop.  In particular, a nested
        // `\the` pushes another copy-small control and never invokes a
        // second `expanded_next`/`get_x_token` call.  We remove the
        // completed control before entering a scalar scanner because a
        // register's own index probe is an independent scalar child.
        if let Some(ActiveControlSnapshot::The(the_control)) = active {
            match (the_control.phase, action) {
                (
                    crate::expansion_work::control::ThePhase::NeedTarget,
                    ExpandedCommandAction::Expand(_),
                ) => {}
                (
                    crate::expansion_work::control::ThePhase::Index { .. },
                    ExpandedCommandAction::Expand(_),
                ) => {}
                (
                    crate::expansion_work::control::ThePhase::Expression { .. },
                    ExpandedCommandAction::Expand(_),
                ) => {}
                (
                    crate::expansion_work::control::ThePhase::CanonicalExpression { .. },
                    ExpandedCommandAction::Expand(_),
                ) => {}
                (
                    crate::expansion_work::control::ThePhase::ExpressionRegisterIndex { .. },
                    ExpandedCommandAction::Expand(_),
                ) => {}
                (
                    crate::expansion_work::control::ThePhase::DimensionExpression { .. },
                    ExpandedCommandAction::Expand(_),
                ) => {}
                (
                    crate::expansion_work::control::ThePhase::Index { .. },
                    ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate,
                ) => {
                    if self.advance_the_index_continuation(*command)? {
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    return Ok(ExpandedHotDispatch::Continue);
                }
                (
                    crate::expansion_work::control::ThePhase::Expression { .. },
                    ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate,
                ) => {
                    if self.advance_the_expression_continuation(*command)? {
                        if self.command.scratch.has_pending_expression_frame() {
                            return Ok(ExpandedHotDispatch::Finished(
                                DeliveryStatus::PendingExpanded,
                            ));
                        }
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    return Ok(ExpandedHotDispatch::Continue);
                }
                (
                    crate::expansion_work::control::ThePhase::CanonicalExpression { .. },
                    ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate,
                ) => {
                    // The canonical scanner owns this token request.  It
                    // re-enters the shared delivery loop and receives this
                    // inert control only as the scanner's return boundary.
                    return Ok(ExpandedHotDispatch::Finished(
                        self.finish_expanded_command(command, *delivery_expanded),
                    ));
                }
                (
                    crate::expansion_work::control::ThePhase::ExpressionRegisterIndex { .. },
                    ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate,
                ) => {
                    if self.advance_the_expression_register_continuation(*command)? {
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    return Ok(ExpandedHotDispatch::Continue);
                }
                (
                    crate::expansion_work::control::ThePhase::DimensionExpression { .. },
                    ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate,
                ) => {
                    if self.advance_the_dimension_expression_continuation(*command)? {
                        if self.command.scratch.has_pending_expression_frame() {
                            return Ok(ExpandedHotDispatch::Finished(
                                DeliveryStatus::PendingExpanded,
                            ));
                        }
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    return Ok(ExpandedHotDispatch::Continue);
                }
                (
                    crate::expansion_work::control::ThePhase::NeedTarget,
                    ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate,
                ) => {
                    let meaning = match command.resolved_meaning() {
                        ResolvedMeaning::Static(meaning) => meaning,
                        ResolvedMeaning::Macro { .. } => Meaning::Undefined,
                    };
                    if Self::compact_the_expression_target(meaning) {
                        self.command.scratch.set_the_phase(
                            crate::expansion_work::control::ThePhase::Expression {
                                target: meaning,
                                expression: 0,
                                expression_sign: 1,
                                expression_started: false,
                                term: 0,
                                term_operator: 0,
                                term_active: false,
                                negative: false,
                                value: 0,
                                seen_digit: false,
                                factor_ready: false,
                                factor_spaced: false,
                            },
                        )?;
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    if Self::compact_the_dimension_expression_target(meaning) {
                        self.command.scratch.set_the_phase(
                            crate::expansion_work::control::ThePhase::DimensionExpression {
                                target: meaning,
                                as_number: false,
                                expression: 0,
                                expression_sign: 1,
                                expression_started: false,
                                term: 0,
                                term_active: false,
                                term_operator: 0,
                                negative: false,
                                value: 0,
                                fraction: 0,
                                fraction_digits: 0,
                                decimal: false,
                                unit: 0,
                                seen_digit: false,
                            },
                        )?;
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    if Self::compact_the_register_target(meaning) {
                        self.command.scratch.set_the_phase(
                            crate::expansion_work::control::ThePhase::Index {
                                target: meaning,
                                negative: false,
                                value: 0,
                                seen_digit: false,
                            },
                        )?;
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    if let Some(value) = self.scan_the_direct_value(meaning)? {
                        let opener = self
                            .command
                            .scratch
                            .pop_the_control()
                            .map_err(crate::scan_toks::scratch_command_error)?;
                        self.expand_the_value(opener, value)?;
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    let _ = self
                        .command
                        .scratch
                        .pop_the_control()
                        .map_err(crate::scan_toks::scratch_command_error)?;
                    let target = command.materialize();
                    match self.complete_the_continuation(&target, the_control.opener) {
                        Ok(()) => {
                            return Ok(ExpandedHotDispatch::Continue);
                        }
                        Err(error) if error.is_resource_suspension() => {
                            *command_parked = true;
                            return self
                                .park_the_continuation(
                                    target,
                                    the_control.opener,
                                    *delivery_expanded,
                                    error,
                                    destination,
                                    depth,
                                )
                                .map(ExpandedHotDispatch::Finished);
                        }
                        Err(error) => {
                            return self.fail_expanded_dispatch(destination, depth, error);
                        }
                    }
                }
            }
        }

        // Once active controls have consumed their own operand phases, every
        // remaining primitive takes the single compact primitive ABI.  This
        // is also the only primitive entry used by the no-active hot loop;
        // the cold continuation dispatcher is reserved for non-primitive
        // control flow and explicit rich boundaries.
        if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(primitive)) = action {
            return self.expand_primitive_hot(
                command,
                primitive,
                active,
                delivery_expanded,
                suppress_first_expansion_trace,
                carried_parent,
                destination,
                depth,
                command_parked,
            );
        }
        if matches!(
            action,
            ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
        ) {
            return self.expand_undefined_hot(
                command,
                active,
                delivery_expanded,
                suppress_first_expansion_trace,
            );
        }
        match action {
            ExpandedCommandAction::Return => Ok(ExpandedHotDispatch::Finished(
                self.finish_expanded_command(command, *delivery_expanded),
            )),
            ExpandedCommandAction::EndTemplate => {
                if matches!(
                    command.alignment_adjustment(),
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                ) {
                    return Ok(ExpandedHotDispatch::Finished(
                        DeliveryStatus::AlignmentEndTemplate,
                    ));
                }
                command.convert_end_template_to_endv(self.state.frozen_endv_token());
                Ok(ExpandedHotDispatch::Finished(
                    self.finish_expanded_command(command, *delivery_expanded),
                ))
            }
            ExpandedCommandAction::Expand(dispatch) => {
                *delivery_expanded = true;
                let _ = std::mem::take(suppress_first_expansion_trace);
                let macro_input_before = (dispatch == ExpansionDispatch::Macro)
                    .then(|| self.command.top_input_level_identity());
                let expandafter_pending = matches!(
                    active,
                    Some(ActiveControlSnapshot::ExpandAfterSync(control))
                        if control.phase
                            == crate::expansion_work::control::SynchronousExpandAfterPhase::NeedSecond
                );
                // Capture the one exact parent before dispatch mutates the
                // control lane. A macro or undefined command does not need
                // an edge: it only changes input, and the next settled token
                // is still owned by the same top control.
                let admission = if let Some(parent) = carried_parent.take() {
                    Some(parent)
                } else if active_control.is_some()
                    && !matches!(
                        dispatch,
                        ExpansionDispatch::Macro | ExpansionDispatch::Undefined
                    )
                {
                    active
                        .and_then(|control| control.awaitable_slot())
                        .map(ParentAdmission::Captured)
                } else {
                    None
                };
                if let Some(admission) = admission
                    && admission.needs_await()
                {
                    // A parent restored from a suspension is already in an
                    // Await phase. Fresh dispatches transition it once here;
                    // the exact slot makes this independent of any child
                    // that may become the new top.
                    self.command
                        .scratch
                        .await_expansion_control_for_child(admission.slot())
                        .map_err(crate::scan_toks::scratch_command_error)?;
                }
                let parent = admission.map(ParentAdmission::slot);
                let failure = match self.expand_classified_occupied(command, dispatch) {
                    Ok(()) => {
                        if let Some(parent) = parent
                            && !starts_synchronous_control(dispatch)
                        {
                            self.command
                                .scratch
                                .resume_expansion_control_parent(parent)
                                .map_err(crate::scan_toks::scratch_command_error)?;
                        }
                        // Some expandable commands consume themselves
                        // without putting a command back on input. In an
                        // `\expandafter` second-operand phase, replay the
                        // saved first token now instead of consuming an
                        // unrelated third token as the second result.
                        let no_output = match dispatch {
                            ExpansionDispatch::Undefined => true,
                            ExpansionDispatch::Primitive(primitive)
                                if crate::conditionals::ConditionalKind::from_primitive(
                                    primitive,
                                )
                                .is_some_and(|kind| {
                                    kind != crate::conditionals::ConditionalKind::IfCsName
                                }) =>
                            {
                                true
                            }
                            ExpansionDispatch::Primitive(
                                ExpandablePrimitive::Else
                                | ExpandablePrimitive::Or
                                | ExpandablePrimitive::Fi,
                            )
                            | ExpansionDispatch::Primitive(ExpandablePrimitive::Unless) => true,
                            ExpansionDispatch::Macro => {
                                let input_changed = macro_input_before.flatten()
                                    != self.command.top_input_level_identity();
                                !(input_changed
                                    && self.command.input.levels.last().is_some_and(|level| {
                                        level.macro_body().is_some_and(|body| !body.body.is_empty())
                                    }))
                            }
                            _ => false,
                        };
                        if no_output && expandafter_pending {
                            self.complete_expandafter_without_second()?;
                        }
                        return Ok(ExpandedHotDispatch::Continue);
                    }
                    Err(failure) => {
                        if admission.is_some_and(ParentAdmission::needs_await)
                            && let Some(parent) = parent
                        {
                            self.command
                                .scratch
                                .resume_expansion_control_parent(parent)
                                .map_err(crate::scan_toks::scratch_command_error)?;
                        }
                        failure
                    }
                };
                match failure {
                    CommandError::ParagraphInMacroArgument | CommandError::OuterInMacroArgument => {
                        Ok(ExpandedHotDispatch::Continue)
                    }
                    failure => self.fail_expanded_dispatch(destination, depth, failure),
                }
            }
        }
    }

    /// Runs a synchronous child admission for a hot expansion branch. The
    /// parent slot is captured before the branch can push a child, moved to
    /// its exact awaiting phase, and then either carried by the newly pushed
    /// frame or resumed directly if the branch produced no control. This is
    /// used by the branches that consume an expandable primitive before the
    /// common dispatch arm gets a chance to perform the same bookkeeping.
    fn run_nested_expansion_with_parent<T>(
        &mut self,
        active: Option<ActiveControlSnapshot<G>>,
        carried_parent: &mut Option<ParentAdmission<G>>,
        start: impl FnOnce(
            &mut Self,
            Option<crate::expansion_work::ExpansionControlSlot<G>>,
        ) -> Result<T, CommandError>,
    ) -> Result<T, CommandError> {
        let admission = if let Some(parent) = carried_parent.take() {
            Some(parent)
        } else {
            active
                .and_then(|control| control.awaitable_slot())
                .map(ParentAdmission::Captured)
        };
        if let Some(admission) = admission
            && admission.needs_await()
        {
            self.command
                .scratch
                .await_expansion_control_for_child(admission.slot())
                .map_err(crate::scan_toks::scratch_command_error)?;
        }
        let parent = admission.map(ParentAdmission::slot);
        let result = start(self, parent);
        if result.is_err()
            && admission.is_some_and(ParentAdmission::needs_await)
            && let Some(parent) = parent
        {
            self.command
                .scratch
                .resume_expansion_control_parent(parent)
                .map_err(crate::scan_toks::scratch_command_error)?;
        }
        result
    }

    /// Starts one compact scanner/control lane from a fixed primitive
    /// invocation projection. The occupied command remains the owner; only
    /// the descriptor's opener identity and primitive selector cross this
    /// boundary.
    #[inline(always)]
    fn begin_hot_primitive_continuation(
        &mut self,
        invocation: HotPrimitiveInvocation<G>,
        active: Option<ActiveControlSnapshot<G>>,
        carried_parent: &mut Option<ParentAdmission<G>>,
    ) -> Result<(), CommandError> {
        self.run_nested_expansion_with_parent(active, carried_parent, |this, parent| {
            let primitive = invocation.primitive;
            if primitive == ExpandablePrimitive::IfCsName {
                return this.begin_ifcsname_continuation_with_parent(false, parent);
            }
            if let Some(kind) = crate::conditionals::ConditionalKind::from_primitive(primitive) {
                return if matches!(
                    kind,
                    crate::conditionals::ConditionalKind::If
                        | crate::conditionals::ConditionalKind::IfCat
                ) {
                    this.begin_if_compare_continuation_with_parent(kind, false, parent)
                } else if matches!(
                    kind,
                    crate::conditionals::ConditionalKind::IfDim
                        | crate::conditionals::ConditionalKind::IfPdfAbsDim
                ) {
                    this.begin_if_dimension_continuation_with_parent(kind, false, parent)
                } else {
                    this.begin_if_number_continuation_with_parent(kind, false, parent)
                };
            }
            match primitive {
                ExpandablePrimitive::Expanded => {
                    this.begin_expanded_continuation_with_parent(invocation.origin, parent)
                }
                ExpandablePrimitive::ExpandAfter => this
                    .command
                    .scratch
                    .push_expandafter_control_with_parent(invocation.origin, parent)
                    .map_err(crate::scan_toks::scratch_command_error),
                ExpandablePrimitive::CsName => {
                    this.begin_csname_continuation_with_parent(invocation.origin, parent)
                }
                ExpandablePrimitive::IfCsName => unreachable!("ifcsname handled above"),
                ExpandablePrimitive::The => {
                    this.begin_the_continuation_with_parent(invocation.origin, parent)
                }
                primitive @ (ExpandablePrimitive::FontName
                | ExpandablePrimitive::PdfFontSize
                | ExpandablePrimitive::PdfFontName
                | ExpandablePrimitive::PdfFontObjectNumber) => match primitive {
                    ExpandablePrimitive::FontName => {
                        this.begin_fontname_continuation_with_parent(invocation.origin, parent)
                    }
                    ExpandablePrimitive::PdfFontSize => {
                        this.begin_pdf_font_size_continuation_with_parent(invocation.origin, parent)
                    }
                    ExpandablePrimitive::PdfFontName => {
                        this.begin_pdf_font_name_continuation_with_parent(invocation.origin, parent)
                    }
                    ExpandablePrimitive::PdfFontObjectNumber => this
                        .begin_pdf_font_object_number_continuation_with_parent(
                            invocation.origin,
                            parent,
                        ),
                    _ => unreachable!("font primitive branch validates its primitive"),
                },
                primitive @ (ExpandablePrimitive::PdfInsertHeight
                | ExpandablePrimitive::PdfXFormName
                | ExpandablePrimitive::PdfPageRef
                | ExpandablePrimitive::PdfLastMatch) => match primitive {
                    ExpandablePrimitive::PdfInsertHeight => this
                        .begin_pdf_insert_height_continuation_with_parent(
                            invocation.origin,
                            parent,
                        ),
                    ExpandablePrimitive::PdfXFormName => this
                        .begin_pdf_xform_name_continuation_with_parent(invocation.origin, parent),
                    ExpandablePrimitive::PdfPageRef => {
                        this.begin_pdf_page_ref_continuation_with_parent(invocation.origin, parent)
                    }
                    ExpandablePrimitive::PdfLastMatch => this
                        .begin_pdf_last_match_continuation_with_parent(invocation.origin, parent),
                    _ => unreachable!("PDF integer branch validates its primitive"),
                },
                ExpandablePrimitive::PdfXImageBBox => {
                    this.begin_pdf_ximage_bbox_continuation_with_parent(invocation.origin, parent)
                }
                primitive @ (ExpandablePrimitive::PdfEscapeString
                | ExpandablePrimitive::PdfEscapeHex
                | ExpandablePrimitive::PdfUnescapeHex
                | ExpandablePrimitive::StringCompare) => {
                    let kind = match primitive {
                        ExpandablePrimitive::PdfEscapeString => crate::expansion_work::control::
                            SynchronousExpandedKind::PdfEscapeString,
                        ExpandablePrimitive::PdfEscapeHex => crate::expansion_work::control::
                            SynchronousExpandedKind::PdfEscapeHex,
                        ExpandablePrimitive::PdfUnescapeHex => crate::expansion_work::control::
                            SynchronousExpandedKind::PdfUnescapeHex,
                        ExpandablePrimitive::StringCompare => crate::expansion_work::control::
                            SynchronousExpandedKind::PdfStringCompareLeft,
                        _ => unreachable!("PDF string branch validates its primitive"),
                    };
                    this.begin_pdf_string_continuation_with_parent(invocation.origin, kind, parent)
                }
                primitive @ (ExpandablePrimitive::TopMarks
                | ExpandablePrimitive::FirstMarks
                | ExpandablePrimitive::BotMarks
                | ExpandablePrimitive::SplitFirstMarks
                | ExpandablePrimitive::SplitBotMarks) => this
                    .begin_mark_class_continuation_with_parent(
                        invocation.origin,
                        primitive,
                        parent,
                    ),
                primitive @ (ExpandablePrimitive::Number | ExpandablePrimitive::RomanNumeral) => {
                    this.begin_number_continuation_with_parent(
                        invocation.origin,
                        primitive == ExpandablePrimitive::RomanNumeral,
                        parent,
                    )
                }
                ExpandablePrimitive::PdfUniformDeviate => this
                    .begin_pdf_uniform_deviate_continuation_with_parent(invocation.origin, parent),
                primitive @ (ExpandablePrimitive::LeftMarginKern
                | ExpandablePrimitive::RightMarginKern) => {
                    let side = if primitive == ExpandablePrimitive::LeftMarginKern {
                        tex_state::node::MarginKernSide::Left
                    } else {
                        tex_state::node::MarginKernSide::Right
                    };
                    this.begin_pdf_margin_kern_continuation_with_parent(
                        invocation.origin,
                        side,
                        parent,
                    )
                }
                _ => Err(CommandError::input_invariant()),
            }
        })
    }

    /// TeX82 §370's undefined-command recovery has no operand and therefore
    /// remains entirely compact unless an outer diagnostic explicitly needs a
    /// richer command owner.
    #[inline(always)]
    fn expand_undefined_hot(
        &mut self,
        command: &HotCommand<G>,
        active: Option<ActiveControlSnapshot<G>>,
        delivery_expanded: &mut bool,
        suppress_first_expansion_trace: &mut bool,
    ) -> Result<ExpandedHotDispatch, CommandError> {
        *delivery_expanded = true;
        let report_trace = !std::mem::take(suppress_first_expansion_trace);
        if report_trace && self.command.delivery_mode.tracing() {
            self.print_hot_command_trace(command);
        }
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_undefined_expansion();
        let context = self.command.output_open_context(self.state);
        let site = Some(self.complete_diagnostic_site(self.capture_hot_diagnostic_site(command)));
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::UndefinedControlSequence { context, site });
        if !self.command.profile().capabilities().supports_etex() {
            self.observe_hot_command_diagnostic("undefined_control_sequence", command);
        }
        if matches!(
            active,
            Some(ActiveControlSnapshot::ExpandAfterSync(control))
                if control.phase
                    == crate::expansion_work::control::SynchronousExpandAfterPhase::NeedSecond
        ) {
            self.complete_expandafter_without_second()?;
        }
        Ok(ExpandedHotDispatch::Continue)
    }

    /// Expand one primitive while the delivery loop still owns its compact
    /// command.  The common synchronous families below consume only the
    /// packed command word and delivery projections; scanner/resource and
    /// diagnostic families take the explicit cold path at the end.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn expand_primitive_hot(
        &mut self,
        command: &mut HotCommand<G>,
        primitive: ExpandablePrimitive,
        active: Option<ActiveControlSnapshot<G>>,
        delivery_expanded: &mut bool,
        suppress_first_expansion_trace: &mut bool,
        carried_parent: &mut Option<ParentAdmission<G>>,
        destination: &mut Option<HotCommand<G>>,
        depth: u32,
        command_parked: &mut bool,
    ) -> Result<ExpandedHotDispatch, CommandError> {
        *delivery_expanded = true;
        let report_trace = !std::mem::take(suppress_first_expansion_trace);

        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_expandable_opcode(
            usize::try_from(primitive.operand()).expect("expandable primitive operand fits usize"),
        );

        // Keep the descriptor construction next to the primitive match.  It
        // is copy-small and does not transfer ownership out of `command`;
        // scanner starters retain only the fields they actually need.
        let invocation = command.primitive_invocation(primitive);

        if !is_hot_synchronous_primitive(primitive) {
            return self.expand_primitive_cold(
                command,
                primitive,
                report_trace,
                *delivery_expanded,
                active,
                carried_parent,
                destination,
                depth,
                command_parked,
            );
        }
        record_primitive_hot_dispatch();

        // TeX82 §367 traces the primitive before it consumes an operand. The
        // hot print projection retains the exact command identity without a
        // `CurrentCommand` bridge.
        if report_trace && self.command.delivery_mode.tracing() {
            self.print_hot_command_trace(command);
        }

        let result = if hot_primitive_starts_control(primitive) {
            self.begin_hot_primitive_continuation(invocation, active, carried_parent)
        } else {
            let admission = if let Some(parent) = carried_parent.take() {
                Some(parent)
            } else {
                active
                    .and_then(|control| control.awaitable_slot())
                    .map(ParentAdmission::Captured)
            };
            if let Some(admission) = admission
                && admission.needs_await()
            {
                self.command
                    .scratch
                    .await_expansion_control_for_child(admission.slot())
                    .map_err(crate::scan_toks::scratch_command_error)?;
            }
            let parent = admission.map(ParentAdmission::slot);
            let result = match primitive {
                primitive @ (ExpandablePrimitive::TopMark
                | ExpandablePrimitive::FirstMark
                | ExpandablePrimitive::BotMark
                | ExpandablePrimitive::SplitFirstMark
                | ExpandablePrimitive::SplitBotMark) => self.expand_mark(primitive),
                ExpandablePrimitive::EndInput => self.expand_endinput(),
                ExpandablePrimitive::JobName => {
                    self.state.unsupported_host_capability();
                    let job_name = self.host.job_name().to_owned();
                    self.push_rendered_text(&job_name, invocation.origin);
                    Ok(())
                }
                ExpandablePrimitive::ETeXRevision => {
                    self.push_rendered_text(".6", invocation.origin);
                    Ok(())
                }
                ExpandablePrimitive::PdfTeXRevision => {
                    self.push_rendered_text("27", invocation.origin);
                    Ok(())
                }
                ExpandablePrimitive::PdfTeXBanner => {
                    self.push_rendered_text(
                        "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) kpathsea version 6.4.2",
                        invocation.origin,
                    );
                    Ok(())
                }
                ExpandablePrimitive::PdfNormalDeviate => {
                    let value = self.state.pdf_normal_deviate();
                    self.push_rendered_text(&value.to_string(), invocation.origin);
                    Ok(())
                }
                ExpandablePrimitive::CreationDate => {
                    let clock = self.state.job_clock();
                    self.push_rendered_text(&format_pdf_date(clock, 0), invocation.origin);
                    Ok(())
                }
                ExpandablePrimitive::ShellEscape => {
                    let status = self
                        .state
                        .internal_integer(tex_state::meaning::InternalInteger::PdfShellEscape)
                        .expect("the shell-escape status is an integer enquiry");
                    self.push_rendered_text(&status.to_string(), invocation.origin);
                    Ok(())
                }
                _ => unreachable!("primitive was filtered by is_hot_synchronous_primitive"),
            };
            if let Some(parent) = parent
                && result.is_ok()
            {
                self.command
                    .scratch
                    .resume_expansion_control_parent(parent)
                    .map_err(crate::scan_toks::scratch_command_error)?;
            } else if let Some(parent) = parent
                && admission.is_some_and(ParentAdmission::needs_await)
                && result.is_err()
            {
                self.command
                    .scratch
                    .resume_expansion_control_parent(parent)
                    .map_err(crate::scan_toks::scratch_command_error)?;
            }
            result
        };

        match result {
            Ok(()) => Ok(ExpandedHotDispatch::Continue),
            Err(error) => self.fail_expanded_dispatch(destination, depth, error),
        }
    }

    /// Explicit cold boundary for primitive families whose scanner, observer,
    /// diagnostic, or host/resource owner still needs a rich command.  The
    /// occupied hot owner is materialized once and reconstructed only when
    /// the operation completes synchronously; suspension parks that one rich
    /// owner and never rebuilds the hot pair on the return edge.
    #[cold]
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn expand_primitive_cold(
        &mut self,
        command: &mut HotCommand<G>,
        primitive: ExpandablePrimitive,
        report_trace: bool,
        delivery_expanded: bool,
        active: Option<ActiveControlSnapshot<G>>,
        carried_parent: &mut Option<ParentAdmission<G>>,
        destination: &mut Option<HotCommand<G>>,
        depth: u32,
        command_parked: &mut bool,
    ) -> Result<ExpandedHotDispatch, CommandError> {
        record_primitive_cold_materialization();
        let admission = if let Some(parent) = carried_parent.take() {
            Some(parent)
        } else {
            active
                .and_then(|control| control.awaitable_slot())
                .map(ParentAdmission::Captured)
        };
        if let Some(admission) = admission
            && admission.needs_await()
        {
            self.command
                .scratch
                .await_expansion_control_for_child(admission.slot())
                .map_err(crate::scan_toks::scratch_command_error)?;
        }
        let parent = admission.map(ParentAdmission::slot);
        let mut rich = command.materialize();
        let prior_scanner_parent = std::mem::replace(&mut self.scanner_return_parent, parent);
        let result = self.expand_classified_rich_occupied(
            &mut rich,
            ExpansionDispatch::Primitive(primitive),
            report_trace,
            delivery_expanded,
            parent,
            command_parked,
        );
        self.scanner_return_parent = prior_scanner_parent;
        if !*command_parked {
            *command = HotCommand::from_current(rich);
        }
        match result {
            Ok(()) => {
                if let Some(parent) = parent
                    && !primitive_owns_parent(primitive)
                {
                    self.command
                        .scratch
                        .resume_expansion_control_parent(parent)
                        .map_err(crate::scan_toks::scratch_command_error)?;
                }
                Ok(ExpandedHotDispatch::Continue)
            }
            Err(error) => {
                if !*command_parked
                    && admission.is_some_and(ParentAdmission::needs_await)
                    && let Some(parent) = parent
                {
                    self.command
                        .scratch
                        .resume_expansion_control_parent(parent)
                        .map_err(crate::scan_toks::scratch_command_error)?;
                }
                self.fail_expanded_dispatch(destination, depth, error)
            }
        }
    }

    /// Completes a source or synthetic `endv` command after the main-loop
    /// reader has crossed a cold input boundary. Such a command still needs
    /// the ordinary delivery settlement, but it never belongs to the warm
    /// character-run body.
    #[cold]
    #[inline(never)]
    fn finish_main_loop_synthetic(
        &mut self,
        command: &mut Option<HotCommand<G>>,
        literal_catcode: Option<Catcode>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        if let Err(failure) = self.fuel.charge() {
            self.invalidate_delivery_freshness();
            return Err(failure);
        }
        let mut command = command.take().ok_or_else(CommandError::input_invariant)?;
        if let Err(failure) = self.settle_hot_delivery(&mut command, literal_catcode) {
            self.invalidate_delivery_freshness();
            return Err(failure);
        }
        *destination = Some(command.materialize());
        Ok(DeliveryStatus::CharacterRunBoundary)
    }

    #[cold]
    #[inline(never)]
    fn finish_main_cold_transition(
        &mut self,
        cold: ResidentColdOutcome,
        command: &mut Option<HotCommand<G>>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<Option<DeliveryStatus>, CommandError> {
        match cold {
            ResidentColdOutcome::Retry => Ok(None),
            ResidentColdOutcome::Finished(status) => Ok(Some(status)),
            ResidentColdOutcome::Synthetic { literal_catcode } => self
                .finish_main_loop_synthetic(command, literal_catcode, destination)
                .map(Some),
        }
    }

    /// Consumes the direct ordinary-character prefix owned by main control.
    /// The consumer is mandatory here; no other delivery loop carries it.
    #[inline(always)]
    pub(super) fn main_character_run(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        consume: &mut impl MainCharacterConsumer<G>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        self.invalidate_delivery_freshness();
        let mut command = None;
        let mut consumed_characters = false;
        #[cfg(feature = "profiling")]
        let mut character_run_count = 0_u32;
        #[cfg(feature = "profiling")]
        let mut character_run_kind = None;

        loop {
            let Some(resident_index) = self.command.roots.input.levels.top.checked_sub(1) else {
                if consumed_characters {
                    self.invalidate_delivery_freshness();
                    #[cfg(feature = "profiling")]
                    if let Some(kind) = character_run_kind.take() {
                        self.fuel.record_raw_run(false, kind, character_run_count);
                    }
                    return Ok(DeliveryStatus::CharacterRun);
                }
                let cold = self.transition_resident_word(
                    ResidentWordRead::NoResident,
                    &mut command,
                    false,
                )?;
                if let Some(status) =
                    self.finish_main_cold_transition(cold, &mut command, destination)?
                {
                    return Ok(status);
                }
                continue;
            };

            let is_source = matches!(
                self.command.roots.input.levels.rows[resident_index],
                InputLevel::Source(_)
            );
            if is_source {
                #[cfg(test)]
                {
                    self.command
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .typed_top_accesses = self
                        .command
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .typed_top_accesses
                        .saturating_add(1);
                    self.command
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .source_branch_entries = self
                        .command
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .source_branch_entries
                        .saturating_add(1);
                    self.command.raw_delivery_path_counters.resident_transitions = self
                        .command
                        .raw_delivery_path_counters
                        .resident_transitions
                        .saturating_add(1);
                }
                if self
                    .advance_source_character_step(resident_index, consume)?
                    .is_some()
                {
                    // The source cursor and its provenance moved in place;
                    // no resident command can retain the previous
                    // freshness proof across this direct admission.
                    self.invalidate_delivery_freshness();
                    return Ok(DeliveryStatus::CharacterRun);
                }
                let cold = self.transition_resident_word(
                    ResidentWordRead::Source { resident_index },
                    &mut command,
                    false,
                )?;
                if let Some(status) =
                    self.finish_main_cold_transition(cold, &mut command, destination)?
                {
                    return Ok(status);
                }
                continue;
            }

            let selected = match self.next_resident_word() {
                Ok(selected) => selected,
                Err(failure) => {
                    self.invalidate_delivery_freshness();
                    return Err(failure);
                }
            };
            if !matches!(selected, ResidentWordRead::Word { .. }) {
                if consumed_characters && matches!(selected, ResidentWordRead::Exhausted { .. }) {
                    self.invalidate_delivery_freshness();
                    #[cfg(feature = "profiling")]
                    if let Some(kind) = character_run_kind.take() {
                        self.fuel.record_raw_run(false, kind, character_run_count);
                    }
                    return Ok(DeliveryStatus::CharacterRun);
                }
                let cold = self.transition_resident_word(selected, &mut command, false)?;
                if let Some(status) =
                    self.finish_main_cold_transition(cold, &mut command, destination)?
                {
                    return Ok(status);
                }
                continue;
            }
            let is_character = matches!(
                &selected,
                ResidentWordRead::Word {
                    word,
                    ..
                } if matches!(
                    word.semantic_token(),
                    Token::Char {
                        cat: Catcode::Letter | Catcode::Other,
                        ..
                    }
                )
            );
            if is_character && self.command.delivery_mode.allows_character_run() {
                let ResidentWordRead::Word {
                    word,
                    origin,
                    #[cfg(feature = "profiling")]
                    raw_kind,
                    ..
                } = selected
                else {
                    unreachable!("character predicate accepts only resident words")
                };
                if let Err(failure) = self.fuel.charge() {
                    self.invalidate_delivery_freshness();
                    return Err(failure);
                }
                consumed_characters = true;
                #[cfg(feature = "profiling")]
                {
                    character_run_kind = Some(raw_kind);
                    character_run_count = character_run_count.saturating_add(1);
                }
                let Token::Char { ch, .. } = word.semantic_token() else {
                    unreachable!("main-loop character predicate accepts only characters")
                };
                if consume
                    .admit(
                        self.state,
                        self.fuel,
                        self.diagnostic_effects,
                        MainCharacterInput::Scalar { ch, origin },
                    )
                    .continue_run()
                {
                    continue;
                }
                self.invalidate_delivery_freshness();
                #[cfg(feature = "profiling")]
                if let Some(kind) = character_run_kind.take() {
                    self.fuel.record_raw_run(false, kind, character_run_count);
                }
                return Ok(DeliveryStatus::CharacterRun);
            }

            if let Err(failure) = self.fuel.charge() {
                self.invalidate_delivery_freshness();
                return Err(failure);
            }
            #[cfg(feature = "profiling")]
            if let Some(kind) = character_run_kind.take() {
                self.fuel.record_raw_run(false, kind, character_run_count);
            }
            let literal_catcode = self.admit_resident_word(selected, &mut command)?;
            let mut command = command.take().ok_or_else(CommandError::input_invariant)?;
            if let Err(failure) = self.settle_hot_delivery(&mut command, literal_catcode) {
                self.invalidate_delivery_freshness();
                return Err(failure);
            }
            *destination = Some(command.materialize());
            return Ok(DeliveryStatus::CharacterRunBoundary);
        }
    }

    /// Replay-aware raw delivery is a cold entry for the same raw owner. The
    /// public ordinary wrapper consumes completion statuses; replay callers
    /// keep them visible.
    #[cold]
    #[inline(never)]
    pub(super) fn raw_next_with_replay_completion(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.raw_next(destination)
    }

    /// Replay-aware ordinary expansion enters the canonical expanded loop and
    /// leaves its completion status visible to the caller.
    #[cold]
    #[inline(never)]
    pub(super) fn expanded_next_with_replay_completion(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next(destination)
    }

    /// The protected entry is intentionally out of line. Its full protected
    /// classifier is installed below once a raw command has been settled.
    #[cold]
    #[inline(never)]
    pub(super) fn protected_expanded_next_with_replay_completion(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_until(destination, ExpandedUntilMode::Protected)
    }

    #[cold]
    #[inline(never)]
    fn expanded_until(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        mode: ExpandedUntilMode,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let status = if destination.is_some() {
                DeliveryStatus::Command
            } else if self.expansion_resume.is_some()
                || self
                    .scanner_resume
                    .as_ref()
                    .is_some_and(crate::ScannerFrameKey::is_expansion)
            {
                self.expanded_next(destination)?
            } else {
                self.raw_next(destination)?
            };
            match status {
                DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_) => return Ok(status),
                DeliveryStatus::Command => {}
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                    continue;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("protected delivery has no character consumer")
                }
            }

            let command = destination
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?;
            let stop = match mode {
                ExpandedUntilMode::Protected => {
                    matches!(
                        command.meaning_ref(),
                        ResolvedMeaning::Macro { flags, .. }
                            if flags.contains(MeaningFlags::PROTECTED)
                    ) || !is_expandable_command(command)
                }
                ExpandedUntilMode::PreserveUndefined => matches!(
                    command.meaning_ref(),
                    ResolvedMeaning::Static(Meaning::Undefined)
                ),
            };
            if stop {
                return Ok(DeliveryStatus::Command);
            }
            match self.expanded_next(destination)? {
                status @ (DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_)) => {
                    return Ok(status);
                }
                DeliveryStatus::Command => return Ok(DeliveryStatus::Command),
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("protected delivery has no character consumer")
                }
            }
        }
    }

    /// Diagnostic callers keep the undefined command instead of entering its
    /// recovery branch. The exceptional wrapper is cold and owns that one
    /// classifier choice.
    #[cold]
    #[inline(never)]
    pub(super) fn expanded_next_preserving_undefined(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_until(destination, ExpandedUntilMode::PreserveUndefined)
    }

    /// `x_token` starts with a command already in hand. Ordinary uses have no
    /// pending command and therefore enter the expanded loop directly.
    #[cold]
    #[inline(never)]
    pub(super) fn x_token_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.x_token_next_with_action(destination, None)
    }

    fn x_token_next_with_action(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut initial_action = initial_action;
        if destination.as_ref().is_some_and(|command| {
            matches!(
                command.meaning_ref(),
                ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                    ExpandablePrimitive::EndTemplate
                ))
            )
        }) {
            let alignment_delimiter = destination.as_ref().is_some_and(|command| {
                matches!(
                    command.alignment_adjustment(),
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                )
            });
            if alignment_delimiter {
                return Ok(DeliveryStatus::AlignmentEndTemplate);
            }
            destination.take();
            self.insert_frozen_endv()?;
            initial_action = None;
        }
        self.expanded_next_with_action(destination, initial_action)
    }

    /// Main-control lookahead first returns a raw character without expansion;
    /// non-character commands continue through the x-token entry.
    #[cold]
    #[inline(never)]
    pub(super) fn main_loop_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.main_action_lookahead(destination, false)
    }

    #[cold]
    #[inline(never)]
    fn main_action_lookahead(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        preflight: bool,
    ) -> Result<DeliveryStatus, CommandError> {
        if destination.is_none() && self.canonical_expression_resume_pending()? {
            self.run_pending_canonical_expression()?;
            return self.expanded_next(destination);
        }
        let existing = destination.is_some();
        let mut hot_destination = if existing {
            destination.as_ref().map(HotCommand::from_current_ref)
        } else {
            None
        };
        if hot_destination.is_none() {
            match self.raw_next_hot(&mut hot_destination)? {
                DeliveryStatus::Command => {}
                status => return Ok(status),
            }
        }
        let hot = hot_destination
            .as_ref()
            .ok_or_else(CommandError::input_invariant)?;
        let is_character = hot.command_word().is_main_loop_character();
        if is_character && !preflight {
            if !existing {
                *destination = hot_destination.take().map(|command| command.materialize());
            }
            return Ok(DeliveryStatus::Command);
        }
        let action = classify_hot_command(hot);
        if matches!(action, ExpandedCommandAction::EndTemplate) {
            if matches!(
                hot.alignment_adjustment(),
                crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
            ) {
                if !existing {
                    *destination = hot_destination.take().map(|command| command.materialize());
                }
                return Ok(DeliveryStatus::AlignmentEndTemplate);
            }
            hot_destination.take();
            destination.take();
            self.insert_frozen_endv()?;
            let result = self.expanded_next_hot(&mut hot_destination, None);
            return self.finish_hot_delivery(destination, &mut hot_destination, result);
        }
        if is_character || matches!(action, ExpandedCommandAction::Return) {
            if existing {
                self.observe_expanded_delivery(
                    destination
                        .as_ref()
                        .ok_or_else(CommandError::input_invariant)?,
                );
            } else {
                let command = hot_destination
                    .take()
                    .ok_or_else(CommandError::input_invariant)?
                    .materialize();
                self.observe_expanded_delivery(&command);
                *destination = Some(command);
            }
            return Ok(DeliveryStatus::Command);
        }
        let result = self.expanded_next_hot(&mut hot_destination, Some(action));
        if matches!(
            result,
            Ok(DeliveryStatus::PendingExpanded)
                if self.command.scratch.has_pending_expression_frame()
        ) {
            hot_destination.take();
            destination.take();
            self.run_pending_canonical_expression()?;
            return self.expanded_next(destination);
        }
        self.finish_hot_delivery(destination, &mut hot_destination, result)
    }

    /// Main-control preflight owns its first raw fetch and then continues from
    /// that resident command through ordinary expansion.
    #[cold]
    #[inline(never)]
    pub(super) fn preflight_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.main_action_lookahead(destination, true)? {
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                result => return Ok(result),
            }
        }
    }

    /// Resumed expansion restores its command once, then uses the x-token
    /// semantics owned by the cold continuation wrapper.
    #[cold]
    #[inline(never)]
    pub(super) fn resumed_expanded_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.x_token_next(destination)? {
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                result => return Ok(result),
            }
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn resumed_main_loop_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.main_loop_next(destination)? {
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                result => return Ok(result),
            }
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn alignment_expanded_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        match self.expanded_next(destination)? {
            DeliveryStatus::PendingExpanded => Ok(DeliveryStatus::Command),
            result => Ok(result),
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn alignment_main_loop_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        match self.main_loop_next(destination)? {
            DeliveryStatus::PendingExpanded => Ok(DeliveryStatus::Command),
            result => Ok(result),
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn tex_alignment_lookahead_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.expanded_next(destination)? {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::AlignmentClosingBrace => return Ok(DeliveryStatus::Command),
                result => return Ok(result),
            }
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn etex_alignment_lookahead_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.protected_expanded_next_with_replay_completion(destination)? {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::AlignmentClosingBrace => return Ok(DeliveryStatus::Command),
                result => return Ok(result),
            }
        }
    }

    /// Delivers one ordinary expanded command through TeX.web's `get_x_token`.
    ///
    /// This thin canonical entry point enters the ordinary expanded loop.
    /// Expansion mutates canonical command state and restarts in that loop;
    /// it never returns a push-bearing dispatch result or enters a second
    /// interpreter.
    pub fn get_x_token(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.get_x_token_into(&mut destination)? {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary expanded delivery returns only commands"),
        }
    }

    /// Delivers one expanded command directly into caller-provided storage.
    pub fn get_x_token_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.get_x_token_into_with_boundary(destination, None)
    }

    fn get_x_token_into_with_boundary(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        return_boundary: Option<(
            crate::expansion_work::ExpansionControlSlot<G>,
            crate::expansion_work::control::ExpansionReturnSink,
        )>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        loop {
            let result = self.expanded_next_with_boundary(destination, None, return_boundary)?;
            match result {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::End | DeliveryStatus::Command => return Ok(result),
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("expanded delivery does not own a character consumer")
                }
            }
        }
    }

    /// Requests one expanded token from the generation-scoped delivery
    /// driver.  Scanner and primitive code uses this typed status boundary;
    /// it never reaches into the driver's loop or recursively calls a
    /// delivery implementation by name.
    pub(crate) fn request_expanded_token(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        use crate::expansion_work::control::ExpansionReturnSink;

        let return_to = ExpansionReturnSink::ScannerToken;
        let resuming_parked_expansion = self
            .scanner_resume
            .as_ref()
            .is_some_and(crate::ScannerFrameKey::is_expansion)
            && self.resumed_return_capability.is_none();
        if resuming_parked_expansion {
            let prior_capability = self.scanner_return_capability.take();
            self.resumed_return_sink = Some(return_to);
            let result = self.get_x_token_into_with_boundary(destination, None);
            self.resumed_return_sink = None;
            let current_capability =
                std::mem::replace(&mut self.scanner_return_capability, prior_capability);
            if result.is_ok() {
                self.command
                    .scratch
                    .finish_expansion_return(
                        current_capability.ok_or_else(CommandError::input_invariant)?,
                    )
                    .map_err(crate::scan_toks::scratch_command_error)?;
            }
            return result;
        }
        let capability = if self
            .resumed_return_capability
            .as_ref()
            .is_some_and(|capability| capability.sink() == return_to)
        {
            self.resumed_return_capability
                .take()
                .ok_or_else(CommandError::input_invariant)?
        } else {
            let parent = self.current_scanner_return_parent();
            self.command
                .scratch
                .begin_expansion_return(return_to, parent)
                .map_err(crate::scan_toks::scratch_command_error)?
        };
        let (return_slot, return_sink) = (capability.slot(), capability.sink());
        let prior_capability =
            std::mem::replace(&mut self.scanner_return_capability, Some(capability));
        let result =
            self.get_x_token_into_with_boundary(destination, Some((return_slot, return_sink)));
        let current_capability =
            std::mem::replace(&mut self.scanner_return_capability, prior_capability);
        if result.is_ok() {
            self.command
                .scratch
                .finish_expansion_return(
                    current_capability.ok_or_else(CommandError::input_invariant)?,
                )
                .map_err(crate::scan_toks::scratch_command_error)?;
        }
        result
    }

    /// Requests one already-delivered command's expansion from the same
    /// driver.  This is the only nested expansion request used by structural
    /// scanners; suspension and completion remain represented by the typed
    /// `Result` status returned here.
    pub(crate) fn request_expansion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        use crate::expansion_work::control::ExpansionReturnSink;

        let return_to = ExpansionReturnSink::ScannerExpansion;
        let resuming_parked_expansion = self
            .scanner_resume
            .as_ref()
            .is_some_and(crate::ScannerFrameKey::is_expansion)
            && self.resumed_return_capability.is_none();
        if resuming_parked_expansion {
            let prior_capability = self.scanner_return_capability.take();
            let result = (|| {
                let result = self.expand_into_with_parent(destination, report_trace, None);
                if result.is_ok() {
                    destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                }
                let Some(capability) = self.scanner_return_capability.as_ref() else {
                    return result.and_then(|_| Err(CommandError::input_invariant()));
                };
                let return_slot = capability.slot();
                let return_sink = capability.sink();
                if result.is_ok() && self.command.scratch.active_control_slot() != Some(return_slot)
                {
                    loop {
                        match self.expanded_next_with_boundary(
                            destination,
                            None,
                            Some((return_slot, return_sink)),
                        )? {
                            DeliveryStatus::ReplayCompleted(_) => continue,
                            DeliveryStatus::PendingExpanded
                                if destination.is_none()
                                    && self.command.scratch.active_control_slot()
                                        == Some(return_slot) =>
                            {
                                break;
                            }
                            DeliveryStatus::Command => {
                                return Err(CommandError::input_invariant());
                            }
                            _ => return Err(CommandError::input_invariant()),
                        }
                    }
                }
                result
            })();
            let current_capability =
                std::mem::replace(&mut self.scanner_return_capability, prior_capability);
            if result.is_ok() {
                self.command
                    .scratch
                    .finish_expansion_return(
                        current_capability.ok_or_else(CommandError::input_invariant)?,
                    )
                    .map_err(crate::scan_toks::scratch_command_error)?;
            }
            return result;
        }
        let use_carried_return = self
            .resumed_return_capability
            .as_ref()
            .is_some_and(|capability| capability.sink() == return_to);
        let capability = if use_carried_return {
            self.resumed_return_capability
                .take()
                .ok_or_else(CommandError::input_invariant)?
        } else {
            let parent = self.current_scanner_return_parent();
            self.command
                .scratch
                .begin_expansion_return(return_to, parent)
                .map_err(crate::scan_toks::scratch_command_error)?
        };
        let (return_slot, return_sink) = (capability.slot(), capability.sink());
        let prior_capability =
            std::mem::replace(&mut self.scanner_return_capability, Some(capability));
        let result = (|| {
            let result = self.expand_into_with_parent(
                destination,
                report_trace,
                (!use_carried_return).then_some(return_slot),
            );
            if result.is_ok() {
                // A successful expansion has consumed the opener in this
                // destination.  It is not a delivered result and must never
                // be re-admitted as the next expanded command.
                destination
                    .take()
                    .ok_or_else(CommandError::input_invariant)?;
            }
            if result.is_ok() && self.command.scratch.active_control_slot() != Some(return_slot) {
                // A synchronous child owns the consumed opener. Continue in
                // the same delivery driver until this exact return edge is
                // exposed; the boundary carries the sink capability through
                // the canonical loop and rejects any unowned status.
                loop {
                    let status = self.expanded_next_with_boundary(
                        destination,
                        None,
                        Some((return_slot, return_sink)),
                    )?;
                    match status {
                        DeliveryStatus::ReplayCompleted(_) => continue,
                        DeliveryStatus::PendingExpanded
                            if destination.is_none()
                                && self.command.scratch.active_control_slot()
                                    == Some(return_slot) =>
                        {
                            break;
                        }
                        DeliveryStatus::Command => {
                            return Err(CommandError::input_invariant());
                        }
                        _ => return Err(CommandError::input_invariant()),
                    }
                }
            }
            result
        })();
        let current_capability =
            std::mem::replace(&mut self.scanner_return_capability, prior_capability);
        if result.is_ok() {
            self.command
                .scratch
                .finish_expansion_return(
                    current_capability.ok_or_else(CommandError::input_invariant)?,
                )
                .map_err(crate::scan_toks::scratch_command_error)?;
        }
        result
    }

    /// A nested scanner request belongs to the capability currently lent to
    /// its caller.  A primitive's explicit parent remains the fallback for a
    /// first request; once a sink is active, chaining through its exact slot
    /// prevents a child from reaching back to an older ambient parent.
    fn current_scanner_return_parent(
        &self,
    ) -> Option<crate::expansion_work::ExpansionControlSlot<G>> {
        self.scanner_return_capability
            .as_ref()
            .map(|capability| capability.slot())
            .or(self.scanner_return_parent)
    }

    /// Delivers protected replay-aware expansion into caller-provided storage.
    pub(crate) fn get_x_or_protected_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let preserve = self.command.profile().capabilities().supports_etex();
        let result = if preserve {
            self.protected_expanded_next_with_replay_completion(destination)?
        } else {
            self.expanded_next_with_replay_completion(destination)?
        };
        debug_assert!(matches!(
            result,
            DeliveryStatus::End | DeliveryStatus::Command | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// Delivers one expanded command to a diagnostic host while preserving
    /// TeX82 §370's undefined command instead of consuming it after recovery.
    pub fn get_x_token_preserving_undefined(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        let result = self.expanded_next_preserving_undefined(&mut destination)?;
        match result {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary expanded delivery returns only commands"),
        }
    }

    /// TeX.web §381's `x_token` entered with `cur_cmd`/`cur_chr` already set.
    ///
    /// §381 does not begin with `get_next`: it expands whatever the caller
    /// left in the current command and only then reads on. Ordinary delivery
    /// leaves nothing, which is [`Self::get_x_token`]; §1152 loads an active
    /// character's meaning directly and passes it here, so that meaning is
    /// expanded without ever having been delivered raw.
    fn x_token_from_into(
        &mut self,
        pending: Option<CurrentCommand<G>>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.x_token_from_into_with_action(pending, destination, None)
    }

    fn x_token_from_into_with_action(
        &mut self,
        pending: Option<CurrentCommand<G>>,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        *destination = pending;
        let result = self.x_token_next_with_action(destination, initial_action)?;
        debug_assert!(matches!(
            result,
            DeliveryStatus::End
                | DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentEndTemplate
                | DeliveryStatus::AlignmentClosingBrace
                | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// Completes TeX82 §1152's active-character `x_token` handoff.
    ///
    /// The ordinary destination-directed expanded entry exposes
    /// `PendingExpanded` and `AlignmentClosingBrace` only as internal
    /// observer transport markers; both already leave the settled command in
    /// `destination`. Active-character treatment has the same settled-command
    /// ownership, so it must normalize those statuses without constructing or
    /// redelivering another command. An intercepted alignment end-template is
    /// the one exceptional boundary: its command is consumed to begin the
    /// scalar v-template, after which `x_token` retries with no pending
    /// command above the newly installed input frame.
    #[cold]
    #[inline(never)]
    fn active_x_token_into(
        &mut self,
        pending: CurrentCommand<G>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut pending = Some(pending);
        loop {
            match self.x_token_from_into(pending.take(), destination)? {
                DeliveryStatus::End => return Ok(DeliveryStatus::End),
                DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentClosingBrace => return Ok(DeliveryStatus::Command),
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                    // The intercepted delimiter has been consumed by the
                    // alignment transition. The next x-token starts with a
                    // fresh input fetch, rather than redelivering it.
                    pending = None;
                }
                DeliveryStatus::ReplayCompleted(_) => {
                    // Stored replay retirement is an input-boundary event,
                    // not a settled active-character command. Continue the
                    // same x-token operation after the continuation retires.
                    pending = None;
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("active-character delivery has no character consumer")
                }
            }
        }
    }

    /// TeX82 §1152's `@<Treat |cur_chr| as an active character@>`:
    ///
    /// ```text
    /// begin cur_cs:=cur_chr+active_base;
    /// cur_cmd:=eq_type(cur_cs); cur_chr:=equiv(cur_cs);
    /// x_token; back_input;
    /// end
    /// ```
    ///
    /// This is the whole of TeX's `\mathcode` escape hatch. §1155's
    /// `set_math_char` and §1151's `scan_math` both branch here when a
    /// character's `math_code` is `@'100000`, which is what makes plain
    /// TeX's ``\mathcode`\'="8000`` route `'` through the active `'` macro
    /// that builds `\prime` lists.
    ///
    /// The character is not backed up and reread. §1152 loads the
    /// `active_base + c` cell's meaning straight into `cur_cmd`/`cur_chr`,
    /// so there is no raw delivery for it at all: `x_token` expands that
    /// meaning in place -- observing a macro push, not a backup -- and only
    /// the unexpandable token expansion settles on is backed up, from where
    /// the caller rereads it. An active character bound to an unexpandable
    /// meaning still reaches §381's tail, so it is still observed as one
    /// expanded delivery and backed up unchanged.
    pub fn treat_as_active_character(
        &mut self,
        ch: char,
        origin: OriginId,
    ) -> Result<(), CommandError> {
        let spelling = TracedTokenWord::pack(
            Token::Char {
                ch,
                cat: Catcode::Active,
            },
            origin,
        );
        let stamp = DeliveryStamp::new(0, 0);
        self.advance_delivery_sequence();
        let command = CurrentCommand::<G>::resolve(spelling, stamp, None, false, None, self.state);
        let mut destination = None;
        let status = self.active_x_token_into(command, &mut destination)?;
        let settled = match status {
            DeliveryStatus::End => return Ok(()),
            DeliveryStatus::Command => destination
                .take()
                .expect("command status initializes destination"),
            _ => unreachable!("active-character delivery normalizes to commands"),
        };
        // §325 needs only `cur_tok`; the settled token is `x_token`'s result
        // rather than a delivery this call is undoing, exactly as in §326.
        self.back_input_saved(settled)
    }

    /// TeX82 §404's `<Get the next non-blank non-relax non-call token>`:
    /// `repeat get_x_token until (cur_cmd<>spacer)and(cur_cmd<>relax)`.
    ///
    /// This is the shared spelling of that module, used by §403's
    /// `scan_left_brace`, §1078, §1084, §1151's `scan_math`, §1160's
    /// non-radical `scan_delimiter`, §1211's `prefixed_command`, §1226 and
    /// §1270's `scan_optional_equals`. It differs from §406's
    /// `<Get the next non-blank non-call token>` only by also skipping
    /// `\relax`, and the two are not interchangeable: §1160 classifies the
    /// token it stops on, so a `\relax` that reached it as a command rather
    /// than as a skipped filler would scan as an invalid delimiter.
    pub fn next_non_blank_non_relax_x_token(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        loop {
            match self.get_x_token_into(&mut destination)? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary expanded delivery returns only commands"),
            }
            let command = destination
                .as_ref()
                .expect("command status initializes destination");
            if !matches!(
                static_meaning(command.meaning_ref()),
                Some(
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    } | Meaning::Relax
                )
            ) {
                return Ok(destination);
            }
            destination = None;
        }
    }

    /// TeX82 §404's expanded nonblank/non-relax fetch for scanners that can
    /// classify the terminal command directly from the compact delivery.
    ///
    /// The hot command is the sole result owner: it is overwritten on each
    /// fetch, and exactly the command that stops the loop remains in the
    /// caller's slot. In particular, no rich command is made merely to hand
    /// one delimiter operand from expansion to `scan_delimiter`.
    pub(crate) fn next_non_blank_non_relax_x_token_hot(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        loop {
            match self.expanded_next_hot(destination, None)? {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::PendingExpanded
                    if self.command.scratch.has_pending_expression_frame() =>
                {
                    destination.take();
                    self.run_pending_canonical_expression()?;
                    continue;
                }
                DeliveryStatus::End => return Ok(DeliveryStatus::End),
                DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentClosingBrace => {}
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?
                        .materialize();
                    self.begin_scalar_alignment_v_template(&command)?;
                    continue;
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    return Err(CommandError::input_invariant());
                }
            }
            let command = destination
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?;
            if !matches!(
                command.command_word().static_meaning(),
                Some(
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    } | Meaning::Relax
                )
            ) {
                return Ok(DeliveryStatus::Command);
            }
            destination.take();
        }
    }

    /// TeX82 §406's `<Get the next non-blank non-call token>`:
    /// `repeat get_x_token until cur_cmd<>spacer`.
    ///
    /// Unlike §404's similarly named helper, this preserves `\relax`. The
    /// returned command is the exact expanded delivery that stopped the
    /// loop: callers such as §1045's `\ignorespaces` dispatch it in place
    /// without backing it up or rebuilding its provenance.
    pub fn next_non_blank_x_token(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        loop {
            match self.get_x_token_into(&mut destination)? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary expanded delivery returns only commands"),
            }
            let command = destination
                .as_ref()
                .expect("command status initializes destination");
            if !matches!(
                static_meaning(command.meaning_ref()),
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                return Ok(destination);
            }
            destination = None;
        }
    }

    /// TeX82 §§785/791's shared alignment lookahead fetch.
    ///
    /// TeX82's `get_x_token` commits the terminal expanded command before
    /// `init_col` backs an ordinary command up. The backup is later read
    /// again above its u-template, producing a second raw/expanded delivery.
    /// Spacers skipped by §406 are complete deliveries and are committed here
    /// normally.
    ///
    /// e-TeX 2.6 change sections [37.785] and [37.791] replace that helper
    /// with `get_x_or_protected`. Its terminal unexpandable command comes
    /// straight from `get_token`, so neither skipped spacers nor a consumed
    /// `\noalign`, `\crcr`, `\omit`, or closing brace has an expanded
    /// delivery. A protected macro is likewise terminal and is backed up as
    /// the first command of the next cell.
    pub fn next_alignment_lookahead(
        &mut self,
    ) -> Result<Option<AlignmentLookahead<G>>, CommandError> {
        loop {
            let etex_protected_fetch = self.command.profile().capabilities().supports_etex();
            let mut destination = None;
            let result = if etex_protected_fetch {
                self.etex_alignment_lookahead_next(&mut destination)
            } else {
                self.tex_alignment_lookahead_next(&mut destination)
            };
            let lookahead = match result? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => AlignmentLookahead::Committed(
                    destination.expect("command status initializes destination"),
                ),
                DeliveryStatus::PendingExpanded => AlignmentLookahead::PendingExpanded(
                    destination.expect("pending status initializes destination"),
                ),
                _ => unreachable!("alignment lookahead consumes replay completions"),
            };
            if matches!(
                lookahead.command().meaning(),
                ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                let _ = self.commit_alignment_lookahead_delivery(lookahead);
                continue;
            }
            return Ok(Some(lookahead));
        }
    }

    /// Commits a terminal TeX82 lookahead delivery that alignment control
    /// consumes instead of passing to an ordinary `back_input` branch.
    pub fn commit_alignment_lookahead_delivery(
        &mut self,
        lookahead: AlignmentLookahead<G>,
    ) -> CurrentCommand<G> {
        match lookahead {
            AlignmentLookahead::Committed(command) => command,
            AlignmentLookahead::PendingExpanded(command) => {
                self.observe_expanded_delivery(&command);
                command
            }
        }
    }

    /// Completes TeX82 §§785/791's ordinary `align_peek`/`init_col` branch.
    ///
    /// A command reached through §380's expansion loop is still pending only
    /// in Umber's observer transport. TeX has already completed
    /// `get_x_token`, so its expanded delivery precedes §789's `back_input`;
    /// the later replay above the u-template is a distinct delivery.
    pub fn back_alignment_lookahead(
        &mut self,
        lookahead: AlignmentLookahead<G>,
    ) -> Result<(), CommandError> {
        let command = self.commit_alignment_lookahead_delivery(lookahead);
        self.back_input(command)
    }

    /// Delivers one expanded command or the completion of an executor-owned
    /// stored replay episode.
    ///
    /// Completion is published after the command machine has retired and
    /// observed the exact stored level, but before it resumes the enclosing
    /// source.  Callers must finish the corresponding isolated execution
    /// lifecycle before requesting another delivery.
    pub fn get_x_token_with_replay_completion(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let result = self.get_x_token_with_replay_completion_into(&mut destination)?;
        Ok(match result {
            DeliveryStatus::End => None,
            DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("ordinary replay-aware delivery has no alignment event"),
        })
    }

    /// Delivers replay-aware expanded input into caller-provided storage.
    pub fn get_x_token_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let result = self.expanded_next_with_replay_completion(destination)?;
            match result {
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::End
                | DeliveryStatus::Command
                | DeliveryStatus::ReplayCompleted(_) => return Ok(result),
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("expanded delivery does not own a character consumer")
                }
            }
        }
    }

    /// Delivers main-control preflight through one raw-fetch/classification
    /// loop. An ordinary unexpandable command publishes its canonical expanded
    /// observation directly, without completing a second expanded-driver
    /// episode; a macro, expandable primitive, or undefined command continues
    /// in place through the canonical expanded loop.
    pub fn preflight_command_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let result = self.preflight_next(destination)?;
        debug_assert!(matches!(
            result,
            DeliveryStatus::End | DeliveryStatus::Command | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// Resumes one genuinely suspended expansion from its stable parked root.
    /// The key is the executor's only command-related retry owner; consuming
    /// it moves the command once into `destination` before scalar expansion
    /// continues at the retained typed phase.
    pub fn resume_expansion_into(
        &mut self,
        key: crate::ExpansionWorkKey<G>,
        main_loop: bool,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        self.install_expansion_resume(key);
        if main_loop {
            self.resumed_main_loop_next(destination)
        } else {
            self.resumed_expanded_next(destination)
        }
    }

    /// Delivers one command through TeX82 §1038's `main_loop_lookahead`.
    ///
    /// `main_control`'s inner character loop (§1034) never returns to
    /// `big_switch`'s `get_x_token` between adjacent characters. §1038 fetches
    /// the next command with a bare `get_next` -- "set only `cur_cmd` and
    /// `cur_chr`, for speed" -- and jumps straight back into the loop when
    /// that raw command is `letter`, `other_char`, or `char_given`. Only a
    /// raw command outside that set reaches `x_token`, which is the sole
    /// reason a run of ordinary characters produces one raw delivery each and
    /// no expanded delivery at all.
    ///
    /// `char_num` is deliberately *not* in the raw set: §1038 accepts it only
    /// after `x_token`, because `\char` can be reached by expansion.
    pub fn main_loop_lookahead(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let result = self.main_loop_lookahead_into(&mut destination)?;
        Ok(match result {
            DeliveryStatus::End => None,
            DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("main-loop lookahead has no alignment event"),
        })
    }

    /// Delivers main-loop lookahead into caller-provided command storage.
    pub fn main_loop_lookahead_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let result = self.main_loop_next(destination)?;
            match result {
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::End
                | DeliveryStatus::Command
                | DeliveryStatus::ReplayCompleted(_) => {
                    return Ok(result);
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("main-loop lookahead has no character consumer")
                }
            }
        }
    }

    /// Lends one main-control source step to the direct list admission, then
    /// settles scalar input through the same owner when the borrowed prefix is
    /// unavailable.  The source row is selected once by `main_character_run`;
    /// this entry never probes it and re-enters a second delivery loop.
    pub fn main_loop_source_step_into<C: MainCharacterConsumer<G>>(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        consume: &mut C,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        debug_assert!(!self.is_observed());
        self.main_character_run(destination, consume)
    }

    #[cold]
    #[inline(never)]
    fn resume_expanded_delivery(
        &mut self,
        destination: Option<HotCommand<G>>,
    ) -> Result<ResumedExpandedDelivery<G>, CommandError> {
        let key = self.expansion_resume.take().or_else(|| {
            self.scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion)
                .then(|| {
                    let wrapper = self
                        .scanner_resume
                        .take()
                        .expect("matched expansion wrapper");
                    self.command
                        .scratch
                        .take_expansion_key(wrapper)
                        .expect("live wrapper owns expansion work")
                })
        });
        let mut retained = self
            .command
            .scratch
            .resume_expansion(key.expect("genuine suspension owns expansion work"))
            .map_err(crate::scan_toks::scratch_command_error)?;
        if destination
            .is_some_and(|command| command != HotCommand::from_current_ref(&retained.command))
        {
            if let Some(child) = retained.take_child() {
                self.abort_continuation(child)?;
            }
            return Err(CommandError::input_invariant());
        }
        if let Some(child) = retained.child.take() {
            let (key, child_destination) = child.restore();
            if child_destination != crate::state::PendingExpansionChildDestination::Dispatch {
                return Err(CommandError::input_invariant());
            }
            self.scanner_resume = Some(key);
        }
        self.resumed_expansion = Some(retained.resume);
        let delivery_expanded = retained.delivery_expanded;
        let parent = retained.parent;
        let return_capability = retained.return_capability;
        self.resume_current_command(&retained.command);
        Ok(ResumedExpandedDelivery {
            command: HotCommand::from_current(retained.command),
            delivery_expanded,
            parent,
            return_capability,
        })
    }

    #[cold]
    #[inline(never)]
    fn fail_hot_expanded_delivery(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
        depth: u32,
        failure: CommandError,
    ) -> Result<DeliveryStatus, CommandError> {
        destination.take();
        self.command.transient.active_expansion_depth = depth;
        self.invalidate_delivery_freshness();
        Err(failure)
    }

    #[cold]
    #[inline(never)]
    fn fail_expanded_dispatch(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
        depth: u32,
        failure: CommandError,
    ) -> Result<ExpandedHotDispatch, CommandError> {
        self.fail_hot_expanded_delivery(destination, depth, failure)
            .map(ExpandedHotDispatch::Finished)
    }

    /// Parks a completed `\the` target and its scalar child at the one cold
    /// resource boundary.  The synchronous control itself was popped before
    /// scanning the target (register indexes and font selectors have their
    /// own expanded lookahead), so the compact resume payload carries only
    /// the opener provenance needed to finish rendering after retry.
    #[cold]
    #[inline(never)]
    fn park_the_continuation(
        &mut self,
        command: CurrentCommand<G>,
        opener: OriginId,
        delivery_expanded: bool,
        error: CommandError,
        destination: &mut Option<HotCommand<G>>,
        depth: u32,
    ) -> Result<DeliveryStatus, CommandError> {
        let child = crate::execution_scratch::ChildContinuation::capture(
            &mut self.scanner_resume,
            crate::state::PendingExpansionChildDestination::Dispatch,
        );
        let pending = crate::state::PendingExpansion {
            command,
            resume: crate::state::PendingExpansionResume::The { opener },
            delivery_expanded,
            parent: None,
            return_capability: None,
            child,
        };
        match self.command.scratch.store_expansion_frame(pending) {
            Ok(key) => {
                self.scanner_resume = Some(key);
                self.fail_hot_expanded_delivery(destination, depth, error)
            }
            Err((store_error, mut pending)) => {
                if let Some(child) = pending.take_child()
                    && let Err(failure) = self.abort_continuation(child)
                {
                    return self.fail_hot_expanded_delivery(destination, depth, failure);
                }
                self.fail_hot_expanded_delivery(
                    destination,
                    depth,
                    crate::scan_toks::scratch_command_error(store_error),
                )
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn transition_source_input_frame(
        &mut self,
        resident_index: usize,
        command: &mut Option<HotCommand<G>>,
    ) -> Result<ResidentColdOutcome, CommandError> {
        let command_state = &mut *self.command;
        let state = &mut *self.state;
        let create_control_sequences = self.create_source_control_sequences;
        let profile = command_state.roots.profile;
        let force_eof_requested = command_state.roots.input.force_eof;
        #[cfg(test)]
        {
            command_state
                .roots
                .input
                .levels
                .cursor_mutations
                .source_branch_entries = command_state
                .roots
                .input
                .levels
                .cursor_mutations
                .source_branch_entries
                .saturating_add(1);
        }
        let InputLevel::Source(source) = &mut command_state.roots.input.levels.rows[resident_index]
        else {
            return Err(CommandError::input_invariant());
        };
        let slot = command_state
            .roots
            .input
            .levels
            .source_slots
            .resident_value_mut(source.slot.0.slot);
        let mut top = ResidentSourceTop { source, slot };
        let force_eof = top.force_eof(force_eof_requested);
        let identity = top.source.identity();
        // The source cursor advances the physical-line pointer when it loads
        // a line, so `next_physical_offset` names the following line rather
        // than this token.  Stamp the token's actual pre-advance byte cursor
        // instead; otherwise every token on one source line would share a
        // coordinate and a backed-up stale copy could pass after a later
        // direct delivery.
        let position = top
            .slot
            .cursor
            .line
            .as_ref()
            .map_or(top.slot.cursor.next_physical_offset, |line| {
                line.cursor.byte_cursor
            });
        let active_source = top.source.frame.source_context();

        match top
            .advance(profile, force_eof, state, create_control_sequences)
            .map_err(|()| CommandError::input_invariant())?
        {
            ResidentSourceAdvance::Delivered(word, origin, location) => {
                let direct_source_line = top
                    .slot
                    .cursor
                    .line
                    .as_ref()
                    .map(|line| u32::try_from(line.physical.number()).unwrap_or(u32::MAX));
                command_state.last_diagnostic_location = Some(location);
                #[cfg(test)]
                {
                    command_state.raw_delivery_path_counters.source_direct = command_state
                        .raw_delivery_path_counters
                        .source_direct
                        .saturating_add(1);
                }
                let resolution = if let Some(command) = command.as_mut() {
                    command.write_resolved_delivery(
                        word,
                        origin,
                        identity.0,
                        position,
                        active_source,
                        true,
                        direct_source_line,
                        false,
                        state,
                    )
                } else {
                    let (resolved, resolution) = HotCommand::from_resolved_delivery(
                        word,
                        origin,
                        identity.0,
                        position,
                        active_source,
                        true,
                        direct_source_line,
                        false,
                        state,
                    );
                    command.replace(resolved);
                    resolution
                };
                #[cfg(feature = "profiling")]
                self.fuel.record_raw_delivery(
                    command_state.delivery_mode.scanner_active(),
                    resolution.meaning_lookup(),
                    crate::fuel::RawDeliveryKind::Source,
                );
                Ok(ResidentColdOutcome::Synthetic {
                    literal_catcode: resolution.literal_catcode(),
                })
            }
            ResidentSourceAdvance::InvalidCharacter => self.transition_input_frame(
                InputFrameTransition::Boundary(ResidentBoundary::InvalidCharacter),
                command,
            ),
            ResidentSourceAdvance::NeedLine(identity) => self.transition_input_frame(
                InputFrameTransition::Boundary(ResidentBoundary::NeedLine(identity)),
                command,
            ),
            ResidentSourceAdvance::Exhausted(identity) => self.transition_input_frame(
                InputFrameTransition::Boundary(ResidentBoundary::SourceExhausted(identity)),
                command,
            ),
        }
    }

    /// Performs one source-character step for `main_character_run`.
    ///
    /// An eligible physical line lends its ordinary prefix to `admit` first;
    /// if that admission cannot accept a character, the same selected source
    /// row falls through to the scalar tokenizer without reopening a
    /// processor or selecting a second top row. Stored source rows and every
    /// cold/source boundary remain on the scalar transition below.
    #[cold]
    #[inline(never)]
    fn advance_source_character_step<C: MainCharacterConsumer<G>>(
        &mut self,
        resident_index: usize,
        consume: &mut C,
    ) -> Result<Option<u32>, CommandError> {
        let command_state = &mut *self.command;
        let state = &mut *self.state;
        let fuel = &mut *self.fuel;
        let diagnostic_effects = &mut *self.diagnostic_effects;
        let InputLevel::Source(source) = &mut command_state.roots.input.levels.rows[resident_index]
        else {
            return Err(CommandError::input_invariant());
        };
        let slot = command_state
            .roots
            .input
            .levels
            .source_slots
            .resident_value_mut(source.slot.0.slot);
        let mut top = ResidentSourceTop { source, slot };
        if !command_state.delivery_mode.allows_character_run() {
            return Ok(None);
        }

        if let Some(mut run) = top
            .borrow_character_run(|ch| {
                matches!(state.catcode(ch), Catcode::Letter | Catcode::Other)
            })
            .map_err(|()| CommandError::input_invariant())?
        {
            let available = usize::try_from(fuel.remaining()).unwrap_or(usize::MAX);
            if available == 0 {
                return Err(fuel.charge().expect_err("zero remaining fuel is exhausted"));
            }
            if run.bytes().len() > available {
                run = run.limit_to(available);
            }
            let run_len = run.bytes().len();
            let admission = consume.admit(
                state,
                fuel,
                diagnostic_effects,
                MainCharacterInput::Borrowed(run),
            );
            let count =
                usize::try_from(admission.count()).map_err(|_| CommandError::input_invariant())?;
            if count > run_len {
                return Err(CommandError::input_invariant());
            }
            if count == 0 && !admission.needs_scalar_fallback() {
                // A consumer failure is represented by its surrounding
                // operation error slot. Do not run scalar admission after it:
                // the source cursor and hmode state must remain owned by this
                // same source step until that failure settles.
                return Ok(Some(0));
            }
            if count == 0 {
                // The borrowed probe already identified the first ordinary
                // byte. Admit that boundary directly instead of reopening
                // the source tokenizer on the same row.
                let byte = *run
                    .bytes()
                    .first()
                    .ok_or_else(CommandError::input_invariant)?;
                let ch = char::from(byte);
                let origin = run.origin(0);
                fuel.charge()?;
                let scalar = consume.admit(
                    state,
                    fuel,
                    diagnostic_effects,
                    MainCharacterInput::Scalar { ch, origin },
                );
                if scalar.count() != 1 {
                    // A scalar admission error is retained by the caller's
                    // side-channel outcome. Do not move either source cursor
                    // until the consumer has accepted this byte.
                    return Ok(Some(0));
                }
                top.commit_character_run(1)
                    .map_err(|()| CommandError::input_invariant())?;
                let line = top
                    .slot
                    .cursor
                    .line
                    .as_ref()
                    .expect("a scalar fallback retains its line");
                command_state.last_diagnostic_location = Some(SourceLocation::new(
                    line.physical.source,
                    line.cursor.byte_cursor.saturating_sub(1),
                ));
                #[cfg(feature = "profiling")]
                fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, 1);
                return Ok(Some(1));
            }
            let count = u32::try_from(count).map_err(|_| CommandError::input_invariant())?;
            fuel.charge_run(count)?;
            top.commit_character_run(usize::try_from(count).expect("u32 fits usize"))
                .map_err(|()| CommandError::input_invariant())?;
            let line = top
                .slot
                .cursor
                .line
                .as_ref()
                .expect("a committed source run retains its line");
            command_state.last_diagnostic_location = Some(SourceLocation::new(
                line.physical.source,
                line.cursor.byte_cursor.saturating_sub(1),
            ));
            #[cfg(feature = "profiling")]
            fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, count);
            return Ok(Some(count));
        }

        let run = top
            .advance_character_run(state, |state, ch, origin| {
                fuel.charge()?;
                Ok(consume
                    .admit(
                        state,
                        fuel,
                        diagnostic_effects,
                        MainCharacterInput::Scalar { ch, origin },
                    )
                    .continue_run())
            })
            .map_err(|()| CommandError::input_invariant())?;
        match run {
            ResidentSourceCharacterRun::Unavailable => Ok(None),
            ResidentSourceCharacterRun::Consumed { count } => {
                let line = top
                    .slot
                    .cursor
                    .line
                    .as_ref()
                    .expect("a consumed source run retains its line");
                command_state.last_diagnostic_location = Some(SourceLocation::new(
                    line.physical.source,
                    line.cursor.byte_cursor.saturating_sub(1),
                ));
                #[cfg(feature = "profiling")]
                fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, count);
                Ok(Some(count))
            }
            ResidentSourceCharacterRun::Failed { count, error } => {
                if count != 0 {
                    let line = top
                        .slot
                        .cursor
                        .line
                        .as_ref()
                        .expect("a consumed source prefix retains its line");
                    command_state.last_diagnostic_location = Some(SourceLocation::new(
                        line.physical.source,
                        line.cursor.byte_cursor.saturating_sub(1),
                    ));
                    #[cfg(feature = "profiling")]
                    fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, count);
                }
                Err(error)
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn finish_resident_eof(&mut self) -> Result<ResidentColdOutcome, CommandError> {
        match self.raw_end_restarts() {
            Ok(true) => Ok(ResidentColdOutcome::Retry),
            Ok(false) => Ok(ResidentColdOutcome::Finished(DeliveryStatus::End)),
            Err(failure) => Err(failure),
        }
    }

    #[cold]
    #[inline(never)]
    fn transition_input_frame(
        &mut self,
        transition: InputFrameTransition<G>,
        command: &mut Option<HotCommand<G>>,
    ) -> Result<ResidentColdOutcome, CommandError> {
        self.invalidate_delivery_freshness();
        let cold = match transition {
            InputFrameTransition::Boundary(boundary) => boundary,
            InputFrameTransition::Source { resident_index } => {
                return self.transition_source_input_frame(resident_index, command);
            }
            InputFrameTransition::ResidentExhausted {
                resident_index,
                identity,
            } => {
                let retirement = self
                    .command
                    .finish_resident_exhaustion(
                        resident_index,
                        identity,
                        &mut self.observer,
                        &mut self.immediate_write_retirement,
                    )
                    .map_err(|()| CommandError::input_invariant())?;
                let Some(retirement) = retirement else {
                    return Ok(ResidentColdOutcome::Retry);
                };
                retirement
            }
            InputFrameTransition::Parameter {
                slot,
                arguments,
                active_source,
            } => {
                #[cfg(test)]
                {
                    self.command
                        .raw_delivery_path_counters
                        .out_parameter_interceptions = self
                        .command
                        .raw_delivery_path_counters
                        .out_parameter_interceptions
                        .saturating_add(1);
                }
                self.command
                    .push_resident_parameter_cursor(
                        slot,
                        arguments,
                        active_source,
                        &mut self.observer,
                    )
                    .map_err(|()| CommandError::input_invariant())?;
                return Ok(ResidentColdOutcome::Retry);
            }
        };
        match cold {
            ResidentBoundary::Empty => {
                observe!(
                    self,
                    CommandObservation::Input(InputRecord {
                        transition: InputTransition::Stop,
                        reason: InputReason::Source,
                        source_name: Some(SourceNameClass::Terminal),
                        source: None,
                        level: 0,
                        position: 0,
                    }),
                );
                self.finish_resident_eof()
            }
            ResidentBoundary::InvalidCharacter => {
                self.report_recoverable(
                    INVALID_SOURCE_CHARACTER_DIAGNOSTIC,
                    "Text line contains an invalid character".into(),
                    &[
                        "A funny symbol that I can't read has just been input.",
                        "Continue, and I'll forget that it ever happened.",
                    ],
                );
                Ok(ResidentColdOutcome::Retry)
            }
            ResidentBoundary::NeedLine(identity) => {
                let line = self.acquire_source_line(true)?;
                if line.is_some() {
                    Ok(ResidentColdOutcome::Retry)
                } else if matches!(
                    self.finish_exhausted_source(identity)?,
                    SourceExhaustionStatus::End
                ) {
                    self.finish_resident_eof()
                } else {
                    Ok(ResidentColdOutcome::Retry)
                }
            }
            ResidentBoundary::SourceExhausted(identity) => {
                #[cfg(test)]
                {
                    self.command
                        .raw_delivery_path_counters
                        .cold_source_retirements = self
                        .command
                        .raw_delivery_path_counters
                        .cold_source_retirements
                        .saturating_add(1);
                }
                if matches!(
                    self.finish_exhausted_source(identity)?,
                    SourceExhaustionStatus::End
                ) {
                    self.finish_resident_eof()
                } else {
                    Ok(ResidentColdOutcome::Retry)
                }
            }
            ResidentBoundary::TokenExhausted { identity, .. } => {
                #[cfg(test)]
                {
                    self.command
                        .raw_delivery_path_counters
                        .exhaustion_status_relays = self
                        .command
                        .raw_delivery_path_counters
                        .exhaustion_status_relays
                        .saturating_add(1);
                }
                let Some((index, active_source)) =
                    self.command
                        .input
                        .levels
                        .last()
                        .and_then(|level| match level {
                            level
                                if level
                                    .stored_common()
                                    .is_some_and(|cursor| cursor.identity() == identity) =>
                            {
                                level.stored_common().map(|cursor| {
                                    (
                                        u32::try_from(
                                            level.stored_position().expect("stored row position"),
                                        )
                                        .expect("stored row position fits u32"),
                                        cursor.frame.source_context(),
                                    )
                                })
                            }
                            _ => None,
                        })
                else {
                    return Err(CommandError::input_invariant());
                };
                let handoff = self.retire_input_top(identity)?;
                match handoff {
                    RetirementHandoff::Stop => match self.raw_end_restarts() {
                        Ok(true) => Ok(ResidentColdOutcome::Retry),
                        Ok(false) => Ok(ResidentColdOutcome::Finished(DeliveryStatus::End)),
                        Err(failure) => Err(failure),
                    },
                    RetirementHandoff::Continue => Ok(ResidentColdOutcome::Retry),
                    RetirementHandoff::Completed(episode) => Ok(ResidentColdOutcome::Finished(
                        DeliveryStatus::ReplayCompleted(episode),
                    )),
                    RetirementHandoff::EndV(level) => {
                        let _resolution = if let Some(command) = command.as_mut() {
                            command.write_resolved_delivery(
                                TokenWord::pack(self.state.frozen_end_template_token()),
                                OriginId::UNKNOWN,
                                level.0,
                                u64::from(index),
                                active_source,
                                false,
                                None,
                                false,
                                self.state,
                            )
                        } else {
                            let (resolved, resolution) = HotCommand::from_resolved_delivery(
                                TokenWord::pack(self.state.frozen_end_template_token()),
                                OriginId::UNKNOWN,
                                level.0,
                                u64::from(index),
                                active_source,
                                false,
                                None,
                                false,
                                self.state,
                            );
                            command.replace(resolved);
                            resolution
                        };
                        #[cfg(feature = "profiling")]
                        self.fuel.record_raw_delivery(
                            self.command.delivery_mode.scanner_active(),
                            _resolution.meaning_lookup(),
                            crate::fuel::RawDeliveryKind::SyntheticEndV,
                        );
                        self.readmit_delivery_stamp(
                            command
                                .as_ref()
                                .ok_or_else(CommandError::input_invariant)?
                                .delivery_stamp(),
                        );
                        Ok(ResidentColdOutcome::Synthetic {
                            literal_catcode: _resolution.literal_catcode(),
                        })
                    }
                }
            }
            ResidentBoundary::ReplayCompleted(episode) => Ok(ResidentColdOutcome::Finished(
                DeliveryStatus::ReplayCompleted(episode),
            )),
        }
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Settles the semantic conditions represented by the authoritative
    /// delivery-mode word without widening the ordinary hot loops.
    #[cold]
    #[inline(never)]
    fn settle_exceptional_delivery(
        &mut self,
        command: &mut HotCommand<G>,
    ) -> Result<(), CommandError> {
        let mode = self.command.delivery_mode;
        if mode.suppresses_next() {
            command.suppress_expandable();
        }
        if mode.scanner_active() && mode.outer() {
            let mut rich = command.materialize();
            self.check_outer_validity_entry(&mut rich)?;
            *command = HotCommand::from_current(rich);
        } else if mode.alignment_active()
            && matches!(
                command.alignment_adjustment(),
                crate::processor::AlignmentDeliveryAdjustment::None
            )
        {
            self.command.roots.alignment.classify_delimiter(command);
        }
        if mode.observing() {
            self.observe_resident_hot_command(command);
        }
        Ok(())
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    #[inline(always)]
    fn finish_expanded_command(
        &mut self,
        command: &HotCommand<G>,
        delivery_expanded: bool,
    ) -> DeliveryStatus {
        #[cfg(feature = "profiling")]
        self.record_expanded_delivery();
        if self.is_observed() {
            self.observe_expanded_hot_delivery(command);
        }
        if self
            .command
            .alignment
            .needs_hot_closing_brace_recovery(command)
        {
            DeliveryStatus::AlignmentClosingBrace
        } else if delivery_expanded {
            DeliveryStatus::PendingExpanded
        } else {
            DeliveryStatus::Command
        }
    }

    #[doc(hidden)]
    pub fn observe_expanded_delivery(&mut self, command: &CurrentCommand<G>) {
        observe!(self, {
            #[cfg(test)]
            {}
            let (command_name, command_operand) =
                crate::observation::canonical_current_command_identity_for_profile(
                    self.command.profile(),
                    command,
                );
            let spelling = self.observed_command_spelling(command);
            let semantic_operand = crate::observation::canonical_sparse_register_operand(
                self.command.profile(),
                command.meaning(),
            );
            CommandObservation::Command(CommandDeliveryRecord {
                boundary: CommandDeliveryBoundary::Expanded,
                spelling,
                command: command_name,
                command_operand,
                semantic_operand,
                provenance: CommandProvenance::from_stamp(
                    command.delivery_stamp(),
                    self.current_delivery_sequence(),
                    command.origin(),
                    self.direct_source_provenance(command),
                ),
            })
        });
    }

    /// Compact observation counterpart for the scanner-owned expanded
    /// delivery.  The terminal command remains in the hot slot while its
    /// canonical identity, spelling, and provenance are projected into the
    /// observer record.
    fn observe_expanded_hot_delivery(&mut self, command: &HotCommand<G>) {
        observe!(self, {
            #[cfg(test)]
            {}
            let meaning = command.resolved_meaning();
            let (command_name, command_operand) =
                crate::observation::canonical_delivery_identity_for_profile(
                    self.command.profile(),
                    command.identity(),
                    meaning,
                );
            let spelling = self.observed_hot_command_spelling(command);
            let semantic_operand = crate::observation::canonical_sparse_register_operand(
                self.command.profile(),
                meaning,
            );
            CommandObservation::Command(CommandDeliveryRecord {
                boundary: CommandDeliveryBoundary::Expanded,
                spelling,
                command: command_name,
                command_operand,
                semantic_operand,
                provenance: CommandProvenance::from_stamp(
                    command.delivery_stamp(),
                    self.current_delivery_sequence(),
                    command.origin(),
                    self.direct_source_provenance_hot(command),
                ),
            })
        });
    }

    /// TeX82 §375's ``@<Insert a token containing |frozen_endv|@>``:
    ///
    /// ```text
    /// begin cur_tok:=cs_token_flag+frozen_endv; back_input;
    /// end
    /// ```
    ///
    /// This is §366 `expand`'s entire `end_template` case, and the reason
    /// §780 installs *two* frozen `\endtemplate` control sequences: the one
    /// stored in a template (`frozen_end_template`, command code
    /// `end_template`) is `>outer_call`, so §336's `check_outer_validity`
    /// still catches a template that ends inside an unfinished scan, and only
    /// once it has been delivered is it replaced by `frozen_endv`, whose
    /// command code is the ordinary unexpandable `endv`.
    ///
    /// §325's stack-conservation loop stops at a `v_template` level, so the
    /// exhausted template stays on the stack underneath this backup and
    /// retires only after `endv` has been acted on.
    pub(crate) fn insert_frozen_endv(&mut self) -> Result<(), CommandError> {
        let frozen_endv = self.state.frozen_endv_token();
        self.back_input_token(TracedTokenWord::pack(frozen_endv, OriginId::UNKNOWN))
    }

    fn expand_into_with_parent(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        mut report_trace: bool,
        explicit_parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        let mut parent = explicit_parent;
        let mut admitted_parent = explicit_parent.is_some();
        if self.resumed_expansion.is_none()
            && self.scanner_resume.is_some()
            && !self
                .scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion)
        {
            return Err(CommandError::input_invariant());
        }
        if self.resumed_expansion.is_none()
            && self
                .scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion)
        {
            let wrapper = self
                .scanner_resume
                .take()
                .expect("matched expansion wrapper");
            let key = self
                .command
                .scratch
                .take_expansion_key(wrapper)
                .map_err(crate::scan_toks::scratch_command_error)?;
            let mut retained = self
                .command
                .scratch
                .resume_expansion(key)
                .map_err(crate::scan_toks::scratch_command_error)?;
            if destination.is_some() {
                if let Some(child) = retained.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            if let Some(child) = retained.child.take() {
                let (key, destination) = child.restore();
                if destination != crate::state::PendingExpansionChildDestination::Dispatch {
                    return Err(CommandError::input_invariant());
                }
                self.scanner_resume = Some(key);
            }
            parent = retained.parent;
            admitted_parent = false;
            if let Some(capability) = retained.return_capability {
                if self.scanner_return_capability.replace(capability).is_some() {
                    return Err(CommandError::input_invariant());
                }
            }
            *destination = Some(retained.command);
            self.resumed_expansion = Some(retained.resume);
            self.resume_current_command(
                destination
                    .as_ref()
                    .expect("resumed expansion restores its command destination"),
            );
            report_trace = false;
        }
        if admitted_parent {
            self.command
                .scratch
                .await_expansion_control_for_child(
                    parent.ok_or_else(CommandError::input_invariant)?,
                )
                .map_err(crate::scan_toks::scratch_command_error)?;
        }
        let dispatch = match classify_expanded_command(
            destination
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?,
        ) {
            ExpandedCommandAction::Expand(dispatch) => dispatch,
            // Direct callers implement TeX82 §366 `expand`, where the
            // `end_template` branch inserts frozen `endv`; only §380's
            // expanded-delivery classifier handles it inline.
            ExpandedCommandAction::EndTemplate => {
                ExpansionDispatch::Primitive(ExpandablePrimitive::EndTemplate)
            }
            ExpandedCommandAction::Return => return Err(CommandError::input_invariant()),
        };
        self.expand_classified_into(destination, dispatch, report_trace, false, parent)
    }

    /// Executes the dispatch selected by the expanded-delivery classifier
    /// without wrapping and rediscriminating it at the expansion boundary.
    fn expand_classified_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        dispatch: ExpansionDispatch,
        report_trace: bool,
        delivery_expanded: bool,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
    ) -> Result<(), CommandError> {
        let mut command = destination
            .take()
            .ok_or_else(CommandError::input_invariant)?;
        let mut command_parked = false;
        let result = self.expand_classified_rich_occupied(
            &mut command,
            dispatch,
            report_trace,
            delivery_expanded,
            parent,
            &mut command_parked,
        );
        if !command_parked {
            *destination = Some(command);
        }
        if result.is_ok()
            && let Some(parent) = parent
            && !starts_synchronous_control(dispatch)
        {
            self.command
                .scratch
                .resume_expansion_control_parent(parent)
                .map_err(crate::scan_toks::scratch_command_error)?;
        }
        result
    }

    fn expand_classified_rich_occupied(
        &mut self,
        command: &mut CurrentCommand<G>,
        dispatch: ExpansionDispatch,
        report_trace: bool,
        delivery_expanded: bool,
        parent: Option<crate::expansion_work::ExpansionControlSlot<G>>,
        command_parked: &mut bool,
    ) -> Result<(), CommandError> {
        let resumed_here = self.resumed_expansion.is_some();
        let mut expansion_resume = self
            .resumed_expansion
            .take()
            .unwrap_or(crate::state::PendingExpansionResume::Dispatch);
        if !resumed_here && self.scanner_resume.is_some() {
            return Err(CommandError::input_invariant());
        }
        #[cfg(feature = "profiling")]
        {
            if !is_ranked_fused_expansion(dispatch) {
                tex_state::measurement::record_hot_core_materialization(
                    tex_state::measurement::HotCoreMaterialization::ExpansionCommand,
                );
            }
            match dispatch {
                ExpansionDispatch::Primitive(primitive) => {
                    tex_state::measurement::record_hot_core_expandable_opcode(
                        usize::try_from(primitive.operand())
                            .expect("expandable primitive operand fits usize"),
                    );
                }
                ExpansionDispatch::Macro => {
                    tex_state::measurement::record_hot_core_macro_expansion();
                }
                ExpansionDispatch::Undefined => {}
            }
        }
        #[cfg(feature = "profiling")]
        if self.write_expansion_depth != 0 {
            self.record_write_expansion();
        }
        // TeX82 §367 traces non-macro expandable commands inside `expand`,
        // before the primitive consumes operands or changes the input stack.
        // Undefined control sequences reach the same branch through §370.
        // Macros and `end_template` take §366's other two branches and do not
        // cross this diagnostic boundary.
        let traceable = matches!(
            dispatch,
            ExpansionDispatch::Primitive(primitive)
                if primitive != ExpandablePrimitive::EndTemplate
        ) || dispatch == ExpansionDispatch::Undefined;
        if report_trace && traceable && self.command.delivery_mode.tracing() {
            self.print_command_trace(crate::PrintCommand::from_current(command));
        }
        let mut suspended_resume = None;
        let result = (|| {
            match dispatch {
                ExpansionDispatch::Macro => {
                    let _activated = self.macro_call(command)?;
                    Ok(())
                }
                ExpansionDispatch::Undefined => {
                    #[cfg(feature = "profiling")]
                    tex_state::measurement::record_hot_core_undefined_expansion();
                    let context = self.command.output_open_context(self.state);
                    let site = Some(self.current_diagnostic_site(Some(command)));
                    self.command.semantic_diagnostics.push(
                        crate::CommandSemanticDiagnostic::UndefinedControlSequence {
                            context,
                            site,
                        },
                    );
                    if !self.command.profile().capabilities().supports_etex() {
                        // TeX82 §370 still owns the recoverable user-visible
                        // error above. The pinned e-TeX 2.6 observer has no
                        // diagnostic seam at that error site, so its detached
                        // event stream advances directly to the next input
                        // transition.
                        self.observe_command_diagnostic("undefined_control_sequence", command);
                    }
                    Ok(())
                }
                ExpansionDispatch::Primitive(primitive)
                    if crate::conditionals::ConditionalKind::from_primitive(primitive)
                        .is_some_and(|kind| {
                            kind != crate::conditionals::ConditionalKind::IfCsName
                        }) =>
                {
                    self.expand_conditional(
                        command,
                        false,
                        &mut expansion_resume,
                        &mut suspended_resume,
                    )
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Unless) => self.expand_unless(
                    command,
                    &mut expansion_resume,
                    &mut suspended_resume,
                    parent,
                ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::IfCsName) => {
                    self.begin_ifcsname_continuation_with_parent(false, parent)
                }
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::Else
                    | ExpandablePrimitive::Or
                    | ExpandablePrimitive::Fi),
                ) => self.expand_conditional_delimiter(command, primitive),
                // TeX82 §375's `end_template` case replaces the inaccessible
                // sentinel that ended a v-template with the distinct frozen
                // `endv` token. Neither sentinel is a user-installable primitive;
                // §780 gives them only frozen control-sequence slots.
                ExpansionDispatch::Primitive(ExpandablePrimitive::EndTemplate) => {
                    self.insert_frozen_endv()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::NoExpand) => {
                    self.expand_noexpand()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::ExpandAfter) => self
                    .command
                    .scratch
                    .push_expandafter_control_with_parent(command.origin(), parent)
                    .map_err(crate::scan_toks::scratch_command_error),
                ExpansionDispatch::Primitive(ExpandablePrimitive::CsName) => {
                    self.begin_csname_continuation_with_parent(command.origin(), parent)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::String) => {
                    self.expand_string(command)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Meaning) => {
                    self.expand_meaning(command)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Number) => {
                    self.expand_number(command, false, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::RomanNumeral) => {
                    self.expand_number(command, true, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::The) => {
                    self.begin_the_continuation_with_parent(command.origin(), parent)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Unexpanded) => {
                    self.expand_unexpanded()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Expanded) => {
                    self.expand_expanded()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Detokenize) => {
                    self.expand_detokenize(command)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Scantokens) => {
                    self.expand_scantokens()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::FontName) => self
                    .expand_fontname(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470's `pdf_font_size_code` conversion prints the
                // selected font size as an ordinary scaled dimension.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfFontSize) => self
                    .expand_pdf_font_size(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470 scans e-TeX's extended box-register domain,
                // then queries typed hlist state for the first non-skipable node
                // at the requested edge.
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::LeftMarginKern
                    | ExpandablePrimitive::RightMarginKern),
                ) => self.expand_margin_kern(
                    command.copy_for_backup(),
                    primitive,
                    &mut expansion_resume,
                    &mut suspended_resume,
                ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::Input) => {
                    self.expand_input(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::EndInput) => {
                    self.expand_endinput()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::JobName) => {
                    self.state.unsupported_host_capability();
                    let job_name = self.host.job_name().to_owned();
                    self.push_rendered_text(&job_name, command.origin());
                    Ok(())
                }
                // e-TeX 2.6 etex.ch §3211 installs `\eTeXrevision` as a
                // `convert` command; §1387 prints the immutable revision string
                // through TeX82 §470's ordinary conversion-token path.
                ExpansionDispatch::Primitive(ExpandablePrimitive::ETeXRevision) => {
                    self.push_rendered_text(".6", command.origin());
                    Ok(())
                }
                // pdfTeX §57.4 exposes the revision suffix independently of the
                // integer `\pdftexversion` parameter.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfTeXRevision) => {
                    self.push_rendered_text("27", command.origin());
                    Ok(())
                }
                // pdftex.web §§494 and 496--498 install `\pdftexbanner` as an
                // operand-free `convert`: `conv_toks` prints the process banner,
                // then returns it through the ordinary `str_toks`/`ins_list`
                // conversion path. `utils.c::makepdftexbanner` appends the pinned
                // TeX Live and kpathsea identities to pdftex.web §2's banner.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfTeXBanner) => {
                    self.push_rendered_text(
                    "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) kpathsea version 6.4.2",
                    command.origin(),
                );
                    Ok(())
                }
                // pdftex.web §§1587--1588 use the ordinary integer scanner for
                // the signed uniform bound, then advance the single checkpointed
                // MetaPost-derived stream shared with the operand-free normal
                // deviate conversion.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfUniformDeviate) => self
                    .expand_pdf_uniform_deviate(
                        command,
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfNormalDeviate) => {
                    let value = self.state.pdf_normal_deviate();
                    self.push_rendered_text(&value.to_string(), command.origin());
                    Ok(())
                }
                // pdftex.web §1590's `pdf_creation_date_code` conversion calls
                // `getcreationdate`, then returns the fixed job-start timestamp
                // through the ordinary `str_toks`/`ins_list` conversion path.
                // Both the LaTeX-compatible `\creationdate` spelling and
                // pdfTeX's `\pdfcreationdate` spelling share this meaning.
                ExpansionDispatch::Primitive(ExpandablePrimitive::CreationDate) => {
                    let clock = self.state.job_clock();
                    self.push_rendered_text(&format_pdf_date(clock, 0), command.origin());
                    Ok(())
                }
                // pdfTeX and XeTeX change section [53a] report shell escape as
                // 0 (disabled), 1 (unrestricted), or 2 (restricted). Umber's
                // LaTeX compatibility spelling is an expandable alias over the
                // same tracked World policy used by `\pdfshellescape`.
                ExpansionDispatch::Primitive(ExpandablePrimitive::ShellEscape) => {
                    let status = self
                        .state
                        .internal_integer(tex_state::meaning::InternalInteger::PdfShellEscape)
                        .expect("the shell-escape status is an integer enquiry");
                    self.push_rendered_text(&status.to_string(), command.origin());
                    Ok(())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::StringCompare) => {
                    self.expand_string_compare(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfEscapeString) => {
                    self.expand_pdf_escape_string(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfEscapeHex) => {
                    self.expand_pdf_escape_hex(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfUnescapeHex) => {
                    self.expand_pdf_unescape_hex(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfColorStackInit) => self
                    .expand_pdf_color_stack_init(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfMatch) => self
                    .expand_pdf_match(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfLastMatch) => self
                    .expand_pdf_last_match(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfFileDump) => self
                    .expand_pdf_file_dump(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::FileSize) => self
                    .expand_pdf_file_size(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfFileModificationDate) => self
                    .expand_pdf_file_modification_date(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfMdFiveSum) => self
                    .expand_pdf_md_five_sum(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfInsertHeight) => self
                    .expand_pdf_insert_height(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470's `pdf_ximage_bbox_code` conversion scans an
                // existing image object before its one-based page-box coordinate.
                // The enquiry reads detached metadata only; it never reserves an
                // image or writer object while expanding.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfXImageBBox) => self
                    .expand_pdf_ximage_bbox(command, &mut expansion_resume, &mut suspended_resume),
                // pdftex.web §1549's `pdf_xform_name_code` conversion scans a
                // form object number and prints its independent resource identity.
                // Unknown object numbers produce zero, matching the other PDF
                // object enquiries rather than manufacturing ledger state.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfXFormName) => self
                    .expand_pdf_xform_name(command, &mut expansion_resume, &mut suspended_resume),
                // pdftex.web §470's `pdf_page_ref_code` conversion scans a one-based
                // shipped-page number and prints its page-object identity. Pages
                // that do not exist yet expand to zero without reserving
                // speculative writer state; nonpositive operands are rejected by
                // the conversion's `pdf_error` guard.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfPageRef) => {
                    self.expand_pdf_page_ref(command, &mut expansion_resume, &mut suspended_resume)
                }
                // pdfTeX §57.1 consumes one raw token and, only for a registered
                // primitive spelling, replays the immutable frozen primitive.
                // The ordinary expanded loop then dispatches that original
                // meaning without consulting the shadowable live cell.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfPrimitive) => {
                    let mut destination = None;
                    match self.get_next_into(&mut destination)? {
                        DeliveryStatus::End => return Err(CommandError::input_invariant()),
                        DeliveryStatus::Command => {}
                        _ => unreachable!("ordinary raw delivery returns only commands"),
                    }
                    let target = destination
                        .take()
                        .expect("command status initializes destination");
                    let Some(symbol) = target.control_sequence() else {
                        return Ok(());
                    };
                    let name = self.state.resolve(symbol);
                    let Some(frozen) = self.state.primitive_token(name) else {
                        return Ok(());
                    };
                    self.back_input_token(TracedTokenWord::pack(frozen, target.origin()))
                }
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::TopMark
                    | ExpandablePrimitive::FirstMark
                    | ExpandablePrimitive::BotMark
                    | ExpandablePrimitive::SplitFirstMark
                    | ExpandablePrimitive::SplitBotMark),
                ) => self.expand_mark(primitive),
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::TopMarks
                    | ExpandablePrimitive::FirstMarks
                    | ExpandablePrimitive::BotMarks
                    | ExpandablePrimitive::SplitFirstMarks
                    | ExpandablePrimitive::SplitBotMarks),
                ) => {
                    self.expand_mark_class(primitive, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(primitive) => {
                    Err(CommandError::UnsupportedExpandablePrimitive(primitive))
                }
            }
        })();
        if result
            .as_ref()
            .is_err_and(CommandError::is_resource_suspension)
        {
            let child = crate::execution_scratch::ChildContinuation::capture(
                &mut self.scanner_resume,
                crate::state::PendingExpansionChildDestination::Dispatch,
            );
            let error = result.expect_err("matched resource suspension");
            let suspended_command = std::mem::replace(command, CurrentCommand::empty());
            *command_parked = true;
            let pending = crate::state::PendingExpansion {
                command: suspended_command,
                resume: suspended_resume
                    .take()
                    .unwrap_or(crate::state::PendingExpansionResume::Dispatch),
                delivery_expanded,
                parent,
                return_capability: self.scanner_return_capability.take(),
                child,
            };
            return match self.command.scratch.store_expansion_frame(pending) {
                Ok(key) => {
                    self.scanner_resume = Some(key);
                    Err(error)
                }
                Err((store_error, mut pending)) => {
                    if let Some(child) = pending.take_child()
                        && let Err(failure) = self.abort_continuation(child)
                    {
                        return Err(failure);
                    }
                    Err(crate::scan_toks::scratch_command_error(store_error))
                }
            };
        } else if let Some(child) = self.scanner_resume.take() {
            self.abort_continuation(child)?;
            if result.is_ok() {
                return Err(CommandError::input_invariant());
            }
        }
        result
    }

    /// Dispatch the already-classified macro branch from the occupied hot
    /// owner. This compatibility-shaped entry contains only the compact macro
    /// ABI; primitive and undefined branches have already returned through
    /// their dedicated hot/cold paths above.
    #[inline(always)]
    fn expand_classified_occupied(
        &mut self,
        command: &mut HotCommand<G>,
        dispatch: ExpansionDispatch,
    ) -> Result<(), CommandError> {
        match dispatch {
            ExpansionDispatch::Macro => {}
            ExpansionDispatch::Primitive(_) | ExpansionDispatch::Undefined => {
                return Err(CommandError::input_invariant());
            }
        }
        if self.resumed_expansion.is_some() || self.scanner_resume.is_some() {
            return Err(CommandError::input_invariant());
        }
        #[cfg(feature = "profiling")]
        {
            tex_state::measurement::record_hot_core_macro_expansion();
            if self.write_expansion_depth != 0 {
                self.record_write_expansion();
            }
        }
        let _activated = self.macro_call_hot(command)?;
        Ok(())
    }

    pub(super) fn retain_expansion_scalar<T>(
        &mut self,
        scan: crate::RetainedScalarScan<G, T>,
        phase: crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<T, CommandError> {
        match scan {
            crate::RetainedScalarScan::Complete(value) => Ok(value),
            crate::RetainedScalarScan::Suspended { error, child } => {
                self.install_scanner_resume(Some(child));
                *suspended = Some(phase);
                Err(error)
            }
            crate::RetainedScalarScan::Failed(error) => Err(error),
        }
    }

    /// Creates one invocation provenance node and atomically exposes its
    /// activation/body ownership pair to the input stack.
    ///
    /// The scalar macro matcher owns argument matching and calls this only
    /// after it has completed every range. Nested invocations use the live
    /// activation chain, not a replay trace, as their provenance parent.
    #[allow(dead_code)] // consumed by the ordered scalar macro matcher issue
    pub(crate) fn push_macro_activation(
        &mut self,
        name: tex_state::interner::Symbol,
        body: tex_state::ResidentMacroBody<G>,
        call_site: OriginId,
        arguments: Option<ArgumentSetId<G>>,
    ) -> InputLevelId {
        let invocation = call_site;
        self.invalidate_delivery_freshness();
        self.command
            .push_macro_activation(name, body, arguments, invocation)
    }
}

/// TeX82 §1038's raw-accepted set: `letter`, `other_char`, and `char_given`.
///
/// These are exactly the three commands §1034's inner loop can continue on
/// without expanding, so they are the only ones the lookahead delivers
/// straight out of `get_next`.
/// TeX82 §366's `cur_cmd>max_command` test for Umber's resolved command.
///
/// `Meaning::Undefined` normally represents §207's `undefined_cs` command,
/// which is expanded solely to perform §370's diagnostic recovery. A compact
/// out-parameter token also carries that meaning as its invalid-slot recovery,
/// but its command remains `out_param<max_command`; its token spelling keeps
/// the two command identities distinct here.
pub(crate) fn is_expandable_command<G>(command: &CurrentCommand<G>) -> bool {
    let meaning = command.meaning_ref();
    matches!(meaning, ResolvedMeaning::Macro { .. })
        || matches!(meaning, ResolvedMeaning::Static(Meaning::ExpandablePrimitive(primitive)) if *primitive != ExpandablePrimitive::EndCsName)
        || (matches!(meaning, ResolvedMeaning::Static(Meaning::Undefined))
            && !matches!(command.spelling().semantic_token(), Token::Param(_)))
}

#[cfg(test)]
mod tests;
