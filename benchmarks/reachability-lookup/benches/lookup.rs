use criterion::{Criterion, black_box, criterion_group, criterion_main};
use tex_state::lookup_benchmark::{LookupCase, LookupFamily, ReachabilityLookupBenchmark};

fn reachability_owned_lookup(c: &mut Criterion) {
    let benchmark = ReachabilityLookupBenchmark::new();
    let mut group = c.benchmark_group("reachability_owned_lookup");
    for family in [LookupFamily::TokenList, LookupFamily::MacroBody] {
        let family_name = match family {
            LookupFamily::TokenList => "token_list",
            LookupFamily::MacroBody => "macro_body",
        };
        for case in LookupCase::ALL {
            group.bench_function(format!("{family_name}/{}", case.name()), |b| {
                b.iter(|| black_box(benchmark.measure(family, case)));
            });
        }
    }
    group.finish();
}

criterion_group!(benches, reachability_owned_lookup);
criterion_main!(benches);
