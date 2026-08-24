use std::hint::black_box;

use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, ResolvedMeaning};
use tex_state::measurement::{
    HotCoreAllocationMeasurement, HotCoreAllocationOwner, HotCoreAllocator, hot_core_census,
    retained_generation_census,
};
use tex_state::node::{Node, NodeTokenList};
use tex_state::page::PageMark;
use tex_state::token::{Token, TokenWord};
use tex_state::{
    AssignmentScope, ReachabilityStore, RetainedStateGeneration, World, with_universe,
};
use tex_state_benchmarks::{DIRECT_READS, PAGE_QUEUE_LEN, WARM_WRITES, engine_budget};

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

#[derive(Clone, Copy, Debug, Default)]
struct AllocationDelta {
    calls: u64,
    requested_bytes: u64,
}

impl AllocationDelta {
    fn between(before: HotCoreAllocationMeasurement, after: HotCoreAllocationMeasurement) -> Self {
        Self {
            calls: after.calls.saturating_sub(before.calls),
            requested_bytes: after.requested_bytes.saturating_sub(before.requested_bytes),
        }
    }
}

fn main() {
    let enforce = std::env::args().any(|argument| argument == "--enforce");
    let mut failures = Vec::new();

    let (reads, writes, page_queue, mark_classes) = hot_state_gate();
    check_zero("warmed direct reads", reads, &mut failures);
    check_zero("warmed same-cell writes", writes, &mut failures);
    check_zero("warmed page queue", page_queue, &mut failures);
    check_zero("warmed mark classes", mark_classes, &mut failures);
    generation_lifecycle_gate(&mut failures);

    println!(
        "FINAL_STATE_GATE direct_reads={} reads_allocations={} reads_bytes={} warm_writes={} writes_allocations={} writes_bytes={} page_nodes={} page_allocations={} page_bytes={} mark_operations={} mark_allocations={} mark_bytes={}",
        DIRECT_READS,
        reads.calls,
        reads.requested_bytes,
        WARM_WRITES,
        writes.calls,
        writes.requested_bytes,
        PAGE_QUEUE_LEN,
        page_queue.calls,
        page_queue.requested_bytes,
        WARM_WRITES,
        mark_classes.calls,
        mark_classes.requested_bytes,
    );

    if failures.is_empty() {
        println!("final-state-gate: all budgets met");
    } else if enforce {
        for failure in failures {
            eprintln!("final-state-gate: {failure}");
        }
        std::process::exit(1);
    } else {
        println!(
            "final-state-gate: {} budget violation(s); rerun with --enforce to fail",
            failures.len()
        );
    }
}

fn hot_state_gate() -> (
    AllocationDelta,
    AllocationDelta,
    AllocationDelta,
    AllocationDelta,
) {
    with_universe(engine_budget(), |universe| {
        let symbol = universe
            .intern("phase-eight-direct-read")
            .expect("intern")
            .symbol();
        {
            let mut context = universe.command_context().expect("admit command context");
            context
                .assign_resolved_meaning(
                    symbol,
                    ResolvedMeaning::Static(Meaning::Relax),
                    AssignmentScope::Global,
                )
                .expect("install direct meaning");
            context
                .assign_count(0, 0, AssignmentScope::Global)
                .expect("warm count cell");
        }

        let reads = {
            let context = universe.command_context().expect("read context");
            measure(HotCoreAllocationOwner::DeliveryAndScan, || {
                direct_reads(&context, symbol)
            })
        };
        let write_mark = universe.journal_cursor().expect("warm journal cursor");
        {
            let mut context = universe.command_context().expect("write context");
            context
                .assign_count(0, 1, AssignmentScope::Global)
                .expect("prime one rollback slice");
        }
        universe
            .restore_state(write_mark)
            .expect("restore priming write");
        let writes = measure(HotCoreAllocationOwner::SemanticApply, || {
            for value in 0..WARM_WRITES {
                {
                    let mut context = universe.command_context().expect("write context");
                    context
                        .assign_count(0, value as i32, AssignmentScope::Global)
                        .expect("same admitted count cell remains writable");
                }
                universe
                    .restore_state(write_mark)
                    .expect("discard one operation-local journal slice");
            }
            black_box(universe.count(0).expect("read restored count"));
        });

        {
            let mut context = universe.command_context().expect("page context");
            for index in 0..PAGE_QUEUE_LEN {
                context.append_page_contribution(Node::Penalty(index as i32));
            }
            while context.pop_page_contribution_front().is_some() {}
        }
        let page_queue = measure(HotCoreAllocationOwner::SemanticApply, || {
            let mut context = universe.command_context().expect("page context");
            for index in 0..PAGE_QUEUE_LEN {
                context.append_page_contribution(Node::Penalty(index as i32));
            }
            for index in 0..PAGE_QUEUE_LEN {
                assert_eq!(
                    context.pop_page_contribution_front(),
                    Some(Node::Penalty(index as i32))
                );
            }
        });
        let mark_words =
            NodeTokenList::new(vec![TokenWord::pack(Token::param(1))].into_boxed_slice());
        {
            let mut context = universe.command_context().expect("page context");
            context.set_page_mark_class(PageMark::Bot, 32_767, mark_words);
        }
        let mark_classes = measure(HotCoreAllocationOwner::SemanticApply, || {
            let mut context = universe.command_context().expect("page context");
            for _ in 0..WARM_WRITES {
                black_box(
                    context
                        .page_mark_class_value(PageMark::Bot, 32_767)
                        .expect("sparse mark class remains live")
                        .words()
                        .len(),
                );
                context.set_page_mark_class(PageMark::Top, 32_767, NodeTokenList::default());
                context.clear_page_mark_class(PageMark::Top, 32_767);
            }
        });
        (reads, writes, page_queue, mark_classes)
    })
    .expect("final state gate universe")
}

fn direct_reads<G>(context: &tex_state::CommandContext<'_, G>, symbol: Symbol) {
    let mut checksum = 0_u64;
    for index in 0..DIRECT_READS {
        checksum ^= match context.meaning(black_box(symbol)) {
            ResolvedMeaning::Static(Meaning::Relax) => index as u64,
            _ => unreachable!("warm fixture retains the direct static meaning"),
        };
        checksum ^= context.count(0).expect("warm count cell") as u64;
    }
    black_box(checksum);
}

fn measure(owner: HotCoreAllocationOwner, operation: impl FnOnce()) -> AllocationDelta {
    let before = hot_core_census().allocations[owner as usize];
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        operation();
    }
    let after = hot_core_census().allocations[owner as usize];
    AllocationDelta::between(before, after)
}

fn check_zero(name: &str, delta: AllocationDelta, failures: &mut Vec<String>) {
    if delta.calls != 0 || delta.requested_bytes != 0 {
        failures.push(format!(
            "{name} allocated {} time(s), requesting {} byte(s)",
            delta.calls, delta.requested_bytes
        ));
    }
}

fn generation_lifecycle_gate(failures: &mut Vec<String>) {
    let baseline = retained_generation_census();
    let store = ReachabilityStore::new(engine_budget());
    let prior = RetainedStateGeneration::new(&store, World::memory()).expect("prior generation");
    let rejected =
        RetainedStateGeneration::new(&store, World::memory()).expect("candidate generation");
    let two_live = retained_generation_census();
    if two_live.live != baseline.live + 2 {
        failures.push(format!(
            "prior/current admission retained {} owners, expected {}",
            two_live.live,
            baseline.live + 2
        ));
    }

    drop(rejected);
    let after_rejection = retained_generation_census();
    if after_rejection.live != baseline.live + 1 {
        failures.push("candidate rejection did not drop exactly current".to_owned());
    }

    let accepted =
        RetainedStateGeneration::new(&store, World::memory()).expect("replacement candidate");
    drop(prior);
    let after_acceptance = retained_generation_census();
    if after_acceptance.live != baseline.live + 1 {
        failures.push("acceptance did not drop whole prior generation".to_owned());
    }
    accepted
        .retire()
        .expect("explicit terminal retirement succeeds");
    let terminal = retained_generation_census();
    if terminal.live != baseline.live
        || terminal.created.saturating_sub(baseline.created) != 3
        || terminal.dropped.saturating_sub(baseline.dropped) != 3
        || terminal
            .retired_explicitly
            .saturating_sub(baseline.retired_explicitly)
            != 1
    {
        failures.push(format!(
            "coarse lifecycle mismatch: baseline={baseline:?} terminal={terminal:?}"
        ));
    }
    println!(
        "RETAINED_GENERATION_GATE created={} dropped={} max_simultaneous=2 terminal_live={} explicit_retire={}",
        terminal.created.saturating_sub(baseline.created),
        terminal.dropped.saturating_sub(baseline.dropped),
        terminal.live.saturating_sub(baseline.live),
        terminal
            .retired_explicitly
            .saturating_sub(baseline.retired_explicitly),
    );
}
