use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedPackingSpec {
    Natural,
    Exactly(Scaled),
    Spread(Scaled),
}

/// The complete command-owned operand of TeX82's `\\vsplit`.
///
/// The keyword's absence is preserved so replay can issue its diagnostic, but
/// both the register and dimension have already been consumed canonically.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedVSplit {
    pub index: u16,
    pub height: Scaled,
    /// TeX82 §1082 reports a missing `to` before scanning the dimension.
    pub missing_to_context: Option<String>,
    /// Context after `scan_dimen`, used by the source-free box-kind check.
    pub split_context: String,
}

/// A completed display diagnostic. Its display-line content and source origin
/// are frozen while command input is borrowed, leaving replay no
/// operand-reading or envelope-decoding work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedDisplayDiagnostic {
    /// The content passed to TeX82 §62's `print_nl`, excluding §1293's
    /// terminating period and error completion.
    pub content: String,
    pub provenance: StructuredProvenance,
}

/// A completed TeX82 `\hrule` or `\vrule` specification.
///
/// The command processor owns the expanded `width`, `height`, and `depth`
/// keyword scans and their scalar operands. Replay receives only these final
/// dimensions, so applying a rule cannot open another source-consumption path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedRuleSpec {
    pub width: Option<Scaled>,
    pub height: Option<Scaled>,
    pub depth: Option<Scaled>,
}
