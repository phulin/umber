use super::*;

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
        ("protected", UnexpandablePrimitive::Protected),
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
        ("read", UnexpandablePrimitive::Read),
        ("show", UnexpandablePrimitive::Show),
        ("showthe", UnexpandablePrimitive::ShowThe),
        ("showtokens", UnexpandablePrimitive::ShowTokens),
        ("message", UnexpandablePrimitive::Message),
        ("errmessage", UnexpandablePrimitive::ErrMessage),
        ("showlists", UnexpandablePrimitive::ShowLists),
        ("uppercase", UnexpandablePrimitive::Uppercase),
        ("lowercase", UnexpandablePrimitive::Lowercase),
        ("ignorespaces", UnexpandablePrimitive::IgnoreSpaces),
        ("end", UnexpandablePrimitive::End),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    install_parameter_meanings(stores);
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
    for &(name, index) in TOK_PARAMS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::TokParam(index));
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
    ("delimiterfactor", 18),
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
    ("newlinechar", 49),
    ("language", 50),
    ("lefthyphenmin", 51),
    ("righthyphenmin", 52),
    ("holdinginserts", 53),
    ("errorcontextlines", 54),
    ("outputpenalty", 55),
    ("maxdeadcycles", 56),
    ("hangafter", 57),
    ("floatingpenalty", 58),
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
    ("delimitershortfall", 10),
    ("nulldelimiterspace", 11),
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
    ("thinmuskip", 15),
    ("medmuskip", 16),
    ("thickmuskip", 17),
];

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
