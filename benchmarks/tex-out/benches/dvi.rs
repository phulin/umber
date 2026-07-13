use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_arith::Scaled;
use tex_out::dvi::{DviPagePlan, DviPagePlanBuilder, DviStreamWriter, write_dvi};
use tex_out::{
    BoxNode, ContentHash, FontResource, GlueOrder, GlueSetRatio, GlueSign, JobInfo, PageArtifact,
    PageNode, UnvalidatedPageArtifact, V10ArtifactBuilder,
};

const FLAT_GLYPHS: usize = 4_096;
const NESTED_LINES: usize = 64;
const GLYPHS_PER_LINE: usize = 80;
const FONT_COUNT: usize = 64;

struct Fixture {
    name: &'static str,
    page: PageArtifact,
    v10: Vec<u8>,
    plan: DviPagePlan,
    nodes: u64,
}

fn dvi(c: &mut Criterion) {
    let fixtures = fixtures();
    fresh_commit(c, &fixtures);
    plan_compile(c, &fixtures);
    final_emit(c, &fixtures);
}

fn fresh_commit(c: &mut Criterion, fixtures: &[Fixture]) {
    let mut group = c.benchmark_group("dvi/fresh_commit");
    for fixture in fixtures {
        group.throughput(Throughput::Elements(fixture.nodes));
        group.bench_with_input(
            BenchmarkId::new("v10_only", fixture.name),
            fixture,
            |b, fixture| {
                let (root, vertical) = root_box(&fixture.page);
                b.iter(|| {
                    let mut artifact = V10ArtifactBuilder::new(
                        fixture.page.job.clone(),
                        fixture.page.counts,
                        root,
                        vertical,
                    );
                    for child in &root.children {
                        artifact.push_node(black_box(child)).expect("encode child");
                    }
                    black_box(
                        artifact
                            .finish(&fixture.page.fonts, &fixture.page.effects)
                            .expect("finish v10 artifact"),
                    );
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("v10_plus_dvi_plan", fixture.name),
            fixture,
            |b, fixture| {
                let (root, vertical) = root_box(&fixture.page);
                b.iter(|| {
                    let mut artifact = V10ArtifactBuilder::new(
                        fixture.page.job.clone(),
                        fixture.page.counts,
                        root,
                        vertical,
                    );
                    let mut plan = DviPagePlanBuilder::new(
                        fixture.page.job.clone(),
                        fixture.page.counts,
                        root,
                        vertical,
                    )
                    .expect("start DVI plan");
                    for child in &root.children {
                        let child = black_box(child);
                        artifact.push_node(child).expect("encode child");
                        plan.push_node(child, &fixture.page.fonts, &fixture.page.effects)
                            .expect("compile DVI child");
                    }
                    let artifact = artifact
                        .finish(&fixture.page.fonts, &fixture.page.effects)
                        .expect("finish v10 artifact");
                    let plan = plan.finish(&fixture.page.fonts).expect("finish DVI plan");
                    black_box((artifact, plan));
                });
            },
        );
    }
    group.finish();
}

fn plan_compile(c: &mut Criterion, fixtures: &[Fixture]) {
    let mut group = c.benchmark_group("dvi/plan_compile");
    for fixture in fixtures {
        group.throughput(Throughput::Elements(fixture.nodes));
        group.bench_with_input(
            BenchmarkId::new("owned_tree", fixture.name),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    black_box(
                        DviPagePlan::compile(black_box(&fixture.page)).expect("compile owned page"),
                    );
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("v10_stream", fixture.name),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    black_box(
                        DviPagePlan::compile_v10(black_box(&fixture.v10))
                            .expect("compile streamed page"),
                    );
                });
            },
        );
    }
    group.finish();
}

fn final_emit(c: &mut Criterion, fixtures: &[Fixture]) {
    let mut group = c.benchmark_group("dvi/final_emit");
    for fixture in fixtures {
        group.throughput(Throughput::Elements(fixture.nodes));
        group.bench_with_input(
            BenchmarkId::new("owned_traversal", fixture.name),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    let mut writer = DviStreamWriter::new(Vec::new());
                    writer
                        .write_page(black_box(&fixture.page))
                        .expect("write owned page");
                    black_box(writer.finish().expect("finish owned DVI"));
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("precompiled_plan", fixture.name),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    let mut writer = DviStreamWriter::new(Vec::new());
                    writer
                        .write_page_plan(black_box(&fixture.plan))
                        .expect("write planned page");
                    black_box(writer.finish().expect("finish planned DVI"));
                });
            },
        );
    }
    group.finish();
}

fn fixtures() -> Vec<Fixture> {
    vec![
        fixture("flat_text", flat_page()),
        fixture("nested_page", nested_page()),
        fixture("font_heavy", font_heavy_page()),
    ]
}

fn fixture(name: &'static str, page: PageArtifact) -> Fixture {
    let nodes = count_nodes(&page.root);
    let v10 = page.to_bytes().expect("serialize fixture");
    let plan = DviPagePlan::compile(&page).expect("compile fixture plan");
    assert_eq!(
        DviPagePlan::compile_v10(&v10).expect("compile fixture v10 plan"),
        plan,
        "owned and v10 plans differ for {name}",
    );
    let mut writer = DviStreamWriter::new(Vec::new());
    writer
        .write_page_plan(&plan)
        .expect("write fixture page plan");
    assert_eq!(
        writer.finish().expect("finish fixture planned DVI"),
        write_dvi(std::slice::from_ref(&page)).expect("write fixture owned DVI"),
        "owned and planned DVI differ for {name}",
    );
    Fixture {
        name,
        page,
        v10,
        plan,
        nodes,
    }
}

fn flat_page() -> PageArtifact {
    let children = (0..FLAT_GLYPHS)
        .map(|index| glyph(0, index))
        .collect::<Vec<_>>();
    page(vec![font(0)], hbox(children))
}

fn nested_page() -> PageArtifact {
    let lines = (0..NESTED_LINES)
        .map(|line| {
            let children = (0..GLYPHS_PER_LINE)
                .map(|column| glyph(0, line * GLYPHS_PER_LINE + column))
                .collect::<Vec<_>>();
            PageNode::HList(box_node(
                GLYPHS_PER_LINE as i32 * 32_768,
                524_288,
                131_072,
                children,
            ))
        })
        .collect::<Vec<_>>();
    page(
        vec![font(0)],
        PageNode::VList(box_node(
            GLYPHS_PER_LINE as i32 * 32_768,
            NESTED_LINES as i32 * 655_360,
            0,
            lines,
        )),
    )
}

fn font_heavy_page() -> PageArtifact {
    let fonts = (0..FONT_COUNT as u32).map(font).collect::<Vec<_>>();
    let children = (0..FLAT_GLYPHS)
        .map(|index| glyph((index % FONT_COUNT) as u32, index))
        .collect::<Vec<_>>();
    page(fonts, hbox(children))
}

fn page(fonts: Vec<FontResource>, root: PageNode) -> PageArtifact {
    UnvalidatedPageArtifact {
        job: JobInfo::default(),
        fonts,
        counts: [1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        root,
        effects: Vec::new(),
    }
    .validate()
    .expect("benchmark page validates")
}

fn hbox(children: Vec<PageNode>) -> PageNode {
    PageNode::HList(box_node(
        children.len() as i32 * 32_768,
        524_288,
        131_072,
        children,
    ))
}

fn box_node(width: i32, height: i32, depth: i32, children: Vec<PageNode>) -> BoxNode {
    BoxNode {
        width: sp(width),
        height: sp(height),
        depth: sp(depth),
        shift: sp(0),
        glue_set: GlueSetRatio::ZERO,
        glue_sign: GlueSign::Normal,
        glue_order: GlueOrder::Normal,
        children,
    }
}

fn glyph(font_id: u32, index: usize) -> PageNode {
    PageNode::Char {
        font_id,
        ch: (b'A' + (index % 26) as u8) as u32,
        width: sp(32_768),
    }
}

fn font(font_id: u32) -> FontResource {
    let name = format!("bench-font-{font_id}");
    FontResource {
        font_id,
        tfm_content_hash: ContentHash::from_bytes(name.as_bytes()),
        name,
        tfm_checksum: 0x1234_5678_u32.wrapping_add(font_id),
        design_size: sp(655_360),
        at_size: sp(655_360),
    }
}

fn root_box(page: &PageArtifact) -> (&BoxNode, bool) {
    match &page.root {
        PageNode::HList(root) => (root, false),
        PageNode::VList(root) => (root, true),
        _ => unreachable!("validated benchmark root is a box"),
    }
}

fn count_nodes(root: &PageNode) -> u64 {
    let mut count = 0_u64;
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        count += 1;
        match node {
            PageNode::HList(node) | PageNode::VList(node) => pending.extend(&node.children),
            PageNode::Disc {
                pre, post, replace, ..
            } => {
                pending.extend(pre);
                pending.extend(post);
                pending.extend(replace);
            }
            PageNode::Insert { content, .. } | PageNode::Adjust(content) => {
                pending.extend(content);
            }
            _ => {}
        }
    }
    count
}

const fn sp(value: i32) -> Scaled {
    Scaled::from_raw(value)
}

criterion_group!(benches, dvi);
criterion_main!(benches);
