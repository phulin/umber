use super::{Interner, InternerError, InternerMark, SYMBOL_CAPACITY};
use crate::interner::Symbol;
use proptest::prelude::*;

fn intern(interner: &mut Interner, name: &str) -> Symbol {
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
    assert_eq!(first.raw(), 0);
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
fn truncate_then_reintern_reuses_dense_symbol_id() {
    let mut interner = Interner::new();

    let kept = intern(&mut interner, "kept");
    let mark = interner.watermark();
    let truncated = intern(&mut interner, "temporary");
    assert_eq!(truncated.raw(), 1);

    interner.truncate_to(mark);
    assert_eq!(interner.len(), 1);
    assert_eq!(interner.resolve(kept), "kept");

    let reinserted = intern(&mut interner, "temporary");
    assert_eq!(reinserted.raw(), truncated.raw());
    assert_eq!(interner.resolve(reinserted), "temporary");
}

#[test]
#[should_panic(expected = "symbol is not live")]
fn stale_symbol_panics_after_truncation() {
    let mut interner = Interner::new();
    let mark = interner.watermark();
    let stale = intern(&mut interner, "rolled-back");

    interner.truncate_to(mark);

    let _ = interner.resolve(stale);
}

#[test]
fn intern_rejects_new_symbol_at_packed_token_capacity() {
    let mut interner = Interner::new();
    interner.next_symbol = SYMBOL_CAPACITY;

    assert_eq!(
        interner.intern("overflow"),
        Err(InternerError::TooManySymbols)
    );
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

                    prop_assert_eq!(symbol.raw() as usize, model_index);
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
                let symbol = super::Symbol::new(raw as u32);
                prop_assert_eq!(interner.resolve(symbol), expected.as_str());
                prop_assert_eq!(intern(&mut interner, expected).raw() as usize, raw);
            }
        }
    }
}
