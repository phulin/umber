use super::*;

#[test]
fn inline_push_pop_mark_and_truncate_preserve_exact_values() {
    let mut stack = PodStack::new();
    for value in 0_u32..6 {
        stack.push(value).expect("inline push fits");
    }
    let mark = stack.mark().expect("inline length fits mark");
    stack.push(6).expect("inline push fits");
    stack.push(7).expect("inline push fits");

    assert_eq!(stack.last(), Some(&7));
    assert_eq!(stack.pop(), Some(7));
    stack.truncate(mark).expect("ancestor mark truncates");
    assert_eq!(stack.len(), 6);
    assert_eq!(stack.get(5), Some(&5));
    assert_eq!(mark.len(), 6);
    assert_eq!(stack.accounting().retained_heap_entries, 0);
}

#[test]
fn non_ancestor_mark_rejects_without_mutation() {
    let mut stack = PodStack::new();
    stack.push(1_u8).expect("inline push fits");
    let later = PodStackMark(2);

    assert_eq!(stack.truncate(later), Err(PodStackError::InvalidMark));
    assert_eq!(stack.get(0), Some(&1));
}

#[test]
fn spilled_storage_plateaus_across_ten_thousand_cycles() {
    let mut stack = PodStack::new();
    let empty = stack.mark().expect("empty mark exists");
    for value in 0_u32..16 {
        stack.push(value).expect("warmup push fits");
    }
    stack.truncate(empty).expect("warmup truncates");
    let plateau = stack.accounting();

    for cycle in 0_u32..10_000 {
        for offset in 0_u32..16 {
            stack.push(cycle + offset).expect("warmed push fits");
        }
        stack.truncate(empty).expect("bounded suffix truncates");
    }

    assert_eq!(stack.accounting(), plateau);
    assert_eq!(plateau.logical_entries, 0);
    assert!(plateau.retained_heap_entries >= 16);
}

#[test]
fn all_live_accounting_is_exact() {
    let mut stack = PodStack::new();
    for value in 0_u64..13 {
        stack.push(value).expect("bounded push fits");
    }
    let accounting = stack.accounting();

    assert_eq!(accounting.logical_entries, 13);
    assert_eq!(accounting.logical_bytes, 13 * size_of::<u64>());
    assert_eq!(accounting.inline_capacity, 8);
    assert!(accounting.retained_heap_entries >= 13);
    assert_eq!(
        accounting.retained_heap_bytes,
        accounting.retained_heap_entries * size_of::<u64>()
    );
}

#[test]
fn stack_marks_are_plain_fixed_width_values() {
    assert_eq!(size_of::<PodStackMark>(), size_of::<u32>());
    assert!(!core::mem::needs_drop::<PodStackMark>());
}
