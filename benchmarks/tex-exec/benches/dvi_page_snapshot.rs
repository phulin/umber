use std::sync::Arc;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use tex_out::{
    BoxNode, GlueOrder, GlueSetRatio, GlueSign, JobInfo, PageArtifactBuilder, PageEffect, PageNode,
    UnvalidatedPageArtifact,
    dvi::DviPagePlan,
};
use tex_state::scaled::Scaled;

const BODY_BYTES: usize = 1 << 20;

fn page_plan() -> DviPagePlan {
    let artifact = PageArtifactBuilder::new(UnvalidatedPageArtifact {
        job: JobInfo::default(),
        fonts: Vec::new(),
        counts: [0; 10],
        root: PageNode::VList(BoxNode {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            glue_set: GlueSetRatio::ZERO,
            glue_sign: GlueSign::Normal,
            glue_order: GlueOrder::Normal,
            children: vec![PageNode::WhatsitAnchor { effect_index: 0 }],
        }),
        effects: vec![PageEffect::Special {
            class: "benchmark".into(),
            payload: vec![b'x'; BODY_BYTES],
        }],
        math_events: Vec::new(),
    })
    .build()
    .expect("bounded benchmark artifact validates");
    DviPagePlan::compile(&artifact).expect("bounded benchmark page compiles")
}

fn dvi_page_snapshot(c: &mut Criterion) {
    let owned = vec![page_plan()];
    let shared = Arc::new(owned.clone());
    let mut group = c.benchmark_group("dvi_page_snapshot");
    group.bench_function("deep_clone", |b| {
        b.iter(|| black_box(black_box(&owned).clone()));
    });
    group.bench_function("shared_clone", |b| {
        b.iter(|| black_box(Arc::clone(black_box(&shared))));
    });
    group.finish();
}

criterion_group!(benches, dvi_page_snapshot);
criterion_main!(benches);
