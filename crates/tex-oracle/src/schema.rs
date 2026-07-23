use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Current canonical semantic-event schema.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineDialect {
    Tex82,
    Etex26,
    Pdftex14027,
}

/// Reproducible identity of one canonical reference build.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EngineIdentity {
    pub dialect: EngineDialect,
    pub banner: String,
    pub web_source_sha256: String,
    /// Ordered canonical upstream change files.
    pub upstream_change_sha256: Vec<String>,
    /// Final Umber-owned observation change file.
    pub instrumentation_change_sha256: String,
}

/// One input whose stable logical name may appear in source locations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestInput {
    pub sha256: String,
    pub bytes: u64,
}

/// Complete audit identity for a committed oracle fixture.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub engine: EngineIdentity,
    /// Stable logical names, never host paths.
    pub inputs: BTreeMap<String, ManifestInput>,
    pub environment: BTreeMap<String, String>,
    pub source_date_epoch: i64,
    pub clock: String,
    pub random_seed: i64,
    pub distribution_sha256: String,
    /// Ordinary-output channel to exact byte hash.
    pub ordinary_output_sha256: BTreeMap<String, String>,
}

impl Manifest {
    #[must_use]
    pub fn new(engine: EngineIdentity) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            engine,
            inputs: BTreeMap::new(),
            environment: BTreeMap::new(),
            source_date_epoch: 0,
            clock: String::new(),
            random_seed: 0,
            distribution_sha256: String::new(),
            ordinary_output_sha256: BTreeMap::new(),
        }
    }
}

/// Stable location in a manifest-named source.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    pub source: String,
    pub line: u32,
    pub byte: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OracleToken {
    pub character: u32,
    pub catcode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_sequence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<SourceLocation>,
}

/// Canonical values used by scanner results, mutations, and diagnostics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CanonicalValue {
    None,
    Bool(bool),
    Integer(i64),
    Character(u32),
    Scaled(i64),
    Glue {
        width: i64,
        stretch: i64,
        stretch_order: String,
        shrink: i64,
        shrink_order: String,
    },
    Token(OracleToken),
    Tokens(Vec<OracleToken>),
    Name(String),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalCommand {
    pub command: String,
    pub operand: CanonicalValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_sequence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<SourceLocation>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandDelivery {
    Raw,
    Expanded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandEvent {
    pub delivery: CommandDelivery,
    pub command: CanonicalCommand,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputTransition {
    Push,
    Retire,
    Stop,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputReason {
    Source,
    TokenList,
    Macro,
    AlignmentTemplate,
    Backup,
    Recovery,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputEvent {
    pub transition: InputTransition,
    pub reason: InputReason,
    /// Logical source/control-sequence name or canonical token-list reason.
    pub name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryKind {
    Backup,
    InsertedToken,
    InsertedControlSequence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryEvent {
    pub kind: RecoveryKind,
    pub tokens: Vec<OracleToken>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScannerStatus {
    Normal,
    Skipping,
    Defining,
    Matching,
    Aligning,
    Absorbing,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScannerStatusEvent {
    pub from: ScannerStatus,
    pub to: ScannerStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "transition", rename_all = "snake_case")]
pub enum MacroEvent {
    Argument {
        parameter: u16,
        tokens: Vec<OracleToken>,
    },
    Activation {
        control_sequence: String,
        argument_count: u16,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionTransition {
    Push,
    LimitChange,
    Branch,
    Pop,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionEvent {
    pub transition: ConditionTransition,
    pub condition: String,
    pub limit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScannerEvent {
    pub scanner: String,
    pub result: CanonicalValue,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenListTransition {
    Splice,
    Complete,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenListEvent {
    pub transition: TokenListTransition,
    pub purpose: String,
    pub tokens: Vec<OracleToken>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentTransition {
    Begin,
    Finish,
    Suspend,
    Resume,
    PreambleStart,
    PreambleFinish,
    StateChange,
    TemplatePush,
    TemplateRetire,
    Delimiter,
    BackupCorrection,
    Recovery,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AlignmentEvent {
    pub transition: AlignmentTransition,
    pub align_state: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// One-based semantic alignment nesting, never a reference-engine pointer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nesting: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_align_state: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delimiter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateTarget {
    Meaning,
    Catcode,
    CodeTable,
    Parameter,
    Register,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MutationEvent {
    pub target: StateTarget,
    pub key: CanonicalValue,
    pub value: CanonicalValue,
    pub scope: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Note,
    Warning,
    Error,
    Fatal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticEvent {
    pub severity: DiagnosticSeverity,
    /// Stable semantic diagnostic name, not formatted transcript text.
    pub diagnostic: String,
    pub arguments: Vec<CanonicalValue>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Message,
    Write,
    Open,
    Close,
    Shipout,
    Terminate,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvent {
    pub kind: EffectKind,
    pub channel: String,
    pub value: CanonicalValue,
}

/// One committed semantic delivery or transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum Event {
    Command(CommandEvent),
    Input(InputEvent),
    Recovery(RecoveryEvent),
    ScannerStatus(ScannerStatusEvent),
    Macro(MacroEvent),
    Condition(ConditionEvent),
    Scanner(ScannerEvent),
    TokenList(TokenListEvent),
    Alignment(AlignmentEvent),
    Mutation(MutationEvent),
    Diagnostic(DiagnosticEvent),
    Effect(EffectEvent),
}
