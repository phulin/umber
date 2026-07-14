use super::*;
use crate::push_traced_tokens;
use tex_lex::MemoryInput;
use tex_state::provenance::SyntheticOriginKind;

#[test]
fn invalid_delimiter_pushback_preserves_traced_origin() {
    let mut stores = Universe::new();
    crate::install_unexpandable_primitives(&mut stores);
    let origin = stores.synthetic_origin(SyntheticOriginKind::Test);
    let invalid = TracedTokenWord::pack(Token::Param(1), origin);
    let mut input = InputStack::new(MemoryInput::new(""));
    push_traced_tokens(&mut input, &mut stores, [invalid]);

    let delimiter = scan_delimiter_token(
        &mut input,
        &mut stores,
        &mut crate::ExecutionContext::new("texput"),
    )
    .expect("invalid delimiter should recover");

    assert_eq!(delimiter, 0);
    let replayed = input
        .next_traced_token(&mut stores)
        .expect("read recovered token")
        .expect("invalid token should be backed up");
    assert_eq!(tex_expand::semantic_token(replayed), Token::Param(1));
    assert_eq!(replayed.origin(), origin);
}

#[test]
fn delimiter_command_scans_all_twenty_seven_bits() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(r#"\delimiter"7FFFFFF "#));

    let delimiter = scan_delimiter_token(
        &mut input,
        &mut stores,
        &mut crate::ExecutionContext::new("texput"),
    )
    .expect("numeric delimiter should scan");

    assert_eq!(delimiter, 0x07ff_ffff);
}
