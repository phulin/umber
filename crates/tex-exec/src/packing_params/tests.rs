use tex_state::glue::GlueSpec;
use tex_state::node::{GlueKind, Node};
use tex_state::scaled::Scaled;
use tex_state::{GeometryObservation, Universe};
use tex_typeset::{PackSpec, VpackParams};

use super::vtop;

#[test]
fn vtop_observes_vpackage_before_readjusting_height_and_depth() {
    // TRIP line 330 exercises `\vtop{\vskip-3mm}`. TeX82 §668 first
    // vpackages that list to (height=-3mm, depth=0); §1087 then gives a
    // leading-glue vtop height zero and transfers the total size to depth.
    const NEGATIVE_THREE_MM: Scaled = Scaled::from_raw(-559_403);
    let mut stores = Universe::new();
    stores.enable_geometry_observation();
    let glue = stores.intern_glue(GlueSpec {
        width: NEGATIVE_THREE_MM,
        ..GlueSpec::ZERO
    });
    let list = stores.freeze_node_list(&[Node::Glue {
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
        }]
    );
    assert_eq!(packed.node.height, Scaled::from_raw(0));
    assert_eq!(packed.node.depth, NEGATIVE_THREE_MM);
}
