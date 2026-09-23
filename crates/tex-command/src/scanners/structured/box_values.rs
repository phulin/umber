use super::*;

/// The complete command-owned operand of TeX82's `\setbox` assignment.
///
/// TeX82 §1241 calls §1084's `scan_box` from inside `prefixed_command`, so
/// the required `make_box` command never returns to §1030's `big_switch` and
/// must not receive a second main-control command trace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedSetBoxAssignment {
    pub index: u16,
    pub path: ScannedSetBoxPath,
}

/// The two distinct TeX82 §1241 paths after `\setbox` has scanned its
/// register and optional equals sign.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScannedSetBoxPath {
    /// `set_box_allowed` was false, so §1241 calls `error` immediately.
    /// No box command has been fetched or backed up.
    Forbidden { error_context: String },
    /// `set_box_allowed` was true and §1084's ordinary `scan_box` ran.
    /// A missing payload has therefore already backed up its rejected command.
    Payload(ScannedBoxShiftPayload),
}

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

/// The completed command-owned operand of TeX82 §1084's `scan_box`.
///
/// Box shifts and `\setbox` share this exact `make_box` vocabulary and
/// recovery; the historical type name is retained as part of the public API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScannedBoxShiftPayload {
    /// `scan_box`'s "A <box> was supposed to be here" recovery: the rejected
    /// command has already been backed up for ordinary replay.
    Missing,
    BoxRegister {
        index: u16,
        copy: bool,
    },
    /// §1081 may diagnose against the live input while taking the last box.
    LastBox {
        error_context: String,
    },
    VSplit(ScannedVSplit),
    Construction(ScannedBoxConstruction),
}

/// A completed TeX82 §1073 box-shift prefix: the already-signed shift amount
/// (tex.web's `box_context`) paired with the following box operand it
/// applies to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedBoxShift {
    pub delta: Scaled,
    pub payload: ScannedBoxShiftPayload,
}

/// The completed register operand of TeX82's `\\box` command.
///
/// `make_box(box_code)` calls §433's `scan_eight_bit_int` before main control
/// can apply the resulting box-list operation. Keeping that scan here
/// preserves the raw digit delivery, bounded recovery, and integer-scanner
/// backup entirely in command control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedBoxRegister {
    pub index: u16,
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

/// The completed payload prefix of TeX82's `\\leaders` family.
///
/// A constructed box deliberately remains a construction request: its body is
/// replayed through the ordinary box lifecycle before command control scans
/// the following glue operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedLeaderPayload {
    Missing,
    BoxRegister { index: u16, copy: bool },
    Construction(ScannedBoxConstruction),
    Rule(ScannedRuleSpec),
}

/// A completed named glue-parameter assignment.
///
/// Command processing owns the optional equals sign and scalar glue scan;
/// replay receives only the parameter selector and its finished value.  The
/// `mu` flag preserves the TeX distinction between ordinary and math glue
/// parameters without exposing another input path to the executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedGlueParameterAssignment {
    pub index: u16,
    pub value: GlueSpec,
    pub mu: bool,
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

/// One step of TeX82 §1123's post-`scan_char_num` lookahead for `\accent`.
///
/// §1123's `make_accent` does not classify the base character directly after
/// the accent code: it runs §1270's `do_assignments` in between, and §1270's
/// loop body is `prefixed_command` -- executor state, not scanner state. The
/// lookahead is therefore delivered one command at a time.
#[derive(Debug, Eq, PartialEq)]
// Assignment delivery is an allocation-free handoff consumed immediately;
// boxing it would put a heap allocation in ordinary accent lookahead.
#[allow(clippy::large_enum_variant)]
pub enum ScannedAccentBase<G> {
    /// §1124's `letter`, `other_char`, `char_given`, or `char_num` base.
    Character {
        character: u8,
        provenance: StructuredProvenance,
    },
    /// §1270's `prefixed_command`: the delivered assignment the executor must
    /// run before the lookahead continues.
    Assignment(CurrentCommand<G>),
    /// §1124's `else back_input`, already performed, or end of input. Either
    /// way §1123 appends the accent by itself.
    Missing,
}

/// Completed command-owned operands for TeX82 §1123's text `\accent`.
///
/// Only `scan_char_num`'s accent code is command-owned. The base character
/// arrives through [`CommandProcessor::scan_accent_base`], one §1270
/// `do_assignments` iteration at a time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedAccent {
    pub accent: i32,
    pub accent_provenance: StructuredProvenance,
}

/// The source position at which TeX82 §1117/§1120 opened one live
/// `\discretionary` part.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedDiscretionaryOpening {
    pub provenance: StructuredProvenance,
}
