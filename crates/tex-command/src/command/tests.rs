use tex_state::Universe;
use tex_state::meaning::{ExpandablePrimitive, InternalInteger, Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{CommandIdentity, ConvertSelector, CurrentCommand, DeliveryStamp, XRaySelector};
use crate::observation::canonical_command_identity;

fn resolve(universe: &mut Universe, token: Token, origin: OriginId) -> CurrentCommand {
    let mut state = universe.command_context();
    CurrentCommand::resolve(
        TracedTokenWord::pack(token, origin),
        DeliveryStamp::new(17, 23, 29),
        None,
        false,
        &mut state,
    )
}

#[test]
fn control_sequence_spelling_survives_a_later_meaning_change() {
    let mut universe = Universe::new();
    let symbol = universe.intern("defined").symbol();
    universe.set_meaning(symbol, Meaning::CharGiven('A'));

    let command = resolve(&mut universe, Token::Cs(symbol), OriginId::UNKNOWN);
    universe.set_meaning(symbol, Meaning::CharGiven('B'));

    assert_eq!(command.spelling().semantic_token(), Token::Cs(symbol));
    assert_eq!(command.control_sequence(), Some(symbol));
    assert_eq!(command.meaning(), Meaning::CharGiven('A'));
    assert_eq!(command.origin(), OriginId::UNKNOWN);
    assert_eq!(command.delivery_stamp().input_level(), 17);
    assert_eq!(command.delivery_stamp().position(), 23);
    assert_eq!(command.delivery_stamp().sequence(), 29);
}

#[test]
fn active_character_uses_its_distinct_control_sequence_meaning() {
    let mut universe = Universe::new();
    let named = universe.intern("~").symbol();
    let active = universe.intern_active_character('~').symbol();
    universe.set_meaning(named, Meaning::CharGiven('N'));
    universe.set_meaning(active, Meaning::CharGiven('A'));

    let command = resolve(
        &mut universe,
        Token::Char {
            ch: '~',
            cat: Catcode::Active,
        },
        OriginId::UNKNOWN,
    );

    assert_eq!(command.control_sequence(), Some(active));
    assert_eq!(command.meaning(), Meaning::CharGiven('A'));
    assert_ne!(command.control_sequence(), Some(named));
}

#[test]
fn ordinary_character_has_its_literal_token_meaning() {
    let mut universe = Universe::new();

    let command = resolve(
        &mut universe,
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
        OriginId::UNKNOWN,
    );

    assert_eq!(command.control_sequence(), None);
    assert_eq!(
        command.meaning(),
        Meaning::CharToken {
            ch: 'x',
            cat: Catcode::Letter,
        }
    );
}

#[test]
fn expandafter_resolves_to_its_tex82_current_command_identity() {
    let mut universe = Universe::new();
    let expandafter = universe.intern("expandafter").symbol();
    universe.set_meaning(
        expandafter,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
    );

    let command = resolve(&mut universe, Token::Cs(expandafter), OriginId::UNKNOWN);

    assert_eq!(command.identity(), CommandIdentity::ExpandAfter);
}

#[test]
fn csname_resolves_to_its_tex82_current_command_identity() {
    let mut universe = Universe::new();
    let csname = universe.intern("csname").symbol();
    universe.set_meaning(
        csname,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName),
    );

    let command = resolve(&mut universe, Token::Cs(csname), OriginId::UNKNOWN);

    assert_eq!(command.identity(), CommandIdentity::CsName);
}

#[test]
fn endcsname_resolves_to_its_tex82_current_command_identity() {
    let mut universe = Universe::new();
    let endcsname = universe.intern("endcsname").symbol();
    universe.set_meaning(
        endcsname,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName),
    );

    let command = resolve(&mut universe, Token::Cs(endcsname), OriginId::UNKNOWN);

    assert_eq!(command.identity(), CommandIdentity::EndCsName);
}

#[test]
fn classic_conversions_resolve_to_the_shared_tex82_convert_identity() {
    let mut universe = Universe::new();

    for (name, primitive, selector) in [
        (
            "number",
            ExpandablePrimitive::Number,
            ConvertSelector::Number,
        ),
        (
            "romannumeral",
            ExpandablePrimitive::RomanNumeral,
            ConvertSelector::RomanNumeral,
        ),
        (
            "string",
            ExpandablePrimitive::String,
            ConvertSelector::String,
        ),
        (
            "meaning",
            ExpandablePrimitive::Meaning,
            ConvertSelector::Meaning,
        ),
        (
            "fontname",
            ExpandablePrimitive::FontName,
            ConvertSelector::FontName,
        ),
        (
            "jobname",
            ExpandablePrimitive::JobName,
            ConvertSelector::JobName,
        ),
    ] {
        let symbol = universe.intern(name).symbol();
        universe.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));

        let command = resolve(&mut universe, Token::Cs(symbol), OriginId::UNKNOWN);

        assert_eq!(command.identity(), CommandIdentity::Convert(selector));
    }
}

#[test]
fn tex82_diagnostics_resolve_to_the_shared_xray_identity() {
    let mut universe = Universe::new();

    for (name, primitive, selector) in [
        ("show", UnexpandablePrimitive::Show, XRaySelector::Show),
        (
            "showbox",
            UnexpandablePrimitive::ShowBox,
            XRaySelector::ShowBox,
        ),
        (
            "showthe",
            UnexpandablePrimitive::ShowThe,
            XRaySelector::ShowThe,
        ),
        (
            "showlists",
            UnexpandablePrimitive::ShowLists,
            XRaySelector::ShowLists,
        ),
    ] {
        let symbol = universe.intern(name).symbol();
        universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));

        let command = resolve(&mut universe, Token::Cs(symbol), OriginId::UNKNOWN);

        assert_eq!(command.identity(), CommandIdentity::XRay(selector));
    }
}

#[test]
fn command_code_partition_classifies_character_internal_unexpandable_and_expandable_ranges() {
    let mut universe = Universe::new();
    let cases = [
        (
            Meaning::CharToken {
                ch: 'x',
                cat: Catcode::Letter,
            },
            "character",
        ),
        (
            Meaning::InternalInteger(InternalInteger::Badness),
            "internal",
        ),
        (
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Def),
            "unexpandable",
        ),
        (
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
            "expandable",
        ),
    ];

    for (index, (meaning, expected_partition)) in cases.into_iter().enumerate() {
        let symbol = universe.intern(&format!("partition{index}")).symbol();
        universe.set_meaning(symbol, meaning);
        let command = resolve(&mut universe, Token::Cs(symbol), OriginId::UNKNOWN);
        let actual_partition = match command.meaning() {
            Meaning::CharToken { .. } | Meaning::CharGiven(_) => "character",
            Meaning::InternalInteger(_)
            | Meaning::CountRegister(_)
            | Meaning::DimenRegister(_)
            | Meaning::SkipRegister(_)
            | Meaning::MuskipRegister(_)
            | Meaning::ToksRegister(_)
            | Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::MuGlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_)
            | Meaning::Font(_) => "internal",
            Meaning::UnexpandablePrimitive(_) | Meaning::EndV => "unexpandable",
            Meaning::ExpandablePrimitive(_) | Meaning::Macro { .. } => "expandable",
            _ => "other",
        };
        assert_eq!(actual_partition, expected_partition, "case {index}");
    }

    assert_eq!(Catcode::Escape as u8, 0);
    assert_eq!(Catcode::Invalid as u8, 15);
    assert_eq!(UnexpandablePrimitive::Def.operand(), 0);
    assert_eq!(ExpandablePrimitive::ExpandAfter.operand(), 0);
}

#[test]
fn lookup_reuses_existing_identity_and_guarded_miss_does_not_intern() {
    let mut universe = Universe::new();
    let first = {
        let mut state = universe.command_context();
        state.intern_control_sequence("already-known")
    };

    {
        let state = universe.command_context();
        assert_eq!(state.known_control_sequence("already-known"), Some(first));
        assert_eq!(state.known_control_sequence("guarded-miss"), None);
    }

    let mut state = universe.command_context();
    assert_eq!(state.intern_control_sequence("already-known"), first);
    assert_ne!(state.intern_control_sequence("guarded-miss"), first);
}

#[test]
fn primitive_installation_binds_spelling_command_operand_and_level() {
    let mut universe = Universe::new();
    let meaning = Meaning::ExpandablePrimitive(ExpandablePrimitive::Number);

    universe.install_primitive_meaning("number", meaning);

    let symbol = universe
        .symbol("number")
        .expect("primitive spelling is interned");
    assert_eq!(universe.primitive_meaning("number"), Some(meaning));
    assert_eq!(universe.primitive_name(meaning), Some("number"));
    assert_eq!(universe.meaning(symbol), meaning);
    let command = resolve(&mut universe, Token::Cs(symbol.symbol()), OriginId::UNKNOWN);
    assert_eq!(
        command.identity(),
        CommandIdentity::Convert(ConvertSelector::Number)
    );
    assert_eq!(ConvertSelector::Number.operand(), 0);
}

#[test]
fn extension_primitives_preserve_tex82_selectors_and_any_mode_dispatch() {
    let mut universe = Universe::new();
    for (name, primitive, selector) in [
        ("openout", UnexpandablePrimitive::OpenOut, 0),
        ("write", UnexpandablePrimitive::Write, 1),
        ("closeout", UnexpandablePrimitive::CloseOut, 2),
        ("special", UnexpandablePrimitive::Special, 3),
        ("immediate", UnexpandablePrimitive::Immediate, 4),
        ("setlanguage", UnexpandablePrimitive::SetLanguage, 5),
    ] {
        let meaning = Meaning::UnexpandablePrimitive(primitive);
        universe.install_primitive_meaning(name, meaning);
        let symbol = universe
            .symbol(name)
            .expect("extension primitive spelling is installed");
        let command = resolve(&mut universe, Token::Cs(symbol.symbol()), OriginId::UNKNOWN);

        assert_eq!(command.meaning(), meaning);
        assert_eq!(
            canonical_command_identity(command.meaning()),
            ("extension".to_owned(), Some(selector))
        );
    }
}

#[test]
fn unknown_extension_selector_fails_loudly_at_dispatch() {
    let unknown = tex_state::meaning::RawMeaning::testing_new(u8::MAX, 6);
    assert_eq!(
        canonical_command_identity(Meaning::Unknown(unknown)),
        ("undecodable_meaning".to_owned(), None)
    );
}
