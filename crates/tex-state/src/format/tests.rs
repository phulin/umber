use super::schema::{FormatCell, FormatDefinition, FormatNodeList, VersionedRows};
use super::{
    DetachedFormatImage, FormatError, FormatPublicationError, with_format_destination,
    with_materialized_format,
};
use crate::interner::InternerBudget;
use crate::meaning::{Meaning, MeaningFlags, MeaningWord, ResolvedMeaning};
use crate::node::Node;
use crate::token::{Catcode, Token, TokenWord};
use crate::world::{JobClock, World};
use crate::{AssignmentScope, CodeTableKind, InteractionMode, with_universe};
use tex_arith::Scaled;
use tex_content::ContentHash;
use tex_fonts::{FontMetrics, LoadedFont};

#[derive(serde::Deserialize, serde::Serialize)]
struct TestStringPoolFormatState {
    version: u8,
    strings: usize,
    characters: usize,
    max_strings: usize,
    pool_size: usize,
    recycled: std::collections::BTreeSet<String>,
    memory: TestMainMemoryFormatState,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct TestMainMemoryFormatState {
    variable_live: usize,
    dynamic_live: usize,
    low_extent: usize,
    high_extent: usize,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct TestFormatMetadata {
    version: u32,
    interaction_mode: u8,
    string_pool: TestStringPoolFormatState,
    pdf: Vec<u8>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct TestEmptyPdfFormatState {
    version: u32,
    enabled: bool,
    next_object: u32,
    next_form_resource: u32,
    raw_objects: Vec<()>,
    forms: Vec<()>,
    external_images: Vec<()>,
    glyph_to_unicode: Vec<crate::PdfGlyphToUnicode>,
}

fn budget() -> InternerBudget {
    InternerBudget::new(32, 64, 4 * 1024).expect("test budget")
}

fn image() -> DetachedFormatImage {
    with_universe(budget(), |universe| {
        universe.set_interaction_mode(InteractionMode::Nonstop);
        universe.capture_format_image().expect("capture")
    })
    .expect("fresh universe")
}

#[test]
fn string_pool_format_baseline_preserves_make_and_recycling_semantics() {
    let (image, before_capacity) = with_universe(budget(), |universe| {
        let mut context = universe.command_context().expect("command context");
        let initial = context.detach_engine_usage_statistics();
        assert_eq!((initial.strings, initial.string_characters), (0, 0));

        context.slow_make_string_pool_string("trip");
        context.slow_make_string_pool_string("trip");
        context.make_string_pool_string("trip");
        context.intern_hash_control_sequence("newcs");
        context.intern_hash_control_sequence("newcs");
        context.intern_hash_control_sequence("x");
        context
            .intern_retained_pool_string("FONT?")
            .expect("font identifier");
        let used = context.detach_engine_usage_statistics();
        assert_eq!((used.strings, used.string_characters), (4, 18));
        assert_eq!(used.control_sequences, 1);
        drop(context);
        (
            universe.capture_format_image().expect("capture format"),
            (
                initial.string_capacity - used.strings,
                initial.string_character_capacity - used.string_characters,
            ),
        )
    })
    .expect("fresh universe");

    with_materialized_format(budget(), World::memory(), &image, |universe| {
        let mut context = universe.command_context().expect("loaded context");
        let loaded = context.detach_engine_usage_statistics();
        assert_eq!((loaded.strings, loaded.string_characters), (0, 0));
        assert_eq!(loaded.control_sequences, 1);
        assert_eq!(
            (loaded.string_capacity, loaded.string_character_capacity),
            before_capacity
        );

        context.slow_make_string_pool_string("trip");
        assert_eq!(context.detach_engine_usage_statistics(), loaded);
        context.slow_make_string_pool_string("fresh");
        context.slow_make_string_pool_string("fresh");
        context.make_string_pool_string("fresh");
        context.intern_hash_control_sequence("newcs");
        assert_eq!(
            context.detach_engine_usage_statistics().control_sequences,
            1,
            "format-loaded lookup reuses permanent occupancy"
        );
        context.intern_hash_control_sequence("freshcs");
        let used = context.detach_engine_usage_statistics();
        assert_eq!((used.strings, used.string_characters), (3, 17));
        assert_eq!(used.control_sequences, 2);
    })
    .expect("materialize format");
}

#[test]
fn pdftex_string_pool_capacity_captures_roundtrips_and_becomes_the_loaded_baseline() {
    let (image, expected_loaded_capacity, retained) = with_universe(budget(), |universe| {
        let trip_capacity = universe
            .command_context()
            .expect("initial capacity")
            .detach_engine_usage_statistics()
            .string_character_capacity;
        universe.set_engine_capacity_profile(crate::EngineCapacityProfile::Pdftex14029);
        let pdftex_capacity = universe
            .command_context()
            .expect("pdfTeX capacity")
            .detach_engine_usage_statistics();
        let retained = "x".repeat(trip_capacity + 1);
        universe
            .command_context()
            .expect("pool allocation")
            .make_string_pool_string(&retained);
        let used = universe
            .command_context()
            .expect("used coordinates")
            .detach_engine_usage_statistics();
        assert_eq!((used.strings, used.string_characters), (1, retained.len()));
        (
            universe
                .capture_format_image()
                .expect("capture pdfTeX format"),
            (
                pdftex_capacity.string_capacity - used.strings,
                pdftex_capacity.string_character_capacity - used.string_characters,
            ),
            retained,
        )
    })
    .expect("fresh universe");
    let bytes = image.as_bytes().to_vec();
    assert_eq!(
        DetachedFormatImage::try_from_bytes(bytes.clone())
            .expect("detached roundtrip")
            .into_bytes(),
        bytes
    );

    with_materialized_format(budget(), World::memory(), &image, |universe| {
        let mut context = universe.command_context().expect("loaded context");
        let loaded = context.detach_engine_usage_statistics();
        assert_eq!((loaded.strings, loaded.string_characters), (0, 0));
        assert_eq!(
            (loaded.string_capacity, loaded.string_character_capacity),
            expected_loaded_capacity
        );
        context.slow_make_string_pool_string(&retained);
        assert_eq!(context.detach_engine_usage_statistics(), loaded);
    })
    .expect("materialize pdfTeX capacity format");
}

#[test]
fn format_capture_disables_texxet_enhancement_without_mutating_the_source() {
    crate::with_universe(
        crate::interner::InternerBudget::new(16, 16, 256).expect("budget"),
        |universe| {
            universe
                .assign_int_param(
                    crate::env::banks::IntParam::TEX_XET_STATE,
                    1,
                    crate::AssignmentScope::Global,
                )
                .expect("enable TeXXeT in the producing job");
            let image = universe.capture_format_image().expect("capture format");
            assert_eq!(
                universe.int_param(crate::env::banks::IntParam::TEX_XET_STATE),
                1,
                "detached format capture must not mutate the producing job"
            );

            crate::with_materialized_format(
                crate::interner::InternerBudget::new(16, 16, 256).expect("destination budget"),
                crate::World::memory(),
                &image,
                |loaded| {
                    assert_eq!(
                        loaded.int_param(crate::env::banks::IntParam::TEX_XET_STATE),
                        0,
                        "e-TeX change 17.11 disables enhancements before dumping eqtb"
                    );
                },
            )
            .expect("materialize canonical format");
        },
    )
    .expect("fresh format source");
}

fn test_font() -> LoadedFont {
    LoadedFont::new(
        "formatfont",
        "/fonts/formatfont.tfm",
        ContentHash::from_bytes(b"format font metrics").bytes(),
        0x1234_5678,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(12 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::default(),
    )
}

#[test]
fn format_roundtrip_preserves_absolute_font_info_usage() {
    // TeX82 §§1320--1321 dump and restore the absolute `fmem_ptr`, including
    // immutable TFM words and fontdimen growth performed before `\dump`.
    let image = with_universe(budget(), |universe| {
        let font = universe
            .command_context()
            .expect("font admission")
            .intern_font(test_font().with_font_info_words(100));
        let mut context = universe.command_context().expect("font growth");
        context
            .set_font_dimen(font, 10, Scaled::from_raw(10))
            .expect("fontdimen growth");
        assert_eq!(
            context.detach_engine_usage_statistics().font_info_words,
            110
        );
        drop(context);
        universe.capture_format_image().expect("capture format")
    })
    .expect("source universe");

    with_materialized_format(budget(), World::memory(), &image, |universe| {
        assert_eq!(
            universe
                .command_context()
                .expect("loaded context")
                .detach_engine_usage_statistics()
                .font_info_words,
            110
        );
    })
    .expect("materialized format");
}

fn replace_section(image: &DetachedFormatImage, kind: u32, bytes: Vec<u8>) -> Vec<u8> {
    let container = crate::format_container::decode(image.as_bytes()).expect("source container");
    let owned = container
        .sections
        .iter()
        .map(|section| {
            (
                section.kind,
                section.alignment,
                if section.kind == kind {
                    bytes.clone()
                } else {
                    section.bytes.clone()
                },
            )
        })
        .collect::<Vec<_>>();
    let sections = owned
        .iter()
        .map(
            |(kind, alignment, bytes)| crate::format_container::SectionInput {
                kind: *kind,
                alignment: *alignment,
                bytes,
            },
        )
        .collect::<Vec<_>>();
    crate::format_container::encode(&sections).expect("mutated container")
}

fn replace_metadata(
    image: &DetachedFormatImage,
    mutate: impl FnOnce(&mut TestFormatMetadata),
) -> Vec<u8> {
    let container = crate::format_container::decode(image.as_bytes()).expect("source container");
    let mut metadata: TestFormatMetadata =
        bincode::deserialize(&container.section(1).expect("metadata section").bytes)
            .expect("metadata");
    mutate(&mut metadata);
    replace_section(
        image,
        1,
        bincode::serialize(&metadata).expect("mutated metadata"),
    )
}

#[test]
fn detached_image_roundtrips_bytes_and_rejects_corruption() {
    let image = image();
    let bytes = image.as_bytes().to_vec();

    assert_eq!(
        DetachedFormatImage::try_from_bytes(bytes.clone())
            .expect("validated bytes")
            .into_bytes(),
        bytes
    );

    let mut bad_magic = bytes.clone();
    bad_magic[0] ^= 1;
    assert_eq!(
        DetachedFormatImage::try_from_bytes(bad_magic).expect_err("bad magic must be rejected"),
        FormatError::BadMagic
    );
    let mut bad_checksum = bytes;
    let last = bad_checksum.len() - 1;
    bad_checksum[last] ^= 1;
    assert_eq!(
        DetachedFormatImage::try_from_bytes(bad_checksum)
            .expect_err("bad checksum must be rejected"),
        FormatError::Checksum
    );
}

#[test]
fn malformed_string_pool_profiles_and_coordinates_are_rejected_exactly() {
    let image = image();
    let mixed_profile = replace_metadata(&image, |metadata| {
        metadata.string_pool.max_strings = 500_000;
    });
    assert_eq!(
        DetachedFormatImage::try_from_bytes(mixed_profile)
            .expect_err("mixed process profile must be rejected"),
        FormatError::InvalidState(
            "unsupported string-pool capacity coordinates: max_strings=500000, pool_size=125000"
                .to_owned()
        )
    );

    let character_overflow = replace_metadata(&image, |metadata| {
        metadata.string_pool.characters = metadata.string_pool.pool_size + 1;
    });
    assert_eq!(
        DetachedFormatImage::try_from_bytes(character_overflow)
            .expect_err("pool coordinate beyond the selected profile must be rejected"),
        FormatError::InvalidState(
            "invalid string-pool format coordinates: strings=1027, characters=125001, max_strings=15000, pool_size=125000, recycled_strings=0, recycled_characters=0"
                .to_owned()
        )
    );
}

#[test]
fn main_memory_coordinates_are_validated_against_the_recorded_producer_profile() {
    let image = image();
    let expanded_memory = |metadata: &mut TestFormatMetadata| {
        metadata.string_pool.memory.dynamic_live = 300_000;
        metadata.string_pool.memory.high_extent = 300_000;
    };
    let tex_bytes = replace_metadata(&image, expanded_memory);
    assert_eq!(
        DetachedFormatImage::try_from_bytes(tex_bytes)
            .expect_err("TeX82 producer capacity must reject the expanded extent"),
        FormatError::InvalidState(
            "invalid main-memory format coordinates: variable_live=20, dynamic_live=300000, low_extent=1021, high_extent=300000, memory_words=301021, capacity=250000, profile=Tex82Etex, failed=memory_words>capacity"
                .to_owned()
        )
    );

    let pdftex_bytes = replace_metadata(&image, |metadata| {
        expanded_memory(metadata);
        metadata.string_pool.max_strings = 500_000;
        metadata.string_pool.pool_size = 6_250_000;
    });
    assert_eq!(
        DetachedFormatImage::try_from_bytes(pdftex_bytes.clone())
            .expect("the same coordinates fit their recorded pdfTeX producer")
            .into_bytes(),
        pdftex_bytes
    );
}

#[test]
fn loaded_tex_format_can_expand_to_pdftex_capacity_and_recapture_that_profile() {
    let tex_image = image();
    with_materialized_format(budget(), World::memory(), &tex_image, |universe| {
        let loaded = universe
            .command_context()
            .expect("loaded context")
            .detach_engine_usage_statistics();
        assert_eq!(
            loaded.capacity_profile,
            crate::EngineCapacityProfile::Tex82Etex
        );
        universe.set_engine_capacity_profile(crate::EngineCapacityProfile::Pdftex14029);
        let expanded = universe
            .command_context()
            .expect("expanded context")
            .detach_engine_usage_statistics();
        assert_eq!(
            (expanded.capacity_profile, expanded.memory_word_capacity),
            (crate::EngineCapacityProfile::Pdftex14029, 5_000_000)
        );
        let recaptured = universe.capture_format_image().expect("recapture");
        assert_eq!(
            DetachedFormatImage::try_from_bytes(recaptured.as_bytes().to_vec())
                .expect("validate recaptured profile")
                .as_bytes(),
            recaptured.as_bytes()
        );
    })
    .expect("materialize TeX format under pdfTeX process");
}

#[test]
fn hash_font_trie_and_pdf_coordinates_use_the_same_recorded_profile() {
    let hash_image = with_universe(
        InternerBudget::new(16_000, 16_000, 256_000).expect("large hash budget"),
        |universe| {
            universe.set_engine_capacity_profile(crate::EngineCapacityProfile::Pdftex14029);
            for index in 0..15_001 {
                universe
                    .intern(&format!("capacity{index}"))
                    .expect("pdfTeX hash entry");
            }
            universe.capture_format_image().expect("pdfTeX hash format")
        },
    )
    .expect("hash source");
    let tex_hash = replace_metadata(&hash_image, select_tex82_metadata_profile);
    assert_eq!(
        DetachedFormatImage::try_from_bytes(tex_hash)
            .expect_err("TeX hash capacity must reject pdfTeX occupancy"),
        FormatError::InvalidState(
            "invalid format hash occupancy: entries=15001, capacity=15000, profile=Tex82Etex"
                .to_owned()
        )
    );

    let font_image = with_universe(budget(), |universe| {
        universe.set_engine_capacity_profile(crate::EngineCapacityProfile::Pdftex14029);
        universe
            .command_context()
            .expect("font context")
            .intern_font(test_font().with_font_info_words(20_001));
        universe.capture_format_image().expect("pdfTeX font format")
    })
    .expect("font source");
    let tex_font = replace_metadata(&font_image, select_tex82_metadata_profile);
    assert_eq!(
        DetachedFormatImage::try_from_bytes(tex_font)
            .expect_err("TeX font-info capacity must reject pdfTeX occupancy"),
        FormatError::InvalidState(
            "invalid format font-info occupancy: words=20008, capacity=20000, profile=Tex82Etex"
                .to_owned()
        )
    );

    let trie_image = with_universe(budget(), |universe| {
        universe.set_engine_capacity_profile(crate::EngineCapacityProfile::Pdftex14029);
        universe
            .command_context()
            .expect("trie context")
            .add_hyphenation_pattern_for_language(
                0,
                crate::hyphenation::PatternSpec {
                    letters: vec!['a'; 8_000],
                    values: Vec::new(),
                },
            )
            .expect("pdfTeX trie occupancy");
        universe.capture_format_image().expect("pdfTeX trie format")
    })
    .expect("trie source");
    let tex_trie = replace_metadata(&trie_image, select_tex82_metadata_profile);
    assert_eq!(
        DetachedFormatImage::try_from_bytes(tex_trie)
            .expect_err("TeX trie capacity must reject pdfTeX occupancy"),
        FormatError::InvalidState(
            "invalid frozen hyphenation trie occupancy: nodes=8001, capacity=8000".to_owned()
        )
    );

    let pdf_image = with_universe(budget(), |universe| {
        universe.set_engine_capacity_profile(crate::EngineCapacityProfile::Pdftex14029);
        universe.enable_pdf_output();
        universe.capture_format_image().expect("pdfTeX PDF format")
    })
    .expect("PDF source");
    let tex_pdf = replace_metadata(&pdf_image, select_tex82_metadata_profile);
    assert_eq!(
        DetachedFormatImage::try_from_bytes(tex_pdf)
            .expect_err("TeX profile must reject retained PDF state"),
        FormatError::InvalidState(
            "non-pdfTeX format profile retains PDF resource state".to_owned()
        )
    );
}

fn select_tex82_metadata_profile(metadata: &mut TestFormatMetadata) {
    metadata.string_pool.strings = 1_027;
    metadata.string_pool.characters = 106_808;
    metadata.string_pool.max_strings = 15_000;
    metadata.string_pool.pool_size = 125_000;
    metadata.string_pool.recycled.clear();
}

#[test]
fn malformed_pdf_object_coordinate_reports_actual_profile_capacity() {
    let image = with_universe(budget(), |universe| {
        universe.set_engine_capacity_profile(crate::EngineCapacityProfile::Pdftex14029);
        universe.capture_format_image().expect("pdfTeX format")
    })
    .expect("PDF source");
    let bytes = replace_metadata(&image, |metadata| {
        let mut pdf: TestEmptyPdfFormatState =
            bincode::deserialize(&metadata.pdf).expect("empty PDF envelope");
        pdf.next_object = 8_388_608;
        metadata.pdf = bincode::serialize(&pdf).expect("malformed PDF envelope");
    });
    assert_eq!(
        DetachedFormatImage::try_from_bytes(bytes)
            .expect_err("pdfTeX object-table coordinate must be bounded"),
        FormatError::InvalidState(
            "invalid PDF format object-table coordinate: next_object=8388608, capacity=8388607"
                .to_owned()
        )
    );
}

#[test]
fn malformed_sections_cross_references_graphs_and_pdf_reject_before_staging() {
    let image = image();
    let container = crate::format_container::decode(image.as_bytes()).expect("source container");
    let owned = container
        .sections
        .iter()
        .filter(|section| section.kind != 336)
        .map(|section| (section.kind, section.alignment, section.bytes.clone()))
        .collect::<Vec<_>>();
    let sections = owned
        .iter()
        .map(
            |(kind, alignment, bytes)| crate::format_container::SectionInput {
                kind: *kind,
                alignment: *alignment,
                bytes,
            },
        )
        .collect::<Vec<_>>();
    let missing = crate::format_container::encode(&sections).expect("missing-kind container");
    assert!(DetachedFormatImage::try_from_bytes(missing).is_err());

    let bad_cells = bincode::serialize(&VersionedRows {
        version: 1,
        rows: vec![FormatCell::TokenRegister(7, u32::MAX)],
    })
    .expect("bad cell section");
    assert!(DetachedFormatImage::try_from_bytes(replace_section(&image, 528, bad_cells)).is_err());

    let recursive: crate::node::Node<u32, u32, u32> = crate::node::Node::Disc {
        kind: crate::node::DiscKind::Discretionary,
        pre: 1,
        post: 0,
        replace: 0,
        physical_replace_count: 0,
    };
    let bad_nodes = bincode::serialize(&VersionedRows {
        version: 1,
        rows: vec![FormatNodeList {
            nodes: vec![bincode::serialize(&recursive).expect("recursive node")],
        }],
    })
    .expect("bad node section");
    assert!(DetachedFormatImage::try_from_bytes(replace_section(&image, 512, bad_nodes)).is_err());

    let mut metadata: super::FormatMetadata =
        bincode::deserialize(&container.section(1).expect("metadata section").bytes)
            .expect("metadata");
    metadata.pdf = b"not a PDF format envelope".to_vec();
    let bad_pdf = bincode::serialize(&metadata).expect("bad PDF metadata");
    assert!(DetachedFormatImage::try_from_bytes(replace_section(&image, 1, bad_pdf)).is_err());
}

#[test]
fn one_borrowed_image_materializes_as_isolated_fresh_jobs() {
    let image = image();
    let first_clock = JobClock {
        time: 10,
        second: 20,
        day: 3,
        month: 4,
        year: 2027,
    };
    let second_clock = JobClock {
        time: 30,
        second: 40,
        day: 5,
        month: 6,
        year: 2028,
    };
    let first = with_materialized_format(
        budget(),
        World::memory_with_clock(first_clock),
        &image,
        |universe| {
            assert_eq!(universe.world().job_clock(), first_clock);
            assert_eq!(universe.interaction_mode(), InteractionMode::Nonstop);
            universe
                .capture_format_image()
                .expect("redump")
                .into_bytes()
        },
    )
    .expect("first load");
    let second = with_materialized_format(
        budget(),
        World::memory_with_clock(second_clock),
        &image,
        |universe| {
            assert_eq!(universe.world().job_clock(), second_clock);
            universe
                .capture_format_image()
                .expect("redump")
                .into_bytes()
        },
    )
    .expect("second load");

    assert_eq!(first, image.as_bytes());
    assert_eq!(second, image.as_bytes());
}

#[test]
fn foreign_staging_is_rejected_before_world_publication() {
    let image = image();
    with_format_destination(budget(), World::memory(), |destination| {
        let mut staging = destination.stage(&image)?;
        staging.destination = staging.destination.wrapping_add(1);
        assert_eq!(
            destination.materialize(staging, |_| ()),
            Err(FormatPublicationError::ForeignDestination)
        );
        assert!(destination.world.is_some());
        Ok(())
    })
    .expect("destination episode");
}

#[test]
fn staging_consumes_destination_once() {
    let image = image();
    with_format_destination(budget(), World::memory(), |destination| {
        let staging = destination.stage(&image)?;
        assert!(matches!(
            destination.stage(&image),
            Err(FormatError::DestinationConsumed)
        ));
        destination
            .materialize(staging, |_| ())
            .expect("matching publication");
        Ok(())
    })
    .expect("destination episode");
}

#[test]
fn logical_rows_roundtrip_aliases_values_codes_and_hyphenation() {
    let image = with_universe(budget(), |universe| {
        let alpha = universe.intern("alpha").expect("alpha name");
        let alias = universe.intern("alias").expect("alias name");
        let replacement = [TokenWord::pack(Token::Cs(alpha.symbol()))];
        let definition = universe
            .allocate_definition(&[], &replacement)
            .expect("definition");
        let meaning = MeaningWord::macro_definition(MeaningFlags::LONG, definition);
        universe
            .assign_meaning(alpha, meaning, AssignmentScope::Global)
            .expect("alpha meaning");
        universe
            .assign_meaning(alias, meaning, AssignmentScope::Global)
            .expect("alias meaning");
        let tokens = universe
            .allocate_token_list(&replacement)
            .expect("token list");
        universe
            .assign_token_register(7, Some(tokens), AssignmentScope::Global)
            .expect("token register");
        let glue_value = crate::glue::GlueSpec {
            width: Scaled::from_raw(123),
            stretch: Scaled::from_raw(4),
            stretch_order: crate::glue::Order::Fil,
            shrink: Scaled::from_raw(5),
            shrink_order: crate::glue::Order::Normal,
        };
        let glue = universe.allocate_glue(glue_value).expect("glue");
        universe
            .assign_glue_register(9, Some(glue), AssignmentScope::Global)
            .expect("glue register");
        universe
            .assign_count(42, 8_675_309, AssignmentScope::Global)
            .expect("count");
        universe
            .assign_code(
                CodeTableKind::Catcode,
                '@',
                i64::from(Catcode::Letter as u8),
                AssignmentScope::Global,
            )
            .expect("catcode");
        {
            let mut context = universe.command_context().expect("hyphenation admission");
            context
                .add_hyphenation_pattern_for_language(
                    3,
                    crate::hyphenation::PatternSpec {
                        letters: vec!['a', 'b'],
                        values: vec![0, 1, 0],
                    },
                )
                .expect("pattern");
            context.close_hyphenation_patterns();
        }
        universe.capture_format_image().expect("logical capture")
    })
    .expect("source universe");

    with_materialized_format(budget(), World::memory(), &image, |universe| {
        let alpha = universe.intern("alpha").expect("restored alpha");
        let alias = universe.intern("alias").expect("restored alias");
        let alpha_meaning = universe.meaning(alpha.symbol()).expect("alpha meaning");
        let alias_meaning = universe.meaning(alias.symbol()).expect("alias meaning");
        assert_eq!(alpha_meaning, alias_meaning);
        let ResolvedMeaning::Macro { flags, definition } = alpha_meaning else {
            panic!("restored macro meaning");
        };
        assert_eq!(flags, MeaningFlags::LONG);
        assert_eq!(
            universe
                .core
                .as_ref()
                .expect("core")
                .admit()
                .definition(definition)
                .replacement_text(),
            [TokenWord::pack(Token::Cs(alpha.symbol()))]
        );
        assert_eq!(universe.count(42).expect("count"), 8_675_309);
        let tokens = universe
            .token_register(7)
            .expect("token register")
            .expect("token root");
        assert_eq!(
            universe
                .core
                .as_ref()
                .expect("core")
                .admit()
                .token_list(tokens)
                .iter()
                .collect::<Vec<_>>(),
            [TokenWord::pack(Token::Cs(alpha.symbol()))]
        );
        let glue = universe
            .glue_register(9)
            .expect("glue register")
            .expect("glue root");
        assert_eq!(universe.glue_value(glue).width, Scaled::from_raw(123));
        assert_eq!(universe.catcode('@'), Catcode::Letter);
        assert!(
            universe
                .command_context()
                .expect("hyphenation admission")
                .contains_hyphenation_pattern_for_language(3, &['a', 'b'])
        );
        assert_eq!(
            universe.capture_format_image().expect("redump").as_bytes(),
            image.as_bytes()
        );
    })
    .expect("materialized logical format");
}

#[test]
fn loaded_format_materializes_only_environment_owned_definitions() {
    let mut image = with_universe(budget(), |universe| {
        let command = universe.intern("owned").expect("command name");
        let stale = [TokenWord::pack(Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        })];
        universe
            .allocate_definition(&[], &stale)
            .expect("unreferenced definition");
        let retained = [TokenWord::pack(Token::Char {
            ch: 'y',
            cat: Catcode::Other,
        })];
        let definition = universe
            .allocate_definition(&[], &retained)
            .expect("retained definition");
        universe
            .assign_meaning(
                command,
                MeaningWord::macro_definition(MeaningFlags::from_bits(0), definition),
                AssignmentScope::Global,
            )
            .expect("retained meaning");
        universe.capture_format_image().expect("capture format")
    })
    .expect("source universe");
    assert_eq!(image.decoded.definitions.len(), 1);
    image.decoded.definitions.push(FormatDefinition {
        parameter_text: Vec::new(),
        replacement_text: vec![
            TokenWord::pack(Token::Char {
                ch: 'x',
                cat: Catcode::Other,
            })
            .raw(),
        ],
    });
    assert_eq!(image.decoded.definitions.len(), 2);

    with_materialized_format(budget(), World::memory(), &image, |universe| {
        let redumped = universe
            .capture_format_image()
            .expect("redump loaded format");
        assert_eq!(redumped.decoded.definitions.len(), 1);
        assert_eq!(
            redumped.decoded.definitions[0].replacement_text,
            [TokenWord::pack(Token::Char {
                ch: 'y',
                cat: Catcode::Other,
            })
            .raw(),]
        );
    })
    .expect("materialized format");
}

#[test]
fn configured_hyphenation_exception_capacity_roundtrips_with_format_usage() {
    // TeX82 §§934/1308/1334 and Web2C `tex.ch` [51.1332]: `hyph_size`
    // is selected by the executable, retained as a format compatibility
    // constant, and reported by the loaded job independently of occupancy.
    let image = with_universe(budget(), |universe| {
        universe.set_hyphenation_exception_capacity(659);
        universe.capture_format_image().expect("capture format")
    })
    .expect("source universe");

    with_materialized_format(budget(), World::memory(), &image, |universe| {
        let usage = universe
            .command_context()
            .expect("loaded context")
            .detach_engine_usage_statistics();
        assert_eq!(usage.hyphenation_exceptions, 0);
        assert_eq!(usage.hyphenation_exception_capacity, 659);
    })
    .expect("materialized format");
}

#[test]
fn logical_roundtrip_preserves_font_node_box_and_pdf_roots() {
    let (image, raw_object, form_object) = with_universe(budget(), |universe| {
        universe.set_engine_capacity_profile(crate::EngineCapacityProfile::Pdftex14029);
        let selector = universe.intern("formatfont").expect("font selector");
        let font = universe
            .command_context()
            .expect("font admission")
            .intern_font(test_font());
        universe
            .assign_meaning(
                selector,
                MeaningWord::from_static(Meaning::Font(font)),
                AssignmentScope::Global,
            )
            .expect("font meaning");
        universe
            .assign_current_font(font, AssignmentScope::Global)
            .expect("current font");
        let page_root = universe.publish_page_nodes(&[Node::Char {
            font,
            ch: 'X',
            origin: crate::token::OriginId::UNKNOWN,
        }]);
        universe
            .assign_page_box(12, Some(page_root), AssignmentScope::Global)
            .expect("box promotion");
        let tokens = universe
            .allocate_token_list(&[TokenWord::pack(Token::Char {
                ch: 'q',
                cat: Catcode::Other,
            })])
            .expect("PDF token root");
        let (raw_object, form_object) = {
            let mut context = universe.command_context().expect("PDF admission");
            let raw = context.reserve_pdf_raw_object().expect("raw object");
            context
                .initialize_pdf_raw_object(raw, false, None, false, tokens, false)
                .expect("raw object data");
            let form = context.reserve_pdf_form().expect("form reservation");
            context
                .initialize_pdf_form(
                    form,
                    page_root,
                    (
                        Scaled::from_raw(10),
                        Scaled::from_raw(20),
                        Scaled::from_raw(3),
                    ),
                    Some(tokens),
                    None,
                    false,
                )
                .expect("form data");
            (raw.raw(), form.0)
        };
        (
            universe
                .capture_format_image()
                .expect("full logical capture"),
            raw_object,
            form_object,
        )
    })
    .expect("source universe");

    with_materialized_format(budget(), World::memory(), &image, |universe| {
        let selector = universe.intern("formatfont").expect("restored selector");
        let ResolvedMeaning::Static(Meaning::Font(font)) =
            universe.meaning(selector.symbol()).expect("font meaning")
        else {
            panic!("restored font selector")
        };
        assert_eq!(font.raw(), 1);
        assert_eq!(
            universe
                .command_context()
                .expect("font admission")
                .font_name(font),
            "formatfont"
        );
        let root = universe
            .box_register(12)
            .expect("box register")
            .expect("box root");
        let admitted = universe.core.as_ref().expect("core").admit();
        assert!(matches!(
            admitted.node_list(root).expect("node list").nodes(),
            [Node::Char { font: node_font, ch: 'X', .. }] if *node_font == font
        ));
        drop(admitted);
        let context = universe.command_context().expect("PDF admission");
        assert!(context.pdf_raw_object(raw_object).is_some());
        assert!(context.pdf_form(form_object).is_some());
        drop(context);
        assert_eq!(
            universe.capture_format_image().expect("redump").as_bytes(),
            image.as_bytes()
        );
    })
    .expect("materialized full logical format");
}
