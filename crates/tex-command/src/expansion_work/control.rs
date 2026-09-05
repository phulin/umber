//! Compact typed control vocabulary for parked expansion work.

use crate::attempt::{AttemptMark, AttemptTokenBufferId};
use crate::command::HotCommand;
use crate::scanner_kernel::ScannerCursor;
use tex_state::meaning::Meaning;
use tex_state::token::OriginId;

use super::{ExpansionCommandSlot, ExpansionNameMark};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TraceState {
    Unseen,
    Complete,
    UnlessOperandPending,
}

/// The compact continuation for an expanded `\the` operand.
///
/// `\the` is a particularly important continuation because the operand is
/// itself delivered through the expanded-token loop.  Keeping only the
/// opener's origin here lets that loop consume an arbitrary chain of
/// `\the`/macro expansions without retaining a `CurrentCommand` or entering
/// a second delivery routine.  The target command is always the hot command
/// currently owned by the delivery loop and is materialised only at the
/// scalar scanner boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ThePhase {
    NeedTarget,
    Index {
        target: Meaning,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
    Expression {
        target: Meaning,
        expression: i64,
        expression_sign: i8,
        expression_started: bool,
        term: i64,
        term_operator: u8,
        term_active: bool,
        negative: bool,
        value: i64,
        seen_digit: bool,
        /// A typed internal value has completed its factor before the next
        /// expanded token arrives.  Character digits keep `false` until the
        /// continuation sees their terminating operator; internal values
        /// need this bit so a following sign is an expression operator, not
        /// another leading sign.
        factor_ready: bool,
        /// At least one space followed a completed factor.  A subsequent
        /// operator still belongs to the expression; a digit or other token
        /// terminates the factor and must be backed up instead of appended.
        factor_spaced: bool,
    },
    /// Selector scan for a register used as a `\numexpr` factor.  The
    /// surrounding expression remains in this one compact control record
    /// while the selector consumes expanded signs/digits.
    ExpressionRegisterIndex {
        target: Meaning,
        expression: i64,
        expression_sign: i8,
        expression_started: bool,
        term: i64,
        term_operator: u8,
        term_active: bool,
        outer_negative: bool,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
    DimensionExpression {
        target: Meaning,
        as_number: bool,
        expression: i32,
        expression_sign: i8,
        expression_started: bool,
        term: i32,
        term_operator: u8,
        term_active: bool,
        negative: bool,
        value: i32,
        fraction: i32,
        fraction_digits: u8,
        decimal: bool,
        unit: u8,
        seen_digit: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TheControl {
    pub(crate) opener: OriginId,
    pub(crate) phase: ThePhase,
}

/// Phase of the compact e-TeX `\expanded` collector.
///
/// The balanced body is expanded by the canonical delivery loop itself.  The
/// collector therefore needs only its opening transition and the reusable
/// brace cursor; the attempt-owned token buffer is identified by a typed
/// coordinate rather than retained as a second command/result owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SynchronousExpandedPhase {
    NeedOpening,
    Collecting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SynchronousExpandedKind {
    Expanded,
    Unexpanded,
    Detokenize,
    PdfEscapeString,
    PdfEscapeHex,
    PdfUnescapeHex,
    PdfStringCompareLeft,
    PdfStringCompareRight,
}

/// Copy-small state for one synchronous `\expanded` token collector.
///
/// The output buffer belongs to the enclosing command attempt.  Keeping its
/// coordinate here lets nested expandable operands run in the same delivery
/// loop without a `TokenCollector` or `CurrentCommand` on the Rust stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SynchronousExpandedControl {
    pub(crate) opener: OriginId,
    pub(crate) attempt_opening: AttemptMark,
    pub(crate) writer: AttemptTokenBufferId,
    pub(crate) cursor: ScannerCursor,
    pub(crate) phase: SynchronousExpandedPhase,
    pub(crate) kind: SynchronousExpandedKind,
    pub(crate) left: Option<crate::AttemptTokenListId>,
}

/// Compact synchronous `\csname` state. The accumulated spelling lives in
/// the generation-owned name lane; this record retains only its mark, opener,
/// and the dynamically scoped `ifincsname` value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SynchronousCsNameControl {
    pub(crate) opener: OriginId,
    pub(crate) name: ExpansionNameMark,
    pub(crate) previous_in_csname: bool,
}

/// Compact synchronous `\ifcsname` state. The condition identity and name
/// mark are enough to complete the predicate after the expanded name stream
/// reaches its delimiter; no rich command is retained in this control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SynchronousIfCsNameControl {
    pub(crate) condition: crate::processor::status::ConditionId,
    pub(crate) inverted: bool,
    pub(crate) name: ExpansionNameMark,
    pub(crate) previous_in_csname: bool,
}

/// Phase of the hot `\expandafter` operand protocol.
///
/// The first token is deliberately retained as a compact [`HotCommand`]
/// instead of a `CurrentCommand`.  The second token remains the live command
/// in the one expanded-delivery loop until it settles; this lets nested
/// expandable primitives return to the same loop without a Rust call frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SynchronousExpandAfterPhase {
    NeedFirst,
    NeedSecond,
    AwaitNested,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct SynchronousExpandAfterControl<G> {
    pub(crate) opener: OriginId,
    pub(crate) saved_first: Option<HotCommand<G>>,
    pub(crate) phase: SynchronousExpandAfterPhase,
}

impl<G> Copy for SynchronousExpandAfterControl<G> {}

impl<G> Clone for SynchronousExpandAfterControl<G> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Compact phases for `\if`/`\ifcat`'s two expanded operands. The awaiting
/// states temporarily hide the parent while a nested scanner consumes its
/// own expanded-token requests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SynchronousIfComparePhase {
    NeedFirst,
    AwaitFirst,
    NeedSecond {
        character: u32,
        category: Option<tex_state::token::Catcode>,
    },
    AwaitSecond {
        character: u32,
        category: Option<tex_state::token::Catcode>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SynchronousIfCompareControl {
    pub(crate) condition: crate::processor::status::ConditionId,
    pub(crate) kind: crate::conditionals::ConditionalKind,
    pub(crate) inverted: bool,
    pub(crate) phase: SynchronousIfComparePhase,
}

/// The compact operand state for an `\ifnum`/`\ifdim` comparison.
///
/// The common case (a character constant or a value emitted by another
/// expandable primitive) needs only a saturating accumulator.  Awaiting
/// phases hide this parent while a nested expandable command is being
/// delivered, exactly as the character-comparison control does.  Rich
/// scanner state remains in the cold scalar continuation lane when a token is
/// not part of this hot literal protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SynchronousIfNumberPhase {
    NeedLeft,
    AwaitLeft {
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
    AwaitRelation {
        left: i32,
    },
    Left {
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
    NeedRelation {
        left: i32,
    },
    AwaitRight {
        left: i32,
        relation: crate::conditionals::IfRelation,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
    Right {
        left: i32,
        relation: crate::conditionals::IfRelation,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
    RegisterIndex {
        target: Meaning,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
    RegisterIndexAwait {
        target: Meaning,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
    /// `\iffontchar` first consumes one expanded font identifier.  A family
    /// selector stores the tiny math-size bank while its four-bit index is
    /// being consumed; the selected font is then carried into the character
    /// code phase below.
    FontSelector,
    FontFamily {
        size: u8,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
    FontCharacter {
        font: tex_state::ids::FontId,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
}

/// Compact hot conditional control for numeric/dimension comparisons.  The
/// opener is represented by the condition identity; no `CurrentCommand` or
/// scanner-owned allocation crosses the delivery loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SynchronousIfNumberControl {
    pub(crate) condition: crate::processor::status::ConditionId,
    pub(crate) kind: crate::conditionals::ConditionalKind,
    pub(crate) inverted: bool,
    pub(crate) phase: SynchronousIfNumberPhase,
}

/// Compact literal `\ifdim` operand state.  The hot protocol intentionally
/// handles the common `<integer>[.<fraction>]pt` form; internal dimensions
/// continue through the typed scalar lane at the cold semantic boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SynchronousIfDimensionPhase {
    NeedLeft,
    AwaitLeft {
        negative: bool,
        value: i64,
        fraction: i32,
        fraction_digits: u8,
        decimal: bool,
        unit: u8,
        seen_digit: bool,
    },
    Left {
        negative: bool,
        value: i64,
        fraction: i32,
        fraction_digits: u8,
        decimal: bool,
        unit: u8,
        seen_digit: bool,
    },
    NeedRelation {
        left: i32,
    },
    AwaitRelation {
        left: i32,
    },
    AwaitRight {
        left: i32,
        relation: crate::conditionals::IfRelation,
        negative: bool,
        value: i64,
        fraction: i32,
        fraction_digits: u8,
        decimal: bool,
        unit: u8,
        seen_digit: bool,
    },
    Right {
        left: i32,
        relation: crate::conditionals::IfRelation,
        negative: bool,
        value: i64,
        fraction: i32,
        fraction_digits: u8,
        decimal: bool,
        unit: u8,
        seen_digit: bool,
    },
    RegisterIndex {
        target: Meaning,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
    RegisterIndexAwait {
        target: Meaning,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SynchronousIfDimensionControl {
    pub(crate) condition: crate::processor::status::ConditionId,
    pub(crate) kind: crate::conditionals::ConditionalKind,
    pub(crate) inverted: bool,
    pub(crate) phase: SynchronousIfDimensionPhase,
}

/// Compact scanner state for `\number` and `\romannumeral`.  The rendered
/// result is inserted only after the scalar boundary; while digits are being
/// requested this record retains no rich command or token-list owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SynchronousNumberPhase {
    /// The scanner has not seen a digit or internal quantity yet.  TeX.web
    /// §440 permits spaces and an unbounded sequence of signs in this state;
    /// every `-` toggles the accumulated polarity.
    Need,
    Leading {
        negative: bool,
    },
    Await {
        negative: bool,
        value: i64,
        seen_digit: bool,
        leading: bool,
    },
    Accumulating {
        negative: bool,
        value: i64,
    },
    RegisterIndex {
        target: Meaning,
        /// Polarity preceding the register itself (for example,
        /// `\number-\count0`).  This is distinct from the selector's
        /// intentionally assignment-style sign handling below.
        outer_negative: bool,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
    RegisterIndexAwait {
        target: Meaning,
        outer_negative: bool,
        negative: bool,
        value: i64,
        seen_digit: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SynchronousNumberControl {
    pub(crate) opener: OriginId,
    pub(crate) purpose: SynchronousNumberPurpose,
    pub(crate) phase: SynchronousNumberPhase,
}

/// The scalar integer consumer shared by ordinary number conversions and
/// integer-valued pdfTeX enquiries.  Keeping the operation selector beside
/// the accumulator means a nested enquiry resumes in the delivery lane after
/// its operand rather than re-entering a scanner-owned delivery call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SynchronousNumberPurpose {
    Decimal,
    Roman,
    PdfUniformDeviate,
    PdfMarginKernLeft,
    PdfMarginKernRight,
    PdfInsertHeight,
    PdfXFormName,
    PdfPageRef,
    PdfLastMatch,
    TopMarkClass,
    FirstMarkClass,
    BotMarkClass,
    SplitFirstMarkClass,
    SplitBotMarkClass,
}

/// Compact operand state for `\fontname`.
///
/// Font-name conversion consumes one expanded font identifier.  The only
/// state needed while that token is delivered is the opener provenance; a
/// nested `\fontname` pushes another copy-small record and therefore never
/// retains a rich command on the Rust stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SynchronousFontPurpose {
    /// TeX82 §471's `\fontname` conversion.
    Name,
    /// pdfTeX's `\pdffontsize` conversion.
    Size,
    /// pdfTeX's `\pdffontname` conversion.
    PdfName,
    /// pdfTeX's `\pdffontobjnum` conversion.
    PdfObjectNumber,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SynchronousFontNameControl {
    pub(crate) opener: OriginId,
    pub(crate) purpose: SynchronousFontPurpose,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ExpansionControl<G> {
    Dispatch {
        command: ExpansionCommandSlot<G>,
        trace: TraceState,
    },
    The(TheControl),
    CsName(SynchronousCsNameControl),
    IfCsName(SynchronousIfCsNameControl),
    ExpandAfterSync(SynchronousExpandAfterControl<G>),
    IfCompare(SynchronousIfCompareControl),
    IfNumber(SynchronousIfNumberControl),
    IfDimension(SynchronousIfDimensionControl),
    Number(SynchronousNumberControl),
    FontName(SynchronousFontNameControl),
    Expanded(SynchronousExpandedControl),
}

const _: () = {
    assert!(core::mem::size_of::<SynchronousExpandAfterControl<()>>() <= 128);
    assert!(core::mem::size_of::<TheControl>() <= 96);
    assert!(core::mem::size_of::<SynchronousIfCompareControl>() <= 64);
    assert!(core::mem::size_of::<SynchronousIfNumberControl>() <= 64);
    assert!(core::mem::size_of::<SynchronousIfDimensionControl>() <= 64);
    assert!(core::mem::size_of::<SynchronousNumberControl>() <= 48);
    assert!(core::mem::size_of::<SynchronousFontNameControl>() <= 32);
    assert!(core::mem::size_of::<SynchronousExpandedControl>() <= 128);
};
