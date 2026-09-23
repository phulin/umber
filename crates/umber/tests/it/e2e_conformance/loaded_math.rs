//! Focused loaded TRIP math layout, box, and immediate-write cases.

use super::*;

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_script_pair_dump_uses_a_normal_kern() {
    // Exact TRIP source through lines 438--440 reaches the malformed formula's
    // sup/sub pair and `\showbox9`. TeX82 §§135/158/184 make its generated
    // separator a normal kern, printed without the explicit-subtype space.
    let log = run_focused_loaded_trip_through(440);
    let expected = ".....\\ip /\n.....\\kern12.3\n.....\\hbox(0.0+0.0)x-0.01";
    assert!(log.contains(expected), "script-pair node dump:\n{log}");
    assert!(!log.contains(".....\\kern 12.3"), "{log}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_final_operator_has_one_zero_before_rebox() {
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..440].join("\n")).into_bytes());
    let (_, observer) = run_loaded_trip_source_observed(source);
    let oracle = b"{\"schema\":2,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let geometry = positionless_geometry(observer, oracle);
    let stream = ObservationStream::from_canonical_json_lines(&geometry).expect("geometry stream");
    let hpacks = stream
        .events
        .iter()
        .filter_map(|event| match event.semantic {
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                width_sp,
                height_sp,
                depth_sp,
                ..
            }) => Some((width_sp, height_sp, depth_sp)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let natural = (64_881, 0, 0);
    let exact = (64_881, 1_284_506, 0);

    assert!(
        hpacks
            .windows(3)
            .any(|packs| packs == [(0, 0, 0), natural, exact])
    );
    assert!(
        !hpacks
            .windows(4)
            .any(|packs| packs == [(0, 0, 0), (0, 0, 0), natural, exact])
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_hairy_display_preserves_appendix_g_pack_order() {
    // TRIP line 285 exercises TeX82 §§720, 724, 733, and 749 together. Keep
    // the repeated package calls in their canonical order; equal dimensions
    // are distinct completed operations, not deduplication candidates.
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..285].join("\n")).into_bytes());
    let (_, observer) = run_loaded_trip_source_observed(source);
    let oracle = b"{\"schema\":2,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let geometry = positionless_geometry(observer, oracle);
    let stream = ObservationStream::from_canonical_json_lines(&geometry).expect("geometry stream");
    let hpacks = stream
        .events
        .iter()
        .filter_map(|event| match event.semantic {
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                width_sp,
                height_sp,
                depth_sp,
                ..
            }) => Some((width_sp, height_sp, depth_sp)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(hpacks.windows(5).any(|packs| {
        packs
            == [
                (0, 0, 0),
                (0, 0, 0),
                (392_561, 1_120_666, 275_251),
                (392_561, 1_120_666, 275_251),
                (392_561, 1_120_666, 275_251),
            ]
    }));
    assert!(hpacks.windows(12).any(|packs| {
        packs
            == [
                (196_608, 524_288, 131_072),
                (196_608, 524_288, 131_072),
                (131_072, 0, 0),
                (131_072, 0, 0),
                (196_608, 786_432, 0),
                (196_608, 0, 0),
                (196_608, 1_835_008, 0),
                (524_288, 0, 0),
                (524_288, 0, 0),
                (524_288, 458_752, 0),
                (131_072, 0, 0),
                (131_072, 0, 0),
            ]
    }));
    assert!(hpacks.windows(3).any(|packs| {
        packs
            == [
                (393_216, 1_048_576, 262_145),
                (393_216, 1_048_576, 262_145),
                (0, 458_752, 0),
            ]
    }));
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_radical_overbar_uses_normal_kerns() {
    // The full loaded stream is necessary: skipping its pre-format prefix
    // does not reproduce the showbox9 list reached through TRIP lines 438--440.
    // TeX82 §§714/135 create both overbar spacers as normal `new_kern` nodes;
    // §184 consequently prints no explicit-subtype space after `\kern`.
    let source: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let log = run_loaded_trip_source(source);
    let overbar = log.lines().collect::<Vec<_>>().windows(4).any(|lines| {
        lines[0].starts_with("..\\vbox(")
            && lines[1].starts_with("...\\kern")
            && !lines[1].starts_with("...\\kern ")
            && lines[2].starts_with("...\\rule(")
            && lines[3].starts_with("...\\kern")
            && !lines[3].starts_with("...\\kern ")
    });
    assert!(overbar, "normal-kern radical overbar:\n{log}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_empty_operator_box_keeps_axis_shift() {
    // Full format-loaded history is required. Its malformed class-Op noad
    // reaches TeX82 §749 with a missing math character; the resulting empty
    // hbox must still pass through the common math-axis centering step.
    let source: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let log = run_loaded_trip_source(source);
    assert!(
        log.contains(
            ".............\\hbox(0.0+0.0)x0.0, shifted -7.0\n.............\\glue(\\nonscript)"
        ),
        "shifted operator nucleus is absent"
    );
}

#[test]
fn loaded_immediate_write_traces_expansion_in_no_mode() {
    // TeX82 §§299/367/1370: immediate `write_out` sets `mode:=0` around its
    // expanded scan. A trace inside the scan says `no mode`, and the next
    // main-control trace names the restored vertical mode because §367 left
    // `shown_mode=0`. The malformed delimited macro is TRIP's generic scanner
    // boundary, reduced independently of the rest of the document.
    let source: Arc<[u8]> = Arc::from(
        &b"\\tracingcommands=2\\tracingmacros=2\\tracingonline=1\\long\\def\\l#1\\l{#1}\\immediate\\write10{\\string\\caution \\l}\\escapechar=92\\end\n"[..],
    );
    let log = run_loaded_trip_source(source);
    let string_trace = log
        .find("{no mode: \\string}")
        .unwrap_or_else(|| panic!("§1370 immediate-write trace:\n{log}"));
    let macro_trace = log
        .find("\\l #1\\l ->#1")
        .unwrap_or_else(|| panic!("write macro trace:\n{log}"));
    let runaway = log
        .find("Runaway argument?")
        .unwrap_or_else(|| panic!("write scanner recovery:\n{log}"));

    assert!(string_trace < macro_trace && macro_trace < runaway, "{log}");
    assert!(log.contains("{vertical mode: \\escapechar}"), "{log}");
}

/// TeX82 §1110 distinguishes a void register from a nonempty incompatible
/// register. TRIP line 396's void `\unhbox234` is silent, while nonvoid
/// `\unhcopy3` in math mode reports before §1166 dispatches the following
/// text accent.
#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_math_unboxing_diagnoses_before_following_accent() {
    let log = run_focused_loaded_trip_through(396);
    let incompatible = log
        .find("Incompatible list can't be unboxed")
        .expect("unhcopy diagnostic");
    let accent = log
        .find("Please use \\mathaccent for accents in math mode")
        .expect("following accent diagnostic");

    assert!(incompatible < accent, "diagnostic order:\n{log}");
    assert_eq!(
        log.matches("Incompatible list can't be unboxed").count(),
        1,
        "void unhbox must remain silent:\n{log}"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_nested_empty_math_box_does_not_republish_source_hpack() {
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..210].join("\n")).into_bytes());
    let (_, observer) = run_loaded_trip_source_observed(source);
    let oracle = b"{\"schema\":2,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let geometry = positionless_geometry(observer, oracle);
    let stream = ObservationStream::from_canonical_json_lines(&geometry).expect("geometry stream");
    let hpacks = stream
        .events
        .iter()
        .filter_map(|event| match event.semantic {
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                width_sp,
                height_sp,
                depth_sp,
                ..
            }) => Some((width_sp, height_sp, depth_sp)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(hpacks.windows(3).any(|packs| {
        packs
            == [
                (7_864_320, 0, 0),
                (7_864_320, 0, 0),
                (6_553_600, 458_752, 65_536),
            ]
    }));
    assert!(
        !hpacks
            .windows(3)
            .any(|packs| { packs == [(7_864_320, 0, 0), (7_864_320, 0, 0), (7_864_320, 0, 0),] })
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_hairy_display_publishes_both_clean_character_packs() {
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..285].join("\n")).into_bytes());
    let (_, observer) = run_loaded_trip_source_observed(source);
    let oracle = b"{\"schema\":2,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let geometry = positionless_geometry(observer, oracle);
    let stream = ObservationStream::from_canonical_json_lines(&geometry).expect("geometry stream");
    let hpacks = stream
        .events
        .iter()
        .filter_map(|event| match event.semantic {
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                width_sp,
                height_sp,
                depth_sp,
                ..
            }) => Some((width_sp, height_sp, depth_sp)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(hpacks.windows(4).any(|packs| {
        packs
            == [
                (0, 393_216, 0),
                (196_608, 458_752, 65_536),
                (196_608, 458_752, 65_536),
                (131_072, 589_824, 0),
            ]
    }));
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_missing_accent_publishes_clean_nucleus_pack() {
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..396].join("\n")).into_bytes());
    let (_, observer) = run_loaded_trip_source_observed(source);
    let oracle = b"{\"schema\":2,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    let geometry = positionless_geometry(observer, oracle);
    let stream = ObservationStream::from_canonical_json_lines(&geometry).expect("geometry stream");
    let hpacks = stream
        .events
        .iter()
        .filter_map(|event| match event.semantic {
            tex_oracle::Event::Geometry(tex_oracle::GeometryEvent::Hpack {
                width_sp,
                height_sp,
                depth_sp,
                ..
            }) => Some((width_sp, height_sp, depth_sp)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(hpacks.windows(5).any(|packs| {
        packs
            == [
                (26_214, 0, 0),
                (0, 0, 0),
                (0, 0, 0),
                (0, 0, 0),
                (6_553_600, 0, 0),
            ]
    }));
}
