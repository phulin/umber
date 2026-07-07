use tex_state::meaning::Meaning;
use tex_state::stores::Stores;

#[test]
#[should_panic(expected = "symbol is not live in this Stores timeline")]
fn stale_rolled_back_symbol_cannot_mutate_reused_meaning_cell() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern("rolled-back");

    stores.rollback(snapshot);
    stores.set_meaning(stale, Meaning::Relax);
}

#[test]
fn rollback_reuse_starts_with_undefined_meaning() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern("rolled-back");

    stores.rollback(snapshot);
    let reused = stores.intern("reused");

    assert_eq!(reused.raw(), stale.raw());
    assert_eq!(stores.meaning(reused), Meaning::Undefined);
}
