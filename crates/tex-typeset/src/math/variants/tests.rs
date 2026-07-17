use tex_fonts::{OpenTypeMathAssembly, OpenTypeMathAssemblyPart, OpenTypeMathGlyph};
use tex_state::scaled::Scaled;

use super::plan_assembly;

fn sc(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

fn part(
    glyph_id: u16,
    full_advance: i32,
    connector: i32,
    extender: bool,
) -> OpenTypeMathAssemblyPart {
    OpenTypeMathAssemblyPart {
        glyph: OpenTypeMathGlyph {
            glyph_id,
            metrics: tex_fonts::CharMetrics {
                width: sc(full_advance),
                height: sc(full_advance),
                depth: sc(0),
                italic_correction: sc(0),
                tag: tex_fonts::MetricCharTag::None,
            },
            italic_correction: sc(0),
            top_accent_attachment: None,
        },
        start_connector: sc(connector),
        end_connector: sc(connector),
        full_advance: sc(full_advance),
        extender,
    }
}

#[test]
fn repeats_extenders_and_distributes_overlap_to_exact_target() {
    let assembly = OpenTypeMathAssembly {
        italic_correction: sc(0),
        min_connector_overlap: sc(10),
        parts: vec![
            part(1, 100, 40, false),
            part(2, 100, 40, true),
            part(3, 100, 40, false),
        ],
    };

    let (parts, overlaps) = plan_assembly(&assembly, sc(450)).expect("valid assembly");

    assert_eq!(
        parts
            .iter()
            .map(|part| part.glyph.glyph_id)
            .collect::<Vec<_>>(),
        [1, 2, 2, 2, 3]
    );
    assert_eq!(overlaps, [sc(20), sc(10), sc(10), sc(10)]);
    let extent: i32 = parts
        .iter()
        .map(|part| part.full_advance.raw())
        .sum::<i32>()
        - overlaps.iter().map(|overlap| overlap.raw()).sum::<i32>();
    assert_eq!(extent, 450);
}

#[test]
fn connector_capacity_is_respected_in_stable_part_order() {
    let assembly = OpenTypeMathAssembly {
        italic_correction: sc(0),
        min_connector_overlap: sc(10),
        parts: vec![
            part(1, 100, 15, false),
            part(2, 100, 30, false),
            part(3, 100, 30, false),
        ],
    };

    let (_, overlaps) = plan_assembly(&assembly, sc(250)).expect("valid assembly");

    assert_eq!(overlaps, [sc(15), sc(30)]);
}

#[test]
fn malformed_connectors_and_non_growing_extenders_fail_closed() {
    let short_connector = OpenTypeMathAssembly {
        italic_correction: sc(0),
        min_connector_overlap: sc(20),
        parts: vec![part(1, 100, 10, false), part(2, 100, 10, false)],
    };
    assert!(plan_assembly(&short_connector, sc(100)).is_none());

    let non_growing_cycle = OpenTypeMathAssembly {
        italic_correction: sc(0),
        min_connector_overlap: sc(100),
        parts: vec![part(7, 100, 100, true), part(7, 100, 100, false)],
    };
    assert!(plan_assembly(&non_growing_cycle, sc(1_000)).is_none());
}
