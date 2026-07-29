use tex_state::Universe;
use tex_state::meaning::{ExpandablePrimitive, Meaning};

/// Installs TeX82's expandable primitive meanings for a fresh INITEX state.
pub fn install_tex82_expandable_primitives(universe: &mut Universe) {
    configure_tex82_expandable_primitives(universe, true);
}

/// Reconstructs TeX82's immutable primitive lookup table after format load.
pub fn register_tex82_expandable_primitives(universe: &mut Universe) {
    configure_tex82_expandable_primitives(universe, false);
}

fn configure_tex82_expandable_primitives(universe: &mut Universe, install: bool) {
    for (name, primitive) in [
        ("expandafter", ExpandablePrimitive::ExpandAfter),
        ("noexpand", ExpandablePrimitive::NoExpand),
        ("csname", ExpandablePrimitive::CsName),
        ("endcsname", ExpandablePrimitive::EndCsName),
        ("string", ExpandablePrimitive::String),
        ("number", ExpandablePrimitive::Number),
        ("romannumeral", ExpandablePrimitive::RomanNumeral),
        ("meaning", ExpandablePrimitive::Meaning),
        ("the", ExpandablePrimitive::The),
        ("input", ExpandablePrimitive::Input),
        ("endinput", ExpandablePrimitive::EndInput),
        ("jobname", ExpandablePrimitive::JobName),
        ("fontname", ExpandablePrimitive::FontName),
        ("topmark", ExpandablePrimitive::TopMark),
        ("firstmark", ExpandablePrimitive::FirstMark),
        ("botmark", ExpandablePrimitive::BotMark),
        ("splitfirstmark", ExpandablePrimitive::SplitFirstMark),
        ("splitbotmark", ExpandablePrimitive::SplitBotMark),
        ("iftrue", ExpandablePrimitive::IfTrue),
        ("iffalse", ExpandablePrimitive::IfFalse),
        ("if", ExpandablePrimitive::If),
        ("ifcat", ExpandablePrimitive::IfCat),
        ("ifx", ExpandablePrimitive::IfX),
        ("ifnum", ExpandablePrimitive::IfNum),
        ("ifdim", ExpandablePrimitive::IfDim),
        ("ifodd", ExpandablePrimitive::IfOdd),
        ("ifcase", ExpandablePrimitive::IfCase),
        ("ifvmode", ExpandablePrimitive::IfVMode),
        ("ifhmode", ExpandablePrimitive::IfHMode),
        ("ifmmode", ExpandablePrimitive::IfMMode),
        ("ifinner", ExpandablePrimitive::IfInner),
        ("ifvoid", ExpandablePrimitive::IfVoid),
        ("ifhbox", ExpandablePrimitive::IfHBox),
        ("ifvbox", ExpandablePrimitive::IfVBox),
        ("ifeof", ExpandablePrimitive::IfEof),
        ("else", ExpandablePrimitive::Else),
        ("or", ExpandablePrimitive::Or),
        ("fi", ExpandablePrimitive::Fi),
    ] {
        let meaning = Meaning::ExpandablePrimitive(primitive);
        universe.register_primitive_meaning(name, meaning);
        if install {
            let symbol = universe.intern(name);
            universe.set_meaning(symbol, meaning);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_and_register_preserve_format_meanings() {
        let mut installed = Universe::new_with_plain_catcodes();
        install_tex82_expandable_primitives(&mut installed);
        let iftrue = installed.intern("iftrue");
        assert_eq!(
            installed.meaning(iftrue),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue)
        );

        let mut registered = Universe::new_with_plain_catcodes();
        let replacement = Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand);
        let iftrue = registered.intern("iftrue");
        registered.set_meaning(iftrue, replacement);
        register_tex82_expandable_primitives(&mut registered);
        assert_eq!(registered.meaning(iftrue), replacement);
        assert_eq!(
            registered.primitive_meaning("iftrue"),
            Some(Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue))
        );
    }
}
