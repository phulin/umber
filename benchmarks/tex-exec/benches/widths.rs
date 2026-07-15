use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_fonts::metrics::CharTag;
use tex_fonts::{CharMetrics, FontMetrics, LoadedFont};
use tex_state::Universe;
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::OriginId;
use tex_typeset::{HpackParams, PackSpec, hpack};

fn width_font(name: &str, salt: u8) -> LoadedFont {
    let mut characters = vec![None; 256];
    for code in 0_u8..=u8::MAX {
        characters[usize::from(code)] = Some(CharMetrics {
            width: Scaled::from_raw(20_000 + i32::from(code) * 97),
            height: Scaled::from_raw(30_000 + i32::from(code % 7)),
            depth: Scaled::from_raw(i32::from(code % 5)),
            italic_correction: Scaled::from_raw(0),
            tag: CharTag::None,
        });
    }
    LoadedFont::new(
        name,
        name,
        [salt; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(characters, Vec::new(), None, None, Vec::new()),
    )
}

fn same_font(count: usize) -> (Universe, tex_state::ids::NodeListId) {
    let mut state = Universe::new();
    let font = state.intern_font(width_font("width-run", 1));
    let nodes = (0..count)
        .map(|i| Node::Char {
            font,
            ch: char::from((32 + i % 95) as u8),
            origin: OriginId::UNKNOWN,
        })
        .collect::<Vec<_>>();
    let list = state.freeze_node_list(&nodes);
    (state, list)
}

fn mixed(count: usize) -> (Universe, tex_state::ids::NodeListId) {
    let mut state = Universe::new();
    let fonts = [
        state.intern_font(width_font("width-mixed-a", 2)),
        state.intern_font(width_font("width-mixed-b", 3)),
    ];
    let nodes = (0..count)
        .map(|i| {
            if i % 11 == 10 {
                Node::Kern {
                    amount: Scaled::from_raw(123),
                    kind: KernKind::Font,
                }
            } else {
                Node::Char {
                    font: fonts[(i / 37) & 1],
                    ch: char::from((32 + i % 95) as u8),
                    origin: OriginId::UNKNOWN,
                }
            }
        })
        .collect::<Vec<_>>();
    let list = state.freeze_node_list(&nodes);
    (state, list)
}

fn bench_widths(c: &mut Criterion) {
    let params = HpackParams {
        hbadness: 10_000,
        hfuzz: Scaled::from_raw(0),
        overfull_rule: Scaled::from_raw(0),
    };
    let mut group = c.benchmark_group("hpack_widths");
    for (name, prepared, count) in [
        ("same_font_64", same_font(64), 64_u64),
        ("same_font_4096", same_font(4096), 4096),
        ("mixed_4096", mixed(4096), 4096),
    ] {
        let (state, list) = prepared;
        group.throughput(Throughput::Elements(count));
        group.bench_function(name, |b| {
            b.iter(|| black_box(hpack(&state, list, PackSpec::Natural, params)))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_widths);
criterion_main!(benches);
