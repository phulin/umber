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

bitset! {
    /// Externally visible effects a command can request.
    pub struct EffectCapabilities(u8);
}

impl EffectCapabilities {
    pub const DEFERRED_STREAM: Self = Self(1 << 0);
    pub const STREAM: Self = Self(1 << 1);
    pub const CLOCK: Self = Self(1 << 2);
    pub const RANDOM: Self = Self(1 << 3);
    pub const MAP_UPDATE: Self = Self(1 << 4);
}

bitset! {
    /// Cold output boundaries a command can reach.
    pub struct OutputCapabilities(u8);
}

impl OutputCapabilities {
    pub const DIAGNOSTIC: Self = Self(1 << 0);
    pub const TRANSCRIPT: Self = Self(1 << 1);
    pub const PAGE_ARTIFACT: Self = Self(1 << 2);
    pub const PDF_STATE: Self = Self(1 << 3);
    pub const FORMAT: Self = Self(1 << 4);
    pub const FINAL_JOB: Self = Self(1 << 5);
}

bitset! {
    /// Recovery behavior selected before semantic mutation.
    pub struct RecoveryCapabilities(u8);
}

impl RecoveryCapabilities {
    pub const MAY_SUSPEND: Self = Self(1 << 0);
    pub const LATE_FAILURE: Self = Self(1 << 1);
    pub const TEX_DIAGNOSTIC: Self = Self(1 << 2);
}

/// Stable semantic bucket used by the capability table.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CanonicalCommandFamily {
    Passive,
    Assignment,
    Grouping,
    Material,
    Alignment,
    Math,
    Resource,
    Effect,
    Publication,
    Diagnostic,
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
    projection: HotSnapshotProjection,
}

impl NarrowTransactionSpec {
    #[must_use]
    pub const fn new(owners: StateOwners) -> Self {
        Self {
            projection: HotSnapshotProjection::for_owners(owners),
        }
    }

    #[must_use]
    pub const fn projection(self) -> HotSnapshotProjection {
        self.projection
    }

    /// Admits only the exact snapshot projection named by preflight.
    pub fn admit(
        self,
        supplied: HotSnapshotProjection,
    ) -> Result<NarrowTransaction, PreflightError> {
        if supplied != self.projection {
            return Err(PreflightError::TransactionProjectionMismatch {
                expected: self.projection,
                supplied,
            });
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

/// Complete static classification for one canonical command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandCapabilities {
    family: CanonicalCommandFamily,
    mutation: StateOwners,
    resources: ResourceCapabilities,
    effects: EffectCapabilities,
    output: OutputCapabilities,
    recovery: RecoveryCapabilities,
    transaction: Option<NarrowTransactionSpec>,
}

impl CommandCapabilities {
    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        family: CanonicalCommandFamily,
        mutation: StateOwners,
        resources: ResourceCapabilities,
        effects: EffectCapabilities,
        output: OutputCapabilities,
        recovery: RecoveryCapabilities,
        transaction: Option<NarrowTransactionSpec>,
    ) -> Self {
        Self {
            family,
            mutation,
            resources,
            effects,
            output,
            recovery,
            transaction,
        }
    }

    /// Checked constructor used by cold adapters and protocol tests.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        family: CanonicalCommandFamily,
        mutation: StateOwners,
        resources: ResourceCapabilities,
        effects: EffectCapabilities,
        output: OutputCapabilities,
        recovery: RecoveryCapabilities,
        transaction: Option<NarrowTransactionSpec>,
    ) -> Result<Self, PreflightError> {
        let needs_transaction = recovery.contains(RecoveryCapabilities::MAY_SUSPEND)
            || recovery.contains(RecoveryCapabilities::LATE_FAILURE);
        if resources.is_empty() == recovery.contains(RecoveryCapabilities::MAY_SUSPEND) {
            return Err(PreflightError::ResourceRecoveryMismatch);
        }
        if needs_transaction != transaction.is_some() {
            return Err(PreflightError::RecoveryTransactionMismatch);
        }
        if let Some(spec) = transaction
            && !mutation.contains(spec.projection().owners())
        {
            return Err(PreflightError::TransactionOwnerNotMutable {
                mutation,
                transaction: spec.projection().owners(),
            });
        }
        Ok(Self::from_parts(
            family,
            mutation,
            resources,
            effects,
            output,
            recovery,
            transaction,
        ))
    }

    #[must_use]
    pub const fn family(self) -> CanonicalCommandFamily {
        self.family
    }

    #[must_use]
    pub const fn mutation(self) -> StateOwners {
        self.mutation
    }

    #[must_use]
    pub const fn resources(self) -> ResourceCapabilities {
        self.resources
    }

    #[must_use]
    pub const fn effects(self) -> EffectCapabilities {
        self.effects
    }

    #[must_use]
    pub const fn output(self) -> OutputCapabilities {
        self.output
    }

    #[must_use]
    pub const fn recovery(self) -> RecoveryCapabilities {
        self.recovery
    }

    #[must_use]
    pub const fn transaction(self) -> Option<NarrowTransactionSpec> {
        self.transaction
    }

    /// Performs mutation-free command admission.
    #[must_use]
    pub const fn preflight(self) -> CommandPreflight {
        if self.recovery.contains(RecoveryCapabilities::MAY_SUSPEND) {
            CommandPreflight::Resource(ResourcePreflight { capabilities: self })
        } else if let Some(transaction) = self.transaction {
            CommandPreflight::Transaction(TransactionPreflight {
                capabilities: self,
                transaction,
            })
        } else {
            CommandPreflight::Ordinary(OrdinaryCommand { capabilities: self })
        }
    }
}

/// Mutation-free result of classifying the next command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPreflight {
    /// Direct execution carries capabilities but no transaction object.
    Ordinary(OrdinaryCommand),
    /// Operand scanning may suspend and therefore names its retry projection.
    Resource(ResourcePreflight),
    /// The operation can fail after mutation and needs exact rollback marks.
    Transaction(TransactionPreflight),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrdinaryCommand {
    capabilities: CommandCapabilities,
}

impl OrdinaryCommand {
    #[must_use]
    pub const fn capabilities(self) -> CommandCapabilities {
        self.capabilities
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourcePreflight {
    capabilities: CommandCapabilities,
}

impl ResourcePreflight {
    #[must_use]
    pub const fn capabilities(self) -> CommandCapabilities {
        self.capabilities
    }

    #[must_use]
    pub const fn retry_transaction(self) -> NarrowTransactionSpec {
        match self.capabilities.transaction {
            Some(transaction) => transaction,
            None => unreachable!(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionPreflight {
    capabilities: CommandCapabilities,
    transaction: NarrowTransactionSpec,
}

impl TransactionPreflight {
    #[must_use]
    pub const fn capabilities(self) -> CommandCapabilities {
        self.capabilities
    }

    #[must_use]
    pub const fn transaction(self) -> NarrowTransactionSpec {
        self.transaction
    }
}

/// Rejected static capability or runtime mark admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreflightError {
    ResourceRecoveryMismatch,
    RecoveryTransactionMismatch,
    TransactionOwnerNotMutable {
        mutation: StateOwners,
        transaction: StateOwners,
    },
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

fn ordinary(family: CanonicalCommandFamily, mutation: StateOwners) -> CommandCapabilities {
    CommandCapabilities::from_parts(
        family,
        mutation,
        ResourceCapabilities::NONE,
        EffectCapabilities::NONE,
        OutputCapabilities::NONE,
        RecoveryCapabilities::NONE,
        None,
    )
}

fn diagnostic(mutation: StateOwners) -> CommandCapabilities {
    CommandCapabilities::from_parts(
        CanonicalCommandFamily::Diagnostic,
        mutation,
        ResourceCapabilities::NONE,
        EffectCapabilities::NONE,
        OutputCapabilities::DIAGNOSTIC.union(OutputCapabilities::TRANSCRIPT),
        RecoveryCapabilities::TEX_DIAGNOSTIC,
        None,
    )
}

fn resource(resource: ResourceCapabilities) -> CommandCapabilities {
    CommandCapabilities::from_parts(
        CanonicalCommandFamily::Resource,
        RETRY_SCAN,
        resource,
        EffectCapabilities::NONE,
        OutputCapabilities::NONE,
        RecoveryCapabilities::MAY_SUSPEND,
        Some(NarrowTransactionSpec::new(RETRY_SCAN)),
    )
}

fn deferred_effect(mutation: StateOwners) -> CommandCapabilities {
    CommandCapabilities::from_parts(
        CanonicalCommandFamily::Effect,
        mutation,
        ResourceCapabilities::NONE,
        EffectCapabilities::DEFERRED_STREAM,
        OutputCapabilities::NONE,
        RecoveryCapabilities::NONE,
        None,
    )
}

fn late_effect(effect: EffectCapabilities) -> CommandCapabilities {
    CommandCapabilities::from_parts(
        CanonicalCommandFamily::Effect,
        EFFECT_TRANSACTION,
        ResourceCapabilities::NONE,
        effect,
        OutputCapabilities::TRANSCRIPT,
        RecoveryCapabilities::LATE_FAILURE,
        Some(NarrowTransactionSpec::new(EFFECT_TRANSACTION)),
    )
}

fn late_state(family: CanonicalCommandFamily, mutation: StateOwners) -> CommandCapabilities {
    CommandCapabilities::from_parts(
        family,
        mutation,
        ResourceCapabilities::NONE,
        EffectCapabilities::NONE,
        OutputCapabilities::NONE,
        RecoveryCapabilities::LATE_FAILURE,
        Some(NarrowTransactionSpec::new(mutation)),
    )
}

fn publication(
    mutation: StateOwners,
    output: OutputCapabilities,
    transaction: StateOwners,
) -> CommandCapabilities {
    CommandCapabilities::from_parts(
        CanonicalCommandFamily::Publication,
        mutation,
        ResourceCapabilities::NONE,
        EffectCapabilities::STREAM,
        output,
        RecoveryCapabilities::LATE_FAILURE,
        Some(NarrowTransactionSpec::new(transaction)),
    )
}

/// Classifies every meaning that can reach canonical main control.
#[must_use]
pub fn canonical_command_capabilities<G>(meaning: ResolvedMeaning<G>) -> CommandCapabilities {
    match meaning {
        ResolvedMeaning::Static(meaning) => static_command_capabilities(meaning),
        ResolvedMeaning::Macro { .. } => {
            ordinary(CanonicalCommandFamily::Passive, StateOwners::NONE)
        }
    }
}

/// Classifies one generation-free static command meaning.
#[must_use]
pub fn canonical_static_command_capabilities(meaning: Meaning) -> CommandCapabilities {
    static_command_capabilities(meaning)
}

fn static_command_capabilities(meaning: Meaning) -> CommandCapabilities {
    match meaning {
        Meaning::Undefined | Meaning::Unknown(_) => diagnostic(StateOwners::NONE),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Input) => {
            resource(ResourceCapabilities::INPUT)
        }
        Meaning::Relax | Meaning::ExpandablePrimitive(_) => {
            ordinary(CanonicalCommandFamily::Passive, StateOwners::NONE)
        }
        Meaning::CharGiven(_) | Meaning::CharToken { .. } | Meaning::MathCharGiven(_) => {
            ordinary(CanonicalCommandFamily::Material, MATERIAL)
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
        | Meaning::Font(_) => ordinary(CanonicalCommandFamily::Assignment, STATE),
        Meaning::InternalInteger(_) => diagnostic(StateOwners::NONE),
        Meaning::EndV => ordinary(CanonicalCommandFamily::Alignment, ALIGNMENT),
        Meaning::UnexpandablePrimitive(primitive) => primitive_capabilities(primitive),
    }
}

fn primitive_capabilities(primitive: UnexpandablePrimitive) -> CommandCapabilities {
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
        P::Immediate => late_effect(EffectCapabilities::STREAM),
        P::PdfMapFile | P::PdfMapLine | P::PdfGlyphToUnicode => {
            late_effect(EffectCapabilities::MAP_UPDATE)
        }
        P::PdfResetTimer => late_effect(EffectCapabilities::CLOCK),
        P::PdfSetRandomSeed => late_effect(EffectCapabilities::RANDOM),
        P::Shipout => publication(
            SHIPOUT_TRANSACTION,
            OutputCapabilities::PAGE_ARTIFACT.union(OutputCapabilities::PDF_STATE),
            SHIPOUT_TRANSACTION,
        ),
        P::End => publication(
            JOB_TRANSACTION,
            OutputCapabilities::FINAL_JOB,
            JOB_TRANSACTION,
        ),
        P::Dump => publication(
            JOB_TRANSACTION,
            OutputCapabilities::FORMAT.union(OutputCapabilities::FINAL_JOB),
            JOB_TRANSACTION,
        ),
        P::Show
        | P::ShowBox
        | P::ShowThe
        | P::ShowTokens
        | P::ShowLists
        | P::ShowGroups
        | P::ShowIfs
        | P::Message
        | P::ErrMessage => diagnostic(StateOwners::NONE),
        P::BeginGroup | P::EndGroup => ordinary(CanonicalCommandFamily::Grouping, GROUP),
        P::HAlign | P::VAlign | P::NoAlign | P::Omit | P::Cr | P::CrCr | P::Span => {
            ordinary(CanonicalCommandFamily::Alignment, ALIGNMENT)
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
        | P::ScriptScriptStyle => ordinary(CanonicalCommandFamily::Math, MATH),
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
        | P::QuitVMode => ordinary(CanonicalCommandFamily::Material, MATERIAL),
        P::PdfStartLink => late_state(CanonicalCommandFamily::Material, PDF),
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
        | P::PdfEndThread => ordinary(CanonicalCommandFamily::Material, PDF),
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
        | P::MuToGlue => ordinary(CanonicalCommandFamily::Assignment, STATE),
    }
}

#[cfg(test)]
mod tests;
