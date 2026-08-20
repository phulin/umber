use crate::PageListTestExt;
use tex_state::Universe;
use tex_state::math::{
    FractionThickness, LimitType, MathChar, MathChoice, MathField, MathFraction, MathNoad,
    MathStyle, NoadClass, NoadKind,
};
use tex_state::node::Node;
use tex_state::scaled::Scaled;
use tex_state::token::OriginId;

use super::{BoxAxis, NativeNodeTransaction};

fn math_char(ch: char) -> MathChar {
    MathChar {
        family: 3,
        character: ch,
        origin: OriginId::UNKNOWN,
    }
}

#[test]
fn tex82_noad_field_layout_initialization_and_release_matrix() {
    let nucleus: MathField<tex_state::node_arena::PageListId> = MathField::MathChar(math_char('x'));
    let classes = [
        NoadClass::Ord,
        NoadClass::Op,
        NoadClass::Bin,
        NoadClass::Rel,
        NoadClass::Open,
        NoadClass::Close,
        NoadClass::Punct,
        NoadClass::Inner,
    ];
    for class in classes {
        let noad = MathNoad::new(NoadKind::Normal(class), nucleus.clone());
        assert_eq!(noad.kind, NoadKind::Normal(class));
        assert_eq!(noad.nucleus, nucleus);
        assert_eq!(noad.subscript, MathField::Empty);
        assert_eq!(noad.superscript, MathField::Empty);
    }

    let special = [
        NoadKind::Operator(LimitType::DisplayLimits),
        NoadKind::Operator(LimitType::Limits),
        NoadKind::Operator(LimitType::NoLimits),
        NoadKind::Radical {
            delimiter: 0x07ff_ffff,
        },
        NoadKind::Accent {
            accent: math_char('^'),
        },
        NoadKind::LeftDelimiter { delimiter: 1 },
        NoadKind::RightDelimiter { delimiter: 2 },
        NoadKind::MiddleDelimiter { delimiter: 3 },
        NoadKind::Underline,
        NoadKind::Overline,
        NoadKind::VCenter,
    ];
    for kind in special {
        let noad = MathNoad::new(kind.clone(), nucleus.clone());
        assert_eq!(noad.kind, kind);
        assert_eq!(noad.subscript, MathField::Empty);
        assert_eq!(noad.superscript, MathField::Empty);
    }

    let mut stores = Universe::new();
    let arms =
        [1, 2, 3, 4].map(|penalty| stores.publish_page_nodes_for_test(&[Node::Penalty(penalty)]));
    let choice = MathChoice {
        display: arms[0].clone(),
        text: arms[1].clone(),
        script: arms[2].clone(),
        script_script: arms[3].clone(),
    };
    assert_eq!(choice.display.to_vec(), [Node::Penalty(1)]);
    assert_eq!(choice.text.to_vec(), [Node::Penalty(2)]);
    assert_eq!(choice.script.to_vec(), [Node::Penalty(3)]);
    assert_eq!(choice.script_script.to_vec(), [Node::Penalty(4)]);

    let fraction = MathFraction {
        numerator: arms[0].clone(),
        denominator: arms[1].clone(),
        thickness: FractionThickness::Explicit(Scaled::from_raw(-1)),
        left_delimiter: Some(0),
        right_delimiter: Some(0x07ff_ffff),
    };
    assert_ne!(fraction.numerator, fraction.denominator);
    assert_eq!(fraction.left_delimiter, Some(0));
    assert_eq!(fraction.right_delimiter, Some(0x07ff_ffff));

    let styles = [
        MathStyle::Display,
        MathStyle::Text,
        MathStyle::Script,
        MathStyle::ScriptScript,
    ];
    for (index, style) in styles.iter().enumerate() {
        assert!(!styles[..index].contains(style));
    }

    drop((choice, fraction, stores));
}

#[test]
fn cached_pack_template_preserves_horizontal_and_vertical_completion_order() {
    let mut layout = NativeNodeTransaction::new();
    let empty = layout.empty();

    let _ = layout.hpack(empty);
    let _ = layout.vpack(empty);
    let template = layout.take_pack_observations_since(0);

    assert_eq!(
        template.iter().map(|pack| pack.axis).collect::<Vec<_>>(),
        [BoxAxis::Horizontal, BoxAxis::Vertical],
        "TeX82 §§651 and 668 complete in call order"
    );

    layout.replay_pack_observations(&template);
    let completed = layout.finish(empty);
    assert_eq!(completed.pack_observations(), template.as_slice());
}
