use super::{GlueSpec, GlueStore, GlueStoreMark, Order};
use crate::ids::GlueId;
use crate::scaled::Scaled;
use ahash::AHashMap;
use proptest::prelude::*;

#[test]
fn zero_is_canonical_and_preinterned() {
    let mut store = GlueStore::new();

    assert_eq!(store.intern(GlueSpec::ZERO), GlueId::ZERO);
    assert_eq!(store.get(GlueId::ZERO), GlueSpec::ZERO);
    assert_eq!(store.specs, vec![GlueSpec::ZERO]);
}

#[test]
fn get_round_trips_interned_spec() {
    let mut store = GlueStore::new();
    let spec = spec(10, 2, Order::Fil, 3, Order::Fill);

    let id = store.intern(spec);

    assert_eq!(store.get(id), spec);
}

#[test]
fn hash_consing_same_spec_twice_returns_same_id() {
    let mut store = GlueStore::new();
    let spec = spec(10, 2, Order::Fil, 3, Order::Fill);

    let first = store.intern(spec);
    let second = store.intern(spec);

    assert_eq!(first, second);
}

#[test]
fn stretch_and_shrink_fields_are_order_sensitive() {
    let mut store = GlueStore::new();
    let left = spec(0, 2, Order::Fil, 3, Order::Fill);
    let right = spec(0, 3, Order::Fill, 2, Order::Fil);

    let left_id = store.intern(left);
    let right_id = store.intern(right);

    assert_ne!(left_id, right_id);
    assert_eq!(store.get(left_id), left);
    assert_eq!(store.get(right_id), right);
}

#[test]
fn zero_survives_truncation_to_later_mark() {
    let mut store = GlueStore::new();
    let mark = store.watermark();
    let stale = store.intern(spec(1, 0, Order::Normal, 0, Order::Normal));

    store.truncate_to(mark);

    assert_eq!(store.get(GlueId::ZERO), GlueSpec::ZERO);
    assert_eq!(store.intern(GlueSpec::ZERO), GlueId::ZERO);
    assert!(!store.contains(stale));
}

#[test]
fn truncate_then_reintern_reuses_dense_glue_id() {
    let mut store = GlueStore::new();
    let kept = store.intern(spec(1, 0, Order::Normal, 0, Order::Normal));
    let mark = store.watermark();
    let truncated = store.intern(spec(2, 0, Order::Normal, 0, Order::Normal));
    assert_eq!(truncated.raw(), 2);

    store.truncate_to(mark);
    assert_eq!(store.get(kept), spec(1, 0, Order::Normal, 0, Order::Normal));

    let reinserted = store.intern(spec(2, 0, Order::Normal, 0, Order::Normal));
    assert_eq!(reinserted.raw(), truncated.raw());
    assert_ne!(reinserted, truncated);
    assert!(!store.contains(truncated));
    assert_eq!(
        store.get(reinserted),
        spec(2, 0, Order::Normal, 0, Order::Normal)
    );
}

#[test]
#[should_panic(expected = "glue id is not live")]
fn stale_glue_id_panics_after_truncation() {
    let mut store = GlueStore::new();
    let mark = store.watermark();
    let stale = store.intern(spec(1, 0, Order::Normal, 0, Order::Normal));

    store.truncate_to(mark);

    let _ = store.get(stale);
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Intern(GlueSpec),
    Mark,
    TruncateToMark(usize),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        glue_spec().prop_map(Op::Intern),
        Just(Op::Mark),
        any::<usize>().prop_map(Op::TruncateToMark),
    ]
}

proptest! {
    #[test]
    fn arbitrary_intern_and_truncate_sequences_match_naive_model(
        ops in prop::collection::vec(op_strategy(), 0..256)
    ) {
        let mut store = GlueStore::new();
        let mut model: Vec<GlueSpec> = vec![GlueSpec::ZERO];
        let mut model_index: AHashMap<GlueSpec, usize> = AHashMap::from([(GlueSpec::ZERO, 0)]);
        let mut marks: Vec<(GlueStoreMark, usize)> = vec![(store.watermark(), model.len())];

        for op in ops {
            match op {
                Op::Intern(spec) => {
                    let id = store.intern(spec);
                    let expected = model_id(&mut model, &mut model_index, spec);
                    prop_assert_eq!(id.raw() as usize, expected);
                }
                Op::Mark => {
                    marks.push((store.watermark(), model.len()));
                }
                Op::TruncateToMark(raw_index) => {
                    let index = raw_index % marks.len();
                    let (mark, model_len) = marks[index];
                    store.truncate_to(mark);
                    model.truncate(model_len);
                    model_index = rebuild_model_index(&model);
                    marks.retain(|&(_, len)| len <= model_len);
                }
            }

            prop_assert_eq!(store.specs.len(), model.len());
            for (raw, expected) in model.iter().copied().enumerate() {
                let id = store
                    .resolve_stored(GlueId::new(raw as u32))
                    .expect("model slot should resolve to a live glue identity");
                prop_assert_eq!(store.get(id), expected);
                prop_assert_eq!(store.intern(expected).raw() as usize, raw);
            }
        }
    }
}

fn model_id(
    model: &mut Vec<GlueSpec>,
    index: &mut AHashMap<GlueSpec, usize>,
    spec: GlueSpec,
) -> usize {
    if let Some(&id) = index.get(&spec) {
        return id;
    }
    let id = model.len();
    model.push(spec);
    index.insert(spec, id);
    id
}

fn rebuild_model_index(model: &[GlueSpec]) -> AHashMap<GlueSpec, usize> {
    model
        .iter()
        .copied()
        .enumerate()
        .map(|(id, spec)| (spec, id))
        .collect()
}

fn glue_spec() -> impl Strategy<Value = GlueSpec> {
    (any::<i32>(), any::<i32>(), order(), any::<i32>(), order()).prop_map(
        |(width, stretch, stretch_order, shrink, shrink_order)| {
            spec(width, stretch, stretch_order, shrink, shrink_order)
        },
    )
}

fn order() -> impl Strategy<Value = Order> {
    prop_oneof![
        Just(Order::Normal),
        Just(Order::Fil),
        Just(Order::Fill),
        Just(Order::Filll),
    ]
}

fn spec(
    width: i32,
    stretch: i32,
    stretch_order: Order,
    shrink: i32,
    shrink_order: Order,
) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(width),
        stretch: Scaled::from_raw(stretch),
        stretch_order,
        shrink: Scaled::from_raw(shrink),
        shrink_order,
    }
}
