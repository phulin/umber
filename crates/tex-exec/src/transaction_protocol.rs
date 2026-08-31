//! Canonical command capabilities and narrow-transaction preflight.
//!
//! This module describes the transaction boundary only. It does not open a
//! `tex_state` snapshot or execute a command. The journal and arena bits mirror
//! the fixed fields of `tex_state`'s `HotSnapshot`; the cutover stages consume
//! these descriptors when borrowing the corresponding marks.

use tex_state::ResolvedMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};

macro_rules! bitset {
    ($(#[$meta:meta])* $visibility:vis struct $name:ident($storage:ty);) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
        $visibility struct $name($storage);

        impl $name {
            pub const NONE: Self = Self(0);

            #[must_use]
            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }

            #[must_use]
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            #[must_use]
            pub const fn union(self, other: Self) -> Self {
                Self(self.0 | other.0)
            }
        }
    };
}

bitset! {
    /// Mutable semantic owners touched by a command or protected by retry.
    pub struct StateOwners(u16);
}

impl StateOwners {
    pub const INPUT: Self = Self(1 << 0);
    pub const PARAMETER: Self = Self(1 << 1);
    pub const CONDITION: Self = Self(1 << 2);
    pub const GROUP: Self = Self(1 << 3);
    pub const SAVE: Self = Self(1 << 4);
    pub const MODE: Self = Self(1 << 5);
    pub const DENSE_STATE: Self = Self(1 << 6);
    pub const PAGE: Self = Self(1 << 7);
    pub const PDF: Self = Self(1 << 8);
    pub const EFFECT: Self = Self(1 << 9);
    pub const OUTPUT: Self = Self(1 << 10);
    pub const SOURCE: Self = Self(1 << 11);
    pub const RESOURCE: Self = Self(1 << 12);
    pub const PROVENANCE: Self = Self(1 << 13);

    /// Exact journal, stack, and arena projection needed to restore these
    /// owners. No call site chooses marks independently of its state owners.
    #[must_use]
    pub const fn required_marks(self) -> HotSnapshotMarks {
        let mut marks = HotSnapshotMarks::NONE;
        if self.contains(Self::INPUT) {
            marks = marks
                .union(HotSnapshotMarks::INPUT_STACK)
                .union(HotSnapshotMarks::TOKEN_ARENA);
        }
        if self.contains(Self::PARAMETER) {
            marks = marks
                .union(HotSnapshotMarks::PARAMETER_STACK)
                .union(HotSnapshotMarks::ARGUMENT_ARENA);
        }
        if self.contains(Self::CONDITION) {
            marks = marks.union(HotSnapshotMarks::CONDITION_STACK);
        }
        if self.contains(Self::GROUP) {
            marks = marks.union(HotSnapshotMarks::GROUP_STACK);
        }
        if self.contains(Self::SAVE) {
            marks = marks.union(HotSnapshotMarks::SAVE_STACK);
        }
        if self.contains(Self::MODE) {
            marks = marks
                .union(HotSnapshotMarks::MODE_STACK)
                .union(HotSnapshotMarks::NODE_ARENA);
        }
        if self.contains(Self::DENSE_STATE) {
            marks = marks.union(HotSnapshotMarks::MUTATION_JOURNAL);
        }
        if self.contains(Self::PAGE) {
            marks = marks
                .union(HotSnapshotMarks::PAGE_JOURNAL)
                .union(HotSnapshotMarks::NODE_ARENA);
        }
        if self.contains(Self::PDF) {
            marks = marks
                .union(HotSnapshotMarks::PDF_JOURNAL)
                .union(HotSnapshotMarks::NODE_ARENA);
        }
        if self.contains(Self::EFFECT) {
            marks = marks.union(HotSnapshotMarks::EFFECT_JOURNAL);
        }
        if self.contains(Self::OUTPUT) {
            marks = marks.union(HotSnapshotMarks::OUTPUT_JOURNAL);
        }
        if self.contains(Self::SOURCE) {
            marks = marks
                .union(HotSnapshotMarks::SOURCE_JOURNAL)
                .union(HotSnapshotMarks::TOKEN_ARENA);
        }
        if self.contains(Self::RESOURCE) {
            marks = marks.union(HotSnapshotMarks::RESOURCE_JOURNAL);
        }
        if self.contains(Self::PROVENANCE) {
            marks = marks.union(HotSnapshotMarks::PROVENANCE_ARENA);
        }
        marks
    }
}

bitset! {
    /// Fixed fields selected from one `HotSnapshot` mark.
    pub struct HotSnapshotMarks(u32);
}

impl HotSnapshotMarks {
    pub const TOKEN_ARENA: Self = Self(1 << 0);
    pub const ARGUMENT_ARENA: Self = Self(1 << 1);
    pub const PROVENANCE_ARENA: Self = Self(1 << 2);
    pub const NODE_ARENA: Self = Self(1 << 3);
    pub const INPUT_STACK: Self = Self(1 << 4);
    pub const PARAMETER_STACK: Self = Self(1 << 5);
    pub const CONDITION_STACK: Self = Self(1 << 6);
    pub const GROUP_STACK: Self = Self(1 << 7);
    pub const SAVE_STACK: Self = Self(1 << 8);
    pub const MODE_STACK: Self = Self(1 << 9);
    pub const MUTATION_JOURNAL: Self = Self(1 << 10);
    pub const PAGE_JOURNAL: Self = Self(1 << 11);
    pub const PDF_JOURNAL: Self = Self(1 << 12);
    pub const EFFECT_JOURNAL: Self = Self(1 << 13);
    pub const OUTPUT_JOURNAL: Self = Self(1 << 14);
    pub const SOURCE_JOURNAL: Self = Self(1 << 15);
    pub const RESOURCE_JOURNAL: Self = Self(1 << 16);
}

bitset! {
    /// Host resources whose absence is established during preflight.
    pub struct ResourceCapabilities(u8);
}

impl ResourceCapabilities {
    pub const INPUT: Self = Self(1 << 0);
    pub const INPUT_PROBE: Self = Self(1 << 1);
    pub const FONT: Self = Self(1 << 2);
    pub const PDF_IMAGE: Self = Self(1 << 3);
    pub const TERMINAL: Self = Self(1 << 4);
}

/// A validated projection of fixed-size `HotSnapshot` fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotSnapshotProjection {
    owners: StateOwners,
    marks: HotSnapshotMarks,
}

impl HotSnapshotProjection {
    /// Builds the one legal mark projection for `owners`.
    #[must_use]
    pub const fn for_owners(owners: StateOwners) -> Self {
        Self {
            owners,
            marks: owners.required_marks(),
        }
    }

    /// Validates a projection supplied by a snapshot adapter.
    pub fn try_new(owners: StateOwners, marks: HotSnapshotMarks) -> Result<Self, PreflightError> {
        let expected = owners.required_marks();
        if marks != expected {
            return Err(PreflightError::InvalidOwnerMarkProjection {
                owners,
                expected,
                supplied: marks,
            });
        }
        Ok(Self { owners, marks })
    }

    #[must_use]
    pub const fn owners(self) -> StateOwners {
        self.owners
    }

    #[must_use]
    pub const fn marks(self) -> HotSnapshotMarks {
        self.marks
    }
}

/// Exact rollback authority for one retryable or late-failing operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NarrowTransactionSpec {
    owners: StateOwners,
}

impl NarrowTransactionSpec {
    #[must_use]
    pub const fn new(owners: StateOwners) -> Self {
        Self { owners }
    }

    #[must_use]
    pub const fn projection(self) -> HotSnapshotProjection {
        HotSnapshotProjection::for_owners(self.owners)
    }

    /// Admits only the exact snapshot projection named by preflight.
    pub fn admit(
        self,
        supplied: HotSnapshotProjection,
    ) -> Result<NarrowTransaction, PreflightError> {
        let expected = self.projection();
        if supplied != expected {
            return Err(PreflightError::TransactionProjectionMismatch { expected, supplied });
        }
        Ok(NarrowTransaction {
            projection: supplied,
        })
    }
}

/// Borrow-scope token proving that the exact narrow marks were admitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NarrowTransaction {
    projection: HotSnapshotProjection,
}

impl NarrowTransaction {
    #[must_use]
    pub const fn projection(self) -> HotSnapshotProjection {
        self.projection
    }
}

/// Mutation-free result of classifying the next command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPreflight {
    /// Direct execution carries only the mutation fact consulted by dynamic
    /// transaction admission.
    Ordinary(OrdinaryCommand),
    /// Operand scanning may suspend and therefore names its retry projection.
    Resource(ResourcePreflight),
    /// The operation can fail after mutation and needs exact rollback marks.
    Transaction(TransactionPreflight),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrdinaryCommand {
    mutation: StateOwners,
}

impl OrdinaryCommand {
    #[must_use]
    pub const fn mutation(self) -> StateOwners {
        self.mutation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourcePreflight {
    resources: ResourceCapabilities,
    retry_transaction: NarrowTransactionSpec,
}

impl ResourcePreflight {
    #[must_use]
    pub const fn resources(self) -> ResourceCapabilities {
        self.resources
    }

    #[must_use]
    pub const fn retry_transaction(self) -> NarrowTransactionSpec {
        self.retry_transaction
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionPreflight {
    transaction: NarrowTransactionSpec,
}

impl TransactionPreflight {
    #[must_use]
    pub const fn transaction(self) -> NarrowTransactionSpec {
        self.transaction
    }
}

/// Rejected static capability or runtime mark admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreflightError {
    InvalidOwnerMarkProjection {
        owners: StateOwners,
        expected: HotSnapshotMarks,
        supplied: HotSnapshotMarks,
    },
    TransactionProjectionMismatch {
        expected: HotSnapshotProjection,
        supplied: HotSnapshotProjection,
    },
}

const STATE: StateOwners = StateOwners::DENSE_STATE.union(StateOwners::SAVE);
const GROUP: StateOwners = STATE.union(StateOwners::GROUP);
const MATERIAL: StateOwners = STATE.union(StateOwners::MODE).union(StateOwners::PAGE);
const ALIGNMENT: StateOwners = MATERIAL
    .union(StateOwners::INPUT)
    .union(StateOwners::PARAMETER)
    .union(StateOwners::CONDITION)
    .union(StateOwners::GROUP);
const MATH: StateOwners = MATERIAL.union(StateOwners::GROUP);
const PDF: StateOwners = MATERIAL.union(StateOwners::PDF);
const RETRY_SCAN: StateOwners = StateOwners::INPUT
    .union(StateOwners::PARAMETER)
    .union(StateOwners::CONDITION)
    .union(StateOwners::SOURCE)
    .union(StateOwners::RESOURCE)
    .union(StateOwners::PROVENANCE);
const EFFECT_TRANSACTION: StateOwners = RETRY_SCAN
    .union(StateOwners::EFFECT)
    .union(StateOwners::OUTPUT);
const SHIPOUT_TRANSACTION: StateOwners = PDF
    .union(StateOwners::EFFECT)
    .union(StateOwners::OUTPUT)
    .union(StateOwners::PROVENANCE);
const JOB_TRANSACTION: StateOwners = MATERIAL
    .union(StateOwners::PDF)
    .union(StateOwners::EFFECT)
    .union(StateOwners::OUTPUT)
    .union(StateOwners::PROVENANCE);

const fn ordinary(mutation: StateOwners) -> CommandPreflight {
    CommandPreflight::Ordinary(OrdinaryCommand { mutation })
}

const fn diagnostic(mutation: StateOwners) -> CommandPreflight {
    ordinary(mutation)
}

const fn resource(resources: ResourceCapabilities) -> CommandPreflight {
    CommandPreflight::Resource(ResourcePreflight {
        resources,
        retry_transaction: NarrowTransactionSpec::new(RETRY_SCAN),
    })
}

const fn deferred_effect(mutation: StateOwners) -> CommandPreflight {
    ordinary(mutation)
}

const fn transaction(owners: StateOwners) -> CommandPreflight {
    CommandPreflight::Transaction(TransactionPreflight {
        transaction: NarrowTransactionSpec::new(owners),
    })
}

const fn late_effect() -> CommandPreflight {
    transaction(EFFECT_TRANSACTION)
}

const fn late_state(mutation: StateOwners) -> CommandPreflight {
    transaction(mutation)
}

/// Classifies every meaning that can reach canonical main control.
#[must_use]
pub fn canonical_command_preflight<G>(meaning: ResolvedMeaning<G>) -> CommandPreflight {
    match meaning {
        ResolvedMeaning::Static(meaning) => static_command_preflight(meaning),
        ResolvedMeaning::Macro { .. } => ordinary(StateOwners::NONE),
    }
}

/// Classifies one generation-free static command meaning.
#[must_use]
pub fn canonical_static_command_preflight(meaning: Meaning) -> CommandPreflight {
    static_command_preflight(meaning)
}

fn static_command_preflight(meaning: Meaning) -> CommandPreflight {
    match meaning {
        Meaning::Undefined | Meaning::Unknown(_) => diagnostic(StateOwners::NONE),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Input) => {
            resource(ResourceCapabilities::INPUT)
        }
        Meaning::Relax | Meaning::ExpandablePrimitive(_) => ordinary(StateOwners::NONE),
        Meaning::CharGiven(_) | Meaning::CharToken { .. } | Meaning::MathCharGiven(_) => {
            ordinary(MATERIAL)
        }
        Meaning::CountRegister(_)
        | Meaning::DimenRegister(_)
        | Meaning::SkipRegister(_)
        | Meaning::MuskipRegister(_)
        | Meaning::ToksRegister(_)
        | Meaning::IntParam(_)
        | Meaning::DimenParam(_)
        | Meaning::GlueParam(_)
        | Meaning::MuGlueParam(_)
        | Meaning::TokParam(_)
        | Meaning::PageDimension(_)
        | Meaning::PageInteger(_)
        | Meaning::Font(_) => ordinary(STATE),
        Meaning::InternalInteger(_) => diagnostic(StateOwners::NONE),
        Meaning::EndV => ordinary(ALIGNMENT),
        Meaning::UnexpandablePrimitive(primitive) => primitive_preflight(primitive),
    }
}

fn primitive_preflight(primitive: UnexpandablePrimitive) -> CommandPreflight {
    use UnexpandablePrimitive as P;

    match primitive {
        P::Font => resource(ResourceCapabilities::FONT),
        P::OpenIn => resource(ResourceCapabilities::INPUT_PROBE),
        P::Read | P::ReadLine => resource(
            ResourceCapabilities::INPUT
                .union(ResourceCapabilities::INPUT_PROBE)
                .union(ResourceCapabilities::TERMINAL),
        ),
        P::PdfXImage => resource(ResourceCapabilities::PDF_IMAGE),
        P::OpenOut | P::CloseOut | P::Write => deferred_effect(MATERIAL),
        P::Immediate => late_effect(),
        P::PdfMapFile | P::PdfMapLine | P::PdfGlyphToUnicode => late_effect(),
        P::PdfResetTimer | P::PdfSetRandomSeed => late_effect(),
        P::Shipout => transaction(SHIPOUT_TRANSACTION),
        P::End | P::Dump => transaction(JOB_TRANSACTION),
        P::Show
        | P::ShowBox
        | P::ShowThe
        | P::ShowTokens
        | P::ShowLists
        | P::ShowGroups
        | P::ShowIfs
        | P::Message
        | P::ErrMessage => diagnostic(StateOwners::NONE),
        P::BeginGroup | P::EndGroup => ordinary(GROUP),
        P::HAlign | P::VAlign | P::NoAlign | P::Omit | P::Cr | P::CrCr | P::Span => {
            ordinary(ALIGNMENT)
        }
        P::MathChar
        | P::Delimiter
        | P::TextFont
        | P::ScriptFont
        | P::ScriptScriptFont
        | P::MathOrd
        | P::MathOp
        | P::MathBin
        | P::MathRel
        | P::MathOpen
        | P::MathClose
        | P::MathPunct
        | P::MathInner
        | P::Underline
        | P::Overline
        | P::Limits
        | P::NoLimits
        | P::DisplayLimits
        | P::Over
        | P::Atop
        | P::Above
        | P::OverWithDelims
        | P::AtopWithDelims
        | P::AboveWithDelims
        | P::Radical
        | P::MathAccent
        | P::VCenter
        | P::MSkip
        | P::MKern
        | P::NonScript
        | P::MathChoice
        | P::Left
        | P::Right
        | P::Middle
        | P::EqNo
        | P::LeftEqNo
        | P::DisplayStyle
        | P::TextStyle
        | P::ScriptStyle
        | P::ScriptScriptStyle => ordinary(MATH),
        P::Par
        | P::Indent
        | P::NoIndent
        | P::HBox
        | P::VBox
        | P::VTop
        | P::Box
        | P::Copy
        | P::VSplit
        | P::UnHBox
        | P::UnHCopy
        | P::UnVBox
        | P::UnVCopy
        | P::LastBox
        | P::Raise
        | P::Lower
        | P::MoveLeft
        | P::MoveRight
        | P::Char
        | P::Kern
        | P::HSkip
        | P::VSkip
        | P::Leaders
        | P::CLeaders
        | P::XLeaders
        | P::HFil
        | P::HFill
        | P::HSs
        | P::HFilNeg
        | P::VFil
        | P::VFill
        | P::VSs
        | P::VFilNeg
        | P::Penalty
        | P::VRule
        | P::HRule
        | P::ControlSpace
        | P::ItalicCorrection
        | P::Discretionary
        | P::DiscretionaryHyphen
        | P::NoBoundary
        | P::Accent
        | P::Mark
        | P::Marks
        | P::VAdjust
        | P::Insert
        | P::UnPenalty
        | P::UnKern
        | P::UnSkip
        | P::Special
        | P::BeginL
        | P::EndL
        | P::BeginR
        | P::EndR
        | P::QuitVMode => ordinary(MATERIAL),
        P::PdfStartLink => late_state(PDF),
        P::PdfLiteral
        | P::PdfSetMatrix
        | P::PdfSave
        | P::PdfRestore
        | P::PdfColorStack
        | P::PdfSavePos
        | P::PdfSnapRefPoint
        | P::PdfSnapY
        | P::PdfSnapYComp
        | P::PdfXForm
        | P::PdfRefXForm
        | P::PdfRefXImage
        | P::PdfObject
        | P::PdfReferenceObject
        | P::PdfInfo
        | P::PdfCatalog
        | P::PdfNames
        | P::PdfTrailer
        | P::PdfTrailerId
        | P::PdfInterwordSpaceOn
        | P::PdfInterwordSpaceOff
        | P::PdfFakeSpace
        | P::PdfSpaceFont
        | P::PdfAnnot
        | P::PdfEndLink
        | P::PdfRunningLinkOn
        | P::PdfRunningLinkOff
        | P::PdfOutline
        | P::PdfDest
        | P::PdfThread
        | P::PdfStartThread
        | P::PdfEndThread => ordinary(PDF),
        P::Def
        | P::Edef
        | P::Gdef
        | P::Xdef
        | P::Let
        | P::FutureLet
        | P::GlobalDefs
        | P::Global
        | P::Long
        | P::Outer
        | P::Protected
        | P::Count
        | P::Dimen
        | P::Skip
        | P::Muskip
        | P::Toks
        | P::CountDef
        | P::DimenDef
        | P::SkipDef
        | P::MuskipDef
        | P::ToksDef
        | P::CharDef
        | P::MathCharDef
        | P::Advance
        | P::Multiply
        | P::Divide
        | P::CatCode
        | P::LcCode
        | P::UcCode
        | P::SfCode
        | P::MathCode
        | P::DelCode
        | P::CloseIn
        | P::FontDimen
        | P::HyphenChar
        | P::SkewChar
        | P::Patterns
        | P::Hyphenation
        | P::ParShape
        | P::PrevDepth
        | P::PrevGraf
        | P::SetBox
        | P::Wd
        | P::Ht
        | P::Dp
        | P::SpaceFactor
        | P::AfterGroup
        | P::AfterAssignment
        | P::Uppercase
        | P::Lowercase
        | P::IgnoreSpaces
        | P::SetLanguage
        | P::PdfLpCode
        | P::PdfRpCode
        | P::PdfEfCode
        | P::PdfTagCode
        | P::PdfKnbsCode
        | P::PdfStbsCode
        | P::PdfShbsCode
        | P::PdfKnbcCode
        | P::PdfKnacCode
        | P::PdfNoLigatures
        | P::LetterspaceFont
        | P::PdfCopyFont
        | P::PdfFontExpand
        | P::PdfFontAttr
        | P::PdfIncludeChars
        | P::PdfNoBuiltinToUnicode
        | P::BatchMode
        | P::NonstopMode
        | P::ScrollMode
        | P::ErrorStopMode
        | P::InteractionMode
        | P::ParShapeLength
        | P::ParShapeIndent
        | P::ParShapeDimen
        | P::InterLinePenalties
        | P::ClubPenalties
        | P::WidowPenalties
        | P::DisplayWidowPenalties
        | P::PageDiscards
        | P::SplitDiscards
        | P::LastPenalty
        | P::LastKern
        | P::LastSkip
        | P::FontCharWd
        | P::FontCharHt
        | P::FontCharDp
        | P::FontCharIc
        | P::NumExpr
        | P::DimExpr
        | P::GlueExpr
        | P::MuExpr
        | P::GlueStretch
        | P::GlueShrink
        | P::GlueStretchOrder
        | P::GlueShrinkOrder
        | P::GlueToMu
        | P::MuToGlue => ordinary(STATE),
    }
}

#[cfg(test)]
mod tests;
