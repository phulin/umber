use std::sync::Arc;

use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use tex_command::{
    CommandDialect, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandProfile,
    CommandState, RegisteredSourceKind, SourceRegistration, install_tex82_expandable_primitives,
};
use tex_state::ProvenanceResolver;
use tex_state::SourceId;
use tex_state::cell::{BankTag, CellId};
use tex_state::glue::Order;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::math::{MathChar, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::meaning::Meaning;
use tex_state::meaning::MeaningFlags;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, KernKind, Node, Sign};
use tex_state::node_arena::NodeListRef;
use tex_state::provenance::{OriginRef, ProvenanceStats};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::source_map::SourceDescriptor;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{DependencyKey, DependencyRuntime, DependencyValue};
use tex_state::{EditorLayout, FragmentStore, LayoutGeneration, Piece, Universe};

const GROUP_SIZES: [usize; 3] = [4, 64, 512];
const ROLLBACK_TOTAL_CELLS: [usize; 2] = [1024, 4096];
const ROLLBACK_SLICE_WRITES: [usize; 3] = [4, 64, 512];
const PAGE_DISTINCT_CELLS: usize = 500;
const PAGE_TOTAL_WRITES: usize = 5_000;
const SOURCE_HEAVY_LINES: usize = 512;
const SOURCE_HEAVY_LINE: &str = "alpha beta gamma delta epsilon zeta eta theta";
const MIXED_UTF8_LINE: &str = "alpha βήτα 世界 café naïve 🦀 zeta";
const CONTROL_SEQUENCE_LINE: &str = "\\alpha\\beta\\gamma\\delta\\epsilon\\zeta\\eta\\theta";
const LONG_LINE_SCALARS: usize = 65_536;
const MACRO_CALLS: usize = 2_048;
const MACRO_BODY_LEN: usize = 16;
const MACRO_PROVENANCE_RETAINED_BYTES_PER_INVOCATION_BUDGET: usize = 64;
const SCANNER_REPETITIONS: usize = 1_024;
const TRANSIENT_BOX_OVERWRITES: usize = 20_000;
const DEEP_BOX_LOCALITY_JOURNAL: usize = 20_000;
const ALLOCATION_LIST_LEN: usize = 1_024;
const PAGE_QUEUE_LEN: usize = 65_536;
const TOKEN_PROJECTION_SIZES: [usize; 3] = [64, 1_024, 16_384];
const TOKEN_PROJECTION_WORKLOADS: [&str; 3] = [
    "characters",
    "control_sequences",
    "internal_control_sequences",
];
const EDIT_STABLE_PIECE_COUNTS: [usize; 5] = [64, 256, 1_024, 4_096, 16_384];
const DEPENDENCY_READS: usize = 4_096;

const HASH_MIX_INCREMENT: u64 = 0x9e37_79b9_7f4a_7c15;
const HASH_INITIAL_STATE: u64 = 0x6a09_e667_f3bc_c909;

fn dependency_recording(c: &mut Criterion) {
    let key = DependencyKey::Cell(CellId::new(BankTag::Meaning, 7));
    let value = DependencyValue::Integer(42);
    let mut group = c.benchmark_group("dependency_recording");
    group.throughput(Throughput::Elements(DEPENDENCY_READS as u64));

    group.bench_function("disabled", |b| {
        b.iter(|| {
            let mut runtime = DependencyRuntime::default();
            for _ in 0..DEPENDENCY_READS {
                runtime.record(key, value.clone());
            }
            black_box(runtime);
        });
    });
    group.bench_function("enabled_deduplicated", |b| {
        b.iter(|| {
            let mut runtime = DependencyRuntime::default();
            let token = runtime.begin_region().expect("begin dependency region");
            for _ in 0..DEPENDENCY_READS {
                runtime.record(key, value.clone());
            }
            black_box(
                runtime
                    .finish_region(token)
                    .expect("finish dependency region"),
            );
        });
    });
    group.bench_function("interleaved_pair", |b| {
        b.iter(|| {
            let mut disabled = DependencyRuntime::default();
            let mut enabled = DependencyRuntime::default();
            let token = enabled.begin_region().expect("begin dependency region");
            for _ in 0..DEPENDENCY_READS {
                disabled.record(key, value.clone());
                enabled.record(key, value.clone());
            }
            let observations = enabled
                .finish_region(token)
                .expect("finish dependency region");
            black_box((disabled, observations));
        });
    });
    group.finish();
}

fn page_contribution_queue(c: &mut Criterion) {
    c.bench_function("page_contribution_queue/drain_65536", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                for index in 0..PAGE_QUEUE_LEN {
                    stores.append_page_contribution(Node::Penalty(index as i32));
                }
                stores
            },
            |mut stores| {
                while let Some(node) = stores.pop_page_contribution_front() {
                    black_box(node);
                }
            },
            BatchSize::LargeInput,
        );
    });
}

fn allocation_node_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_node_append");
    for shape in ["inline", "box", "math", "mixed"] {
        group.throughput(Throughput::Elements(ALLOCATION_LIST_LEN as u64));
        group.bench_function(shape, |b| {
            b.iter_batched(
                || allocation_append_case(shape),
                |(mut stores, nodes)| black_box(stores.freeze_node_list(&nodes)),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn allocation_append_case(shape: &str) -> (Universe, Vec<Node>) {
    let mut stores = Universe::new();
    let empty = stores.freeze_node_list(&[]);
    let font = stores.current_font();
    let nodes = (0..ALLOCATION_LIST_LEN)
        .map(|index| match shape {
            "inline" => Node::Kern {
                amount: Scaled::from_raw(index as i32),
                kind: KernKind::Explicit,
            },
            "box" => Node::HList(benchmark_box(empty.clone(), index as i32)),
            "math" => Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::MathChar(MathChar {
                    family: (index % 16) as u8,
                    character: char::from(b'a' + (index % 26) as u8),
                    origin: OriginId::UNKNOWN,
                }),
            )),
            "mixed" => match index % 4 {
                0 => Node::Char {
                    font,
                    ch: char::from(b'a' + (index % 26) as u8),
                    origin: OriginRef::unknown(),
                },
                1 => Node::HList(benchmark_box(empty.clone(), index as i32)),
                2 => Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Ord),
                    MathField::Empty,
                )),
                _ => Node::Rule {
                    width: Some(Scaled::from_raw(index as i32)),
                    height: None,
                    depth: None,
                },
            },
            _ => unreachable!("known allocation append shape"),
        })
        .collect();
    (stores, nodes)
}

fn deep_journal_box_locality(c: &mut Criterion) {
    c.bench_function("box_locality/same_level_after_20000_entries", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                let value = stores.freeze_node_list(&[Node::Penalty(1)]);
                stores.enter_group();
                stores.set_box_reg_ref(0, value);
                for write in 0..DEEP_BOX_LOCALITY_JOURNAL {
                    let symbol = stores.intern(&format!("deepbox{write}"));
                    stores.set_meaning(symbol, Meaning::Relax);
                }
                stores
            },
            |mut stores| {
                let value = stores
                    .box_reg_ref(0)
                    .expect("benchmark box remains live");
                for _ in 0..1_000 {
                    stores.set_box_reg_same_level(0, value.clone());
                }
                black_box(stores.box_reg_ref(0))
            },
            BatchSize::LargeInput,
        );
    });
}

fn allocation_traced_freeze(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_traced_freeze");
    for &(name, len, preintern) in &[
        ("short_miss", 8, false),
        ("short_hit", 8, true),
        ("long_miss", 1_024, false),
        ("long_hit_distinct_origins", 1_024, true),
    ] {
        group.throughput(Throughput::Elements(len as u64));
        group.bench_function(name, |b| {
            b.iter_batched(
                || traced_freeze_case(len, preintern),
                |(mut stores, traced)| {
                    black_box(stores.finish_traced_token_list(&traced));
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn benchmark_box(children: NodeListRef, value: i32) -> BoxNode {
    BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(value),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    })
}

fn traced_freeze_case(len: usize, preintern: bool) -> (Universe, Vec<TracedTokenWord>) {
    let mut stores = Universe::new();
    let semantic = (0..len)
        .map(|index| char_token(char::from(b'a' + (index % 26) as u8)))
        .collect::<Vec<_>>();
    if preintern {
        stores.intern_token_list(&semantic);
    }
    let mut traced = Vec::with_capacity(len);
    for (index, token) in semantic.into_iter().enumerate() {
        let origin = stores.source_origin(SourceId::new(7), index as u64, 1, index as u32 + 1);
        traced.push(TracedTokenWord::pack(token, origin));
    }
    (stores, traced)
}

fn meaning_lookup(c: &mut Criterion) {
    let mut stores = Universe::new();
    let symbol = stores.intern("warm-cell");
    stores.set_meaning(symbol, Meaning::Relax);

    c.bench_function("meaning_lookup/warm_cell_hit", |b| {
        b.iter(|| black_box(stores.meaning(black_box(symbol))));
    });
}

fn barrier_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("barrier_write");

    group.bench_function("journal_push_path", |b| {
        let mut stores = Universe::new();
        let symbol = stores.intern("push-path");
        let mut operand = 0_u64;

        b.iter(|| {
            operand = operand.wrapping_add(1);
            stores.set_meaning(black_box(symbol), black_box(raw_meaning(operand)));
            stores.enter_group();
            black_box(stores.leave_group());
        });

        black_box(stores);
    });

    group.bench_function("already_stamped_skip_path", |b| {
        let mut stores = Universe::new();
        let symbol = stores.intern("skip-path");
        stores.set_meaning(symbol, Meaning::Relax);
        let mut operand = 0_u64;

        b.iter(|| {
            operand = operand.wrapping_add(1);
            stores.set_meaning(black_box(symbol), black_box(raw_meaning(operand)));
        });

        black_box(stores);
    });

    group.finish();
}

fn snapshot_take(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_take");

    group.bench_function("steady_empty_slice", |b| {
        let mut stores = Universe::new();
        b.iter(|| {
            let snapshot = stores.snapshot();
            black_box(snapshot.state_hash());
        });
        black_box(stores);
    });

    group.finish();
}

fn checkpoint_state_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("checkpoint_state_hash");

    group.bench_function("after_synthetic_page", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                let symbols = synthetic_page_symbols(&mut stores);
                for write_index in 0..PAGE_TOTAL_WRITES {
                    let symbol = symbols[write_index % symbols.len()];
                    stores.set_meaning(symbol, raw_meaning(write_index as u64));
                }
                stores
            },
            |mut stores| {
                let snapshot = stores.snapshot();
                black_box(snapshot.state_hash());
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("font_dense_box_tree", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                let _ = stores.snapshot();
                let font = stores.current_font();
                let chars = (0..ALLOCATION_LIST_LEN)
                    .map(|index| Node::Char {
                        font,
                        ch: char::from(b'a' + (index % 26) as u8),
                        origin: OriginRef::unknown(),
                    })
                    .collect::<Vec<_>>();
                let list = stores.freeze_node_list(&chars);
                stores.set_box_reg_ref(0, list);
                stores
            },
            |mut stores| black_box(stores.snapshot().state_hash()),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("shared_token_list_256x1024", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                let symbols = (0..64)
                    .map(|index| stores.intern(&format!("checkpoint-symbol-{index}")))
                    .collect::<Vec<_>>();
                let tokens = (0..1_024)
                    .map(|index| {
                        if index % 3 == 0 {
                            Token::Cs(symbols[index % symbols.len()].symbol())
                        } else {
                            char_token(char::from(b'a' + (index % 26) as u8))
                        }
                    })
                    .collect::<Vec<_>>();
                let list = stores.intern_token_list(&tokens);
                let _ = stores.snapshot();
                for index in 0..256 {
                    stores.set_toks(index, list);
                }
                stores
            },
            |mut stores| black_box(stores.snapshot().state_hash()),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("distinct_macro_bodies_256x64", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                let targets = (0..256)
                    .map(|index| stores.intern(&format!("checkpoint-macro-{index}")))
                    .collect::<Vec<_>>();
                let body_symbols = (0..32)
                    .map(|index| stores.intern(&format!("checkpoint-body-symbol-{index}")))
                    .collect::<Vec<_>>();
                let empty = stores.intern_token_list(&[]);
                let _ = stores.snapshot();
                for (index, target) in targets.into_iter().enumerate() {
                    let body = (0..64)
                        .map(|offset| {
                            if offset % 4 == 0 {
                                Token::Cs(
                                    body_symbols[(index + offset) % body_symbols.len()].symbol(),
                                )
                            } else {
                                char_token(char::from(b'a' + ((index + offset) % 26) as u8))
                            }
                        })
                        .collect::<Vec<_>>();
                    let body = stores.intern_token_list(&body);
                    stores.set_macro_meaning(
                        target,
                        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
                    );
                }
                stores
            },
            |mut stores| black_box(stores.snapshot().state_hash()),
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn token_semantic_projection(c: &mut Criterion) {
    for workload in TOKEN_PROJECTION_WORKLOADS {
        let mut group = c.benchmark_group(format!("token_semantic_projection/{workload}"));
        for size in TOKEN_PROJECTION_SIZES {
            let (stores, tokens) = token_projection_case(workload, size);
            let mut projection = Vec::new();
            encode_token_projection(&stores, &tokens, &mut projection);
            let fingerprint = hash_projection(&projection);

            group.throughput(Throughput::Elements(size as u64));
            group.bench_with_input(BenchmarkId::new("direct", size), &size, |b, _| {
                b.iter(|| black_box(hash_tokens_direct(&stores, &tokens)));
            });
            group.bench_with_input(BenchmarkId::new("encode", size), &size, |b, _| {
                let mut output = Vec::with_capacity(projection.len());
                b.iter(|| {
                    output.clear();
                    encode_token_projection(&stores, &tokens, &mut output);
                    black_box(&output);
                });
            });
            group.bench_with_input(BenchmarkId::new("replay", size), &size, |b, _| {
                b.iter(|| black_box(hash_projection(&projection)));
            });
            group.bench_with_input(BenchmarkId::new("compose", size), &size, |b, _| {
                let mut changing_fingerprint = fingerprint;
                b.iter(|| {
                    changing_fingerprint = black_box(changing_fingerprint).wrapping_add(1);
                    black_box(hash_words(0x7061_7265_6e74, [changing_fingerprint]))
                });
            });
        }
        group.finish();
    }
}

fn token_projection_freeze_cost(c: &mut Criterion) {
    for workload in TOKEN_PROJECTION_WORKLOADS {
        let mut group = c.benchmark_group(format!("token_projection_freeze/{workload}"));
        for size in TOKEN_PROJECTION_SIZES {
            group.throughput(Throughput::Elements(size as u64));
            group.bench_with_input(BenchmarkId::new("current", size), &size, |b, &size| {
                b.iter_batched(
                    || token_projection_case(workload, size),
                    |(mut stores, tokens)| black_box(stores.intern_token_list(&tokens)),
                    BatchSize::SmallInput,
                );
            });
            group.bench_with_input(
                BenchmarkId::new("plus_canonical_fingerprint", size),
                &size,
                |b, &size| {
                    b.iter_batched(
                        || token_projection_case(workload, size),
                        |(mut stores, tokens)| {
                            let fingerprint = hash_tokens_direct(&stores, &tokens);
                            let id = stores.intern_token_list(&tokens);
                            black_box((id, fingerprint));
                        },
                        BatchSize::SmallInput,
                    );
                },
            );
        }
        group.finish();
    }
}

fn token_projection_case(workload: &str, size: usize) -> (Universe, Vec<Token>) {
    let mut stores = Universe::new();
    let tokens = match workload {
        "characters" => (0..size)
            .map(|index| Token::Char {
                ch: char::from(b'a' + (index % 26) as u8),
                cat: Catcode::Letter,
            })
            .collect(),
        "control_sequences" => {
            let symbols = (0..64)
                .map(|index| stores.intern(&format!("projection-control-sequence-{index}")))
                .collect::<Vec<_>>();
            (0..size)
                .map(|index| Token::Cs(symbols[index % symbols.len()].symbol()))
                .collect()
        }
        "internal_control_sequences" => {
            let symbols = (0..64)
                .map(|index| {
                    stores.intern_internal_control_sequence(&format!(
                        "projection-internal-control-sequence-{index}"
                    ))
                })
                .collect::<Vec<_>>();
            (0..size)
                .map(|index| Token::Cs(symbols[index % symbols.len()].symbol()))
                .collect()
        }
        _ => unreachable!("unknown token projection workload"),
    };
    (stores, tokens)
}

fn hash_tokens_direct(stores: &Universe, tokens: &[Token]) -> u64 {
    let mut state = HASH_INITIAL_STATE ^ 0x746f_6b65_6e5f_6c73;
    mix_hash(&mut state, 0x50);
    mix_hash(&mut state, tokens.len() as u64);
    for &token in tokens {
        encode_token(stores, token, |word| mix_hash(&mut state, word));
    }
    splitmix64(state)
}

fn encode_token_projection(stores: &Universe, tokens: &[Token], output: &mut Vec<u64>) {
    output.push(0x50);
    output.push(tokens.len() as u64);
    for &token in tokens {
        encode_token(stores, token, |word| output.push(word));
    }
}

fn encode_token(stores: &Universe, token: Token, mut push: impl FnMut(u64)) {
    match token {
        Token::Char { ch, cat } => {
            push(0);
            push(ch as u32 as u64);
            push(cat as u8 as u64);
        }
        Token::Cs(symbol) => {
            push(1);
            push(match stores.control_sequence_kind(symbol) {
                tex_state::interner::ControlSequenceKind::Null
                | tex_state::interner::ControlSequenceKind::SingleCharacter
                | tex_state::interner::ControlSequenceKind::Named => 0,
                tex_state::interner::ControlSequenceKind::ActiveCharacter => 1,
                tex_state::interner::ControlSequenceKind::Internal => 2,
            });
            let bytes = stores.resolve(symbol).as_bytes();
            push(bytes.len() as u64);
            for chunk in bytes.chunks(8) {
                let mut word = 0_u64;
                for (offset, byte) in chunk.iter().copied().enumerate() {
                    word |= u64::from(byte) << (offset * 8);
                }
                push(word);
            }
        }
        Token::Param(slot) => {
            push(2);
            push(u64::from(slot));
        }
        Token::Frozen(_) => unreachable!("benchmark cannot construct frozen tokens"),
    }
}

fn hash_projection(words: &[u64]) -> u64 {
    hash_words(0x746f_6b65_6e5f_6c73, words.iter().copied())
}

fn hash_words(domain: u64, words: impl IntoIterator<Item = u64>) -> u64 {
    let mut state = HASH_INITIAL_STATE ^ domain;
    for word in words {
        mix_hash(&mut state, word);
    }
    splitmix64(state)
}

fn mix_hash(state: &mut u64, value: u64) {
    *state = splitmix64(*state ^ value.wrapping_add(HASH_MIX_INCREMENT));
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(HASH_MIX_INCREMENT);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn transient_box_overwrite_checkpoint(c: &mut Criterion) {
    c.bench_function("checkpoint_state_hash/transient_box_overwrites", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                let _ = stores.snapshot();
                for amount in 0..TRANSIENT_BOX_OVERWRITES {
                    let children = stores.freeze_node_list(&[Node::Kern {
                        amount: Scaled::from_raw(amount as i32),
                        kind: KernKind::Explicit,
                    }]);
                    let list =
                        stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
                            width: Scaled::from_raw(amount as i32),
                            height: Scaled::from_raw(0),
                            depth: Scaled::from_raw(0),
                            shift: Scaled::from_raw(0),
                            box_lr: BoxLr::Normal,
                            glue_set: GlueSetRatio::ZERO,
                            glue_sign: Sign::Normal,
                            glue_order: Order::Normal,
                            children,
                        }))]);
                    stores.set_box_reg_ref(0, list);
                }
                stores
            },
            |mut stores| black_box(stores.snapshot().state_hash()),
            BatchSize::LargeInput,
        );
    });
}

fn group_cycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("group_cycle");

    for write_count in GROUP_SIZES {
        group.throughput(Throughput::Elements(write_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(write_count),
            &write_count,
            |b, &write_count| {
                let mut stores = Universe::new();
                let symbols = (0..write_count)
                    .map(|index| stores.intern(&format!("group-cell-{index}")))
                    .collect::<Vec<_>>();

                b.iter(|| {
                    stores.enter_group();
                    for (index, &symbol) in symbols.iter().enumerate() {
                        stores.set_meaning(black_box(symbol), black_box(raw_meaning(index as u64)));
                    }
                    black_box(stores.leave_group());
                });

                black_box(stores);
            },
        );
    }

    group.finish();
}

fn rollback_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("rollback_scaling");

    for total_cells in ROLLBACK_TOTAL_CELLS {
        for slice_writes in ROLLBACK_SLICE_WRITES {
            group.throughput(Throughput::Elements(slice_writes as u64));
            group.bench_with_input(
                BenchmarkId::new(format!("total_{total_cells}"), slice_writes),
                &(total_cells, slice_writes),
                |b, &(total_cells, slice_writes)| {
                    b.iter_batched_ref(
                        || rollback_case(total_cells, slice_writes),
                        |(stores, snapshot)| {
                            stores.rollback(black_box(snapshot));
                            black_box(stores);
                        },
                        BatchSize::SmallInput,
                    );
                },
            );
        }
    }

    group.finish();
}

fn group_global_compaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("group_global_compaction");

    group.bench_function("mixed_global_local_same_cell", |b| {
        let mut stores = Universe::new();
        let symbol = stores.intern("global-compaction-cell");
        let mut operand = 0_u64;

        b.iter(|| {
            operand = operand.wrapping_add(1);
            stores.enter_group();
            stores.set_meaning(black_box(symbol), black_box(raw_meaning(operand)));
            stores.set_meaning_global(black_box(symbol), black_box(raw_meaning(operand + 1)));
            stores.set_meaning(black_box(symbol), black_box(raw_meaning(operand + 2)));
            stores.set_meaning_global(black_box(symbol), black_box(raw_meaning(operand + 3)));
            black_box(stores.leave_group());
        });

        black_box(stores);
    });

    group.finish();
}

fn synthetic_page_journal_volume(c: &mut Criterion) {
    let bytes = synthetic_page_journal_bytes();
    let mut group = c.benchmark_group("synthetic_page");
    group.throughput(Throughput::Bytes(bytes as u64));

    group.bench_function("500_distinct_cells_5000_total_writes", |b| {
        b.iter(|| {
            let bytes = write_synthetic_page();
            black_box(bytes);
        });
    });

    group.finish();
}

fn provenance_source_lexing(c: &mut Criterion) {
    let mut group = c.benchmark_group("provenance_source_lexing");
    for (name, input, needs_control_sequences) in source_workloads() {
        let token_count = source_heavy_token_count(&input);
        group.throughput(Throughput::Elements(token_count as u64));
        group.bench_with_input(BenchmarkId::new("traced", name), &input, |b, input| {
            b.iter_batched(
                || source_case(input.clone(), needs_control_sequences),
                |(mut stores, mut command, mut capabilities)| {
                    black_box(drain_traced_source_timed(
                        &mut stores,
                        &mut command,
                        &mut capabilities,
                    ));
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn provenance_diagnostic_rendering(c: &mut Criterion) {
    let input = mixed_utf8_text();
    let mut group = c.benchmark_group("provenance_diagnostic_rendering");

    group.bench_function("cold", |b| {
        b.iter_batched(
            || diagnostic_case(input.clone()),
            |(stores, origin)| {
                black_box(
                    ProvenanceResolver::new(&stores)
                        .render_diagnostic("measured diagnostic", Some(origin)),
                );
            },
            BatchSize::SmallInput,
        );
    });

    let (stores, origin) = diagnostic_case(input);
    group.bench_function("repeated_warm", |b| {
        let resolver = ProvenanceResolver::new(&stores);
        b.iter(|| {
            black_box(resolver.render_diagnostic("measured diagnostic", Some(origin)));
        });
    });
    group.finish();
}

fn edit_stable_source_coordinates(c: &mut Criterion) {
    let universe = Universe::new();
    let mut group = c.benchmark_group("edit_stable_source_coordinates");
    let resolver = ProvenanceResolver::new(&universe);

    let (fragments, pieces) = edit_stable_piece_case(4_096);
    group.throughput(Throughput::Elements(4_096));
    group.bench_function("layout_build_4096_pieces", |b| {
        b.iter(|| {
            black_box(
                EditorLayout::new(
                    "<benchmark>",
                    LayoutGeneration::new(1),
                    black_box(pieces.clone()),
                    black_box(&fragments),
                )
                .expect("benchmark layout is valid"),
            );
        });
    });
    group.bench_function("resolve_last_piece_cold_4096_pieces", |b| {
        b.iter_batched(
            || edit_stable_layout_case(4_096),
            |(fragments, layout, origin, _)| {
                black_box(resolver.resolve_layout_origin(origin, &fragments, &layout));
            },
            BatchSize::SmallInput,
        );
    });

    for piece_count in EDIT_STABLE_PIECE_COUNTS {
        let (fragments, layout, origin, deleted_origin) = edit_stable_layout_case(piece_count);
        group.throughput(Throughput::Elements(piece_count as u64));

        black_box(resolver.resolve_layout_origin(origin, &fragments, &layout));
        group.bench_with_input(
            BenchmarkId::new("resolve_last_piece_warm", piece_count),
            &piece_count,
            |b, _| {
                b.iter(|| {
                    black_box(resolver.resolve_layout_origin(origin, &fragments, &layout));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("resolve_deleted", piece_count),
            &piece_count,
            |b, _| {
                b.iter(|| {
                    black_box(resolver.resolve_layout_origin(deleted_origin, &fragments, &layout));
                });
            },
        );
    }
    group.finish();
}

fn edit_stable_layout_case(
    piece_count: usize,
) -> (FragmentStore, EditorLayout, OriginId, OriginId) {
    let (fragments, pieces) = edit_stable_piece_case(piece_count);
    let registration = fragments
        .registration(pieces[0].fragment())
        .expect("benchmark fragment remains registered");
    let layout = EditorLayout::new("<benchmark>", LayoutGeneration::new(1), pieces, &fragments)
        .expect("benchmark layout is valid");
    let offset = ((piece_count - 1) * 2 + 2) as u64;
    let origin = registration
        .direct_origin(offset, offset + 1)
        .expect("benchmark origin is directly encodable");
    let deleted_origin = registration
        .direct_origin(0, 1)
        .expect("deleted benchmark origin is directly encodable");
    (fragments, layout, origin, deleted_origin)
}

fn edit_stable_piece_case(piece_count: usize) -> (FragmentStore, Vec<Piece>) {
    let bytes = format!("d\n{}", "x\n".repeat(piece_count));
    let mut fragments = FragmentStore::new();
    let (fragment, _) = fragments
        .append(Arc::from(bytes.as_bytes()), 1)
        .expect("benchmark fragment fits logical position space");
    let pieces = (0..piece_count)
        .map(|index| Piece::new(fragment, (index * 2 + 2) as u32, (index * 2 + 4) as u32))
        .collect();
    (fragments, pieces)
}

fn provenance_expansion(c: &mut Criterion) {
    let mut group = c.benchmark_group("provenance_expansion");
    group.throughput(Throughput::Elements(MACRO_CALLS as u64));

    group.bench_function("macro_body_replay_invocation_origins", |b| {
        b.iter_batched(
            macro_heavy_case,
            |(mut stores, mut command, mut capabilities, baseline)| {
                let count = drain_expansion(&mut stores, &mut command, &mut capabilities);
                black_box((count, stores.provenance_stats().saturating_sub(baseline)));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("scanner_number_runs", |b| {
        b.iter_batched(
            scanner_heavy_case,
            |(mut stores, mut command, mut capabilities, baseline)| {
                let count = drain_expansion(&mut stores, &mut command, &mut capabilities);
                black_box((count, stores.provenance_stats().saturating_sub(baseline)));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("generated_value_origin_sharing", |b| {
        b.iter_batched(
            generated_run_case,
            |(mut stores, mut command, mut capabilities, baseline)| {
                let count = drain_expansion(&mut stores, &mut command, &mut capabilities);
                black_box((count, stores.provenance_stats().saturating_sub(baseline)));
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn provenance_memory_invariants(c: &mut Criterion) {
    assert_macro_long_run_budget();
    if std::env::var_os("UMBER_PROVENANCE_REPORT").is_some() {
        print_provenance_report();
    }
    let mut group = c.benchmark_group("provenance_memory");

    group.bench_function("macro_long_run_arena_growth", |b| {
        b.iter(|| black_box(macro_long_run_growth()));
    });

    group.bench_function("rollback_truncates_discarded_fork", |b| {
        b.iter(|| black_box(discarded_fork_growth_after_rollback()));
    });

    group.finish();
}

fn synthetic_page_journal_bytes() -> usize {
    write_synthetic_page()
}

fn synthetic_page_symbols(stores: &mut Universe) -> Vec<tex_state::interner::Symbol> {
    (0..PAGE_DISTINCT_CELLS)
        .map(|index| stores.intern(&format!("page-cell-{index}")).symbol())
        .collect()
}

fn write_synthetic_page() -> usize {
    let mut stores = Universe::new();
    let symbols = synthetic_page_symbols(&mut stores);
    let snapshot = stores.snapshot();

    for write_index in 0..PAGE_TOTAL_WRITES {
        let symbol = symbols[write_index % symbols.len()];
        stores.set_meaning(
            black_box(symbol),
            black_box(raw_meaning(write_index as u64)),
        );
    }

    stores.env_journal_bytes_since(&snapshot)
}

fn rollback_case(total_cells: usize, slice_writes: usize) -> (Universe, tex_state::Snapshot) {
    let mut stores = Universe::new();
    let symbols = (0..total_cells)
        .map(|index| stores.intern(&format!("rollback-cell-{index}")))
        .collect::<Vec<_>>();

    for (index, &symbol) in symbols.iter().enumerate() {
        stores.set_meaning(symbol, raw_meaning(index as u64));
    }

    let snapshot = stores.snapshot();
    for (write_index, &symbol) in symbols.iter().take(slice_writes).enumerate() {
        stores.set_meaning(symbol, raw_meaning((write_index + total_cells) as u64));
    }

    (stores, snapshot)
}

fn raw_meaning(operand: u64) -> Meaning {
    Meaning::CharGiven(char::from_u32(32 + (operand as u32 % 95)).expect("ASCII graphic"))
}

fn source_heavy_text() -> String {
    let mut input = String::new();
    for _ in 0..SOURCE_HEAVY_LINES {
        input.push_str(SOURCE_HEAVY_LINE);
        input.push('\n');
    }
    input
}

fn mixed_utf8_text() -> String {
    repeated_lines(MIXED_UTF8_LINE)
}

fn control_sequence_text() -> String {
    repeated_lines(CONTROL_SEQUENCE_LINE)
}

fn repeated_lines(line: &str) -> String {
    let mut input = String::new();
    for _ in 0..SOURCE_HEAVY_LINES {
        input.push_str(line);
        input.push('\n');
    }
    input
}

fn source_workloads() -> Vec<(&'static str, String, bool)> {
    vec![
        ("ascii", source_heavy_text(), false),
        ("mixed_utf8", mixed_utf8_text(), false),
        (
            "single_long_line",
            format!("{}\n", "a".repeat(LONG_LINE_SCALARS)),
            false,
        ),
        ("control_sequences", control_sequence_text(), true),
    ]
}

fn drain_traced_source(
    stores: &mut Universe,
    command: &mut CommandState,
    capabilities: &mut CommandHostCapabilities,
) -> (usize, usize, ProvenanceStats, ProvenanceStats) {
    let baseline = stores.provenance_stats();
    let mut count = 0;
    let mut direct = 0;
    let mut processor = CommandProcessor::new(
        command,
        stores.command_context(),
        CommandHostContext::new(capabilities),
    );
    while let Some(token) = processor.get_token().expect("source lexing should succeed") {
        count += 1;
        direct += usize::from(token.source_range().is_some());
        black_box(token);
    }
    let final_stats = stores.provenance_stats();
    (
        count,
        direct,
        final_stats.saturating_sub(baseline),
        final_stats.saturating_sub(baseline),
    )
}

fn drain_traced_source_timed(
    stores: &mut Universe,
    command: &mut CommandState,
    capabilities: &mut CommandHostCapabilities,
) -> usize {
    let mut processor = CommandProcessor::new(
        command,
        stores.command_context(),
        CommandHostContext::new(capabilities),
    );
    let mut count = 0;
    while let Some(token) = processor.get_token().expect("source lexing should succeed") {
        black_box(token);
        count += 1;
    }
    count
}

fn diagnostic_case(input: String) -> (Universe, tex_state::token::OriginId) {
    let (mut stores, mut command, mut capabilities) = source_case(input, false);
    let token = CommandProcessor::new(
        &mut command,
        stores.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .get_token()
    .expect("diagnostic source should lex")
    .expect("diagnostic source should contain a token");
    (stores, token.origin())
}

fn print_provenance_report() {
    for (name, text, needs_control_sequences) in source_workloads() {
        let mut stores = source_universe(needs_control_sequences);
        let (mut command, mut capabilities) = source_command(text);
        let (tokens, direct, live, peak) =
            drain_traced_source(&mut stores, &mut command, &mut capabilities);
        eprintln!(
            "provenance-report {name}: tokens={tokens} direct={direct} records={} spans={} entries={} regions={} backings={} live_bytes={} retained_bytes={} peak_live_bytes={} peak_retained_bytes={} cache_bytes=0",
            live.origin_records(),
            live.origin_list_spans(),
            live.origin_list_entries(),
            live.source_regions(),
            live.generated_source_backings(),
            live.estimated_bytes(),
            live.retained_bytes(),
            live.estimated_bytes(),
            peak.retained_bytes(),
        );
    }

    let (mut stores, mut command, mut capabilities, baseline) = generated_run_case();
    let snapshot = stores.snapshot();
    let _ = drain_expansion(&mut stores, &mut command, &mut capabilities);
    let peak = stores.provenance_stats().saturating_sub(baseline);
    stores.rollback(&snapshot);
    let post = stores.provenance_stats().saturating_sub(baseline);
    eprintln!(
        "provenance-report rollback_reuse: peak_live_bytes={} peak_retained_bytes={} post_live_bytes={} post_retained_bytes={}",
        peak.estimated_bytes(),
        peak.retained_bytes(),
        post.estimated_bytes(),
        post.retained_bytes(),
    );
}

fn source_heavy_token_count(input: &str) -> usize {
    let (mut stores, mut command, mut capabilities) =
        source_case(input.to_owned(), input.contains('\\'));
    drain_traced_source_timed(&mut stores, &mut command, &mut capabilities)
}

type SourceCase = (Universe, CommandState, CommandHostCapabilities);

fn source_case(input: String, needs_control_sequences: bool) -> SourceCase {
    let stores = source_universe(needs_control_sequences);
    let (command, capabilities) = source_command(input);
    (stores, command, capabilities)
}

fn source_command(input: String) -> (CommandState, CommandHostCapabilities) {
    let profile = CommandProfile::unicode_extended(CommandDialect::Pdftex14029);
    let mut command = CommandState::new(profile);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            input.into_bytes(),
        ))
        .expect("benchmark source registers");
    command
        .open_registered_source(source)
        .expect("benchmark source opens");
    (command, CommandHostCapabilities::default())
}

fn source_universe(needs_control_sequences: bool) -> Universe {
    let mut stores = Universe::new();
    if needs_control_sequences {
        for name in [
            "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta",
        ] {
            stores.intern(name);
        }
    }
    stores
}

type ExpansionCase = (
    Universe,
    CommandState,
    CommandHostCapabilities,
    ProvenanceStats,
);

fn macro_heavy_case() -> ExpansionCase {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("hotmacro");
    let params = stores.intern_token_list(&[]);
    let body_tokens = (0..MACRO_BODY_LEN)
        .map(|index| char_token(char::from(b'a' + (index % 26) as u8)))
        .collect::<Vec<_>>();
    let body = stores.intern_token_list(&body_tokens);
    let definition_origin_id = stores.source_origin(SourceId::new(1), 0, 1, 1);
    let definition_origin = stores
        .origin_ref(definition_origin_id)
        .expect("fresh definition origin");
    let body_origin_roots = vec![definition_origin.clone(); body_tokens.len()];
    let body_origins = stores.allocate_origin_list_ref(&body_origin_roots);
    stores.set_macro_meaning_with_provenance(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        MacroDefinitionProvenance::new(
            definition_origin,
            tex_state::provenance::OriginListRef::empty(),
            body_origins,
        ),
    );

    let call_tokens = vec![Token::Cs(macro_cs.symbol()); MACRO_CALLS];
    let call_origin = stores.source_origin(SourceId::new(1), 80, 2, 1);
    let traced_calls = call_tokens
        .into_iter()
        .map(|token| TracedTokenWord::pack(token, call_origin))
        .collect::<Vec<_>>();
    let calls = stores.finish_traced_token_list(&traced_calls);
    let baseline = stores.provenance_stats();
    let mut command = CommandState::default();
    command.push_everyjob(&stores.command_context(), calls);
    (
        stores,
        command,
        CommandHostCapabilities::default(),
        baseline,
    )
}

fn scanner_heavy_case() -> ExpansionCase {
    let mut stores = Universe::new();
    install_tex82_expandable_primitives(&mut stores);
    let number = stores.symbol("number").expect("number primitive");
    let mut tokens = Vec::with_capacity(SCANNER_REPETITIONS * 7);
    for _ in 0..SCANNER_REPETITIONS {
        tokens.push(Token::Cs(number.symbol()));
        for digit in ['1', '2', '3', '4', '5'] {
            tokens.push(char_token(digit));
        }
        tokens.push(space_token());
    }
    traced_token_list_input(stores, tokens)
}

fn generated_run_case() -> ExpansionCase {
    let mut stores = Universe::new();
    install_tex82_expandable_primitives(&mut stores);
    let roman = stores
        .symbol("romannumeral")
        .expect("romannumeral primitive");
    let mut tokens = Vec::with_capacity(SCANNER_REPETITIONS * 6);
    for _ in 0..SCANNER_REPETITIONS {
        tokens.push(Token::Cs(roman.symbol()));
        for digit in ['3', '8', '8', '8'] {
            tokens.push(char_token(digit));
        }
        tokens.push(space_token());
    }
    traced_token_list_input(stores, tokens)
}

fn traced_token_list_input(mut stores: Universe, tokens: Vec<Token>) -> ExpansionCase {
    let origin = stores.source_origin(SourceId::new(2), 0, 1, 1);
    let traced = tokens
        .into_iter()
        .map(|token| TracedTokenWord::pack(token, origin))
        .collect::<Vec<_>>();
    let token_list = stores.finish_traced_token_list(&traced);
    let baseline = stores.provenance_stats();
    let mut command = CommandState::default();
    command.push_everyjob(&stores.command_context(), token_list);
    (
        stores,
        command,
        CommandHostCapabilities::default(),
        baseline,
    )
}

fn drain_expansion(
    stores: &mut Universe,
    command: &mut CommandState,
    capabilities: &mut CommandHostCapabilities,
) -> usize {
    let mut processor = CommandProcessor::new(
        command,
        stores.command_context(),
        CommandHostContext::new(capabilities),
    );
    let mut count = 0;
    while let Some(command) = processor
        .get_x_token()
        .expect("canonical expansion should succeed")
    {
        black_box(command);
        count += 1;
    }
    count
}

fn macro_long_run_growth() -> ProvenanceStats {
    let (mut stores, mut command, mut capabilities, baseline) = macro_heavy_case();
    let count = drain_expansion(&mut stores, &mut command, &mut capabilities);
    assert_eq!(count, MACRO_CALLS * MACRO_BODY_LEN);
    stores.provenance_stats().saturating_sub(baseline)
}

fn assert_macro_long_run_budget() {
    let (mut stores, mut command, mut capabilities, _) = macro_heavy_case();
    let count = drain_expansion(&mut stores, &mut command, &mut capabilities);
    assert_eq!(count, MACRO_CALLS * MACRO_BODY_LEN);
    let macro_stats = stores.macro_invocation_provenance_stats();
    assert_eq!(macro_stats.invocations(), MACRO_CALLS);
    assert!(
        macro_stats.retained_bytes()
            <= MACRO_CALLS * MACRO_PROVENANCE_RETAINED_BYTES_PER_INVOCATION_BUDGET,
        "macro provenance retained {} bytes for {} invocations (budget: {} bytes/invocation)",
        macro_stats.retained_bytes(),
        MACRO_CALLS,
        MACRO_PROVENANCE_RETAINED_BYTES_PER_INVOCATION_BUDGET,
    );
}

fn discarded_fork_growth_after_rollback() -> ProvenanceStats {
    let (mut stores, mut command, mut capabilities, baseline) = generated_run_case();
    let snapshot = stores.snapshot();
    let _ = drain_expansion(&mut stores, &mut command, &mut capabilities);
    stores.rollback(&snapshot);
    stores.provenance_stats().saturating_sub(baseline)
}

fn char_token(ch: char) -> Token {
    let cat = match ch {
        '0'..='9' | '[' | ']' | '!' | '<' | '=' | '>' | '-' => Catcode::Other,
        _ => Catcode::Letter,
    };
    Token::Char { ch, cat }
}

fn repeated_source_registration(c: &mut Criterion) {
    let bytes: Arc<[u8]> = (0..(1024 * 1024))
        .map(|index| if index % 80 == 79 { b'\n' } else { b'x' })
        .collect::<Vec<_>>()
        .into();
    let descriptor = SourceDescriptor::generated(bytes);
    let source = SourceId::new(0);
    let mut universe = Universe::new();
    universe
        .register_source(source, descriptor.clone())
        .expect("benchmark source registers");

    c.bench_function("repeated_source_registration/1mib", |b| {
        b.iter(|| {
            black_box(
                universe
                    .register_source(source, black_box(descriptor.clone()))
                    .expect("identical registration remains live"),
            );
        });
    });
}

fn space_token() -> Token {
    Token::Char {
        ch: ' ',
        cat: Catcode::Space,
    }
}

criterion_group!(
    benches,
    dependency_recording,
    allocation_node_append,
    deep_journal_box_locality,
    allocation_traced_freeze,
    page_contribution_queue,
    meaning_lookup,
    barrier_write,
    snapshot_take,
    checkpoint_state_hash,
    token_semantic_projection,
    token_projection_freeze_cost,
    transient_box_overwrite_checkpoint,
    group_cycle,
    rollback_scaling,
    group_global_compaction,
    synthetic_page_journal_volume,
    provenance_source_lexing,
    provenance_expansion,
    provenance_memory_invariants,
    provenance_diagnostic_rendering,
    edit_stable_source_coordinates,
    repeated_source_registration
);
criterion_main!(benches);
