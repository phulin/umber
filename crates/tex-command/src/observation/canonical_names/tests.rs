use super::*;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};

/// tex.web §207 shares numeric codes between the catcode table and the command
/// codes only where a catcode can emerge from §341's scanning routine. The two
/// tables must therefore stay distinct exactly at codes 0, 9, 13, 14, and 15.
#[test]
fn catcode_and_character_command_tables_agree_only_where_tex_web_does() {
    let shared = [
        Catcode::BeginGroup,
        Catcode::EndGroup,
        Catcode::MathShift,
        Catcode::AlignmentTab,
        Catcode::EndLine,
        Catcode::Parameter,
        Catcode::Superscript,
        Catcode::Subscript,
        Catcode::Space,
        Catcode::Letter,
        Catcode::Other,
    ];
    for catcode in shared {
        assert_eq!(character_command_name(catcode), Some(catcode_name(catcode)));
    }
    for catcode in [
        Catcode::Escape,
        Catcode::Ignored,
        Catcode::Active,
        Catcode::Comment,
        Catcode::Invalid,
    ] {
        assert_eq!(character_command_name(catcode), None);
    }
}

/// The two names `umber2-johp.140` found wrong: Umber's Rust variants are
/// `Superscript`/`Subscript`, but tex.web §207 names the codes `sup_mark` and
/// `sub_mark`, and the trace must carry tex.web's spelling.
#[test]
fn superscript_and_subscript_use_tex_web_names() {
    assert_eq!(catcode_name(Catcode::Superscript), "sup_mark");
    assert_eq!(catcode_name(Catcode::Subscript), "sub_mark");
    assert_eq!(
        character_command_name(Catcode::Superscript),
        Some("sup_mark")
    );
    assert_eq!(character_command_name(Catcode::Subscript), Some("sub_mark"));
}

/// Every other catcode whose Rust variant spelling differs from tex.web §207.
#[test]
fn catcode_names_never_borrow_umbers_rust_variant_spelling() {
    assert_eq!(catcode_name(Catcode::EndLine), "car_ret");
    assert_eq!(catcode_name(Catcode::Parameter), "mac_param");
    assert_eq!(catcode_name(Catcode::Ignored), "ignore");
    assert_eq!(catcode_name(Catcode::Active), "active_char");
    assert_eq!(catcode_name(Catcode::Invalid), "invalid_char");
}

#[test]
fn catcode_assignment_names_cover_exactly_tex_webs_sixteen_codes() {
    assert_eq!(catcode_assignment_name(0), Some("escape"));
    assert_eq!(catcode_assignment_name(7), Some("sup_mark"));
    assert_eq!(catcode_assignment_name(15), Some("invalid_char"));
    assert_eq!(catcode_assignment_name(16), None);
    assert_eq!(catcode_assignment_name(-1), None);
}

#[test]
fn glue_orders_use_tex_web_135_names() {
    assert_eq!(glue_order_name(Order::Normal), "normal");
    assert_eq!(glue_order_name(Order::Fil), "fil");
    assert_eq!(glue_order_name(Order::Fill), "fill");
    assert_eq!(glue_order_name(Order::Filll), "filll");
}

/// §289's token representation is not the `\catcode` table: `match`,
/// `end_match`, and `out_param` are token-only codes, and a control sequence
/// is reported under `escape` with its spelling.
#[test]
fn observed_tokens_use_tex_webs_289_token_vocabulary() {
    assert_eq!(observed_token_catcode(&ObservedToken::MacroMatch), "match");
    assert_eq!(
        observed_token_catcode(&ObservedToken::MacroEndMatch),
        "end_match"
    );
    assert_eq!(
        observed_token_catcode(&ObservedToken::Parameter(3)),
        "out_parameter"
    );
    assert_eq!(observed_token_character(&ObservedToken::Parameter(3)), 3);
    assert_eq!(observed_token_character(&ObservedToken::MacroMatch), 35);
    assert_eq!(
        observed_token_catcode(&ObservedToken::FrozenEndTemplate),
        "escape"
    );
    assert_eq!(
        observed_token_control_sequence(&ObservedToken::FrozenEndV),
        Some("endtemplate")
    );
}

/// A `\let` names the copied meaning by its command code, so the observer
/// entry point must be the same total classification raw delivery uses.
#[test]
fn meaning_command_names_come_from_the_delivery_classifier() {
    assert_eq!(meaning_command_name(Meaning::Relax), "relax");
    assert_eq!(meaning_command_name(Meaning::Undefined), "undefined_cs");
    assert_eq!(
        meaning_command_name(Meaning::CharToken {
            ch: '_',
            cat: Catcode::Subscript
        }),
        "sub_mark"
    );
    assert_eq!(
        meaning_command_name(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)),
        "par_end"
    );
}
