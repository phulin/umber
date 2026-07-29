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
//! - TeX82's `int_pars` is 62, not tex.web's 55: `tex.ch` adds MLTeX's three
//!   (`\charsubdefmin`, `\charsubdefmax`, `\tracingcharsubdef`) and
//!   `enctexdir/enctex2.ch` adds encTeX's four (`\mubytein`, `\mubyteout`,
//!   `\mubytelog`, `\specialout`).
//! - e-TeX's `int_pars` is 73 in the pinned build: e-TeX [17.236] appends
//!   its ten cells to those 62, then `synctex-e-mem.ch1` appends
//!   `\synctex` before `count_base`.
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
//! The immutable command profile selects one complete layout. TeX82 follows
//! tex.web §§224/230/236/247; e-TeX 2.6's `etex.ch` inserts `\everyeof`, four
//! penalty-list cells, and its integer block; pdfTeX 1.40.27's
//! `pdftexdir/pdftex.web` §§5406, 5714-5727, and 9804-9829 additionally
//! inserts four pdf token parameters, 37 pdf integer parameters, and fourteen
//! read-only integer selectors. Umber-only hidden cells return `None`; no
//! selector from one dialect is reused as an approximation for another.

use tex_state::meaning::InternalInteger;

use crate::CommandDialect;

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
/// The TeX82 address; [`toks_base`] applies dialect insertions.
pub(crate) const TOKS_BASE: i64 = LOCAL_BASE + 10;
/// e-TeX `every_eof_loc = local_base + 10` (`etex.ch`'s `etex_toks_base`).
pub(crate) const EVERY_EOF_LOC: i64 = LOCAL_BASE + 10;
/// tex.web §236 `int_base`, the start of region 5, shifted by MLTeX's
/// `char_sub_code` region in the pinned oracle build (see the module docs).
pub(crate) const INT_BASE: i64 = 27_167;
/// Translates an Umber `IntParam` bank slot to tex.web's `int_par` code.
///
/// TeX82 codes are tex.web §236's; e-TeX codes are `etex.ch`'s
/// `etex_int_base` block, shifted after the pinned Web2C build's seven
/// MLTeX/encTeX integer parameters. `None` marks a slot Umber's dense bank
/// owns that the TeX82/e-TeX dialect has no `assign_int` selector for (see the
/// module documentation).
pub(crate) fn int_parameter_code(dialect: CommandDialect, slot: u16) -> Option<i64> {
    // The left column is `crates/tex-exec/src/assignments/primitives.rs`'s
    // `INT_PARAMS` (plus its e-TeX and pdfTeX tables); the right column is
    // tex.web §236 / `etex.ch`.
    Some(match slot {
        0 => 0,                            // \pretolerance
        1 => 1,                            // \tolerance
        2 => 2,                            // \linepenalty
        3 => 3,                            // \hyphenpenalty
        4 => 4,                            // \exhyphenpenalty
        5 => 5,                            // \clubpenalty
        6 => 6,                            // \widowpenalty
        7 => 7,                            // \displaywidowpenalty
        8 => 8,                            // \brokenpenalty
        9 => 9,                            // \binoppenalty
        10 => 10,                          // \relpenalty
        11 => 11,                          // \predisplaypenalty
        12 => 12,                          // \postdisplaypenalty
        13 => 13,                          // \interlinepenalty
        14 => 14,                          // \doublehyphendemerits
        15 => 15,                          // \finalhyphendemerits
        16 => 16,                          // \adjdemerits
        17 => 17,                          // \mag
        18 => 18,                          // \delimiterfactor
        19 => 19,                          // \looseness
        20 => 20,                          // \time
        21 => 21,                          // \day
        22 => 22,                          // \month
        23 => 23,                          // \year
        24 => 24,                          // \showboxbreadth
        25 => 25,                          // \showboxdepth
        26 => 26,                          // \hbadness
        27 => 27,                          // \vbadness
        28 => 28,                          // \pausing
        29 => 29,                          // \tracingonline
        30 => 30,                          // \tracingmacros
        31 => 31,                          // \tracingstats
        32 => 43,                          // \globaldefs
        33 => 32,                          // \tracingparagraphs
        34 => 33,                          // \tracingpages
        35 => 34,                          // \tracingoutput
        36 => 35,                          // \tracinglostchars
        37 => 36,                          // \tracingcommands
        38 => 37,                          // \tracingrestores
        39 => 38,                          // \uchyph
        40 => 45,                          // \escapechar
        41 => 46,                          // \defaulthyphenchar
        42 => 47,                          // \defaultskewchar
        48 => 48,                          // \endlinechar
        49 => 49,                          // \newlinechar
        50 => 50,                          // \language
        51 => 51,                          // \lefthyphenmin
        52 => 52,                          // \righthyphenmin
        53 => 53,                          // \holdinginserts
        54 => 54,                          // \errorcontextlines
        55 => 39,                          // \outputpenalty
        56 => 40,                          // \maxdeadcycles
        57 => 41,                          // \hangafter
        58 => 42,                          // \floatingpenalty
        59 => 44,                          // \fam
        61 => etex_int_base(dialect)? + 3, // \tracingscantokens
        62 => {
            etex_int_base(dialect)?
                + if matches!(dialect, CommandDialect::Pdftex14027) {
                    10
                } else {
                    9
                }
        }
        63 => etex_int_base(dialect)? + 5, // \predisplaydirection
        64 => etex_int_base(dialect)?,     // \tracingassigns
        65 => etex_int_base(dialect)? + 1, // \tracinggroups
        66 => etex_int_base(dialect)? + 2, // \tracingifs
        67 => etex_int_base(dialect)? + 4, // \tracingnesting
        68 => etex_int_base(dialect)? + 7, // \savingvdiscards
        69 => etex_int_base(dialect)? + 6, // \lastlinefit
        70 => etex_int_base(dialect)? + 8, // \savinghyphcodes
        72..=109 if matches!(dialect, CommandDialect::Pdftex14027) => match slot {
            72 => 55,   // \pdfoutput
            73 => 56,   // \pdfcompresslevel
            74 => 76,   // \pdfobjcompresslevel
            75 => 57,   // \pdfdecimaldigits
            76 => 58,   // \pdfmovechars
            77 => 59,   // \pdfimageresolution
            78 => 60,   // \pdfpkresolution
            79 => 61,   // \pdfuniqueresname
            80 => 65,   // \pdfminorversion
            81 => 66,   // \pdfforcepagebox
            82 => 67,   // \pdfpagebox
            83 => 68,   // \pdfinclusionerrorlevel
            84 => 64,   // \pdfmajorversion
            85 => 69,   // \pdfgamma
            86 => 70,   // \pdfimagegamma
            87 => 71,   // \pdfimagehicolor
            88 => 72,   // \pdfimageapplygamma
            89 => 73,   // \pdfadjustspacing
            90 => 74,   // \pdfprotrudechars
            91 => 75,   // \pdftracingfonts
            92 => 77,   // \pdfadjustinterwordglue
            93 => 78,   // \pdfprependkern
            94 => 79,   // \pdfappendkern
            95 => 80,   // \pdfgentounicode
            96 => 81,   // \pdfdraftmode
            97 => 82,   // \pdfinclusioncopyfonts
            98 => 83,   // \pdfsuppresswarningdupdest
            99 => 84,   // \pdfsuppresswarningdupmap
            100 => 85,  // \pdfsuppresswarningpagegroup
            101 => 86,  // \pdfinfoomitdate
            102 => 87,  // \pdfsuppressptexinfo
            103 => 88,  // \pdfomitcharset
            104 => 89,  // \pdfomitinfodict
            105 => 90,  // \pdfomitprocset
            106 => 91,  // \pdfptexuseunderscore
            107 => 62,  // \pdfoptionalwaysusepdfpagebox
            108 => 63,  // \pdfoptionpdfinclusionerrorlevel
            109 => 101, // \ignoreprimitiveerror (`etex_int_base+9`)
            _ => unreachable!(),
        },
        // 43..=47 and 60 are dense-bank cells with no `assign_int`
        // primitive: 60 stores `\badness`, which is read through
        // `last_item`, and 43..=47 are unallocated. 71 is Umber's hidden
        // e-TeX extended-mode flag, and 72..=109 are pdfTeX-only parameters
        // whose codes belong to pdfTeX's own renumbered block.
        _ => return None,
    })
}

const fn etex_int_base(dialect: CommandDialect) -> Option<i64> {
    match dialect {
        CommandDialect::Tex82 => None,
        // etex.ch [17.236] starts this block after TeX's 55 parameters.
        // The pinned Web2C change chain inserts three MLTeX and four encTeX
        // parameters first, so its effective `etex_int_base` is 62.
        CommandDialect::Etex26 => Some(62),
        CommandDialect::Pdftex14027 => Some(92),
    }
}

pub(crate) const fn int_base(dialect: CommandDialect) -> i64 {
    INT_BASE
        + match dialect {
            CommandDialect::Tex82 => 0,
            CommandDialect::Etex26 => 5,
            CommandDialect::Pdftex14027 => 9,
        }
}

pub(crate) const fn count_base(dialect: CommandDialect) -> i64 {
    int_base(dialect)
        + match dialect {
            CommandDialect::Tex82 => 62,
            // e-TeX [17.236] supplies 72 cells after the pinned Web2C and
            // encTeX changes; synctex-e-mem.ch1 appends `\synctex` as the
            // seventy-third before tex.web §236 derives `count_base`.
            CommandDialect::Etex26 => 73,
            CommandDialect::Pdftex14027 => 110,
        }
}

pub(crate) const fn dimen_base(dialect: CommandDialect) -> i64 {
    count_base(dialect) + 512
}

pub(crate) const fn scaled_base(dialect: CommandDialect) -> i64 {
    dimen_base(dialect) + 21
}

pub(crate) const fn toks_base(dialect: CommandDialect) -> i64 {
    TOKS_BASE
        + match dialect {
            CommandDialect::Tex82 => 0,
            CommandDialect::Etex26 => 1,
            CommandDialect::Pdftex14027 => 5,
        }
}

/// Translates an Umber `DimenParam` bank slot to tex.web's `dimen_par` code.
///
/// tex.web §247's twenty-one dimension parameters are stored in Umber's bank
/// in tex.web's own order, so the TeX82 range is an identity map; pdfTeX's
/// additions (slots 21..=33) have no TeX82/e-TeX selector.
pub(crate) fn dimen_parameter_code(slot: u16) -> Option<i64> {
    (slot <= 20).then(|| i64::from(slot))
}

/// Returns the observed eqtb selector for a named dimension parameter.
///
/// e-TeX 2.6 [17.236] defines these selectors from `dimen_base`. The pinned
/// SyncTeX change shifts that base, its named parameters, and every following
/// dimension register together, so the region-chain functions above remain
/// the one owner of the insertion.
pub(crate) fn dimen_parameter_address(dialect: CommandDialect, slot: u16) -> Option<i64> {
    dimen_parameter_code(slot).map(|code| dimen_base(dialect) + code)
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
pub(crate) fn token_parameter_address(dialect: CommandDialect, slot: u16) -> Option<i64> {
    match slot {
        0..=8 => Some(OUTPUT_ROUTINE_LOC + i64::from(slot)),
        9..=12 if matches!(dialect, CommandDialect::Pdftex14027) => {
            Some(LOCAL_BASE + 10 + i64::from(slot - 9))
        }
        13 if !matches!(dialect, CommandDialect::Tex82) => Some(
            EVERY_EOF_LOC
                + if matches!(dialect, CommandDialect::Pdftex14027) {
                    4
                } else {
                    0
                },
        ),
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
    parameter_mutation_key_for_dialect(CommandDialect::Tex82, class, slot)
}

pub fn parameter_mutation_key_for_dialect(
    dialect: CommandDialect,
    class: ParameterClass,
    slot: u16,
) -> String {
    if matches!(class, ParameterClass::Integer)
        && let Some(name) = extension_integer_parameter_name(dialect, slot)
    {
        return name.into();
    }
    let (family, code) = match class {
        ParameterClass::Integer => ("integer_parameter", int_parameter_code(dialect, slot)),
        ParameterClass::Dimension => ("dimension_parameter", dimen_parameter_code(slot)),
        ParameterClass::Glue => ("glue_parameter", glue_parameter_code(slot)),
        ParameterClass::Token => (
            "token_parameter",
            // §230's token-list parameters are named by their offset from
            // `output_routine_loc`, the first one the instrumentation names.
            token_parameter_address(dialect, slot).map(|address| address - OUTPUT_ROUTINE_LOC),
        ),
    };
    match code {
        Some(code) => format!("{family}:{code}"),
        None => format!("{family}:umber{slot}"),
    }
}

/// Names the complete e-TeX integer block the way the reference
/// instrumentation's `umber_trace_named_slot` does.
///
/// `etex.ch` [17.236] assigns semantic names to all ten externally reachable
/// cells instead of exposing their build-dependent offsets as
/// `integer_parameter:<n>`. pdfTeX carries the same e-TeX block at a different
/// offset; the Umber bank slots remain common across both profiles.
const fn extension_integer_parameter_name(
    dialect: CommandDialect,
    slot: u16,
) -> Option<&'static str> {
    if matches!(dialect, CommandDialect::Tex82) {
        return None;
    }
    Some(match slot {
        61 => "tracingscantokens",
        62 => "TeXXeTstate",
        63 => "predisplaydirection",
        64 => "tracingassigns",
        65 => "tracinggroups",
        66 => "tracingifs",
        67 => "tracingnesting",
        68 => "savingvdiscards",
        69 => "lastlinefit",
        70 => "savinghyphcodes",
        _ => return None,
    })
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
pub(crate) fn internal_integer_code(
    dialect: CommandDialect,
    integer: InternalInteger,
) -> Option<i64> {
    use InternalInteger as I;
    Some(match integer {
        I::LastNodeType if !matches!(dialect, CommandDialect::Tex82) => 3,
        I::LastNodeType => return None,
        I::InputLineNumber => {
            if matches!(dialect, CommandDialect::Tex82) {
                3
            } else {
                4
            }
        }
        I::Badness => {
            if matches!(dialect, CommandDialect::Tex82) {
                4
            } else {
                5
            }
        }
        I::ETeXVersion => etex_last_item_base(dialect)?,
        I::CurrentGroupLevel => etex_last_item_base(dialect)? + 1,
        I::CurrentGroupType => etex_last_item_base(dialect)? + 2,
        I::CurrentIfLevel => etex_last_item_base(dialect)? + 3,
        I::CurrentIfType => etex_last_item_base(dialect)? + 4,
        I::CurrentIfBranch => etex_last_item_base(dialect)? + 5,
        I::PdfTeXVersion => pdf_last_item_base(dialect)?,
        I::PdfLastObject => pdf_last_item_base(dialect)? + 1,
        I::PdfLastXForm => pdf_last_item_base(dialect)? + 2,
        I::PdfLastXImage => pdf_last_item_base(dialect)? + 3,
        I::PdfLastXImagePages => pdf_last_item_base(dialect)? + 4,
        I::PdfLastAnnot => pdf_last_item_base(dialect)? + 5,
        I::PdfLastXPos => pdf_last_item_base(dialect)? + 6,
        I::PdfLastYPos => pdf_last_item_base(dialect)? + 7,
        I::PdfReturnValue => pdf_last_item_base(dialect)? + 8,
        I::PdfLastXImageColorDepth => pdf_last_item_base(dialect)? + 9,
        I::PdfElapsedTime => pdf_last_item_base(dialect)? + 10,
        I::PdfShellEscape => pdf_last_item_base(dialect)? + 11,
        I::PdfRandomSeed => pdf_last_item_base(dialect)? + 12,
        I::PdfLastLink => pdf_last_item_base(dialect)? + 13,
    })
}

const fn pdf_last_item_base(dialect: CommandDialect) -> Option<i64> {
    if matches!(dialect, CommandDialect::Pdftex14027) {
        Some(6)
    } else {
        None
    }
}

const fn etex_last_item_base(dialect: CommandDialect) -> Option<i64> {
    match dialect {
        CommandDialect::Tex82 => None,
        CommandDialect::Etex26 => Some(6),
        CommandDialect::Pdftex14027 => Some(20),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_bases_match_the_pinned_oracle_probes() {
        // Every value here is a selector the committed TeX82 document traces
        // record for a control sequence whose eqtb address plain.tex fixes.
        assert_eq!(count_base(CommandDialect::Tex82) + 22, 27_251); // \m@ne
        assert_eq!(count_base(CommandDialect::Tex82) + 255, 27_484); // \count@
        assert_eq!(scaled_base(CommandDialect::Tex82) + 10, 27_772); // \maxdimen
        assert_eq!(SKIP_BASE + 10, 24_555); // \hideskip, \newskip -> \skip10
        assert_eq!(TOKS_BASE + 10, 25_077); // \headline, \newtoks -> \toks10
        assert_eq!(dimen_base(CommandDialect::Tex82), 27_741); // \parindent
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
                int_parameter_code(CommandDialect::Tex82, slot).map(|code| INT_BASE + code),
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
            .filter_map(|slot| int_parameter_code(CommandDialect::Tex82, slot))
            .filter(|code| *code < 55)
            .collect();
        codes.sort_unstable();
        assert_eq!(codes, (0..55).collect::<Vec<_>>());
    }

    #[test]
    fn etex_int_parameter_codes_follow_the_complete_pinned_web2c_block() {
        let mut codes: Vec<i64> = (61..=70)
            .filter_map(|slot| int_parameter_code(CommandDialect::Etex26, slot))
            .collect();
        codes.sort_unstable();
        assert_eq!(codes, (62..72).collect::<Vec<_>>());
        assert!(
            (61..=70).all(|slot| { int_parameter_code(CommandDialect::Tex82, slot).is_none() })
        );
    }

    #[test]
    fn extension_integer_mutation_names_are_profile_bounded_and_exhaustive() {
        let expected = [
            "tracingscantokens",
            "TeXXeTstate",
            "predisplaydirection",
            "tracingassigns",
            "tracinggroups",
            "tracingifs",
            "tracingnesting",
            "savingvdiscards",
            "lastlinefit",
            "savinghyphcodes",
        ];
        for (slot, name) in (61_u16..=70).zip(expected) {
            assert_eq!(
                parameter_mutation_key_for_dialect(
                    CommandDialect::Etex26,
                    ParameterClass::Integer,
                    slot,
                ),
                name
            );
            assert_eq!(
                parameter_mutation_key_for_dialect(
                    CommandDialect::Pdftex14027,
                    ParameterClass::Integer,
                    slot,
                ),
                name
            );
            assert_eq!(
                parameter_mutation_key_for_dialect(
                    CommandDialect::Tex82,
                    ParameterClass::Integer,
                    slot,
                ),
                format!("integer_parameter:umber{slot}")
            );
        }
    }

    #[test]
    fn dialect_layouts_follow_the_pinned_eqtb_chains() {
        for (dialect, expected) in [
            (
                CommandDialect::Tex82,
                (25_067, 27_167, 27_229, 27_741, 27_762),
            ),
            (
                CommandDialect::Etex26,
                (25_068, 27_172, 27_245, 27_757, 27_778),
            ),
            (
                CommandDialect::Pdftex14027,
                (25_072, 27_176, 27_286, 27_798, 27_819),
            ),
        ] {
            assert_eq!(
                (
                    toks_base(dialect),
                    int_base(dialect),
                    count_base(dialect),
                    dimen_base(dialect),
                    scaled_base(dialect),
                ),
                expected,
                "{dialect:?}"
            );
        }
    }

    #[test]
    fn etex_synctex_cell_shifts_register_regions_after_named_integers() {
        // e-TeX [17.236] ends its integer block at 72 in the pinned
        // MLTeX/encTeX build. `synctex-e-mem.ch1` then defines
        // `synctex_code=etex_int_pars; int_pars=synctex_code+1`, so tex.web
        // §236 derives every later register base from 73 cells. The translated
        // oracle independently records these exact selectors.
        assert_eq!(int_base(CommandDialect::Etex26), 27_172);
        assert_eq!(count_base(CommandDialect::Etex26), 27_245);
        assert_eq!(count_base(CommandDialect::Etex26) + 255, 27_500);
        assert_eq!(dimen_base(CommandDialect::Etex26), 27_757);
        assert_eq!(scaled_base(CommandDialect::Etex26) + 255, 28_033);
    }

    #[test]
    fn dimension_parameter_and_register_banks_are_profile_exact_and_bounded() {
        for (dialect, parameter_base) in [
            (CommandDialect::Tex82, 27_741_i64),
            (CommandDialect::Etex26, 27_757),
            (CommandDialect::Pdftex14027, 27_798),
        ] {
            for slot in 0_u16..=20 {
                assert_eq!(
                    dimen_parameter_address(dialect, slot),
                    Some(parameter_base + i64::from(slot)),
                    "{dialect:?} dimension parameter slot {slot}"
                );
            }
            assert_eq!(dimen_parameter_code(21), None, "{dialect:?}");

            for register in 0_u16..=255 {
                assert_eq!(
                    scaled_base(dialect) + i64::from(register),
                    dimen_base(dialect) + 21 + i64::from(register),
                    "{dialect:?} dimension register {register}"
                );
            }
        }
    }

    #[test]
    fn everyeof_and_pdf_token_parameters_have_dialect_addresses() {
        assert_eq!(token_parameter_address(CommandDialect::Tex82, 13), None);
        assert_eq!(
            token_parameter_address(CommandDialect::Etex26, 13),
            Some(25_067)
        );
        for (slot, address) in (9_u16..=12).zip(25_067_i64..=25_070) {
            assert_eq!(
                token_parameter_address(CommandDialect::Pdftex14027, slot),
                Some(address)
            );
        }
        assert_eq!(
            token_parameter_address(CommandDialect::Pdftex14027, 13),
            Some(25_071)
        );
    }

    #[test]
    fn every_pdf_integer_parameter_uses_its_pdftex_code() {
        let mut codes: Vec<_> = (72_u16..=108)
            .map(|slot| int_parameter_code(CommandDialect::Pdftex14027, slot))
            .collect();
        codes.sort_unstable();
        assert_eq!(codes, (55_i64..=91).map(Some).collect::<Vec<_>>());
        assert_eq!(int_parameter_code(CommandDialect::Etex26, 72), None);
        assert_eq!(
            int_parameter_code(CommandDialect::Pdftex14027, 109),
            Some(101)
        );
    }

    #[test]
    fn last_item_integer_blocks_are_profile_exact() {
        use InternalInteger as I;
        for (integer, tex, etex, pdftex) in [
            (I::LastNodeType, None, Some(3), Some(3)),
            (I::InputLineNumber, Some(3), Some(4), Some(4)),
            (I::Badness, Some(4), Some(5), Some(5)),
            (I::ETeXVersion, None, Some(6), Some(20)),
            (I::CurrentGroupLevel, None, Some(7), Some(21)),
            (I::CurrentGroupType, None, Some(8), Some(22)),
            (I::CurrentIfLevel, None, Some(9), Some(23)),
            (I::CurrentIfType, None, Some(10), Some(24)),
            (I::CurrentIfBranch, None, Some(11), Some(25)),
            (I::PdfTeXVersion, None, None, Some(6)),
            (I::PdfLastObject, None, None, Some(7)),
            (I::PdfLastXForm, None, None, Some(8)),
            (I::PdfLastXImage, None, None, Some(9)),
            (I::PdfLastXImagePages, None, None, Some(10)),
            (I::PdfLastAnnot, None, None, Some(11)),
            (I::PdfLastXPos, None, None, Some(12)),
            (I::PdfLastYPos, None, None, Some(13)),
            (I::PdfReturnValue, None, None, Some(14)),
            (I::PdfLastXImageColorDepth, None, None, Some(15)),
            (I::PdfElapsedTime, None, None, Some(16)),
            (I::PdfShellEscape, None, None, Some(17)),
            (I::PdfRandomSeed, None, None, Some(18)),
            (I::PdfLastLink, None, None, Some(19)),
        ] {
            assert_eq!(internal_integer_code(CommandDialect::Tex82, integer), tex);
            assert_eq!(internal_integer_code(CommandDialect::Etex26, integer), etex);
            assert_eq!(
                internal_integer_code(CommandDialect::Pdftex14027, integer),
                pdftex
            );
        }
    }
}
