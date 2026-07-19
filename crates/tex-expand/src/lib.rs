//! TeX expansion engine core loop.
//!
//! This crate owns the gullet's single `get_x_token` interpreter loop. It
//! reads meanings through the aggregate state facade and pushes expansion
//! output back through `tex-lex` token-list replay frames.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
#[cfg(feature = "profiling-stats")]
use tex_state::World;

use tex_lex::{
    InputSource, InputStack, LexError, MacroArguments, MacroReplaySite, TokenListReplayKind,
    TracedExpansionToken,
};
use tex_state::glue::GlueSpec;
use tex_state::ids::{OriginListId, TokenListId};
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::provenance::{DiagnosticSite, InsertedOriginKind, OriginRecord};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
pub use tex_state::{
    DependencyBank as ReadBank, DependencyCodeTable as ReadCodeTable,
    DependencyEngineField as ReadEngineField, DependencyFontField as ReadFontField,
    DependencyKey as ReadDependency,
};
use tex_state::{
    ExpansionState, FileContent, InputReadState, JobClock, MeaningCacheGuard, Universe,
};

const MEANING_SITE_CACHE_LEN: usize = 64;
pub const PARAGRAPH_SCANTOKENS_BARRIER_DOMAIN: u32 = 0x5053_434e;
pub const PARAGRAPH_INPUT_OPEN_BARRIER_DOMAIN: u32 = 0x5049_4e50;
pub const PARAGRAPH_END_INPUT_BARRIER_DOMAIN: u32 = 0x5045_4e44;

/// Expansion-side operations which prevent a containing paragraph region
/// from being replayed. `\csname` is intentionally absent: the meaning of its
/// constructed symbol is recorded like any other meaning read.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ParagraphExpansionBarrier {
    InputOpen,
    EndInput,
    Scantokens,
}

#[derive(Clone, Copy, Debug)]
struct MeaningSiteCacheEntry {
    site: MacroReplaySite,
    symbol: Symbol,
    guard: MeaningCacheGuard,
    meaning: Meaning,
}

macro_rules! record_dependency {
    ($expansion:expr, $dependency:expr) => {
        $expansion.record_dependency($dependency);
    };
}
pub(crate) use record_dependency;

pub mod args;
pub mod scan;
pub mod scan_dimen;
pub mod scan_glue;
pub mod scan_int;

mod conditionals;
mod dispatch;
mod pdf_files;
mod pdf_random;
mod pdf_regex;
mod pdf_strings;
mod primitives;
mod scan_helpers;
#[cfg(test)]
mod tests;
mod values;

pub use dispatch::{dispatch, dispatch_expandable_opcode, dispatch_with_context};
pub use scan_helpers::scan_optional_keyword_with_context;
pub use values::{
    append_token_show_text, append_token_string_text, meaning_text, scan_the_text_with_context,
    token_text,
};

/// Installs the expandable TeX82 primitives currently implemented by this
/// crate into the provided state facade.
pub fn install_expandable_primitives(stores: &mut Universe) {
    configure_expandable_primitives(stores, true);
}

/// Reconstructs TeX82's immutable primitive lookup table without replacing
/// live meanings restored from a format.
pub fn register_expandable_primitives(stores: &mut Universe) {
    configure_expandable_primitives(stores, false);
}

fn configure_expandable_primitives(stores: &mut Universe, install: bool) {
    for (name, primitive) in [
        (
            "expandafter",
            tex_state::meaning::ExpandablePrimitive::ExpandAfter,
        ),
        (
            "noexpand",
            tex_state::meaning::ExpandablePrimitive::NoExpand,
        ),
        ("csname", tex_state::meaning::ExpandablePrimitive::CsName),
        (
            "endcsname",
            tex_state::meaning::ExpandablePrimitive::EndCsName,
        ),
        ("string", tex_state::meaning::ExpandablePrimitive::String),
        ("number", tex_state::meaning::ExpandablePrimitive::Number),
        (
            "romannumeral",
            tex_state::meaning::ExpandablePrimitive::RomanNumeral,
        ),
        ("meaning", tex_state::meaning::ExpandablePrimitive::Meaning),
        ("the", tex_state::meaning::ExpandablePrimitive::The),
        ("input", tex_state::meaning::ExpandablePrimitive::Input),
        (
            "endinput",
            tex_state::meaning::ExpandablePrimitive::EndInput,
        ),
        ("jobname", tex_state::meaning::ExpandablePrimitive::JobName),
        (
            "fontname",
            tex_state::meaning::ExpandablePrimitive::FontName,
        ),
        ("topmark", tex_state::meaning::ExpandablePrimitive::TopMark),
        (
            "firstmark",
            tex_state::meaning::ExpandablePrimitive::FirstMark,
        ),
        ("botmark", tex_state::meaning::ExpandablePrimitive::BotMark),
        (
            "splitfirstmark",
            tex_state::meaning::ExpandablePrimitive::SplitFirstMark,
        ),
        (
            "splitbotmark",
            tex_state::meaning::ExpandablePrimitive::SplitBotMark,
        ),
        ("iftrue", tex_state::meaning::ExpandablePrimitive::IfTrue),
        ("iffalse", tex_state::meaning::ExpandablePrimitive::IfFalse),
        ("if", tex_state::meaning::ExpandablePrimitive::If),
        ("ifcat", tex_state::meaning::ExpandablePrimitive::IfCat),
        ("ifx", tex_state::meaning::ExpandablePrimitive::IfX),
        ("ifnum", tex_state::meaning::ExpandablePrimitive::IfNum),
        ("ifdim", tex_state::meaning::ExpandablePrimitive::IfDim),
        ("ifodd", tex_state::meaning::ExpandablePrimitive::IfOdd),
        ("ifcase", tex_state::meaning::ExpandablePrimitive::IfCase),
        ("ifvmode", tex_state::meaning::ExpandablePrimitive::IfVMode),
        ("ifhmode", tex_state::meaning::ExpandablePrimitive::IfHMode),
        ("ifmmode", tex_state::meaning::ExpandablePrimitive::IfMMode),
        ("ifinner", tex_state::meaning::ExpandablePrimitive::IfInner),
        ("ifvoid", tex_state::meaning::ExpandablePrimitive::IfVoid),
        ("ifhbox", tex_state::meaning::ExpandablePrimitive::IfHBox),
        ("ifvbox", tex_state::meaning::ExpandablePrimitive::IfVBox),
        ("ifeof", tex_state::meaning::ExpandablePrimitive::IfEof),
        ("else", tex_state::meaning::ExpandablePrimitive::Else),
        ("or", tex_state::meaning::ExpandablePrimitive::Or),
        ("fi", tex_state::meaning::ExpandablePrimitive::Fi),
    ] {
        let meaning = Meaning::ExpandablePrimitive(primitive);
        stores.register_primitive_meaning(name, meaning);
        if install {
            let symbol = stores.intern(name);
            stores.set_meaning(symbol, meaning);
        }
    }
}

/// Installs expandable primitives that exist only in e-TeX extended mode.
pub fn install_etex_expandable_primitives(stores: &mut Universe) {
    stores.set_int_param_global(tex_state::env::banks::IntParam::ETEX_EXTENDED_MODE, 1);
    configure_etex_expandable_primitives(stores, true);
}

/// Reconstructs e-TeX's immutable primitive lookup table after format load.
pub fn register_etex_expandable_primitives(stores: &mut Universe) {
    configure_etex_expandable_primitives(stores, false);
}

fn configure_etex_expandable_primitives(stores: &mut Universe, install: bool) {
    for (name, primitive) in [
        (
            "unexpanded",
            tex_state::meaning::ExpandablePrimitive::Unexpanded,
        ),
        (
            "detokenize",
            tex_state::meaning::ExpandablePrimitive::Detokenize,
        ),
        ("unless", tex_state::meaning::ExpandablePrimitive::Unless),
        (
            "scantokens",
            tex_state::meaning::ExpandablePrimitive::Scantokens,
        ),
        (
            "eTeXrevision",
            tex_state::meaning::ExpandablePrimitive::ETeXRevision,
        ),
        (
            "ifdefined",
            tex_state::meaning::ExpandablePrimitive::IfDefined,
        ),
        (
            "ifcsname",
            tex_state::meaning::ExpandablePrimitive::IfCsName,
        ),
        (
            "ifincsname",
            tex_state::meaning::ExpandablePrimitive::IfInCsName,
        ),
        (
            "iffontchar",
            tex_state::meaning::ExpandablePrimitive::IfFontChar,
        ),
        (
            "topmarks",
            tex_state::meaning::ExpandablePrimitive::TopMarks,
        ),
        (
            "firstmarks",
            tex_state::meaning::ExpandablePrimitive::FirstMarks,
        ),
        (
            "botmarks",
            tex_state::meaning::ExpandablePrimitive::BotMarks,
        ),
        (
            "splitfirstmarks",
            tex_state::meaning::ExpandablePrimitive::SplitFirstMarks,
        ),
        (
            "splitbotmarks",
            tex_state::meaning::ExpandablePrimitive::SplitBotMarks,
        ),
    ] {
        let meaning = Meaning::ExpandablePrimitive(primitive);
        stores.register_primitive_meaning(name, meaning);
        if install {
            let symbol = stores.intern(name);
            stores.set_meaning(symbol, meaning);
        }
    }
    for (name, value) in [
        (
            "eTeXversion",
            tex_state::meaning::InternalInteger::ETeXVersion,
        ),
        (
            "currentgrouplevel",
            tex_state::meaning::InternalInteger::CurrentGroupLevel,
        ),
        (
            "currentgrouptype",
            tex_state::meaning::InternalInteger::CurrentGroupType,
        ),
        (
            "currentiflevel",
            tex_state::meaning::InternalInteger::CurrentIfLevel,
        ),
        (
            "currentiftype",
            tex_state::meaning::InternalInteger::CurrentIfType,
        ),
        (
            "currentifbranch",
            tex_state::meaning::InternalInteger::CurrentIfBranch,
        ),
        (
            "lastnodetype",
            tex_state::meaning::InternalInteger::LastNodeType,
        ),
    ] {
        let meaning = Meaning::InternalInteger(value);
        stores.register_primitive_meaning(name, meaning);
        if install {
            let symbol = stores.intern(name);
            stores.set_meaning(symbol, meaning);
        }
    }
}

/// Installs expandable primitives required by the supported LaTeX engine
/// contract but not provided by e-TeX V2 itself.
pub fn install_latex_expandable_primitives(stores: &mut Universe) {
    configure_latex_expandable_primitives(stores, true);
}

/// Reconstructs the LaTeX compatibility primitive table after format load.
pub fn register_latex_expandable_primitives(stores: &mut Universe) {
    configure_latex_expandable_primitives(stores, false);
}

fn configure_latex_expandable_primitives(stores: &mut Universe, install: bool) {
    for (name, primitive) in [
        (
            "expanded",
            tex_state::meaning::ExpandablePrimitive::Expanded,
        ),
        (
            "filesize",
            tex_state::meaning::ExpandablePrimitive::FileSize,
        ),
        (
            "strcmp",
            tex_state::meaning::ExpandablePrimitive::StringCompare,
        ),
        (
            "shellescape",
            tex_state::meaning::ExpandablePrimitive::ShellEscape,
        ),
        (
            "creationdate",
            tex_state::meaning::ExpandablePrimitive::CreationDate,
        ),
    ] {
        let meaning = Meaning::ExpandablePrimitive(primitive);
        stores.register_primitive_meaning(name, meaning);
        if install {
            let symbol = stores.intern(name);
            stores.set_meaning(symbol, meaning);
        }
    }
}

/// Installs the implemented expandable identity surface of pdfTeX 1.40.27.
///
/// The remaining pdfTeX-layer names are registered by the driver as explicit
/// unsupported placeholders until their owning parity issues replace them.
pub fn install_pdftex_expandable_primitives(stores: &mut Universe) {
    for (name, primitive) in [
        (
            "expanded",
            tex_state::meaning::ExpandablePrimitive::Expanded,
        ),
        (
            "pdftexrevision",
            tex_state::meaning::ExpandablePrimitive::PdfTeXRevision,
        ),
        (
            "pdftexbanner",
            tex_state::meaning::ExpandablePrimitive::PdfTeXBanner,
        ),
        (
            "pdffontsize",
            tex_state::meaning::ExpandablePrimitive::PdfFontSize,
        ),
        (
            "pdffontname",
            tex_state::meaning::ExpandablePrimitive::PdfFontName,
        ),
        (
            "pdffontobjnum",
            tex_state::meaning::ExpandablePrimitive::PdfFontObjectNumber,
        ),
        (
            "leftmarginkern",
            tex_state::meaning::ExpandablePrimitive::LeftMarginKern,
        ),
        (
            "rightmarginkern",
            tex_state::meaning::ExpandablePrimitive::RightMarginKern,
        ),
        (
            "pdfprimitive",
            tex_state::meaning::ExpandablePrimitive::PdfPrimitive,
        ),
        (
            "ifpdfprimitive",
            tex_state::meaning::ExpandablePrimitive::IfPdfPrimitive,
        ),
        (
            "ifpdfabsnum",
            tex_state::meaning::ExpandablePrimitive::IfPdfAbsNum,
        ),
        (
            "ifpdfabsdim",
            tex_state::meaning::ExpandablePrimitive::IfPdfAbsDim,
        ),
        (
            "pdfescapestring",
            tex_state::meaning::ExpandablePrimitive::PdfEscapeString,
        ),
        (
            "pdfescapename",
            tex_state::meaning::ExpandablePrimitive::PdfEscapeName,
        ),
        (
            "pdfescapehex",
            tex_state::meaning::ExpandablePrimitive::PdfEscapeHex,
        ),
        (
            "pdfunescapehex",
            tex_state::meaning::ExpandablePrimitive::PdfUnescapeHex,
        ),
        (
            "pdfstrcmp",
            tex_state::meaning::ExpandablePrimitive::StringCompare,
        ),
        (
            "pdfcreationdate",
            tex_state::meaning::ExpandablePrimitive::CreationDate,
        ),
        (
            "pdffilemoddate",
            tex_state::meaning::ExpandablePrimitive::PdfFileModificationDate,
        ),
        (
            "pdffilesize",
            tex_state::meaning::ExpandablePrimitive::FileSize,
        ),
        (
            "pdfmdfivesum",
            tex_state::meaning::ExpandablePrimitive::PdfMdFiveSum,
        ),
        (
            "pdffiledump",
            tex_state::meaning::ExpandablePrimitive::PdfFileDump,
        ),
        (
            "pdfmatch",
            tex_state::meaning::ExpandablePrimitive::PdfMatch,
        ),
        (
            "pdflastmatch",
            tex_state::meaning::ExpandablePrimitive::PdfLastMatch,
        ),
        (
            "pdfuniformdeviate",
            tex_state::meaning::ExpandablePrimitive::PdfUniformDeviate,
        ),
        (
            "pdfnormaldeviate",
            tex_state::meaning::ExpandablePrimitive::PdfNormalDeviate,
        ),
        (
            "pdfinsertht",
            tex_state::meaning::ExpandablePrimitive::PdfInsertHeight,
        ),
        (
            "pdfximagebbox",
            tex_state::meaning::ExpandablePrimitive::PdfXImageBBox,
        ),
        (
            "pdfcolorstackinit",
            tex_state::meaning::ExpandablePrimitive::PdfColorStackInit,
        ),
        (
            "pdfxformname",
            tex_state::meaning::ExpandablePrimitive::PdfXFormName,
        ),
        (
            "pdfpageref",
            tex_state::meaning::ExpandablePrimitive::PdfPageRef,
        ),
    ] {
        stores.install_primitive_meaning(name, Meaning::ExpandablePrimitive(primitive));
    }
    stores.install_primitive_meaning(
        "pdftexversion",
        Meaning::InternalInteger(tex_state::meaning::InternalInteger::PdfTeXVersion),
    );
    stores.install_primitive_meaning(
        "pdflastobj",
        Meaning::InternalInteger(tex_state::meaning::InternalInteger::PdfLastObject),
    );
    stores.install_primitive_meaning(
        "pdflastxform",
        Meaning::InternalInteger(tex_state::meaning::InternalInteger::PdfLastXForm),
    );
}

/// Records state reads performed by expansion.
///
/// Recorder implementations are erased behind [`ExpansionContext`]. Ordinary
/// runs install no recorder and skip each event with a predictable conditional
/// branch; incremental and diagnostic runs may install any implementation
/// without creating another scanner monomorphization.
pub trait ReadRecorder {
    fn record_meaning(&mut self, symbol: Symbol, _meaning: Meaning) {
        self.record_dependency(ReadDependency::Meaning(symbol.raw()));
    }

    fn record_dependency(&mut self, _dependency: ReadDependency) {}
}

/// Deterministic concrete recorder for memoization and speculation clients.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReadSetRecorder {
    dependencies: std::collections::BTreeSet<ReadDependency>,
}

impl ReadSetRecorder {
    #[must_use]
    pub fn dependencies(&self) -> impl ExactSizeIterator<Item = ReadDependency> + '_ {
        self.dependencies.iter().copied()
    }
}

impl ReadRecorder for ReadSetRecorder {
    fn record_dependency(&mut self, dependency: ReadDependency) {
        self.dependencies.insert(dependency);
    }
}

pub(crate) fn record_code_dependency(
    expansion: &mut ExpansionContext<'_>,
    table: ReadCodeTable,
    ch: char,
) {
    crate::record_dependency!(
        expansion,
        ReadDependency::Code {
            table,
            scalar: ch as u32,
        }
    );
}

/// Why `tex-expand` is replaying a frozen token list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpansionReplayKind {
    MacroBody,
    TheOutput,
    NumberOutput,
    JobName,
    Mark,
    Unexpanded,
    Inserted,
}

impl ExpansionReplayKind {
    #[must_use]
    pub const fn as_lex_kind(self) -> TokenListReplayKind {
        match self {
            Self::MacroBody => TokenListReplayKind::MacroBody,
            Self::TheOutput | Self::NumberOutput | Self::JobName => TokenListReplayKind::Inserted,
            Self::Mark => TokenListReplayKind::Mark,
            Self::Unexpanded => TokenListReplayKind::Unexpanded,
            Self::Inserted => TokenListReplayKind::Inserted,
        }
    }
}

/// Expandable operation families owned by the gullet epic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpandableOpcode {
    Macro,
    ExpandAfter,
    NoExpand,
    CsName,
    EndCsName,
    String,
    Number,
    RomanNumeral,
    Meaning,
    The,
    Expanded,
    FileSize,
    StringCompare,
    ShellEscape,
    CreationDate,
    Unexpanded,
    Detokenize,
    Unless,
    Scantokens,
    ETeXVersion,
    ETeXRevision,
    PdfTeXRevision,
    PdfTeXBanner,
    PdfFontName,
    PdfFontObjectNumber,
    PdfPrimitive,
    IfPdfPrimitive,
    IfPdfAbsNum,
    IfPdfAbsDim,
    PdfEscapeString,
    PdfEscapeName,
    PdfEscapeHex,
    PdfUnescapeHex,
    PdfFileModificationDate,
    PdfMdFiveSum,
    PdfFileDump,
    PdfMatch,
    PdfLastMatch,
    PdfUniformDeviate,
    PdfNormalDeviate,
    PdfInsertHeight,
    PdfXImageBBox,
    PdfColorStackInit,
    PdfXFormName,
    PdfPageRef,
    IfDefined,
    IfCsName,
    IfInCsName,
    IfFontChar,
    Input,
    EndInput,
    JobName,
    FontName,
    Mark,
    If,
    Else,
    Or,
    Fi,
}

/// Current semantic mode as reported by the engine driver.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum EngineMode {
    #[default]
    Vertical,
    Horizontal,
    Math,
}

/// Read-only execution facts needed by expansion-time internal quantities.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EngineStateSnapshot {
    pub mode: EngineMode,
    pub is_inner_mode: bool,
    pub space_factor: i32,
    pub prev_depth: Scaled,
    pub prev_graf: i32,
    pub par_shape_len: i32,
    pub last_penalty: i32,
    pub last_kern: Scaled,
    pub last_skip: GlueSpec,
    pub last_node_type: i32,
}

impl Default for EngineStateSnapshot {
    fn default() -> Self {
        Self {
            mode: EngineMode::Vertical,
            is_inner_mode: false,
            space_factor: 1000,
            prev_depth: Scaled::from_raw(0),
            prev_graf: 0,
            par_shape_len: 0,
            last_penalty: 0,
            last_kern: Scaled::from_raw(0),
            last_skip: GlueSpec::ZERO,
            last_node_type: -1,
        }
    }
}

/// Result of one expansion dispatch.
// Generation-tagged replay handles and transient buffers make push variants
// larger than the direct-delivery cases.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Dispatch {
    Continue,
    Deliver(TracedTokenWord),
    DeliverNoExpand(TracedTokenWord),
    Push {
        replay_kind: ExpansionReplayKind,
        token_list: tex_state::ids::TokenListId,
        origin_list: tex_state::ids::OriginListId,
        macro_arguments: MacroArguments,
        macro_invocation: OriginId,
    },
    PushTransient {
        replay_kind: ExpansionReplayKind,
        tokens: Vec<TracedTokenWord>,
    },
}

/// A TeX error that expansion reports and recovers from without aborting the
/// enclosing scanner. The driver owns presentation of these diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoverableExpansionDiagnostic {
    MacroDoesNotMatchDefinition {
        macro_name: String,
        context: TracedTokenWord,
    },
}

/// Errors raised by `get_x_token`.
#[derive(Debug)]
pub enum ExpandError {
    Captured {
        error: Box<ExpandError>,
        site: DiagnosticSite,
    },
    Lex(LexError),
    MacroCall(args::MacroCallError),
    UnimplementedExpandable {
        opcode: ExpandableOpcode,
        context: TracedTokenWord,
    },
    MissingTokenAfterPrimitive {
        opcode: ExpandableOpcode,
        context: TracedTokenWord,
    },
    MissingEndCsName {
        context: TracedTokenWord,
    },
    MissingInputName {
        context: TracedTokenWord,
    },
    NonCharacterInInputName {
        context: TracedTokenWord,
    },
    InputOpen {
        name: String,
        message: String,
        context: TracedTokenWord,
    },
    UndefinedControlSequence {
        name: String,
        context: TracedTokenWord,
    },
    ScanInt(Box<scan_int::ScanIntError>),
    ScanDimen(Box<scan_dimen::ScanDimenError>),
    ScanGlue(Box<scan_glue::ScanGlueError>),
    ScanGeneralText(Box<scan::ScanToksError>),
    UnsupportedTheTarget {
        context: TracedTokenWord,
    },
    MissingFontIdentifier {
        context: TracedTokenWord,
    },
    MathFamilyOutOfRange {
        value: i32,
        context: TracedTokenWord,
    },
    FontDimenOutOfRange {
        font_name: String,
        number: i32,
        available: u16,
        context: TracedTokenWord,
    },
    MarginKernExpectedHBox {
        context: TracedTokenWord,
    },
    PdfInvalidFontIdentifier {
        context: TracedTokenWord,
    },
    PdfObjectCapacity {
        context: TracedTokenWord,
    },
    PdfExternalImageNotFound {
        object: i32,
        context: TracedTokenWord,
    },
    PdfFormNotFound {
        object: i32,
        context: TracedTokenWord,
    },
    PdfXImageBBoxInvalidParameter {
        index: i32,
        context: TracedTokenWord,
    },
    InvalidConditionalRelation {
        context: TracedTokenWord,
    },
    IncompleteIf {
        context: TracedTokenWord,
    },
    ExtraConditionalControl {
        name: &'static str,
        context: TracedTokenWord,
    },
    ForbiddenOuterTokenInSkippedConditional {
        name: String,
        context: TracedTokenWord,
    },
    ForbiddenOuterTokenInAlignment {
        context: TracedTokenWord,
    },
    ExpansionWorkLimitExceeded {
        limit: u64,
    },
}

impl fmt::Display for ExpandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Captured { error, .. } => write!(f, "{error}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::MacroCall(err) => write!(f, "{err}"),
            Self::UnimplementedExpandable { opcode, .. } => {
                write!(f, "expandable opcode {opcode:?} is not implemented yet")
            }
            Self::MissingTokenAfterPrimitive { opcode, .. } => {
                write!(f, "missing token after expandable primitive {opcode:?}")
            }
            Self::MissingEndCsName { .. } => write!(f, "missing \\endcsname for \\csname"),
            Self::MissingInputName { .. } => write!(f, "missing file name after \\input"),
            Self::NonCharacterInInputName { context } => {
                write!(
                    f,
                    "non-character token {:?} while scanning \\input file name",
                    semantic_token(*context)
                )
            }
            Self::InputOpen { name, message, .. } => {
                write!(f, "failed to open input {name:?}: {message}")
            }
            Self::UndefinedControlSequence { name, .. } => {
                write!(f, "Undefined control sequence \\{name}")
            }
            Self::ScanInt(err) => write!(f, "{err}"),
            Self::ScanDimen(err) => write!(f, "{err}"),
            Self::ScanGlue(err) => write!(f, "{err}"),
            Self::ScanGeneralText(err) => write!(f, "{err}"),
            Self::UnsupportedTheTarget { context } => {
                write!(
                    f,
                    "unsupported token {:?} after \\the",
                    semantic_token(*context)
                )
            }
            Self::MissingFontIdentifier { context } => write!(
                f,
                "missing font identifier at token {:?}",
                semantic_token(*context)
            ),
            Self::MathFamilyOutOfRange { .. } => f.write_str("Bad number"),
            Self::FontDimenOutOfRange {
                font_name,
                available,
                ..
            } => write!(
                f,
                "Font \\{font_name} has only {available} fontdimen parameters"
            ),
            Self::MarginKernExpectedHBox { .. } => {
                f.write_str("pdfTeX error (marginkern): a non-empty hbox expected")
            }
            Self::PdfInvalidFontIdentifier { .. } => {
                f.write_str("pdfTeX error (font): invalid font identifier.")
            }
            Self::PdfObjectCapacity { .. } => {
                f.write_str("pdfTeX error (font): too many PDF objects.")
            }
            Self::PdfExternalImageNotFound { .. } => {
                f.write_str("pdfTeX error (ext1): cannot find referenced object.")
            }
            Self::PdfFormNotFound { .. } => {
                f.write_str("pdfTeX error (ext1): cannot find referenced object.")
            }
            Self::PdfXImageBBoxInvalidParameter { .. } => {
                f.write_str("pdfTeX error (pdfximagebbox): invalid parameter.")
            }
            Self::InvalidConditionalRelation { context } => {
                write!(
                    f,
                    "invalid conditional relation token {:?}",
                    semantic_token(*context)
                )
            }
            Self::IncompleteIf { .. } => {
                write!(f, "Incomplete \\if; all text was ignored after line")
            }
            Self::ExtraConditionalControl { name, .. } => write!(f, "Extra \\{name}"),
            Self::ForbiddenOuterTokenInSkippedConditional { name, .. } => {
                write!(
                    f,
                    "Forbidden control sequence found while scanning conditional text: {name}"
                )
            }
            Self::ForbiddenOuterTokenInAlignment { .. } => {
                f.write_str("Forbidden control sequence found while scanning an alignment")
            }
            Self::ExpansionWorkLimitExceeded { limit } => {
                write!(f, "expansion work limit of {limit} steps exceeded")
            }
        }
    }
}

impl std::error::Error for ExpandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Captured { error, .. } => Some(error),
            Self::Lex(err) => Some(err),
            Self::MacroCall(err) => Some(err),
            Self::ScanInt(err) => Some(err),
            Self::ScanDimen(err) => Some(err),
            Self::ScanGlue(err) => Some(err),
            Self::ScanGeneralText(err) => Some(err),
            Self::UnimplementedExpandable { .. }
            | Self::MissingTokenAfterPrimitive { .. }
            | Self::MissingEndCsName { .. }
            | Self::MissingInputName { .. }
            | Self::NonCharacterInInputName { .. }
            | Self::InputOpen { .. }
            | Self::UndefinedControlSequence { .. }
            | Self::UnsupportedTheTarget { .. }
            | Self::MissingFontIdentifier { .. }
            | Self::MathFamilyOutOfRange { .. }
            | Self::FontDimenOutOfRange { .. }
            | Self::MarginKernExpectedHBox { .. }
            | Self::PdfInvalidFontIdentifier { .. }
            | Self::PdfObjectCapacity { .. }
            | Self::PdfExternalImageNotFound { .. }
            | Self::PdfFormNotFound { .. }
            | Self::PdfXImageBBoxInvalidParameter { .. }
            | Self::InvalidConditionalRelation { .. }
            | Self::IncompleteIf { .. }
            | Self::ExtraConditionalControl { .. }
            | Self::ForbiddenOuterTokenInSkippedConditional { .. }
            | Self::ForbiddenOuterTokenInAlignment { .. }
            | Self::ExpansionWorkLimitExceeded { .. } => None,
        }
    }
}

impl ExpandError {
    #[must_use]
    pub fn primary_origin(&self) -> Option<OriginId> {
        match self {
            Self::Captured { site, .. } => site.primary_origin(),
            Self::UnimplementedExpandable { context, .. }
            | Self::MissingTokenAfterPrimitive { context, .. }
            | Self::MissingEndCsName { context }
            | Self::MissingInputName { context }
            | Self::InputOpen { context, .. }
            | Self::UndefinedControlSequence { context, .. }
            | Self::ExtraConditionalControl { context, .. }
            | Self::ForbiddenOuterTokenInSkippedConditional { context, .. } => {
                Some(context.origin())
            }
            Self::ForbiddenOuterTokenInAlignment { context } => Some(context.origin()),
            Self::NonCharacterInInputName { context }
            | Self::UnsupportedTheTarget { context }
            | Self::MissingFontIdentifier { context }
            | Self::MathFamilyOutOfRange { context, .. }
            | Self::FontDimenOutOfRange { context, .. }
            | Self::MarginKernExpectedHBox { context }
            | Self::PdfInvalidFontIdentifier { context }
            | Self::PdfObjectCapacity { context }
            | Self::PdfExternalImageNotFound { context, .. }
            | Self::PdfFormNotFound { context, .. }
            | Self::PdfXImageBBoxInvalidParameter { context, .. }
            | Self::InvalidConditionalRelation { context }
            | Self::IncompleteIf { context } => Some(context.origin()),
            Self::ScanInt(err) => err.primary_origin(),
            Self::ScanDimen(err) => err.primary_origin(),
            Self::ScanGlue(err) => err.primary_origin(),
            Self::ScanGeneralText(err) => err.primary_origin(),
            Self::MacroCall(err) => err.primary_origin(),
            Self::Lex(err) => err.diagnostic_site().primary_origin(),
            Self::ExpansionWorkLimitExceeded { .. } => None,
        }
    }

    #[must_use]
    pub fn diagnostic_site(&self) -> DiagnosticSite {
        match self {
            Self::Captured { site, .. } => site.clone(),
            Self::Lex(err) => err.diagnostic_site().clone(),
            _ => DiagnosticSite::new(self.primary_origin(), [], None),
        }
    }

    #[cold]
    #[inline(never)]
    fn capture(self, input: &InputStack) -> Self {
        if matches!(self, Self::Captured { .. }) {
            return self;
        }
        let site = input.diagnostic_site(self.primary_origin(), []);
        if site.expansion_head().is_none() {
            self
        } else {
            Self::Captured {
                error: Box::new(self),
                site,
            }
        }
    }
}

/// Object-safe host boundary for expansion-time input access and enquiries.
pub trait InputResolver {
    fn open_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        request_index: u64,
    ) -> Result<Box<dyn InputSource>, String>;

    /// Resolves an input and returns its byte size.
    ///
    /// Resolvers with a meaningful not-found state should override this and
    /// return `Ok(None)`. The default preserves retry behavior for virtual
    /// resolvers by delegating to `open_input`.
    fn input_file_size(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        request_index: u64,
    ) -> Result<Option<u64>, String> {
        let source = self.open_input(input, name, request_index)?;
        Ok(source
            .source_descriptor()
            .map(|descriptor| descriptor.byte_len()))
    }

    /// Resolves immutable file bytes and metadata for read-only enquiries.
    fn input_file_content(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        request_index: u64,
    ) -> Result<Option<FileContent>, String> {
        self.open_stream_input(input, name, request_index)
    }

    /// Resolves content for a TeX input stream such as `\openin`.
    ///
    /// Missing streams are not fatal in TeX, so the default converts any
    /// direct World read failure into `Ok(None)`.
    fn open_stream_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        _request_index: u64,
    ) -> Result<Option<FileContent>, String> {
        Ok(input.read_input_file(Path::new(name)).ok())
    }
}

/// Plain session data available to top-level expansion dispatch.
///
/// Scanners use one concrete input stack and state facade. Host input policy
/// and optional read recording are erased behind object-safe interfaces;
/// input policy is invoked only by primitives that explicitly access files.
pub struct ExpansionContext<'a> {
    pub engine: EngineStateSnapshot,
    pub job_name: &'a str,
    pub job_clock: JobClock,
    input_resolver: Option<&'a mut dyn InputResolver>,
    pub(crate) recorder: Option<&'a mut dyn ReadRecorder>,
    resolution_index: u64,
    meaning_site_cache: Box<[Option<MeaningSiteCacheEntry>; MEANING_SITE_CACHE_LEN]>,
    last_macro_replay_site: Option<MacroReplaySite>,
    csname_depth: u32,
    recoverable_diagnostics: Vec<RecoverableExpansionDiagnostic>,
    fuel_limit: u64,
    remaining_fuel: u64,
    fuel_scope_depth: u32,
    // Meanings use generation-marked dense deduplication below. The remaining
    // read kinds stay append-only here and are sorted once at publication.
    paragraph_reads: Option<Vec<ReadDependency>>,
    paragraph_read_tracking: bool,
    paragraph_meanings: Vec<u32>,
    paragraph_meaning_marks: Vec<u32>,
    /// Locally supplied meanings whose first paragraph read follows their
    /// definition. Reads remain source-proven until the defining group exits.
    paragraph_local_meanings: Vec<(u32, u32)>,
    paragraph_recording_generation: u32,
    paragraph_barriers: BTreeSet<ParagraphExpansionBarrier>,
}

/// Default number of expansion-loop steps available to one expansion request.
pub const DEFAULT_EXPANSION_FUEL: u64 = 250_000;

impl<'a> ExpansionContext<'a> {
    #[must_use]
    pub fn new(job_name: &'a str) -> Self {
        Self {
            engine: EngineStateSnapshot::default(),
            job_name,
            job_clock: JobClock::DEFAULT,
            input_resolver: None,
            recorder: None,
            resolution_index: 0,
            meaning_site_cache: Box::new([None; MEANING_SITE_CACHE_LEN]),
            last_macro_replay_site: None,
            csname_depth: 0,
            recoverable_diagnostics: Vec::new(),
            fuel_limit: DEFAULT_EXPANSION_FUEL,
            remaining_fuel: DEFAULT_EXPANSION_FUEL,
            fuel_scope_depth: 0,
            paragraph_reads: None,
            paragraph_read_tracking: false,
            paragraph_meanings: Vec::new(),
            paragraph_meaning_marks: Vec::new(),
            paragraph_local_meanings: Vec::new(),
            paragraph_recording_generation: 0,
            paragraph_barriers: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn with_input_resolver(
        job_name: &'a str,
        input_resolver: &'a mut dyn InputResolver,
    ) -> Self {
        Self {
            engine: EngineStateSnapshot::default(),
            job_name,
            job_clock: JobClock::DEFAULT,
            input_resolver: Some(input_resolver),
            recorder: None,
            resolution_index: 0,
            meaning_site_cache: Box::new([None; MEANING_SITE_CACHE_LEN]),
            last_macro_replay_site: None,
            csname_depth: 0,
            recoverable_diagnostics: Vec::new(),
            fuel_limit: DEFAULT_EXPANSION_FUEL,
            remaining_fuel: DEFAULT_EXPANSION_FUEL,
            fuel_scope_depth: 0,
            paragraph_reads: None,
            paragraph_read_tracking: false,
            paragraph_meanings: Vec::new(),
            paragraph_meaning_marks: Vec::new(),
            paragraph_local_meanings: Vec::new(),
            paragraph_recording_generation: 0,
            paragraph_barriers: BTreeSet::new(),
        }
    }

    /// Replaces the expansion work budget for this session.
    #[must_use]
    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.fuel_limit = fuel;
        self.remaining_fuel = fuel;
        self
    }

    #[inline(always)]
    fn burn_fuel(&mut self) -> Result<(), ExpandError> {
        if self.remaining_fuel == 0 {
            return expansion_work_limit_exceeded(self.fuel_limit);
        }
        self.remaining_fuel -= 1;
        Ok(())
    }

    fn begin_fuel_scope(&mut self) {
        if self.fuel_scope_depth == 0 {
            self.remaining_fuel = self.fuel_limit;
        }
        self.fuel_scope_depth = self
            .fuel_scope_depth
            .checked_add(1)
            .expect("expansion fuel scope depth overflowed");
    }

    fn end_fuel_scope(&mut self) {
        self.fuel_scope_depth = self
            .fuel_scope_depth
            .checked_sub(1)
            .expect("expansion fuel scope depth underflowed");
    }

    /// Installs an erased read recorder for this expansion session.
    #[must_use]
    pub fn recording(mut self, recorder: &'a mut dyn ReadRecorder) -> Self {
        self.recorder = Some(recorder);
        self
    }

    #[doc(hidden)]
    pub fn begin_paragraph_recording(&mut self) {
        debug_assert!(self.paragraph_reads.is_none());
        self.paragraph_reads = Some(Vec::new());
        self.paragraph_read_tracking = true;
        self.paragraph_meanings.clear();
        self.paragraph_local_meanings.clear();
        self.paragraph_recording_generation = self.paragraph_recording_generation.wrapping_add(1);
        if self.paragraph_recording_generation == 0 {
            self.paragraph_meaning_marks.fill(0);
            self.paragraph_recording_generation = 1;
        }
        self.paragraph_barriers.clear();
    }

    #[doc(hidden)]
    pub fn finish_paragraph_recording(
        &mut self,
    ) -> (Vec<ReadDependency>, Vec<ParagraphExpansionBarrier>) {
        let mut reads = self.paragraph_reads.take().unwrap_or_default();
        self.paragraph_read_tracking = false;
        self.paragraph_local_meanings.clear();
        reads.extend(
            self.paragraph_meanings
                .drain(..)
                .map(ReadDependency::Meaning),
        );
        reads.sort_unstable();
        reads.dedup();
        let barriers = std::mem::take(&mut self.paragraph_barriers)
            .into_iter()
            .collect();
        (reads, barriers)
    }

    pub(crate) fn mark_paragraph_barrier(&mut self, barrier: ParagraphExpansionBarrier) {
        if self.paragraph_reads.is_some() {
            self.paragraph_barriers.insert(barrier);
            self.stop_paragraph_read_tracking();
        }
    }

    #[doc(hidden)]
    pub fn stop_paragraph_read_tracking(&mut self) {
        self.paragraph_read_tracking = false;
        if let Some(reads) = &mut self.paragraph_reads {
            reads.clear();
        }
        self.paragraph_meanings.clear();
        self.paragraph_local_meanings.clear();
    }

    #[doc(hidden)]
    pub fn mark_paragraph_local_meaning(&mut self, symbol: Symbol, group_depth: u32) {
        if !self.paragraph_read_tracking {
            return;
        }
        let raw = symbol.raw();
        let read_before_write = self.paragraph_meaning_marks.get(raw as usize).copied()
            == Some(self.paragraph_recording_generation);
        if !read_before_write {
            self.paragraph_local_meanings.push((raw, group_depth));
        }
    }

    #[doc(hidden)]
    pub fn paragraph_group_exited(&mut self, remaining_depth: u32) {
        self.paragraph_local_meanings
            .retain(|&(_, definition_depth)| definition_depth <= remaining_depth);
    }

    #[inline(always)]
    fn record_dependency(&mut self, dependency: ReadDependency) {
        if let Some(recorder) = self.recorder.as_deref_mut() {
            recorder.record_dependency(dependency);
        }
        if self.paragraph_read_tracking
            && let Some(recorder) = &mut self.paragraph_reads
        {
            recorder.push(dependency);
        }
    }

    #[inline(always)]
    fn observe_read(&mut self, read: TracedExpansionToken) {
        self.last_macro_replay_site = read.macro_replay_site();
    }

    #[inline(always)]
    fn resolve_meaning(
        &mut self,
        input: &mut InputStack,
        stores: &impl ExpansionState,
        symbol: Symbol,
    ) -> Meaning {
        #[cfg(feature = "profiling-stats")]
        let started = input
            .should_sample_expansion_meaning_timer()
            .then(World::start_profiling_timer);
        let meaning = self.resolve_meaning_inner(input, stores, symbol);
        #[cfg(feature = "profiling-stats")]
        if let Some(started) = started {
            input.record_expansion_meaning_resolution_nanos(
                u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            );
        }
        meaning
    }

    fn resolve_meaning_inner(
        &mut self,
        input: &mut InputStack,
        stores: &impl ExpansionState,
        symbol: Symbol,
    ) -> Meaning {
        let Some(site) = self.last_macro_replay_site else {
            input.record_expansion_meaning_lookup();
            return stores.meaning(symbol);
        };
        let Some(guard) = stores.meaning_cache_guard() else {
            input.record_expansion_meaning_lookup();
            return stores.meaning(symbol);
        };
        let slot = ((site.token_list().raw() as usize).wrapping_mul(0x9e37_79b1)
            ^ site.token_index())
            & (MEANING_SITE_CACHE_LEN - 1);
        if let Some(entry) = self.meaning_site_cache[slot]
            && entry.site == site
            && entry.symbol == symbol
            && entry.guard == guard
        {
            #[cfg(debug_assertions)]
            {
                let live_meaning = stores.meaning(symbol);
                debug_assert_eq!(
                    entry.meaning, live_meaning,
                    "meaning-site cache hit disagrees with aggregate state"
                );
            }
            input.record_expansion_meaning_cache_hit();
            return entry.meaning;
        }
        input.record_expansion_meaning_cache_miss();
        input.record_expansion_meaning_lookup();
        let meaning = stores.meaning(symbol);
        self.meaning_site_cache[slot] = Some(MeaningSiteCacheEntry {
            site,
            symbol,
            guard,
            meaning,
        });
        meaning
    }

    /// Creates a context for a nested in-memory expansion while preserving
    /// session facts and reborrowing read-only host resolution and recording.
    ///
    /// The restricted expansion driver still rejects `\input`, while
    /// read-only enquiries such as `\filesize` retain their World-backed
    /// resolver. Resolution indices remain session-global across the nested
    /// operation.
    pub fn with_nested<O>(&mut self, operation: impl FnOnce(&mut ExpansionContext<'a>) -> O) -> O {
        let mut nested = ExpansionContext {
            engine: self.engine,
            job_name: self.job_name,
            job_clock: self.job_clock,
            input_resolver: self.input_resolver.take(),
            recorder: self.recorder.take(),
            resolution_index: self.resolution_index,
            meaning_site_cache: Box::new([None; MEANING_SITE_CACHE_LEN]),
            last_macro_replay_site: None,
            csname_depth: self.csname_depth,
            recoverable_diagnostics: Vec::new(),
            fuel_limit: self.fuel_limit,
            remaining_fuel: self.remaining_fuel,
            fuel_scope_depth: self.fuel_scope_depth,
            paragraph_reads: self.paragraph_reads.take(),
            paragraph_read_tracking: self.paragraph_read_tracking,
            paragraph_meanings: std::mem::take(&mut self.paragraph_meanings),
            paragraph_meaning_marks: std::mem::take(&mut self.paragraph_meaning_marks),
            paragraph_local_meanings: std::mem::take(&mut self.paragraph_local_meanings),
            paragraph_recording_generation: self.paragraph_recording_generation,
            paragraph_barriers: std::mem::take(&mut self.paragraph_barriers),
        };
        let output = operation(&mut nested);
        self.input_resolver = nested.input_resolver.take();
        self.recorder = nested.recorder.take();
        self.resolution_index = nested.resolution_index;
        self.remaining_fuel = nested.remaining_fuel;
        self.recoverable_diagnostics
            .append(&mut nested.recoverable_diagnostics);
        self.paragraph_reads = nested.paragraph_reads.take();
        self.paragraph_read_tracking = nested.paragraph_read_tracking;
        self.paragraph_meanings = std::mem::take(&mut nested.paragraph_meanings);
        self.paragraph_meaning_marks = std::mem::take(&mut nested.paragraph_meaning_marks);
        self.paragraph_local_meanings = std::mem::take(&mut nested.paragraph_local_meanings);
        self.paragraph_recording_generation = nested.paragraph_recording_generation;
        self.paragraph_barriers = std::mem::take(&mut nested.paragraph_barriers);
        output
    }

    /// Drains recoverable TeX diagnostics accumulated by nested expansion.
    pub fn take_recoverable_diagnostics(
        &mut self,
    ) -> impl Iterator<Item = RecoverableExpansionDiagnostic> + '_ {
        self.recoverable_diagnostics.drain(..)
    }

    fn recover_macro_mismatch(&mut self, error: ExpandError) -> Result<(), ExpandError> {
        match error {
            ExpandError::MacroCall(args::MacroCallError::DoesNotMatchDefinition {
                macro_name,
                context,
            }) => {
                self.recoverable_diagnostics.push(
                    RecoverableExpansionDiagnostic::MacroDoesNotMatchDefinition {
                        macro_name,
                        context,
                    },
                );
                Ok(())
            }
            ExpandError::Captured { error, .. } => self.recover_macro_mismatch(*error),
            error => Err(error),
        }
    }

    #[inline(always)]
    pub fn record_meaning(&mut self, symbol: Symbol, meaning: Meaning) {
        if let Some(recorder) = self.recorder.as_deref_mut() {
            recorder.record_meaning(symbol, meaning);
        }
        if self.paragraph_read_tracking {
            if self
                .paragraph_local_meanings
                .iter()
                .rev()
                .any(|&(raw, _)| raw == symbol.raw())
            {
                return;
            }
            let index = symbol.raw() as usize;
            if self.paragraph_meaning_marks.len() <= index {
                self.paragraph_meaning_marks.resize(index + 1, 0);
            }
            if self.paragraph_meaning_marks[index] != self.paragraph_recording_generation {
                self.paragraph_meaning_marks[index] = self.paragraph_recording_generation;
                self.paragraph_meanings.push(symbol.raw());
            }
        }
    }

    pub fn open_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
    ) -> Result<Box<dyn InputSource>, String> {
        self.mark_paragraph_barrier(ParagraphExpansionBarrier::InputOpen);
        self.record_dependency(ReadDependency::Query {
            domain: PARAGRAPH_INPUT_OPEN_BARRIER_DOMAIN,
            identity: 0,
        });
        let request_index = self.next_resolution_index();
        self.input_resolver
            .as_deref_mut()
            .ok_or_else(|| "no input resolver is installed".to_owned())?
            .open_input(input, name, request_index)
    }

    pub fn input_file_size(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
    ) -> Result<Option<u64>, String> {
        let request_index = self.next_resolution_index();
        match self.input_resolver.as_deref_mut() {
            Some(resolver) => resolver.input_file_size(input, name, request_index),
            None => Ok(input
                .read_input_file(Path::new(name))
                .ok()
                .map(|content| u64::try_from(content.bytes().len()).unwrap_or(u64::MAX))),
        }
    }

    pub fn input_file_content(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
    ) -> Result<Option<FileContent>, String> {
        let request_index = self.next_resolution_index();
        match self.input_resolver.as_deref_mut() {
            Some(resolver) => resolver.input_file_content(input, name, request_index),
            None => Ok(input.read_input_file(Path::new(name)).ok()),
        }
    }

    pub fn open_stream_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
    ) -> Result<Option<FileContent>, String> {
        let request_index = self.next_resolution_index();
        match self.input_resolver.as_deref_mut() {
            Some(resolver) => resolver.open_stream_input(input, name, request_index),
            None => Ok(input.read_input_file(Path::new(name)).ok()),
        }
    }

    pub fn next_resolution_index(&mut self) -> u64 {
        let index = self.resolution_index;
        self.resolution_index = self.resolution_index.wrapping_add(1);
        index
    }
}

/// Erased policy for recursive expansion from scanner code.
///
/// Driver mode may execute `\input`; restricted mode preserves the
/// [`ExpansionState`]-only helper boundary. Scanner functions take this as a
/// trait object so the policy does not become a monomorphization axis.
pub trait ExpansionMode {
    fn next_expanded_token(
        &mut self,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Option<TracedTokenWord>, ExpandError>;

    /// Pulls a token at a nested command boundary.
    ///
    /// TeX scanners use ordinary `get_x_token` semantics here. In particular,
    /// tokens returned by `\unexpanded` expand normally once they leave an
    /// expanded-token-list builder.
    fn next_command_token(
        &mut self,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Option<TracedTokenWord>, ExpandError> {
        self.next_expanded_token(input, stores, expansion)
    }

    /// Pulls a token using ordinary `get_x_token` suppression semantics.
    ///
    /// This is used by scanners such as `\the` whose operand belongs to the
    /// current expansion request even when the scanner was entered recursively.
    fn next_ordinary_token(
        &mut self,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Option<TracedTokenWord>, ExpandError> {
        self.next_expanded_token(input, stores, expansion)
    }

    fn dispatch_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Dispatch, ExpandError>;

    fn dispatch_known_meaning(
        &mut self,
        token: TracedTokenWord,
        meaning: Meaning,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Dispatch, ExpandError>;

    fn dispatch_inverted_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Dispatch, ExpandError>;

    fn dispatch_raw_token_after(
        &mut self,
        saved: TracedTokenWord,
        target: TracedTokenWord,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<(), ExpandError> {
        // `\noexpand` suppresses exactly the token fetched by TeX's raw
        // `get_token`. Historical `\unexpanded` provenance does not: e-TeX
        // limits that suppression to expanded-token-list construction.
        let suppress =
            stores.origin_is_inserted_kind(target.origin(), InsertedOriginKind::NoExpand);
        let dispatched = if suppress {
            Ok(Dispatch::DeliverNoExpand(target))
        } else {
            self.dispatch_raw_token(target, input, stores, expansion)
        };
        let result = dispatched.map(|dispatch| push_dispatch_result(input, stores, dispatch));
        // TeX's `back_input` cancels the first `get_token` delivery before
        // the saved token is read again. Without this, a brace held by
        // `\expandafter` changes `align_state` twice.
        input.undo_alignment_delivery(classify_alignment_token(stores, saved).0);
        push_inserted_token(input, stores, saved, InsertedOriginKind::ExpandAfter);
        result
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RestrictedExpansionMode;

impl ExpansionMode for RestrictedExpansionMode {
    fn next_expanded_token(
        &mut self,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Option<TracedTokenWord>, ExpandError> {
        get_x_token_without_input_open(input, stores, expansion)
    }

    fn dispatch_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Dispatch, ExpandError> {
        dispatch_one_raw_token_with_context(token, input, stores, expansion)
    }

    fn dispatch_known_meaning(
        &mut self,
        token: TracedTokenWord,
        meaning: Meaning,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Dispatch, ExpandError> {
        dispatch::dispatch_without_input_open(
            semantic_token(token),
            token.origin(),
            input,
            stores,
            expansion,
            meaning,
        )
    }

    fn dispatch_inverted_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Dispatch, ExpandError> {
        let Some(symbol) = expandable_symbol(stores, token) else {
            return Ok(Dispatch::Deliver(token));
        };
        let meaning = expansion.resolve_meaning(input, stores, symbol);
        expansion.record_meaning(symbol, meaning);
        dispatch::dispatch_without_input_open_inverted(
            semantic_token(token),
            token.origin(),
            input,
            stores,
            expansion,
            meaning,
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DriverExpansionMode;

impl ExpansionMode for DriverExpansionMode {
    fn next_expanded_token(
        &mut self,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Option<TracedTokenWord>, ExpandError> {
        get_x_token_with_context(input, stores, expansion)
    }

    fn next_command_token(
        &mut self,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Option<TracedTokenWord>, ExpandError> {
        get_x_token_with_context(input, stores, expansion)
    }

    fn next_ordinary_token(
        &mut self,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Option<TracedTokenWord>, ExpandError> {
        get_x_token_with_context(input, stores, expansion)
    }

    fn dispatch_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Dispatch, ExpandError> {
        let Some(symbol) = expandable_symbol(stores, token) else {
            return Ok(Dispatch::Deliver(token));
        };
        let meaning = expansion.resolve_meaning(input, stores, symbol);
        expansion.record_meaning(symbol, meaning);
        dispatch::dispatch_with_context(
            semantic_token(token),
            token.origin(),
            input,
            stores,
            expansion,
            meaning,
        )
    }

    fn dispatch_known_meaning(
        &mut self,
        token: TracedTokenWord,
        meaning: Meaning,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Dispatch, ExpandError> {
        dispatch::dispatch_with_context(
            semantic_token(token),
            token.origin(),
            input,
            stores,
            expansion,
            meaning,
        )
    }

    fn dispatch_inverted_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack,
        stores: &mut tex_state::ExpansionContext<'_>,
        expansion: &mut ExpansionContext<'_>,
    ) -> Result<Dispatch, ExpandError> {
        let Some(symbol) = expandable_symbol(stores, token) else {
            return Ok(Dispatch::Deliver(token));
        };
        let meaning = expansion.resolve_meaning(input, stores, symbol);
        expansion.record_meaning(symbol, meaning);
        dispatch::dispatch_with_context_inverted(
            semantic_token(token),
            token.origin(),
            input,
            stores,
            expansion,
            meaning,
        )
    }
}

impl From<LexError> for ExpandError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<args::MacroCallError> for ExpandError {
    fn from(value: args::MacroCallError) -> Self {
        Self::MacroCall(value)
    }
}

impl From<scan_int::ScanIntError> for ExpandError {
    fn from(value: scan_int::ScanIntError) -> Self {
        Self::ScanInt(Box::new(value))
    }
}

impl From<scan_dimen::ScanDimenError> for ExpandError {
    fn from(value: scan_dimen::ScanDimenError) -> Self {
        Self::ScanDimen(Box::new(value))
    }
}

impl From<scan_glue::ScanGlueError> for ExpandError {
    fn from(value: scan_glue::ScanGlueError) -> Self {
        Self::ScanGlue(Box::new(value))
    }
}

impl From<scan::ScanToksError> for ExpandError {
    fn from(value: scan::ScanToksError) -> Self {
        Self::ScanGeneralText(Box::new(value))
    }
}

/// Pulls the next fully expanded token.
pub fn get_x_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExpandError> {
    get_x_token_with_context(input, stores, &mut ExpansionContext::new("texput"))
}

/// Pulls the next fully expanded token using driver-provided expansion context.
pub fn get_x_token_with_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExpandError> {
    expansion.begin_fuel_scope();
    let result = get_x_token_with_context_inner(input, stores, expansion, false, true, None);
    expansion.end_fuel_scope();
    match result {
        Ok(token) => Ok(token),
        Err(error) => Err(error.capture(input)),
    }
}

/// Pulls the next expanded command token after a prefix.
///
/// This is an explicit name for the ordinary TeX `get_x_token` operation used
/// by prefix scanners; it does not introduce a distinct expansion policy.
pub fn get_command_token_with_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExpandError> {
    get_x_token_with_context(input, stores, expansion)
}

/// Pulls the next expanded token while leaving e-TeX protected macros
/// unexpanded. This is the `get_x_or_protected` operation used by alignments.
pub fn get_x_or_protected_with_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExpandError> {
    // e-TeX's `get_x_or_protected` adds only the protected-macro stopping rule
    // to ordinary x-token expansion.
    expansion.begin_fuel_scope();
    let result = get_x_token_with_context_inner(input, stores, expansion, true, true, None);
    expansion.end_fuel_scope();
    match result {
        Ok(token) => Ok(token),
        Err(error) => Err(error.capture(input)),
    }
}

/// Pulls the next alignment-lookahead token while leaving protected macros
/// unexpanded.
///
/// This remains a named alignment entry point, but e-TeX gives it the same
/// unexpanded-token behavior as ordinary `get_x_or_protected`.
pub fn get_alignment_x_or_protected_with_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExpandError> {
    get_x_or_protected_with_context(input, stores, expansion)
}

/// A token whose alignment-sensitive `get_next` work has already completed.
///
/// This is the synchronous, stack-local equivalent of TeX82's `cur_cmd`,
/// `cur_chr`, and `cur_cs` state between `get_next` and `x_token`. It must not
/// be stored in the input stack or any checkpoint-visible engine state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PreparedExpansionToken(TracedExpansionToken);

impl PreparedExpansionToken {
    #[must_use]
    pub(crate) const fn expansion_token(self) -> TracedExpansionToken {
        self.0
    }

    #[must_use]
    pub(crate) fn traced_token(self) -> TracedTokenWord {
        self.0.traced_token()
    }

    #[must_use]
    pub(crate) const fn suppress_expansion(self) -> bool {
        self.0.suppress_expansion()
    }
}

/// TeX82's `x_token`: expand a token already obtained under `get_next`
/// semantics, while sharing the ordinary `get_x_token` interpreter.
pub(crate) fn get_x_or_protected_from_prepared_with_context(
    prepared: PreparedExpansionToken,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExpandError> {
    expansion.begin_fuel_scope();
    let result =
        get_x_token_with_context_inner(input, stores, expansion, true, false, Some(prepared));
    expansion.end_fuel_scope();
    match result {
        Ok(token) => Ok(token),
        Err(error) => Err(error.capture(input)),
    }
}

fn get_x_token_with_context_inner(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    protect_macros: bool,
    expand_unexpanded_replay: bool,
    first: Option<PreparedExpansionToken>,
) -> Result<Option<TracedTokenWord>, ExpandError> {
    let mut first = first;
    loop {
        expansion.burn_fuel()?;
        let (read, alignment_prepared) = if let Some(prepared) = first.take() {
            (prepared.0, true)
        } else {
            let read = match input.next_traced_expansion_token(stores) {
                Ok(Some(read)) => read,
                Ok(None) => return Ok(None),
                Err(tex_lex::LexError::InvalidCharacter { .. }) => {
                    // TeX.web `get_next` reports a catcode-15 character and
                    // restarts after consuming it. Keeping recovery here prevents
                    // nested scanners (notably \read) from unwinding.
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            (read, false)
        };
        expansion.observe_read(read);
        let token = read.token();
        let traced = read.traced_token();

        if token.is_frozen_end_template() {
            return Ok(Some(TracedTokenWord::pack(
                stores.frozen_endv_token(),
                read.origin(),
            )));
        }

        if let Some(meaning) = stores.frozen_primitive_meaning(token) {
            let dispatched =
                dispatch_with_context(token, read.origin(), input, stores, expansion, meaning);
            match dispatched {
                Ok(Dispatch::Continue) => continue,
                Ok(Dispatch::Deliver(delivered) | Dispatch::DeliverNoExpand(delivered)) => {
                    return Ok(Some(delivered));
                }
                Ok(push @ (Dispatch::Push { .. } | Dispatch::PushTransient { .. })) => {
                    apply_dispatch_push(input, push);
                    continue;
                }
                Err(error) => return Err(error.capture(input)),
            }
        }

        if read.suppress_expansion()
            && !(expand_unexpanded_replay && read.expand_in_ordinary_context())
        {
            if !alignment_prepared && intercept_suppressed_alignment_token(input, stores, traced) {
                continue;
            }
            return Ok(Some(traced));
        }

        let symbol = match expandable_symbol_for_token(stores, token) {
            Some(symbol) => symbol,
            None => {
                if !alignment_prepared && intercept_alignment_token(input, stores, traced) {
                    continue;
                }
                return Ok(Some(traced));
            }
        };

        let meaning = expansion.resolve_meaning(input, stores, symbol);
        expansion.record_meaning(symbol, meaning);
        if protect_macros
            && matches!(meaning, Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::PROTECTED))
        {
            return Ok(Some(traced));
        }
        if input.has_active_alignment_cell()
            && matches!(meaning, Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::OUTER))
        {
            return Err(ExpandError::ForbiddenOuterTokenInAlignment { context: traced });
        }

        let dispatched =
            dispatch_with_context(token, read.origin(), input, stores, expansion, meaning);
        let dispatched = match dispatched {
            Ok(dispatched) => dispatched,
            Err(error) => match expansion.recover_macro_mismatch(error) {
                Ok(()) => continue,
                Err(error) if replay_macro_eof_is_clean(stores, &error) => return Ok(None),
                Err(error) => return Err(error),
            },
        };
        match dispatched {
            Dispatch::Continue => {}
            Dispatch::DeliverNoExpand(token) => {
                if intercept_suppressed_alignment_token(input, stores, token) {
                    continue;
                }
                return Ok(Some(token));
            }
            Dispatch::Deliver(token) => {
                if !alignment_prepared && intercept_alignment_token(input, stores, token) {
                    continue;
                }
                return Ok(Some(token));
            }
            push @ (Dispatch::Push { .. } | Dispatch::PushTransient { .. }) => {
                apply_dispatch_push(input, push);
            }
        }
    }
}

#[cold]
#[inline(never)]
fn expansion_work_limit_exceeded(limit: u64) -> Result<(), ExpandError> {
    Err(ExpandError::ExpansionWorkLimitExceeded { limit })
}

/// Reads one token under the semantic `get_next` rules needed before TeX82's
/// `x_token`, retaining one-shot expansion suppression for the shared loop.
pub(crate) fn next_prepared_expansion_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Option<PreparedExpansionToken>, tex_lex::LexError> {
    loop {
        let Some(read) = input.next_traced_expansion_token(stores)? else {
            return Ok(None);
        };
        expansion.observe_read(read);
        let traced = read.traced_token();
        let intercepted = if read.suppress_expansion() {
            intercept_suppressed_alignment_token(input, stores, traced)
        } else {
            intercept_alignment_token(input, stores, traced)
        };
        if !intercepted {
            return Ok(Some(PreparedExpansionToken(read)));
        }
    }
}

pub(crate) fn intercept_alignment_token(
    input: &mut InputStack,
    stores: &impl ExpansionState,
    traced: TracedTokenWord,
) -> bool {
    if !input.has_active_alignment() {
        return false;
    }
    let (delivery, terminator) = classify_alignment_token(stores, traced);
    input.intercept_alignment_token(traced, delivery, terminator, stores.execution_group_depth())
}

fn classify_alignment_token(
    stores: &impl ExpansionState,
    traced: TracedTokenWord,
) -> (
    tex_lex::AlignmentTokenDelivery,
    Option<tex_lex::AlignmentTerminator>,
) {
    let token = semantic_token(traced);
    let meaning = match token {
        Token::Cs(symbol) => Some(stores.meaning(symbol)),
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => stores
            .active_character_symbol(ch)
            .map(|symbol| stores.meaning(symbol)),
        Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => None,
    };
    // tex.web updates align_state only in the character-token branch of
    // get_next. Control-sequence aliases such as \bgroup and \egroup still
    // delimit semantic groups, but they do not change alignment brace depth.
    let delivery = if matches!(
        token,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    ) {
        tex_lex::AlignmentTokenDelivery::LeftBrace
    } else if matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    ) {
        tex_lex::AlignmentTokenDelivery::RightBrace
    } else {
        tex_lex::AlignmentTokenDelivery::Other
    };
    let terminator = if matches!(
        token,
        Token::Char {
            cat: Catcode::AlignmentTab,
            ..
        }
    ) || matches!(
        meaning,
        Some(Meaning::CharToken {
            cat: Catcode::AlignmentTab,
            ..
        })
    ) {
        Some(tex_lex::AlignmentTerminator::Tab)
    } else {
        match meaning {
            Some(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr,
            )) => Some(tex_lex::AlignmentTerminator::Cr),
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Span)) => {
                Some(tex_lex::AlignmentTerminator::Span)
            }
            _ => None,
        }
    };
    (delivery, terminator)
}

/// Canonical TeX `get_next`-style raw semantic delivery.
///
/// Expansion primitives and scanners must use this path whenever they consume
/// a raw token. It applies alignment brace accounting and cell-terminator
/// interception before the token can be observed. The lower-level
/// `InputStack` reads remain reserved for the expansion loop and `\noexpand`,
/// which must first classify one-shot suppression.
pub fn next_semantic_raw_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, tex_lex::LexError> {
    loop {
        let Some(traced) = input.next_traced_token(stores)? else {
            return Ok(None);
        };
        if !intercept_alignment_token(input, stores, traced) {
            return Ok(Some(traced));
        }
    }
}

/// Raw replay lookahead used by `\noexpand`, which must restore the next
/// meaning before applying the alignment-sensitive part of `get_next`.
pub(crate) fn next_unintercepted_raw_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, tex_lex::LexError> {
    input.next_traced_token(stores)
}

pub(crate) fn next_suppressed_semantic_raw_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, tex_lex::LexError> {
    loop {
        let Some(traced) = input.next_traced_token(stores)? else {
            return Ok(None);
        };
        if !intercept_suppressed_alignment_token(input, stores, traced) {
            return Ok(Some(traced));
        }
    }
}

/// Applies TeX82's `dont_expand` command-code test to alignment delivery.
///
/// An expandable meaning is delivered as the one-shot `no_expand_flag`
/// variant of `\relax`. An already-unexpandable meaning retains its ordinary
/// brace or delimiter behavior, including `\cr` interception.
fn intercept_suppressed_alignment_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    traced: TracedTokenWord,
) -> bool {
    if !input.has_active_alignment() {
        return false;
    }
    let expandable = expandable_symbol(stores, traced).is_some_and(|symbol| {
        matches!(
            stores.meaning(symbol),
            Meaning::Undefined | Meaning::Macro { .. } | Meaning::ExpandablePrimitive(_)
        )
    });
    if expandable {
        input.intercept_alignment_token(
            traced,
            tex_lex::AlignmentTokenDelivery::Other,
            None,
            stores.execution_group_depth(),
        )
    } else {
        intercept_alignment_token(input, stores, traced)
    }
}

pub fn back_input<I>(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    tokens: I,
) where
    I: IntoIterator<Item = TracedTokenWord>,
{
    back_input_with_kind(input, stores, tokens, TokenListReplayKind::Inserted);
}

fn back_input_with_kind<I>(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    tokens: I,
    replay_kind: TokenListReplayKind,
) where
    I: IntoIterator<Item = TracedTokenWord>,
{
    #[cfg(test)]
    BACK_INPUT_CALLS.with(|calls| calls.set(calls.get() + 1));
    let mut traced = tokens.into_iter();
    let Some(first) = traced.next() else {
        return;
    };
    input.undo_alignment_delivery(classify_alignment_token(stores, first).0);
    let Some(second) = traced.next() else {
        if let Some((list, replay_kind, index)) = input.current_token_list_frame()
            && matches!(
                replay_kind,
                TokenListReplayKind::MacroBody | TokenListReplayKind::MacroArgument
            )
            && index > 0
            && stores.tokens(list).get(index - 1).copied() == Some(semantic_token(first))
            && input.rewind_current_token_list_frame()
        {
            return;
        }
        if input.push_current_source_pending(first) {
            return;
        }
        let mut buffer = input.take_transient_token_buffer();
        buffer.push(first);
        input.push_transient_tokens(buffer, replay_kind);
        return;
    };

    input.undo_alignment_delivery(classify_alignment_token(stores, second).0);
    let (lower, _) = traced.size_hint();
    let mut buffer = input.take_transient_token_buffer();
    buffer.reserve(lower.saturating_add(2));
    buffer.extend([first, second]);
    for token in traced {
        input.undo_alignment_delivery(classify_alignment_token(stores, token).0);
        buffer.push(token);
    }
    input.push_transient_tokens(buffer, replay_kind);
}

#[cfg(test)]
thread_local! {
    static BACK_INPUT_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_back_input_call_count() {
    BACK_INPUT_CALLS.with(|calls| calls.set(0));
}

#[cfg(test)]
pub(crate) fn back_input_call_count() -> usize {
    BACK_INPUT_CALLS.with(std::cell::Cell::get)
}

/// Inserts traced tokens that were not previously delivered by `get_next`.
///
/// Unlike [`back_input`], this does not reverse alignment brace accounting.
pub fn insert_input<I>(
    input: &mut InputStack,
    _stores: &mut tex_state::ExpansionContext<'_>,
    tokens: I,
) where
    I: IntoIterator<Item = TracedTokenWord>,
{
    let mut traced = input.take_transient_token_buffer();
    traced.extend(tokens);
    if traced.is_empty() {
        input.recycle_transient_token_buffer(traced);
        return;
    }
    input.push_transient_tokens(traced, TokenListReplayKind::Inserted);
}

/// Implements TeX's unexpanded `get_token`, including alignment delimiter
/// interception performed by `get_next` before the token reaches its caller.
pub fn get_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExpandError> {
    Ok(next_semantic_raw_token(input, stores)?)
}

pub(crate) fn get_token_with_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExpandError> {
    Ok(next_prepared_expansion_token(input, stores, expansion)?
        .map(PreparedExpansionToken::traced_token))
}

/// Implements TeX82's `get_preamble_token` operation after `\span`: fetch
/// one raw token, expand that token once when it is expandable, then fetch
/// one raw token from the resulting input. Unlike `get_x_token`, this does
/// not recursively expand the token produced by that single expansion.
pub fn expand_once_then_get_token_with_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExpandError> {
    let Some(target) = get_token_with_context(input, stores, expansion)? else {
        return Ok(None);
    };
    let Some(symbol) = expandable_symbol(stores, target) else {
        return Ok(Some(target));
    };
    let meaning = expansion.resolve_meaning(input, stores, symbol);
    expansion.record_meaning(symbol, meaning);
    let dispatch = dispatch_with_context(
        semantic_token(target),
        target.origin(),
        input,
        stores,
        expansion,
        meaning,
    )?;
    push_dispatch_result(input, stores, dispatch);
    get_token(input, stores)
}

pub(crate) fn get_x_token_without_input_open(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExpandError> {
    loop {
        let read = match input.next_traced_expansion_token(stores) {
            Ok(Some(read)) => read,
            Ok(None) => return Ok(None),
            Err(tex_lex::LexError::InvalidCharacter { .. }) => continue,
            Err(error) => return Err(error.into()),
        };
        expansion.observe_read(read);
        let token = read.token();
        let traced = read.traced_token();

        if read.suppress_expansion() && !read.expand_in_ordinary_context() {
            if intercept_suppressed_alignment_token(input, stores, traced) {
                continue;
            }
            return Ok(Some(traced));
        }

        let symbol = match expandable_symbol(stores, traced) {
            Some(symbol) => symbol,
            None => {
                if intercept_alignment_token(input, stores, traced) {
                    continue;
                }
                return Ok(Some(traced));
            }
        };

        let meaning = expansion.resolve_meaning(input, stores, symbol);
        expansion.record_meaning(symbol, meaning);

        let dispatched = dispatch::dispatch_without_input_open(
            token,
            read.origin(),
            input,
            stores,
            expansion,
            meaning,
        );
        let dispatched = match dispatched {
            Ok(dispatched) => dispatched,
            Err(error) => match expansion.recover_macro_mismatch(error) {
                Ok(()) => continue,
                Err(error) if replay_macro_eof_is_clean(stores, &error) => return Ok(None),
                Err(error) => return Err(error),
            },
        };
        match dispatched {
            Dispatch::Continue => {}
            Dispatch::DeliverNoExpand(token) => {
                if intercept_suppressed_alignment_token(input, stores, token) {
                    continue;
                }
                return Ok(Some(token));
            }
            Dispatch::Deliver(token) => {
                if intercept_alignment_token(input, stores, token) {
                    continue;
                }
                return Ok(Some(token));
            }
            push @ (Dispatch::Push { .. } | Dispatch::PushTransient { .. }) => {
                apply_dispatch_push(input, push);
            }
        }
    }
}

fn replay_macro_eof_is_clean(
    stores: &tex_state::ExpansionContext<'_>,
    error: &ExpandError,
) -> bool {
    let ExpandError::MacroCall(args::MacroCallError::EndOfInput { context, .. }) = error else {
        return false;
    };
    matches!(
        stores.origin(context.origin()),
        OriginRecord::Inserted(inserted)
            if matches!(
                inserted.kind(),
                InsertedOriginKind::TokenListReplay(TokenListReplayKind::Inserted)
            )
    )
}

pub(crate) fn dispatch_one_raw_token_with_context(
    token: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Dispatch, ExpandError> {
    let semantic = semantic_token(token);
    let symbol = match expandable_symbol(stores, token) {
        Some(symbol) => symbol,
        None => return Ok(Dispatch::Deliver(token)),
    };

    let meaning = expansion.resolve_meaning(input, stores, symbol);
    expansion.record_meaning(symbol, meaning);
    dispatch::dispatch_without_input_open(
        semantic,
        token.origin(),
        input,
        stores,
        expansion,
        meaning,
    )
}

pub(crate) fn expandable_symbol(
    stores: &mut tex_state::ExpansionContext<'_>,
    token: TracedTokenWord,
) -> Option<Symbol> {
    expandable_symbol_for_token(stores, semantic_token(token))
}

fn expandable_symbol_for_token(
    stores: &mut tex_state::ExpansionContext<'_>,
    token: Token,
) -> Option<Symbol> {
    match token {
        Token::Cs(symbol) => Some(symbol),
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => Some(stores.intern_active_character(ch)),
        Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => None,
    }
}

pub(crate) fn push_dispatch_result(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    dispatch: Dispatch,
) {
    match dispatch {
        Dispatch::Deliver(token) => {
            push_inserted_token(input, stores, token, InsertedOriginKind::ExpandAfter);
        }
        Dispatch::DeliverNoExpand(token) => push_noexpand_token(input, stores, token),
        Dispatch::Continue => {}
        push @ (Dispatch::Push { .. } | Dispatch::PushTransient { .. }) => {
            apply_dispatch_push(input, push);
        }
    }
}

pub(crate) fn apply_dispatch_push(input: &mut InputStack, dispatch: Dispatch) {
    match dispatch {
        Dispatch::Push {
            replay_kind,
            token_list,
            origin_list,
            macro_arguments,
            macro_invocation,
        } => {
            if replay_kind == ExpansionReplayKind::MacroBody {
                input.push_macro_body_with_origins_and_invocation(
                    token_list,
                    origin_list,
                    macro_arguments,
                    macro_invocation,
                );
            } else {
                input.push_token_list_with_origins(
                    token_list,
                    origin_list,
                    replay_kind.as_lex_kind(),
                );
            }
        }
        Dispatch::PushTransient {
            replay_kind,
            tokens,
        } => {
            input.push_transient_tokens(tokens, replay_kind.as_lex_kind());
        }
        Dispatch::Continue | Dispatch::Deliver(_) | Dispatch::DeliverNoExpand(_) => {}
    }
}

pub(crate) fn push_inserted_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    token: TracedTokenWord,
    kind: InsertedOriginKind,
) {
    let semantic = semantic_token(token);
    let origin = stores.inserted_origin(kind, semantic, token.origin());
    let mut buffer = input.take_transient_token_buffer();
    buffer.push(TracedTokenWord::pack(semantic, origin));
    input.push_transient_tokens(buffer, TokenListReplayKind::Inserted);
}

pub(crate) fn push_noexpand_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    token: TracedTokenWord,
) {
    let semantic = semantic_token(token);
    let origin = stores.inserted_origin(InsertedOriginKind::NoExpand, semantic, token.origin());
    let mut buffer = input.take_transient_token_buffer();
    buffer.push(TracedTokenWord::pack(semantic, origin));
    input.push_transient_tokens(buffer, TokenListReplayKind::NoExpand);
}

pub(crate) fn expansion_suppressed_origin_list(
    stores: &mut tex_state::ExpansionContext<'_>,
    token_list: TokenListId,
    source_origins: OriginListId,
    fallback_parent: OriginId,
) -> OriginListId {
    let tokens = stores.tokens(token_list).to_vec();
    let parents = if source_origins == OriginListId::EMPTY {
        vec![fallback_parent; tokens.len()]
    } else {
        stores.origin_list(source_origins).to_vec()
    };
    let mut origins = stores.origin_list_builder();
    for (&token, parent) in tokens.iter().zip(parents) {
        origins.push(stores.inserted_origin(InsertedOriginKind::Unexpanded, token, parent));
    }
    stores.finish_origin_list(&mut origins)
}

pub fn semantic_token(token: TracedTokenWord) -> Token {
    token
        .token()
        .expect("expansion must only receive valid traced tokens")
}
