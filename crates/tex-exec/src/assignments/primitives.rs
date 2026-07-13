use super::*;
use tex_state::meaning::InternalInteger;
use tex_state::page::{PageDimension, PageInteger};

pub fn install_unexpandable_primitives(stores: &mut Universe) {
    for (name, primitive) in [
        ("def", UnexpandablePrimitive::Def),
        ("edef", UnexpandablePrimitive::Edef),
        ("gdef", UnexpandablePrimitive::Gdef),
        ("xdef", UnexpandablePrimitive::Xdef),
        ("let", UnexpandablePrimitive::Let),
        ("futurelet", UnexpandablePrimitive::FutureLet),
        ("globaldefs", UnexpandablePrimitive::GlobalDefs),
        ("global", UnexpandablePrimitive::Global),
        ("begingroup", UnexpandablePrimitive::BeginGroup),
        ("endgroup", UnexpandablePrimitive::EndGroup),
        ("aftergroup", UnexpandablePrimitive::AfterGroup),
        ("afterassignment", UnexpandablePrimitive::AfterAssignment),
        ("long", UnexpandablePrimitive::Long),
        ("outer", UnexpandablePrimitive::Outer),
        ("count", UnexpandablePrimitive::Count),
        ("dimen", UnexpandablePrimitive::Dimen),
        ("skip", UnexpandablePrimitive::Skip),
        ("muskip", UnexpandablePrimitive::Muskip),
        ("toks", UnexpandablePrimitive::Toks),
        ("countdef", UnexpandablePrimitive::CountDef),
        ("dimendef", UnexpandablePrimitive::DimenDef),
        ("skipdef", UnexpandablePrimitive::SkipDef),
        ("muskipdef", UnexpandablePrimitive::MuskipDef),
        ("toksdef", UnexpandablePrimitive::ToksDef),
        ("chardef", UnexpandablePrimitive::CharDef),
        ("mathchardef", UnexpandablePrimitive::MathCharDef),
        ("advance", UnexpandablePrimitive::Advance),
        ("multiply", UnexpandablePrimitive::Multiply),
        ("divide", UnexpandablePrimitive::Divide),
        ("catcode", UnexpandablePrimitive::CatCode),
        ("lccode", UnexpandablePrimitive::LcCode),
        ("uccode", UnexpandablePrimitive::UcCode),
        ("sfcode", UnexpandablePrimitive::SfCode),
        ("mathcode", UnexpandablePrimitive::MathCode),
        ("delcode", UnexpandablePrimitive::DelCode),
        ("font", UnexpandablePrimitive::Font),
        ("fontdimen", UnexpandablePrimitive::FontDimen),
        ("hyphenchar", UnexpandablePrimitive::HyphenChar),
        ("skewchar", UnexpandablePrimitive::SkewChar),
        ("patterns", UnexpandablePrimitive::Patterns),
        ("hyphenation", UnexpandablePrimitive::Hyphenation),
        ("par", UnexpandablePrimitive::Par),
        ("endgraf", UnexpandablePrimitive::EndGraf),
        ("indent", UnexpandablePrimitive::Indent),
        ("noindent", UnexpandablePrimitive::NoIndent),
        ("parshape", UnexpandablePrimitive::ParShape),
        ("prevdepth", UnexpandablePrimitive::PrevDepth),
        ("prevgraf", UnexpandablePrimitive::PrevGraf),
        ("nointerlineskip", UnexpandablePrimitive::NoInterlineSkip),
        ("halign", UnexpandablePrimitive::HAlign),
        ("valign", UnexpandablePrimitive::VAlign),
        ("noalign", UnexpandablePrimitive::NoAlign),
        ("omit", UnexpandablePrimitive::Omit),
        ("cr", UnexpandablePrimitive::Cr),
        ("crcr", UnexpandablePrimitive::CrCr),
        ("span", UnexpandablePrimitive::Span),
        ("hbox", UnexpandablePrimitive::HBox),
        ("vbox", UnexpandablePrimitive::VBox),
        ("vtop", UnexpandablePrimitive::VTop),
        ("setbox", UnexpandablePrimitive::SetBox),
        ("box", UnexpandablePrimitive::Box),
        ("copy", UnexpandablePrimitive::Copy),
        ("vsplit", UnexpandablePrimitive::VSplit),
        ("unhbox", UnexpandablePrimitive::UnHBox),
        ("unhcopy", UnexpandablePrimitive::UnHCopy),
        ("unvbox", UnexpandablePrimitive::UnVBox),
        ("unvcopy", UnexpandablePrimitive::UnVCopy),
        ("lastbox", UnexpandablePrimitive::LastBox),
        ("wd", UnexpandablePrimitive::Wd),
        ("ht", UnexpandablePrimitive::Ht),
        ("dp", UnexpandablePrimitive::Dp),
        ("raise", UnexpandablePrimitive::Raise),
        ("lower", UnexpandablePrimitive::Lower),
        ("moveleft", UnexpandablePrimitive::MoveLeft),
        ("moveright", UnexpandablePrimitive::MoveRight),
        ("char", UnexpandablePrimitive::Char),
        ("kern", UnexpandablePrimitive::Kern),
        ("hskip", UnexpandablePrimitive::HSkip),
        ("vskip", UnexpandablePrimitive::VSkip),
        ("leaders", UnexpandablePrimitive::Leaders),
        ("cleaders", UnexpandablePrimitive::CLeaders),
        ("xleaders", UnexpandablePrimitive::XLeaders),
        ("hfil", UnexpandablePrimitive::HFil),
        ("hfill", UnexpandablePrimitive::HFill),
        ("hss", UnexpandablePrimitive::HSs),
        ("hfilneg", UnexpandablePrimitive::HFilNeg),
        ("vfil", UnexpandablePrimitive::VFil),
        ("vfill", UnexpandablePrimitive::VFill),
        ("vss", UnexpandablePrimitive::VSs),
        ("vfilneg", UnexpandablePrimitive::VFilNeg),
        ("penalty", UnexpandablePrimitive::Penalty),
        ("vrule", UnexpandablePrimitive::VRule),
        ("hrule", UnexpandablePrimitive::HRule),
        (" ", UnexpandablePrimitive::ControlSpace),
        ("lastpenalty", UnexpandablePrimitive::LastPenalty),
        ("lastkern", UnexpandablePrimitive::LastKern),
        ("lastskip", UnexpandablePrimitive::LastSkip),
        ("unpenalty", UnexpandablePrimitive::UnPenalty),
        ("unkern", UnexpandablePrimitive::UnKern),
        ("unskip", UnexpandablePrimitive::UnSkip),
        ("/", UnexpandablePrimitive::ItalicCorrection),
        ("discretionary", UnexpandablePrimitive::Discretionary),
        ("-", UnexpandablePrimitive::DiscretionaryHyphen),
        ("noboundary", UnexpandablePrimitive::NoBoundary),
        ("spacefactor", UnexpandablePrimitive::SpaceFactor),
        ("accent", UnexpandablePrimitive::Accent),
        ("mark", UnexpandablePrimitive::Mark),
        ("vadjust", UnexpandablePrimitive::VAdjust),
        ("insert", UnexpandablePrimitive::Insert),
        ("openin", UnexpandablePrimitive::OpenIn),
        ("closein", UnexpandablePrimitive::CloseIn),
        ("openout", UnexpandablePrimitive::OpenOut),
        ("closeout", UnexpandablePrimitive::CloseOut),
        ("immediate", UnexpandablePrimitive::Immediate),
        ("write", UnexpandablePrimitive::Write),
        ("special", UnexpandablePrimitive::Special),
        ("setlanguage", UnexpandablePrimitive::SetLanguage),
        ("read", UnexpandablePrimitive::Read),
        ("shipout", UnexpandablePrimitive::Shipout),
        ("show", UnexpandablePrimitive::Show),
        ("showbox", UnexpandablePrimitive::ShowBox),
        ("showthe", UnexpandablePrimitive::ShowThe),
        ("showtokens", UnexpandablePrimitive::ShowTokens),
        ("message", UnexpandablePrimitive::Message),
        ("errmessage", UnexpandablePrimitive::ErrMessage),
        ("showlists", UnexpandablePrimitive::ShowLists),
        ("showhyphens", UnexpandablePrimitive::ShowHyphens),
        ("uppercase", UnexpandablePrimitive::Uppercase),
        ("lowercase", UnexpandablePrimitive::Lowercase),
        ("ignorespaces", UnexpandablePrimitive::IgnoreSpaces),
        ("mathchar", UnexpandablePrimitive::MathChar),
        ("delimiter", UnexpandablePrimitive::Delimiter),
        ("textfont", UnexpandablePrimitive::TextFont),
        ("scriptfont", UnexpandablePrimitive::ScriptFont),
        ("scriptscriptfont", UnexpandablePrimitive::ScriptScriptFont),
        ("mathord", UnexpandablePrimitive::MathOrd),
        ("mathop", UnexpandablePrimitive::MathOp),
        ("mathbin", UnexpandablePrimitive::MathBin),
        ("mathrel", UnexpandablePrimitive::MathRel),
        ("mathopen", UnexpandablePrimitive::MathOpen),
        ("mathclose", UnexpandablePrimitive::MathClose),
        ("mathpunct", UnexpandablePrimitive::MathPunct),
        ("mathinner", UnexpandablePrimitive::MathInner),
        ("underline", UnexpandablePrimitive::Underline),
        ("overline", UnexpandablePrimitive::Overline),
        ("limits", UnexpandablePrimitive::Limits),
        ("nolimits", UnexpandablePrimitive::NoLimits),
        ("displaylimits", UnexpandablePrimitive::DisplayLimits),
        ("over", UnexpandablePrimitive::Over),
        ("atop", UnexpandablePrimitive::Atop),
        ("above", UnexpandablePrimitive::Above),
        ("overwithdelims", UnexpandablePrimitive::OverWithDelims),
        ("atopwithdelims", UnexpandablePrimitive::AtopWithDelims),
        ("abovewithdelims", UnexpandablePrimitive::AboveWithDelims),
        ("radical", UnexpandablePrimitive::Radical),
        ("mathaccent", UnexpandablePrimitive::MathAccent),
        ("vcenter", UnexpandablePrimitive::VCenter),
        ("mskip", UnexpandablePrimitive::MSkip),
        ("mkern", UnexpandablePrimitive::MKern),
        ("nonscript", UnexpandablePrimitive::NonScript),
        ("mathchoice", UnexpandablePrimitive::MathChoice),
        ("left", UnexpandablePrimitive::Left),
        ("right", UnexpandablePrimitive::Right),
        ("eqno", UnexpandablePrimitive::EqNo),
        ("leqno", UnexpandablePrimitive::LeftEqNo),
        ("displaystyle", UnexpandablePrimitive::DisplayStyle),
        ("textstyle", UnexpandablePrimitive::TextStyle),
        ("scriptstyle", UnexpandablePrimitive::ScriptStyle),
        (
            "scriptscriptstyle",
            UnexpandablePrimitive::ScriptScriptStyle,
        ),
        ("batchmode", UnexpandablePrimitive::BatchMode),
        ("nonstopmode", UnexpandablePrimitive::NonstopMode),
        ("scrollmode", UnexpandablePrimitive::ScrollMode),
        ("errorstopmode", UnexpandablePrimitive::ErrorStopMode),
        ("end", UnexpandablePrimitive::End),
        ("dump", UnexpandablePrimitive::Dump),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let nullfont = stores.intern("nullfont");
    stores.set_meaning(nullfont, Meaning::Font(tex_state::font::NULL_FONT));
    stores.set_font_identifier_symbol(tex_state::font::NULL_FONT, nullfont);
    stores.set_current_font_selector_global(nullfont, tex_state::font::NULL_FONT);
    install_parameter_meanings(stores);
    install_page_meanings(stores);
    let badness = stores.intern("badness");
    stores.set_meaning(badness, Meaning::InternalInteger(InternalInteger::Badness));
    let inputlineno = stores.intern("inputlineno");
    stores.set_meaning(
        inputlineno,
        Meaning::InternalInteger(InternalInteger::InputLineNumber),
    );
}

fn install_parameter_meanings(stores: &mut Universe) {
    for &(name, index) in INT_PARAMS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::IntParam(index));
    }
    for &(name, index) in DIMEN_PARAMS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::DimenParam(index));
    }
    for &(name, index) in GLUE_PARAMS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::GlueParam(index));
    }
    for &(name, index) in MU_GLUE_PARAMS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::MuGlueParam(index));
    }
    for &(name, index) in TOK_PARAMS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::TokParam(index));
    }
}

/// Installs unexpandable primitives that exist only in e-TeX extended mode.
pub fn install_etex_unexpandable_primitives(stores: &mut Universe) {
    for (name, primitive) in [
        ("protected", UnexpandablePrimitive::Protected),
        ("readline", UnexpandablePrimitive::ReadLine),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let everyeof = stores.intern("everyeof");
    stores.set_meaning(everyeof, Meaning::TokParam(TokParam::EVERY_EOF.raw()));
}

fn install_page_meanings(stores: &mut Universe) {
    for &(name, dimension) in PAGE_DIMENSIONS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::PageDimension(dimension));
    }
    for &(name, integer) in PAGE_INTEGERS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::PageInteger(integer));
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
    ("pausing", 28),
    ("tracingonline", 29),
    ("tracingmacros", 30),
    ("tracingstats", 31),
    ("globaldefs", IntParam::GLOBAL_DEFS.raw()),
    ("tracingparagraphs", 33),
    ("tracingpages", 34),
    ("tracingoutput", 35),
    ("tracinglostchars", 36),
    ("tracingcommands", 37),
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
    ("errhelp", 8),
];

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
