use super::*;

/// The completed command-owned prefix of a TeX82 box construction.
///
/// `scan_spec` (§645) "scans a box specification and left brace": the optional
/// `to`/`spread` clause and then the mandatory opening brace, which it
/// consumes. Keeping both operations here means replay only receives a typed
/// construction request and never needs to reopen input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedBoxConstruction {
    pub kind: ScannedBoxKind,
    pub packing: ScannedPackingSpec,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedBoxKind {
    HBox,
    VBox,
    VTop,
    /// TeX82 §1167's `mmode+vcenter`: `scan_spec(vcenter_group,false);
    /// normal_paragraph; push_nest; mode:=-vmode`. `\vcenter` opens the same
    /// §645 `scan_spec` prefix and the same internal vertical list as
    /// `\vbox`; only §1168's closing action differs (a `vcenter_noad`
    /// nucleus instead of §1075's `box_end`), which is why it shares this
    /// scan and not the math-text-field scan a noad-building primitive would
    /// otherwise take.
    VCenter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedPackingSpec {
    Natural,
    Exactly(Scaled),
    Spread(Scaled),
}

/// The completed command-owned prefix of TeX82 §1099's `begin_insert_or_adjust`
/// for `\insert` and `\vadjust`.
///
/// `scan_eight_bit_int`'s range clamp and §1099's reserved-255 recovery both
/// need to write a `Universe`-routed diagnostic, so this keeps only the raw
/// scanned class number (any `i32` the integer scanner produced); the
/// mandatory opening brace is consumed here, exactly as §1099's
/// `new_save_level(insert_group); scan_left_brace` does. Replay performs the
/// bounded 0..=255 recovery and the `\insert255` rejection immediately before
/// opening the insertion group -- but only for `\insert`: `\vadjust` sets
/// `class:=255` unconditionally (`if cur_cmd=vadjust then cur_val:=255`)
/// without ever calling `scan_eight_bit_int`, so `is_vadjust` tells replay to
/// skip both diagnostics for that already-valid sentinel class.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedInsertConstruction {
    pub class: i32,
    pub is_vadjust: bool,
    pub pre: bool,
    /// TeX82 §1099 calls §82's `error` before `scan_left_brace`, so preserve
    /// the live input display at the point the reserved class is detected.
    pub reserved_class_context: Option<String>,
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
