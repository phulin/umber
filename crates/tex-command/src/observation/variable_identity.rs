//! Exhaustive TeX82/e-TeX identities for eqtb-addressed variable meanings.
//!
//! `canonical_command_identity` classifies a delivered `Meaning`. Primitive
//! meanings are classified by `primitive_identity`; the remaining meanings --
//! classical registers, named parameters, page quantities, read-only internal
//! integers, and font identifiers -- are *variables*, and tex.web gives each
//! of them a command whose `chr_code` is a real eqtb address rather than a
//! small ordinal. This module owns those addresses and the parameter-code
//! translation they need.
//!
//! Two facts make that translation necessary rather than an identity map.
//!
//! 1. tex.web's parameter *code* is a position in a per-class list
//!    (§236's `int_par` codes, §247's `dimen_par` codes, §224's `glue_par`
//!    codes, §230's `local_base` block). Umber's `IntParam`/`DimenParam`/
//!    `GlueParam`/`TokParam` index is a slot in its own dense environment
//!    bank (`tex_state::env::banks`), which was allocated in Umber's own
//!    order and is *not* tex.web's order: `\globaldefs` is Umber slot 32 but
//!    tex.web code 43, and `\fam` is Umber slot 59 but tex.web code 44.
//!    Mapping one to the other by identity misreports every parameter after
//!    `\tracingstats`.
//! 2. A register's command selector is the base of its eqtb region plus the
//!    register number (§1224's `shorthand_def`: `\countdef` defines its
//!    target as `define(p,assign_int,count_base+cur_val)`), so the classifier
//!    needs the region bases themselves.
//!
//! ## Where the region bases come from
//!
//! tex.web §§224/230/232/236/247 chain the bases from one another:
//!
//! ```text
//! skip_base    = glue_base + glue_pars(18)     dimen_base  = del_code_base + 256
//! mu_skip_base = skip_base + 256               scaled_base = dimen_base + dimen_pars(21)
//! local_base   = mu_skip_base + 256            count_base  = int_base + int_pars
//! toks_base    = local_base + 10               del_code_base = count_base + 256
//! ```
//!
//! Only two links in that chain are build-specific, and both are pinned by
//! the same oracle this repository's committed traces come from
//! (`scripts/build-tex82-oracle.sh`, TeX Live 2025 `tex.web` + `tex.ch` +
//! the encTeX change files):
//!
//! - `int_base` is `char_sub_code_base+256`, not tex.web's
//!   `math_code_base+256`, because web2c's `tex.ch` inserts MLTeX's
//!   256-entry `char_sub_code` region ahead of region 5.
//! - `int_pars` is 62, not tex.web's 55: `tex.ch` adds MLTeX's three
//!   (`\charsubdefmin`, `\charsubdefmax`, `\tracingcharsubdef`) and
//!   `enctexdir/enctex2.ch` adds encTeX's four (`\mubytein`, `\mubyteout`,
//!   `\mubytelog`, `\specialout`).
//!
//! Every constant below is cross-checked against the committed document
//! traces (`tests/corpus/command/tex82-documents`), which record the pinned
//! oracle's own selector for a control sequence whose eqtb address is known
//! from plain.tex: `\m@ne` (`\countdef` 22) is 27251, `\count@`
//! (`\countdef` 255) is 27484, `\maxdimen` (`\newdimen`, `\dimen10`) is
//! 27772, `\hideskip` (`\newskip`, `\skip10`) is 24555, `\headline`
//! (`\newtoks`, `\toks10`) is 25077, and `\escapechar` (§236 code 45) is
//! 27212.
//!
//! ## Dialect
//!
//! TeX82 selectors are authoritative and verified as above. e-TeX selectors
//! come from the pinned `etex.ch` and are correct for a pure e-TeX 2.6
//! engine; pdfTeX renumbers the same quantities (it inserts its own
//! parameter block below e-TeX's), which this classifier does not model per
//! dialect, exactly as `primitive_identity` notes for `\jobname` and
//! `\eTeXrevision`. A selector with no code in the TeX82/e-TeX dialect --
//! every pdfTeX-only parameter, and Umber's own hidden cells -- is reported
//! as `None` rather than as a fabricated ordinal: the command family stays
//! exact and the missing selector is visible as a missing selector.

use tex_state::meaning::InternalInteger;

/// tex.web §224 `glue_base`, the first named glue parameter (`\lineskip`).
pub(crate) const GLUE_BASE: i64 = 24_527;
/// tex.web §224 `skip_base = glue_base + glue_pars`, `glue_pars = 18`.
pub(crate) const SKIP_BASE: i64 = GLUE_BASE + 18;
/// tex.web §224 `mu_skip_base = skip_base + 256`.
pub(crate) const MU_SKIP_BASE: i64 = SKIP_BASE + 256;
/// tex.web §224 `local_base = mu_skip_base + 256`, the start of region 4.
pub(crate) const LOCAL_BASE: i64 = MU_SKIP_BASE + 256;
/// tex.web §230 `output_routine_loc = local_base + 1`, the first token-list
/// parameter (`par_shape_loc` occupies `local_base` itself).
pub(crate) const OUTPUT_ROUTINE_LOC: i64 = LOCAL_BASE + 1;
/// tex.web §230 `toks_base = local_base + 10`, the 256 `\toks` registers.
///
/// e-TeX inserts `\everyeof` at this address and shifts `toks_base` up by
/// one; the pinned TeX82 oracle keeps tex.web's layout.
pub(crate) const TOKS_BASE: i64 = LOCAL_BASE + 10;
/// e-TeX `every_eof_loc = local_base + 10` (`etex.ch`'s `etex_toks_base`).
pub(crate) const EVERY_EOF_LOC: i64 = LOCAL_BASE + 10;
/// tex.web §236 `int_base`, the start of region 5, shifted by MLTeX's
/// `char_sub_code` region in the pinned oracle build (see the module docs).
pub(crate) const INT_BASE: i64 = 27_167;
/// tex.web §236 `count_base = int_base + int_pars`, `int_pars = 62` in the
/// pinned oracle build (see the module docs).
pub(crate) const COUNT_BASE: i64 = INT_BASE + 62;
/// tex.web §247 `dimen_base = del_code_base + 256 = count_base + 512`.
pub(crate) const DIMEN_BASE: i64 = COUNT_BASE + 512;
/// tex.web §247 `scaled_base = dimen_base + dimen_pars`, `dimen_pars = 21`.
pub(crate) const SCALED_BASE: i64 = DIMEN_BASE + 21;

/// Translates an Umber `IntParam` bank slot to tex.web's `int_par` code.
///
/// TeX82 codes are tex.web §236's; e-TeX codes are `etex.ch`'s
/// `etex_int_base = tex_int_pars = 55` block. `None` marks a slot Umber's
/// dense bank owns that the TeX82/e-TeX dialect has no `assign_int` selector
/// for (see the module documentation).
pub(crate) fn int_parameter_code(slot: u16) -> Option<i64> {
    // The left column is `crates/tex-exec/src/assignments/primitives.rs`'s
    // `INT_PARAMS` (plus its e-TeX and pdfTeX tables); the right column is
    // tex.web §236 / `etex.ch`.
    Some(match slot {
        0 => 0,   // \pretolerance
        1 => 1,   // \tolerance
        2 => 2,   // \linepenalty
        3 => 3,   // \hyphenpenalty
        4 => 4,   // \exhyphenpenalty
        5 => 5,   // \clubpenalty
        6 => 6,   // \widowpenalty
        7 => 7,   // \displaywidowpenalty
        8 => 8,   // \brokenpenalty
        9 => 9,   // \binoppenalty
        10 => 10, // \relpenalty
        11 => 11, // \predisplaypenalty
        12 => 12, // \postdisplaypenalty
        13 => 13, // \interlinepenalty
        14 => 14, // \doublehyphendemerits
        15 => 15, // \finalhyphendemerits
        16 => 16, // \adjdemerits
        17 => 17, // \mag
        18 => 18, // \delimiterfactor
        19 => 19, // \looseness
        20 => 20, // \time
        21 => 21, // \day
        22 => 22, // \month
        23 => 23, // \year
        24 => 24, // \showboxbreadth
        25 => 25, // \showboxdepth
        26 => 26, // \hbadness
        27 => 27, // \vbadness
        28 => 28, // \pausing
        29 => 29, // \tracingonline
        30 => 30, // \tracingmacros
        31 => 31, // \tracingstats
        32 => 43, // \globaldefs
        33 => 32, // \tracingparagraphs
        34 => 33, // \tracingpages
        35 => 34, // \tracingoutput
        36 => 35, // \tracinglostchars
        37 => 36, // \tracingcommands
        38 => 37, // \tracingrestores
        39 => 38, // \uchyph
        40 => 45, // \escapechar
        41 => 46, // \defaulthyphenchar
        42 => 47, // \defaultskewchar
        48 => 48, // \endlinechar
        49 => 49, // \newlinechar
        50 => 50, // \language
        51 => 51, // \lefthyphenmin
        52 => 52, // \righthyphenmin
        53 => 53, // \holdinginserts
        54 => 54, // \errorcontextlines
        55 => 39, // \outputpenalty
        56 => 40, // \maxdeadcycles
        57 => 41, // \hangafter
        58 => 42, // \floatingpenalty
        59 => 44, // \fam
        61 => 58, // \tracingscantokens (e-TeX)
        62 => 64, // \TeXXeTstate (e-TeX `eTeX_state_code`)
        63 => 60, // \predisplaydirection (e-TeX)
        64 => 55, // \tracingassigns (e-TeX)
        65 => 56, // \tracinggroups (e-TeX)
        66 => 57, // \tracingifs (e-TeX)
        67 => 59, // \tracingnesting (e-TeX)
        68 => 62, // \savingvdiscards (e-TeX)
        69 => 61, // \lastlinefit (e-TeX)
        70 => 63, // \savinghyphcodes (e-TeX)
        // 43..=47 and 60 are dense-bank cells with no `assign_int`
        // primitive: 60 stores `\badness`, which is read through
        // `last_item`, and 43..=47 are unallocated. 71 is Umber's hidden
        // e-TeX extended-mode flag, and 72..=109 are pdfTeX-only parameters
        // whose codes belong to pdfTeX's own renumbered block.
        _ => return None,
    })
}

/// Translates an Umber `DimenParam` bank slot to tex.web's `dimen_par` code.
///
/// tex.web §247's twenty-one dimension parameters are stored in Umber's bank
/// in tex.web's own order, so the TeX82 range is an identity map; pdfTeX's
/// additions (slots 21..=33) have no TeX82/e-TeX selector.
pub(crate) fn dimen_parameter_code(slot: u16) -> Option<i64> {
    (slot <= 20).then(|| i64::from(slot))
}

/// Translates an Umber `GlueParam`/`MuGlueParam` bank slot to tex.web's
/// `glue_par` code.
///
/// tex.web §224's eighteen glue parameters -- fifteen glue and the three mu
/// glue parameters `\thinmuskip`/`\medmuskip`/`\thickmuskip` (codes 15..17,
/// reached through `assign_mu_glue`) -- are stored in Umber's bank in
/// tex.web's order, so this is an identity map over the whole class.
pub(crate) fn glue_parameter_code(slot: u16) -> Option<i64> {
    (slot <= 17).then(|| i64::from(slot))
}

/// Translates an Umber `TokParam` bank slot to its tex.web §230 eqtb address.
///
/// The nine TeX82 token-list parameters are stored in tex.web's order
/// starting at `output_routine_loc`. `\everyeof` is e-TeX's and lives at its
/// own address; pdfTeX's four token parameters and Umber's five internal
/// shape-storage cells have no TeX82/e-TeX selector.
pub(crate) fn token_parameter_address(slot: u16) -> Option<i64> {
    match slot {
        0..=8 => Some(OUTPUT_ROUTINE_LOC + i64::from(slot)),
        13 => Some(EVERY_EOF_LOC),
        _ => None,
    }
}

/// One eqtb-addressed named-parameter class an observed mutation can name.
///
/// The reference instrumentation's `umber_trace_named_slot` names a mutated
/// parameter by *family* plus the parameter's position inside its own eqtb
/// region (`glue_parameter`, `token_parameter`, `integer_parameter`, and
/// `dimension_parameter`). That position is tex.web's parameter code, not a
/// dense-bank slot, so a mutation observation has to translate exactly as
/// `canonical_command_identity` does for a delivered `Meaning`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterClass {
    /// tex.web §236's `int_par` block, reached through `assign_int`.
    Integer,
    /// tex.web §247's `dimen_par` block, reached through `assign_dimen`.
    Dimension,
    /// tex.web §224's `glue_par` block, reached through `assign_glue` and
    /// `assign_mu_glue`; both commands share one region and one family name.
    Glue,
    /// tex.web §230's `local_base` token-list parameters, reached through
    /// `assign_toks`.
    Token,
}

/// Names a mutated named parameter the way the reference instrumentation's
/// `umber_trace_named_slot` does: `<family>:<tex.web parameter code>`.
///
/// This is the only supported way to build a parameter mutation key. Emitting
/// the raw `IntParam`/`DimenParam`/`GlueParam`/`TokParam` bank slot instead
/// silently misnames every integer parameter whose dense slot is not its
/// tex.web §236 code -- `\tracinglostchars` is Umber slot 36 and code 35,
/// `\defaulthyphenchar` is slot 41 and code 46 (umber2-johp.134).
///
/// A slot with no code in the TeX82/e-TeX dialect (pdfTeX-only parameters and
/// Umber's own hidden cells, see the module docs) is named with its bank slot
/// under an `umber` marker rather than a bare number. A bare number would be
/// indistinguishable from -- and could silently agree with -- a real code for
/// a different parameter. The record is still emitted: dropping it would
/// remove an event the oracle produces and desynchronize the whole trace.
pub fn parameter_mutation_key(class: ParameterClass, slot: u16) -> String {
    let (family, code) = match class {
        ParameterClass::Integer => ("integer_parameter", int_parameter_code(slot)),
        ParameterClass::Dimension => ("dimension_parameter", dimen_parameter_code(slot)),
        ParameterClass::Glue => ("glue_parameter", glue_parameter_code(slot)),
        ParameterClass::Token => (
            "token_parameter",
            // §230's token-list parameters are named by their offset from
            // `output_routine_loc`, the first one the instrumentation names.
            token_parameter_address(slot).map(|address| address - OUTPUT_ROUTINE_LOC),
        ),
    };
    match code {
        Some(code) => format!("{family}:{code}"),
        None => format!("{family}:umber{slot}"),
    }
}

/// Returns the `last_item` selector for a read-only internal integer.
///
/// tex.web §413 reads these through `last_item`; §416 gives TeX82's
/// `input_line_no_code`/`badness_code`, and `etex.ch` shifts both up by one
/// to make room for `\lastnodetype` before opening its own `eTeX_int` block.
/// The e-TeX numbering is used throughout, matching `primitive_identity`'s
/// `last_item` arms. pdfTeX's read-only integers have no selector in that
/// dialect (pdfTeX inserts its own block between `badness_code` and
/// `eTeX_int`), so they report no selector rather than a fabricated one.
pub(crate) fn internal_integer_code(integer: InternalInteger) -> Option<i64> {
    use InternalInteger as I;
    Some(match integer {
        I::LastNodeType => 3,
        I::InputLineNumber => 4,
        I::Badness => 5,
        I::ETeXVersion => 6,
        I::CurrentGroupLevel => 7,
        I::CurrentGroupType => 8,
        I::CurrentIfLevel => 9,
        I::CurrentIfType => 10,
        I::CurrentIfBranch => 11,
        I::PdfTeXVersion
        | I::PdfElapsedTime
        | I::PdfRandomSeed
        | I::PdfShellEscape
        | I::PdfLastObject
        | I::PdfLastAnnot
        | I::PdfLastLink
        | I::PdfLastXPos
        | I::PdfLastYPos
        | I::PdfLastXForm
        | I::PdfLastXImage
        | I::PdfReturnValue
        | I::PdfLastXImagePages
        | I::PdfLastXImageColorDepth => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_bases_match_the_pinned_oracle_probes() {
        // Every value here is a selector the committed TeX82 document traces
        // record for a control sequence whose eqtb address plain.tex fixes.
        assert_eq!(COUNT_BASE + 22, 27_251); // \m@ne, \countdef 22
        assert_eq!(COUNT_BASE + 255, 27_484); // \count@, \countdef 255
        assert_eq!(SCALED_BASE + 10, 27_772); // \maxdimen, \newdimen -> \dimen10
        assert_eq!(SKIP_BASE + 10, 24_555); // \hideskip, \newskip -> \skip10
        assert_eq!(TOKS_BASE + 10, 25_077); // \headline, \newtoks -> \toks10
        assert_eq!(DIMEN_BASE, 27_741); // \parindent
        assert_eq!(OUTPUT_ROUTINE_LOC, 25_058); // \output
        assert_eq!(GLUE_BASE + 15, 24_542); // \thinmuskip
    }

    #[test]
    fn int_parameter_codes_match_the_pinned_oracle_probes() {
        for (slot, selector) in [
            (0_u16, 27_167_i64), // \pretolerance
            (36, 27_202),        // \tracinglostchars
            (39, 27_205),        // \uchyph
            (40, 27_212),        // \escapechar
            (41, 27_213),        // \defaulthyphenchar
            (42, 27_214),        // \defaultskewchar
            (49, 27_216),        // \newlinechar
            (50, 27_217),        // \language
            (54, 27_221),        // \errorcontextlines
            (59, 27_211),        // \fam
        ] {
            assert_eq!(
                int_parameter_code(slot).map(|code| INT_BASE + code),
                Some(selector),
                "int parameter slot {slot}"
            );
        }
    }

    #[test]
    fn parameter_mutation_keys_name_tex_web_parameter_codes() {
        // The four parameters plain.tex assigns whose dense bank slot is not
        // their tex.web §236 code (umber2-johp.134).
        for (slot, key) in [
            (36_u16, "integer_parameter:35"), // \tracinglostchars
            (39, "integer_parameter:38"),     // \uchyph
            (41, "integer_parameter:46"),     // \defaulthyphenchar
            (42, "integer_parameter:47"),     // \defaultskewchar
            (32, "integer_parameter:43"),     // \globaldefs
            (59, "integer_parameter:44"),     // \fam
            (23, "integer_parameter:23"),     // \year, where the map is identity
        ] {
            assert_eq!(parameter_mutation_key(ParameterClass::Integer, slot), key);
        }
        // §247, §224, and §230 store their parameters in tex.web's own order,
        // so those classes name the slot itself.
        assert_eq!(
            parameter_mutation_key(ParameterClass::Dimension, 6),
            "dimension_parameter:6"
        );
        assert_eq!(
            parameter_mutation_key(ParameterClass::Glue, 11),
            "glue_parameter:11"
        );
        assert_eq!(
            parameter_mutation_key(ParameterClass::Token, 0),
            "token_parameter:0"
        );
    }

    #[test]
    fn parameter_mutation_keys_mark_slots_with_no_tex82_or_etex_code() {
        // Umber slot 72 is `\pdfoutput`, whose code belongs to pdfTeX's own
        // renumbered block; naming it `integer_parameter:72` would collide
        // with a real TeX82/e-TeX code for an unrelated parameter.
        assert_eq!(
            parameter_mutation_key(ParameterClass::Integer, 72),
            "integer_parameter:umber72"
        );
        assert_eq!(
            parameter_mutation_key(ParameterClass::Dimension, 21),
            "dimension_parameter:umber21"
        );
    }

    #[test]
    fn tex82_int_parameter_codes_are_a_bijection_onto_tex_web_int_pars() {
        let mut codes: Vec<i64> = (0..=59)
            .filter_map(int_parameter_code)
            .filter(|code| *code < 55)
            .collect();
        codes.sort_unstable();
        assert_eq!(codes, (0..55).collect::<Vec<_>>());
    }

    #[test]
    fn etex_int_parameter_codes_are_distinct_and_above_tex82() {
        let mut codes: Vec<i64> = (61..=70).filter_map(int_parameter_code).collect();
        codes.sort_unstable();
        assert_eq!(codes, (55..65).collect::<Vec<_>>());
    }
}
