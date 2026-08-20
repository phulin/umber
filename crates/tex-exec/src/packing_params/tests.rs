use tex_state::glue::GlueSpec;
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::{EffectRecord, GeometryObservation, SourceId, Universe};
use tex_typeset::{HpackParams, PackSpec, VpackParams};

use super::{hpack, vtop};

fn log_text(stores: &Universe) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: tex_state::PrintSink::Log | tex_state::PrintSink::TerminalAndLog,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn ordinary_hpack_reports_once_without_decorating_its_list() {
    let mut stores = Universe::new();
    stores.set_interaction_mode(tex_state::InteractionMode::Batch);
    let list = stores.publish_page_nodes(&[Node::Kern {
        amount: Scaled::from_raw(2 * Scaled::UNITY),
        kind: KernKind::Explicit,
    }]);
    let expected_children = list.clone();

    let packed = hpack(
        &mut stores,
        list,
        PackSpec::Exactly(Scaled::from_raw(Scaled::UNITY)),
        HpackParams {
            hbadness: 0,
            hfuzz: Scaled::from_raw(0),
            overfull_rule: Scaled::from_raw(5 * Scaled::UNITY),
        },
    );

    let log = log_text(&stores);
    assert_eq!(log.matches("Overfull \\hbox").count(), 1, "{log}");
    assert_eq!(packed.node.children, expected_children);
    assert!(!log.contains("\n|\n"), "{log}");
}

#[test]
fn vtop_observes_vpackage_before_readjusting_height_and_depth() {
    // TRIP line 330 exercises `\vtop{\vskip-3mm}`. TeX82 §668 first
    // vpackages that list to (height=-3mm, depth=0); §1087 then gives a
    // leading-glue vtop height zero and transfers the total size to depth.
    const NEGATIVE_THREE_MM: Scaled = Scaled::from_raw(-559_403);
    let mut stores = Universe::new();
    stores.enable_geometry_observation();
    stores.set_current_input_position(330, Some(SourceId::new(7)));
    let glue = GlueSpec {
        width: NEGATIVE_THREE_MM,
        ..GlueSpec::ZERO
    };
    let list = stores.publish_page_nodes(&[Node::Glue {
        spec: glue,
        kind: GlueKind::Normal,
        leader: None,
    }]);

    let packed = vtop(
        &mut stores,
        list,
        PackSpec::Natural,
        VpackParams {
            vbadness: tex_typeset::INF_BAD,
            vfuzz: Scaled::from_raw(0),
            box_max_depth: Scaled::MAX,
        },
    );

    assert_eq!(
        stores.geometry_observations_since(0),
        &[GeometryObservation::Vpack {
            width_sp: 0,
            height_sp: i64::from(NEGATIVE_THREE_MM.raw()),
            depth_sp: 0,
            line: 330,
            source: Some(SourceId::new(7)),
        }]
    );
    assert_eq!(packed.node.height, Scaled::from_raw(0));
    assert_eq!(packed.node.depth, NEGATIVE_THREE_MM);
}
