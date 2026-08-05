use tex_state::Universe;
use tex_state::env::banks::{DimenParam, IntParam, TokParam};
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::InternalInteger;
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::page::{PageDimension, PageInteger};

/// Installs TeX82's unexpandable primitive table.
///
/// The names registered here, together with `tex-command`'s expandable table,
/// are exactly the 325 strings tex.web passes to `primitive(...)` (§264's
/// `@p @!init procedure primitive`), plus §1369's frozen `\endwrite`
/// sentinel. It is never a superset: a control sequence that plain.tex or
/// latex.ltx merely *defines* as a macro -- `\endgraf`, `\nointerlineskip`,
/// `\showhyphens`, and their kin -- must remain undefined until the format
/// source defines it, so `\let`/`\def` over that name reports §210's
/// `undefined_cs` (the `eq_type` §222 gives `undefined_control_sequence`)
/// exactly as the reference engine does.
pub fn install_unexpandable_primitives(stores: &mut Universe) {
    configure_unexpandable_primitives(stores, true);
}

/// Reconstructs TeX82's original primitive table without replacing meanings
/// restored from a format.
pub fn register_unexpandable_primitives(stores: &mut Universe) {
    configure_unexpandable_primitives(stores, false);
}

fn configure_unexpandable_primitives(stores: &mut Universe, install: bool) {
    if install {
        tex_command::install_tex82_unexpandable_primitives(stores);
    } else {
        tex_command::register_tex82_unexpandable_primitives(stores);
    }

    configure_primitive(stores, install, "relax", Meaning::Relax);
    let nullfont = stores.intern_internal_control_sequence("nullfont");
    let nullfont_meaning = Meaning::Font(tex_state::font::NULL_FONT);
    stores.register_primitive_meaning("nullfont", nullfont_meaning);
    if install {
        stores.set_meaning(nullfont, nullfont_meaning);
    }
    if install {
        stores.set_font_identifier_symbol(tex_state::font::NULL_FONT, nullfont);
        stores.set_current_font_selector_global(nullfont, tex_state::font::NULL_FONT);
    }
    configure_parameter_meanings(stores, install);
    configure_page_meanings(stores, install);
    configure_primitive(
        stores,
        install,
        "badness",
        Meaning::InternalInteger(InternalInteger::Badness),
    );
    configure_primitive(
        stores,
        install,
        "inputlineno",
        Meaning::InternalInteger(InternalInteger::InputLineNumber),
    );
    configure_write_stopper(stores);
}

/// Registers TeX82's inaccessible outer `\endwrite` sentinel with every fresh
/// and format-restored primitive table.
///
/// §222 reserves it at `end_write=frozen_control_sequence+8` and §1369 gives
/// it `text(end_write):="endwrite"` with `eq_type:=outer_call`, so it is the
/// one non-`primitive(...)` name this table owns. (The previous citation here,
/// "§53", is TeX.POOL's check sum and was never about `\endwrite`.)
fn configure_write_stopper(stores: &mut Universe) {
    if let Some(meaning) = stores.primitive_meaning("endwrite") {
        stores.register_primitive_meaning("endwrite", meaning);
        return;
    }
    let empty = stores.intern_token_list(&[]);
    let definition = stores.intern_macro(MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    stores.register_primitive_meaning(
        "endwrite",
        Meaning::Macro {
            flags: MeaningFlags::OUTER,
            definition,
        },
    );
}

fn configure_primitive(stores: &mut Universe, install: bool, name: &str, meaning: Meaning) {
    stores.register_primitive_meaning(name, meaning);
    if install {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, meaning);
    }
}

fn configure_parameter_meanings(stores: &mut Universe, install: bool) {
    for &(name, index) in INT_PARAMS {
        configure_primitive(stores, install, name, Meaning::IntParam(index));
    }
    for &(name, index) in DIMEN_PARAMS {
        configure_primitive(stores, install, name, Meaning::DimenParam(index));
    }
    for &(name, index) in GLUE_PARAMS {
        configure_primitive(stores, install, name, Meaning::GlueParam(index));
    }
    for &(name, index) in MU_GLUE_PARAMS {
        configure_primitive(stores, install, name, Meaning::MuGlueParam(index));
    }
    for &(name, index) in TOK_PARAMS {
        configure_primitive(stores, install, name, Meaning::TokParam(index));
    }
}

/// Installs unexpandable primitives that exist only in e-TeX extended mode.
pub fn install_etex_unexpandable_primitives(stores: &mut Universe) {
    stores.set_int_param_global(IntParam::ETEX_EXTENDED_MODE, 1);
    configure_etex_unexpandable_primitives(stores, true);
}

/// Reconstructs e-TeX's original primitive table after format load.
pub fn register_etex_unexpandable_primitives(stores: &mut Universe) {
    configure_etex_unexpandable_primitives(stores, false);
}

fn configure_etex_unexpandable_primitives(stores: &mut Universe, install: bool) {
    if install {
        tex_command::install_etex_unexpandable_primitives(stores);
    } else {
        tex_command::register_etex_unexpandable_primitives(stores);
    }

    configure_primitive(
        stores,
        install,
        "everyeof",
        Meaning::TokParam(TokParam::EVERY_EOF.raw()),
    );
    configure_primitive(
        stores,
        install,
        "tracingscantokens",
        Meaning::IntParam(IntParam::TRACING_SCAN_TOKENS.raw()),
    );
    for &(name, parameter) in ETEX_INT_PARAMS {
        configure_primitive(stores, install, name, Meaning::IntParam(parameter.raw()));
    }
}

/// e-TeX 2.6's own integer parameters, kept separate from TeX82's
/// [`INT_PARAMS`] because they are installed by a different primitive table
/// (`configure_etex_unexpandable_primitives`, e-TeX-extended-mode only) but
/// still need a name for `\tracingassigns`'s [`int_param_name`].
const ETEX_INT_PARAMS: &[(&str, IntParam)] = &[
    ("TeXXeTstate", IntParam::TEX_XET_STATE),
    ("predisplaydirection", IntParam::PRE_DISPLAY_DIRECTION),
    ("tracingassigns", IntParam::TRACING_ASSIGNS),
    ("tracinggroups", IntParam::TRACING_GROUPS),
    ("tracingifs", IntParam::TRACING_IFS),
    ("tracingnesting", IntParam::TRACING_NESTING),
    ("savingvdiscards", IntParam::SAVING_V_DISCARDS),
    ("lastlinefit", IntParam::LAST_LINE_FIT),
    ("savinghyphcodes", IntParam::SAVING_HYPH_CODES),
    ("tracingscantokens", IntParam::TRACING_SCAN_TOKENS),
];

fn configure_page_meanings(stores: &mut Universe, install: bool) {
    for &(name, dimension) in PAGE_DIMENSIONS {
        configure_primitive(stores, install, name, Meaning::PageDimension(dimension));
    }
    for &(name, integer) in PAGE_INTEGERS {
        configure_primitive(stores, install, name, Meaning::PageInteger(integer));
    }
}

const INT_PARAMS: &[(&str, u16)] = &[
    ("pretolerance", 0),
    ("tolerance", 1),
    ("linepenalty", 2),
    ("hyphenpenalty", 3),
    ("exhyphenpenalty", 4),
    ("clubpenalty", 5),
    ("widowpenalty", 6),
    ("displaywidowpenalty", 7),
    ("brokenpenalty", 8),
    ("binoppenalty", 9),
    ("relpenalty", 10),
    ("predisplaypenalty", 11),
    ("postdisplaypenalty", 12),
    ("interlinepenalty", 13),
    ("doublehyphendemerits", 14),
    ("finalhyphendemerits", 15),
    ("adjdemerits", 16),
    ("mag", IntParam::MAG.raw()),
    ("delimiterfactor", IntParam::DELIMITER_FACTOR.raw()),
    ("looseness", 19),
    ("time", 20),
    ("day", 21),
    ("month", 22),
    ("year", 23),
    ("showboxbreadth", 24),
    ("showboxdepth", 25),
    ("hbadness", 26),
    ("vbadness", 27),
    ("pausing", IntParam::PAUSING.raw()),
    ("tracingonline", IntParam::TRACING_ONLINE.raw()),
    ("tracingmacros", 30),
    ("tracingstats", 31),
    ("globaldefs", IntParam::GLOBAL_DEFS.raw()),
    ("tracingparagraphs", 33),
    ("tracingpages", IntParam::TRACING_PAGES.raw()),
    ("tracingoutput", IntParam::TRACING_OUTPUT.raw()),
    ("tracinglostchars", 36),
    ("tracingcommands", IntParam::TRACING_COMMANDS.raw()),
    ("tracingrestores", 38),
    ("uchyph", 39),
    ("escapechar", IntParam::ESCAPE_CHAR.raw()),
    ("defaulthyphenchar", 41),
    ("defaultskewchar", 42),
    ("endlinechar", IntParam::END_LINE_CHAR.raw()),
    ("newlinechar", IntParam::NEWLINE_CHAR.raw()),
    ("language", 50),
    ("lefthyphenmin", 51),
    ("righthyphenmin", 52),
    ("holdinginserts", 53),
    ("errorcontextlines", 54),
    ("outputpenalty", 55),
    ("maxdeadcycles", 56),
    ("hangafter", 57),
    ("floatingpenalty", 58),
    ("fam", IntParam::FAM.raw()),
];

const DIMEN_PARAMS: &[(&str, u16)] = &[
    ("parindent", 0),
    ("mathsurround", 1),
    ("lineskiplimit", 2),
    ("hsize", 3),
    ("vsize", 4),
    ("maxdepth", 5),
    ("splitmaxdepth", 6),
    ("boxmaxdepth", 7),
    ("hfuzz", 8),
    ("vfuzz", 9),
    ("delimitershortfall", DimenParam::DELIMITER_SHORTFALL.raw()),
    ("nulldelimiterspace", DimenParam::NULL_DELIMITER_SPACE.raw()),
    ("scriptspace", 12),
    ("predisplaysize", 13),
    ("displaywidth", 14),
    ("displayindent", 15),
    ("overfullrule", 16),
    ("hangindent", 17),
    ("hoffset", 18),
    ("voffset", 19),
    ("emergencystretch", 20),
];

const GLUE_PARAMS: &[(&str, u16)] = &[
    ("lineskip", 0),
    ("baselineskip", 1),
    ("parskip", 2),
    ("abovedisplayskip", 3),
    ("belowdisplayskip", 4),
    ("abovedisplayshortskip", 5),
    ("belowdisplayshortskip", 6),
    ("leftskip", 7),
    ("rightskip", 8),
    ("topskip", 9),
    ("splittopskip", 10),
    ("tabskip", 11),
    ("spaceskip", 12),
    ("xspaceskip", 13),
    ("parfillskip", 14),
];

const MU_GLUE_PARAMS: &[(&str, u16)] =
    &[("thinmuskip", 15), ("medmuskip", 16), ("thickmuskip", 17)];

const TOK_PARAMS: &[(&str, u16)] = &[
    ("output", 0),
    ("everypar", 1),
    ("everymath", 2),
    ("everydisplay", 3),
    ("everyhbox", 4),
    ("everyvbox", 5),
    ("everyjob", 6),
    ("everycr", 7),
    ("errhelp", TokParam::ERR_HELP.raw()),
];

/// e-TeX `\tracingassigns`'s `show_eqtb`-equivalent parameter name, used to
/// render its `{into ...}`/`{changing ...}`/`{reassigning ...}` trace lines.
/// The fallback for an index this table does not name should be unreachable
/// in practice -- every declared parameter index is listed here -- but never
/// panics, matching this table's role as a display aid rather than a gate.
pub(crate) fn int_param_name(index: u16) -> String {
    if let Some((name, _)) = INT_PARAMS.iter().find(|(_, i)| *i == index) {
        return (*name).to_owned();
    }
    if let Some((name, _)) = ETEX_INT_PARAMS
        .iter()
        .find(|(_, parameter)| parameter.raw() == index)
    {
        return (*name).to_owned();
    }
    format!("IntParam{index}")
}

pub(crate) fn dimen_param_name(index: u16) -> String {
    lookup_name(DIMEN_PARAMS, index, "DimenParam")
}

pub(crate) fn tok_param_name(index: u16) -> String {
    lookup_name(TOK_PARAMS, index, "TokParam")
}

/// Looks up a glue parameter's name and unit ("pt" for ordinary glue
/// parameters, "mu" for the three math-glue parameters that share the same
/// index space per e-TeX 2.6 [20.281]'s `glue_pars` layout).
pub(crate) fn glue_param_name(index: u16) -> (String, &'static str) {
    if let Some((name, _)) = GLUE_PARAMS.iter().find(|(_, i)| *i == index) {
        return ((*name).to_owned(), "pt");
    }
    if let Some((name, _)) = MU_GLUE_PARAMS.iter().find(|(_, i)| *i == index) {
        return ((*name).to_owned(), "mu");
    }
    (format!("GlueParam{index}"), "pt")
}

fn lookup_name(table: &[(&str, u16)], index: u16, fallback: &str) -> String {
    table.iter().find(|(_, i)| *i == index).map_or_else(
        || format!("{fallback}{index}"),
        |(name, _)| (*name).to_owned(),
    )
}

const PAGE_DIMENSIONS: &[(&str, PageDimension)] = &[
    ("pagegoal", PageDimension::Goal),
    ("pagetotal", PageDimension::Total),
    ("pagestretch", PageDimension::Stretch),
    ("pagefilstretch", PageDimension::FilStretch),
    ("pagefillstretch", PageDimension::FillStretch),
    ("pagefilllstretch", PageDimension::FilllStretch),
    ("pageshrink", PageDimension::Shrink),
    ("pagedepth", PageDimension::Depth),
];

const PAGE_INTEGERS: &[(&str, PageInteger)] = &[
    ("deadcycles", PageInteger::DeadCycles),
    ("insertpenalties", PageInteger::InsertPenalties),
];
