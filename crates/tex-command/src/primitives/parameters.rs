//! Generated parameter-primitive and initial-default views.

use tex_state::env::banks::{DimenParam, IntParam, TokParam};
use tex_state::meaning::Meaning;

use super::{
    GlueParameterDefault, InstallationPolicy, JobClockField, ParameterBankClass, ParameterCell,
    ParameterDefault, PrimitiveProfile,
};

const ZERO_GLUE: GlueParameterDefault = GlueParameterDefault {
    width: 0,
    stretch: 0,
    stretch_order: 0,
    shrink: 0,
    shrink_order: 0,
};

/// One parameter spelling, cell, meaning, and fresh-engine default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrimitiveParameterView {
    pub name: &'static str,
    pub profile: PrimitiveProfile,
    pub cell: ParameterCell,
    pub meaning: Meaning,
    pub default: ParameterDefault,
    pub installation: InstallationPolicy,
}

/// Returns parameter rows in primitive-installation order for one profile
/// layer. A composed engine profile concatenates the required layer views.
#[must_use]
pub fn primitive_parameter_views(profile: PrimitiveProfile) -> Vec<PrimitiveParameterView> {
    match profile {
        PrimitiveProfile::Tex82 => tex82_parameters(),
        PrimitiveProfile::Etex26 => etex_parameters(),
        PrimitiveProfile::LatexCompatibility => Vec::new(),
        PrimitiveProfile::Pdftex14029 => pdftex_parameters(),
    }
}

fn tex82_parameters() -> Vec<PrimitiveParameterView> {
    let mut rows = Vec::with_capacity(103);
    for index in (0..=42).chain(48..=59) {
        let parameter = IntParam::new(index);
        let name = TEX82_INT_PARAMETER_NAMES[index as usize];
        let default = match parameter {
            IntParam::TOLERANCE => ParameterDefault::Integer(10_000),
            IntParam::MAG => ParameterDefault::Integer(1_000),
            IntParam::TIME => ParameterDefault::JobClock(JobClockField::MinutesSinceMidnight),
            IntParam::DAY => ParameterDefault::JobClock(JobClockField::Day),
            IntParam::MONTH => ParameterDefault::JobClock(JobClockField::Month),
            IntParam::YEAR => ParameterDefault::JobClock(JobClockField::Year),
            IntParam::ESCAPE_CHAR => ParameterDefault::Integer(i32::from(b'\\')),
            IntParam::END_LINE_CHAR => ParameterDefault::Integer(i32::from(b'\r')),
            IntParam::MAX_DEAD_CYCLES => ParameterDefault::Integer(25),
            IntParam::HANG_AFTER => ParameterDefault::Integer(1),
            _ => ParameterDefault::Integer(0),
        };
        rows.push(parameter_view(
            name,
            PrimitiveProfile::Tex82,
            ParameterBankClass::Integer,
            index,
            Meaning::IntParam(index),
            default,
        ));
    }
    for index in 0..=20 {
        rows.push(parameter_view(
            TEX82_DIMEN_PARAMETER_NAMES[index as usize],
            PrimitiveProfile::Tex82,
            ParameterBankClass::Dimension,
            index,
            Meaning::DimenParam(index),
            ParameterDefault::Scaled(0),
        ));
    }
    for (index, name) in TEX82_GLUE_PARAMETER_NAMES.into_iter().enumerate() {
        let index = index as u16;
        let (class, meaning) = if index < 15 {
            (ParameterBankClass::Glue, Meaning::GlueParam(index))
        } else {
            (ParameterBankClass::MathGlue, Meaning::MuGlueParam(index))
        };
        rows.push(parameter_view(
            name,
            PrimitiveProfile::Tex82,
            class,
            index,
            meaning,
            ParameterDefault::Glue(ZERO_GLUE),
        ));
    }
    for index in 0..=8 {
        rows.push(parameter_view(
            TEX82_TOK_PARAMETER_NAMES[index as usize],
            PrimitiveProfile::Tex82,
            ParameterBankClass::Tokens,
            index,
            Meaning::TokParam(index),
            ParameterDefault::EmptyTokens,
        ));
    }
    rows
}

/// Returns one canonical default per physical dense-bank cell for a fresh
/// profile layer. Primitive aliases remain catalogue rows but cannot become
/// duplicate state owners.
pub(crate) fn fresh_parameter_defaults(
    profile: PrimitiveProfile,
) -> Vec<tex_state::FreshParameterDefault> {
    let mut seen = [[None; tex_state::env::banks::PARAMETER_COUNT]; 4];
    let mut defaults = Vec::new();
    for row in primitive_parameter_views(profile) {
        let bank = match row.cell.class {
            ParameterBankClass::Integer => 0,
            ParameterBankClass::Dimension => 1,
            ParameterBankClass::Glue | ParameterBankClass::MathGlue => 2,
            ParameterBankClass::Tokens => 3,
        };
        let slot = &mut seen[bank][usize::from(row.cell.index)];
        if let Some(previous) = *slot {
            assert_eq!(
                previous, row.default,
                "conflicting parameter alias defaults"
            );
            continue;
        }
        *slot = Some(row.default);
        let default = match row.default {
            ParameterDefault::Integer(value) => {
                tex_state::FreshParameterDefault::Integer(IntParam::new(row.cell.index), value)
            }
            ParameterDefault::Scaled(value) => tex_state::FreshParameterDefault::Dimension(
                DimenParam::new(row.cell.index),
                tex_state::scaled::Scaled::from_raw(value),
            ),
            ParameterDefault::Glue(value) => {
                assert_eq!(value, ZERO_GLUE, "fresh glue default requires allocation");
                tex_state::FreshParameterDefault::EmptyGlue(tex_state::env::banks::GlueParam::new(
                    row.cell.index,
                ))
            }
            ParameterDefault::EmptyTokens => {
                tex_state::FreshParameterDefault::EmptyTokens(TokParam::new(row.cell.index))
            }
            ParameterDefault::JobClock(_) => continue,
        };
        defaults.push(default);
    }
    if profile == PrimitiveProfile::Etex26 {
        defaults.push(tex_state::FreshParameterDefault::Integer(
            IntParam::ETEX_EXTENDED_MODE,
            1,
        ));
    }
    defaults
}

fn etex_parameters() -> Vec<PrimitiveParameterView> {
    let mut rows = Vec::with_capacity(12);
    rows.push(parameter_view(
        "everyeof",
        PrimitiveProfile::Etex26,
        ParameterBankClass::Tokens,
        TokParam::EVERY_EOF.raw(),
        Meaning::TokParam(TokParam::EVERY_EOF.raw()),
        ParameterDefault::EmptyTokens,
    ));
    for &(name, parameter) in ETEX_INT_PARAMETERS {
        rows.push(parameter_view(
            name,
            PrimitiveProfile::Etex26,
            ParameterBankClass::Integer,
            parameter.raw(),
            Meaning::IntParam(parameter.raw()),
            ParameterDefault::Integer(0),
        ));
    }
    rows
}

const TEX82_INT_PARAMETER_NAMES: [&str; 60] = [
    "pretolerance",
    "tolerance",
    "linepenalty",
    "hyphenpenalty",
    "exhyphenpenalty",
    "clubpenalty",
    "widowpenalty",
    "displaywidowpenalty",
    "brokenpenalty",
    "binoppenalty",
    "relpenalty",
    "predisplaypenalty",
    "postdisplaypenalty",
    "interlinepenalty",
    "doublehyphendemerits",
    "finalhyphendemerits",
    "adjdemerits",
    "mag",
    "delimiterfactor",
    "looseness",
    "time",
    "day",
    "month",
    "year",
    "showboxbreadth",
    "showboxdepth",
    "hbadness",
    "vbadness",
    "pausing",
    "tracingonline",
    "tracingmacros",
    "tracingstats",
    "globaldefs",
    "tracingparagraphs",
    "tracingpages",
    "tracingoutput",
    "tracinglostchars",
    "tracingcommands",
    "tracingrestores",
    "uchyph",
    "escapechar",
    "defaulthyphenchar",
    "defaultskewchar",
    "",
    "",
    "",
    "",
    "",
    "endlinechar",
    "newlinechar",
    "language",
    "lefthyphenmin",
    "righthyphenmin",
    "holdinginserts",
    "errorcontextlines",
    "outputpenalty",
    "maxdeadcycles",
    "hangafter",
    "floatingpenalty",
    "fam",
];

const TEX82_DIMEN_PARAMETER_NAMES: [&str; 21] = [
    "parindent",
    "mathsurround",
    "lineskiplimit",
    "hsize",
    "vsize",
    "maxdepth",
    "splitmaxdepth",
    "boxmaxdepth",
    "hfuzz",
    "vfuzz",
    "delimitershortfall",
    "nulldelimiterspace",
    "scriptspace",
    "predisplaysize",
    "displaywidth",
    "displayindent",
    "overfullrule",
    "hangindent",
    "hoffset",
    "voffset",
    "emergencystretch",
];

const TEX82_GLUE_PARAMETER_NAMES: [&str; 18] = [
    "lineskip",
    "baselineskip",
    "parskip",
    "abovedisplayskip",
    "belowdisplayskip",
    "abovedisplayshortskip",
    "belowdisplayshortskip",
    "leftskip",
    "rightskip",
    "topskip",
    "splittopskip",
    "tabskip",
    "spaceskip",
    "xspaceskip",
    "parfillskip",
    "thinmuskip",
    "medmuskip",
    "thickmuskip",
];

const TEX82_TOK_PARAMETER_NAMES: [&str; 9] = [
    "output",
    "everypar",
    "everymath",
    "everydisplay",
    "everyhbox",
    "everyvbox",
    "everyjob",
    "everycr",
    "errhelp",
];

const ETEX_INT_PARAMETERS: &[(&str, IntParam)] = &[
    ("tracingscantokens", IntParam::TRACING_SCAN_TOKENS),
    ("TeXXeTstate", IntParam::TEX_XET_STATE),
    ("predisplaydirection", IntParam::PRE_DISPLAY_DIRECTION),
    ("tracingassigns", IntParam::TRACING_ASSIGNS),
    ("tracinggroups", IntParam::TRACING_GROUPS),
    ("tracingifs", IntParam::TRACING_IFS),
    ("tracingnesting", IntParam::TRACING_NESTING),
    ("savingvdiscards", IntParam::SAVING_V_DISCARDS),
    ("lastlinefit", IntParam::LAST_LINE_FIT),
    ("savinghyphcodes", IntParam::SAVING_HYPH_CODES),
    ("synctex", IntParam::SYNCTEX),
];

fn pdftex_parameters() -> Vec<PrimitiveParameterView> {
    let mut rows = Vec::with_capacity(57);
    for &(name, parameter) in PDFTEX_INT_PARAMETERS {
        rows.push(parameter_view(
            name,
            PrimitiveProfile::Pdftex14029,
            ParameterBankClass::Integer,
            parameter.raw(),
            Meaning::IntParam(parameter.raw()),
            ParameterDefault::Integer(pdftex_int_default(parameter)),
        ));
    }
    for &(name, parameter, default) in PDFTEX_DIMEN_PARAMETERS {
        rows.push(parameter_view(
            name,
            PrimitiveProfile::Pdftex14029,
            ParameterBankClass::Dimension,
            parameter.raw(),
            Meaning::DimenParam(parameter.raw()),
            ParameterDefault::Scaled(default),
        ));
    }
    for &(name, parameter) in PDFTEX_TOK_PARAMETERS {
        rows.push(parameter_view(
            name,
            PrimitiveProfile::Pdftex14029,
            ParameterBankClass::Tokens,
            parameter.raw(),
            Meaning::TokParam(parameter.raw()),
            ParameterDefault::EmptyTokens,
        ));
    }
    rows
}

const fn parameter_view(
    name: &'static str,
    profile: PrimitiveProfile,
    class: ParameterBankClass,
    index: u16,
    meaning: Meaning,
    default: ParameterDefault,
) -> PrimitiveParameterView {
    PrimitiveParameterView {
        name,
        profile,
        cell: ParameterCell { class, index },
        meaning,
        default,
        installation: InstallationPolicy::BOTH,
    }
}

const PDFTEX_INT_PARAMETERS: &[(&str, IntParam)] = &[
    ("pdfoutput", IntParam::PDF_OUTPUT),
    ("pdfcompresslevel", IntParam::PDF_COMPRESS_LEVEL),
    ("pdfobjcompresslevel", IntParam::PDF_OBJ_COMPRESS_LEVEL),
    ("pdfdecimaldigits", IntParam::PDF_DECIMAL_DIGITS),
    ("pdfmovechars", IntParam::PDF_MOVE_CHARS),
    ("pdfimageresolution", IntParam::PDF_IMAGE_RESOLUTION),
    ("pdfpkresolution", IntParam::PDF_PK_RESOLUTION),
    ("pdfuniqueresname", IntParam::PDF_UNIQUE_RESNAME),
    ("pdfoptionpdfminorversion", IntParam::PDF_MINOR_VERSION),
    (
        "pdfoptionalwaysusepdfpagebox",
        IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX,
    ),
    (
        "pdfoptionpdfinclusionerrorlevel",
        IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL,
    ),
    ("pdfmajorversion", IntParam::PDF_MAJOR_VERSION),
    ("pdfminorversion", IntParam::PDF_MINOR_VERSION),
    ("pdfforcepagebox", IntParam::PDF_FORCE_PAGE_BOX),
    ("pdfpagebox", IntParam::PDF_PAGE_BOX),
    (
        "pdfinclusionerrorlevel",
        IntParam::PDF_INCLUSION_ERROR_LEVEL,
    ),
    ("pdfgamma", IntParam::PDF_GAMMA),
    ("pdfimagegamma", IntParam::PDF_IMAGE_GAMMA),
    ("pdfimagehicolor", IntParam::PDF_IMAGE_HICOLOR),
    ("pdfimageapplygamma", IntParam::PDF_IMAGE_APPLY_GAMMA),
    ("pdfadjustspacing", IntParam::PDF_ADJUST_SPACING),
    ("pdfprotrudechars", IntParam::PDF_PROTRUDE_CHARS),
    ("pdftracingfonts", IntParam::PDF_TRACING_FONTS),
    (
        "pdfadjustinterwordglue",
        IntParam::PDF_ADJUST_INTERWORD_GLUE,
    ),
    ("pdfprependkern", IntParam::PDF_PREPEND_KERN),
    ("pdfappendkern", IntParam::PDF_APPEND_KERN),
    ("pdfgentounicode", IntParam::PDF_GEN_TO_UNICODE),
    ("pdfdraftmode", IntParam::PDF_DRAFT_MODE),
    ("pdfinclusioncopyfonts", IntParam::PDF_INCLUSION_COPY_FONTS),
    (
        "pdfsuppresswarningdupdest",
        IntParam::PDF_SUPPRESS_WARNING_DUP_DEST,
    ),
    (
        "pdfsuppresswarningdupmap",
        IntParam::PDF_SUPPRESS_WARNING_DUP_MAP,
    ),
    (
        "pdfsuppresswarningpagegroup",
        IntParam::PDF_SUPPRESS_WARNING_PAGE_GROUP,
    ),
    ("pdfinfoomitdate", IntParam::PDF_INFO_OMIT_DATE),
    ("pdfsuppressptexinfo", IntParam::PDF_SUPPRESS_PTEX_INFO),
    ("pdfomitcharset", IntParam::PDF_OMIT_CHARSET),
    ("pdfomitinfodict", IntParam::PDF_OMIT_INFO_DICT),
    ("pdfomitprocset", IntParam::PDF_OMIT_PROCSET),
    ("pdfptexuseunderscore", IntParam::PDF_PTEX_USE_UNDERSCORE),
    ("ignoreprimitiveerror", IntParam::IGNORE_PRIMITIVE_ERROR),
    ("partokencontext", IntParam::PAR_TOKEN_CONTEXT),
];

const fn pdftex_int_default(parameter: IntParam) -> i32 {
    match parameter {
        IntParam::PDF_COMPRESS_LEVEL => 9,
        IntParam::PDF_DECIMAL_DIGITS => 3,
        IntParam::PDF_IMAGE_RESOLUTION => 72,
        IntParam::PDF_MINOR_VERSION => 4,
        IntParam::PDF_MAJOR_VERSION => 1,
        IntParam::PDF_GAMMA => 1000,
        IntParam::PDF_IMAGE_GAMMA => 2200,
        IntParam::PDF_IMAGE_HICOLOR => 1,
        _ => 0,
    }
}

const PDFTEX_DIMEN_PARAMETERS: &[(&str, DimenParam, i32)] = &[
    ("pdfhorigin", DimenParam::PDF_H_ORIGIN, 4_736_287),
    ("pdfvorigin", DimenParam::PDF_V_ORIGIN, 4_736_287),
    ("pdfpagewidth", DimenParam::PDF_PAGE_WIDTH, 0),
    ("pdfpageheight", DimenParam::PDF_PAGE_HEIGHT, 0),
    ("pdflinkmargin", DimenParam::PDF_LINK_MARGIN, 0),
    ("pdfdestmargin", DimenParam::PDF_DEST_MARGIN, 0),
    ("pdfthreadmargin", DimenParam::PDF_THREAD_MARGIN, 0),
    (
        "pdffirstlineheight",
        DimenParam::PDF_FIRST_LINE_HEIGHT,
        -65_536_000,
    ),
    (
        "pdflastlinedepth",
        DimenParam::PDF_LAST_LINE_DEPTH,
        -65_536_000,
    ),
    (
        "pdfeachlineheight",
        DimenParam::PDF_EACH_LINE_HEIGHT,
        -65_536_000,
    ),
    (
        "pdfeachlinedepth",
        DimenParam::PDF_EACH_LINE_DEPTH,
        -65_536_000,
    ),
    (
        "pdfignoreddimen",
        DimenParam::PDF_IGNORED_DIMEN,
        -65_536_000,
    ),
    ("pdfpxdimen", DimenParam::PDF_PX_DIMEN, 65_782),
];

const PDFTEX_TOK_PARAMETERS: &[(&str, TokParam)] = &[
    ("pdfpagesattr", TokParam::PDF_PAGES_ATTR),
    ("pdfpageattr", TokParam::PDF_PAGE_ATTR),
    ("pdfpageresources", TokParam::PDF_PAGE_RESOURCES),
    ("pdfpkmode", TokParam::PDF_PK_MODE),
];

#[cfg(test)]
mod tests;
