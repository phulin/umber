use super::{
    ControlSequenceKind, Interner, InternerError, InternerMark, SYMBOL_CAPACITY,
    symbol_for_global_len,
};
use crate::interner::{Symbol, SymbolId};
use proptest::prelude::*;

fn intern(interner: &mut Interner, name: &str) -> Symbol {
    interner
        .intern(name)
        .expect("test interner should not reach symbol capacity")
        .symbol()
}

fn intern_id(interner: &mut Interner, name: &str) -> SymbolId {
    interner
        .intern(name)
        .expect("test interner should not reach symbol capacity")
}

#[test]
fn intern_is_idempotent() {
    let mut interner = Interner::new();

    let first = intern(&mut interner, "count");
    let second = intern(&mut interner, "count");

    assert_eq!(first, second);
    assert!(first.raw() < SYMBOL_CAPACITY);
    assert_eq!(interner.len(), 1);
}

#[test]
fn resolve_round_trips_ascii_and_non_ascii() {
    let mut interner = Interner::new();

    let ascii = intern(&mut interner, "par");
    let non_ascii = intern(&mut interner, "é漢字🙂");

    assert_eq!(interner.resolve(ascii), "par");
    assert_eq!(interner.resolve(non_ascii), "é漢字🙂");
}

#[test]
fn active_character_and_same_spelling_named_sequence_are_distinct() {
    let mut interner = Interner::new();

    let named = intern(&mut interner, "~");
    let active = interner.intern_active('~').expect("active symbol").symbol();

    assert_ne!(named, active);
    assert_eq!(interner.get("~").map(SymbolId::symbol), Some(named));
    assert_eq!(interner.get_active('~').map(SymbolId::symbol), Some(active));
    assert_eq!(interner.resolve(named), "~");
    assert_eq!(interner.resolve(active), "~");
    assert_eq!(interner.kind(named), ControlSequenceKind::Named);
    assert_eq!(interner.kind(active), ControlSequenceKind::ActiveCharacter);
}

#[test]
fn rollback_rebuild_preserves_control_sequence_namespace() {
    let mut interner = Interner::new();
    let named = intern(&mut interner, "~");
    let mark = interner.watermark();
    let discarded_active = interner.intern_active('~').expect("active symbol").symbol();

    interner.truncate_to(mark);
    assert_eq!(interner.get("~").map(SymbolId::symbol), Some(named));
    assert_eq!(interner.get_active('~'), None);

    let active = interner
        .intern_active('~')
        .expect("reintern active symbol")
        .symbol();
    assert_eq!(active, discarded_active);
    assert_ne!(active, named);
}

#[test]
fn concurrent_interners_share_stable_global_name_identity() {
    let threads = (0..16)
        .map(|_| {
            std::thread::spawn(|| {
                let mut interner = Interner::new();
                ["relax", "hrule", "vbox", "setbox", "looseness"]
                    .map(|name| intern(&mut interner, name))
            })
        })
        .collect::<Vec<_>>();
    let symbols = threads
        .into_iter()
        .map(|thread| thread.join().expect("interner thread"))
        .collect::<Vec<_>>();

    assert!(symbols.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn truncate_then_reintern_reuses_dense_symbol_id() {
    let mut interner = Interner::new();

    let kept = intern(&mut interner, "kept");
    let mark = interner.watermark();
    let truncated = intern_id(&mut interner, "temporary");
    assert_eq!(truncated.raw(), 1);

    interner.truncate_to(mark);
    assert_eq!(interner.len(), 1);
    assert_eq!(interner.resolve(kept), "kept");

    let reinserted = intern_id(&mut interner, "temporary");
    assert_eq!(reinserted.raw(), truncated.raw());
    assert_ne!(reinserted, truncated);
    assert!(!interner.contains_id(truncated));
    assert_eq!(interner.resolve_id(reinserted), "temporary");
}

#[test]
#[should_panic(expected = "symbol is not live")]
fn stale_symbol_panics_after_truncation() {
    let mut interner = Interner::new();
    let mark = interner.watermark();
    let stale = intern_id(&mut interner, "rolled-back");

    interner.truncate_to(mark);

    let _ = interner.resolve_id(stale);
}

#[test]
fn fork_preserves_inherited_symbol_ids_and_separates_new_allocations() {
    let mut parent = Interner::new();
    let inherited = intern_id(&mut parent, "inherited");
    let mut child = parent.clone();
    assert_eq!(child.resolve_id(inherited), "inherited");
    assert_eq!(child.get("inherited"), Some(inherited));
    let parent_only = intern_id(&mut parent, "parent");
    let child_only = intern_id(&mut child, "child");
    assert_eq!(parent_only.raw(), child_only.raw());
    assert_ne!(parent_only.symbol(), child_only.symbol());
    assert!(!child.contains_id(parent_only));
    assert!(!parent.contains_id(child_only));
}

#[test]
fn compact_symbol_token_and_cell_layouts_match_their_bit_domains() {
    assert_eq!(core::mem::size_of::<Symbol>(), 4);
    assert_eq!(core::mem::size_of::<crate::token::Token>(), 8);
    assert_eq!(core::mem::size_of::<crate::token::TracedTokenWord>(), 8);
    assert_eq!(core::mem::size_of::<crate::cell::CellId>(), 8);
}

#[test]
fn intern_rejects_new_symbol_at_packed_token_capacity() {
    assert_eq!(
        symbol_for_global_len(SYMBOL_CAPACITY - 1).map(Symbol::raw),
        Ok(SYMBOL_CAPACITY - 1)
    );
    assert_eq!(
        symbol_for_global_len(SYMBOL_CAPACITY),
        Err(InternerError::TooManySymbols)
    );
    assert_eq!(
        symbol_for_global_len(u32::MAX),
        Err(InternerError::TooManySymbols)
    );
}

#[test]
fn compact_key_never_revives_after_dense_slot_reuse() {
    let mut interner = Interner::new();
    let mark = interner.watermark();
    let stale = intern_id(&mut interner, "stale");
    interner.truncate_to(mark);
    let replacement = intern_id(&mut interner, "replacement");

    assert_eq!(stale.raw(), replacement.raw());
    assert_ne!(stale.symbol(), replacement.symbol());
    assert_eq!(interner.resolve_stored(stale.symbol()), None);
    assert_eq!(
        interner.resolve_stored(replacement.symbol()),
        Some(replacement)
    );
}

#[test]
fn independent_interners_share_permanent_name_identity() {
    let mut left = Interner::new();
    let mut right = Interner::new();
    let left = intern_id(&mut left, "same");
    let right = intern_id(&mut right, "same");

    assert_eq!(left.raw(), right.raw());
    assert_eq!(left.symbol(), right.symbol());
}

#[derive(Clone, Debug)]
enum Op {
    Intern(String),
    Mark,
    TruncateToMark(usize),
}

prop_compose! {
    fn intern_name()(name in "\\PC{0,8}") -> String {
        name
    }
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        intern_name().prop_map(Op::Intern),
        Just(Op::Mark),
        any::<usize>().prop_map(Op::TruncateToMark),
    ]
}

proptest! {
    #[test]
    fn arbitrary_intern_and_truncate_sequences_match_naive_model(
        ops in prop::collection::vec(op_strategy(), 0..256)
    ) {
        let mut interner = Interner::new();
        let mut model: Vec<String> = Vec::new();
        let mut marks: Vec<(InternerMark, usize)> = vec![(interner.watermark(), 0)];

        for op in ops {
            match op {
                Op::Intern(name) => {
                    let symbol = intern(&mut interner, &name);
                    let model_index = match model.iter().position(|existing| existing == &name) {
                        Some(index) => index,
                        None => {
                            model.push(name.clone());
                            model.len() - 1
                        }
                    };

                    prop_assert_eq!(interner.resolve_stored(symbol).map(SymbolId::raw), Some(model_index as u32));
                    prop_assert_eq!(interner.resolve(symbol), name.as_str());
                }
                Op::Mark => {
                    marks.push((interner.watermark(), model.len()));
                }
                Op::TruncateToMark(raw_index) => {
                    let index = raw_index % marks.len();
                    let (mark, model_len) = marks[index];
                    interner.truncate_to(mark);
                    model.truncate(model_len);
                    marks.retain(|&(_, len)| len <= model_len);
                }
            }

            prop_assert_eq!(interner.len(), model.len());
            for (raw, expected) in model.iter().enumerate() {
                let symbol = interner.symbol_at_slot(raw as u32).expect("model slot should be live");
                prop_assert_eq!(interner.resolve(symbol), expected.as_str());
                prop_assert_eq!(intern(&mut interner, expected), symbol);
            }
        }
    }
}
