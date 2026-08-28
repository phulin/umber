use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{KernKind, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};

use super::{BoxAxis, Context, MathBox, MathNode, MathTypesetState, sub};

pub(super) fn rebox(
    ctx: &mut Context<'_, impl MathTypesetState>,
    boxed: &mut MathBox,
    width: Scaled,
) {
    let slack = sub(width, boxed.width);
    // TeX82 §715 changes the width field directly for an empty box.
    if slack.raw() != 0 && !boxed.list.is_empty() {
        let mut payload = if matches!(boxed.axis, BoxAxis::Vertical) {
            let list = ctx.layout.hlist([MathNode::VList(*boxed)]);
            // TeX82 §715 naturally hpacks every nonempty vertical source
            // whose width changes, including a zero-width source.
            let natural = ctx.layout.hpack(list);
            boxed.height = natural.height;
            boxed.depth = natural.depth;
            natural.list
        } else {
            boxed.list
        };
        // TeX82 §715: `clean_box` (§720) can leave a one-character
        // payload whose packed width still includes the physically removed
        // italic correction. `rebox` restores that difference as a normal
        // kern before adding the two `ss_glue` nodes. The packed width, not
        // the surviving list width, owns the correction at this boundary.
        if let Some(character @ MathNode::Char { metrics, .. }) =
            ctx.layout.single_node(payload).copied()
        {
            let correction = sub(boxed.width, metrics.width);
            if correction.raw() != 0 {
                payload = ctx.layout.hlist([
                    character,
                    MathNode::Kern {
                        amount: correction,
                        kind: KernKind::Font,
                    },
                ]);
            }
        }
        let ss_glue = MathNode::Glue {
            spec: GlueSpec {
                width: Scaled::from_raw(0),
                stretch: Scaled::from_raw(Scaled::UNITY),
                stretch_order: Order::Fil,
                shrink: Scaled::from_raw(Scaled::UNITY),
                shrink_order: Order::Fil,
            },
            kind: tex_state::node::GlueKind::Normal,
            leader: None,
        };
        boxed.list = ctx
            .layout
            .hlist([ss_glue, MathNode::Sequence(payload), ss_glue]);
        boxed.axis = BoxAxis::Horizontal;
        boxed.glue_set = GlueSetRatio::from_scaled_ratio(
            Scaled::from_raw(slack.raw().abs()),
            Scaled::from_raw(2 * Scaled::UNITY),
        );
        boxed.glue_sign = if slack.raw() > 0 {
            Sign::Stretching
        } else {
            Sign::Shrinking
        };
        boxed.glue_order = Order::Fil;
    }
    boxed.width = width;
    if slack.raw() != 0 && !boxed.list.is_empty() {
        ctx.layout.observe_completed_pack(boxed);
    }
}

#[cfg(test)]
pub(crate) fn test_rebox(
    state: &impl MathTypesetState,
    params: &super::MathParams,
    source_width: Scaled,
    target_width: Scaled,
    empty: bool,
    vertical: bool,
) -> (super::MathLayout, MathBox) {
    let mut ctx = Context {
        state,
        params,
        style: super::Style::TEXT,
        mu: Scaled::from_raw(0),
        layout: super::NativeNodeTransaction::new(),
        converted: Default::default(),
        source_lists: Default::default(),
        conversion_events: Default::default(),
        capture_replay: false,
        pack_replays: Default::default(),
        event_replays: Default::default(),
        recovered: Default::default(),
        scratch: Default::default(),
    };
    let list = if empty {
        ctx.layout.empty()
    } else {
        ctx.layout.hlist([MathNode::Kern {
            amount: source_width,
            kind: super::KernKind::Explicit,
        }])
    };
    let mut boxed = ctx.layout.hpack(list);
    if vertical {
        boxed.axis = BoxAxis::Vertical;
    }
    assert_eq!(boxed.width, source_width);
    rebox(&mut ctx, &mut boxed, target_width);
    let layout = ctx.layout.finish(boxed.list);
    (layout, boxed)
}

#[cfg(test)]
pub(crate) fn test_rebox_clean_character(
    state: &impl MathTypesetState,
    params: &super::MathParams,
    character: MathNode,
    retained_width: Scaled,
    target_width: Scaled,
) -> (super::MathLayout, MathBox) {
    let mut ctx = Context {
        state,
        params,
        style: super::Style::TEXT,
        mu: Scaled::from_raw(0),
        layout: super::NativeNodeTransaction::new(),
        converted: Default::default(),
        source_lists: Default::default(),
        conversion_events: Default::default(),
        capture_replay: false,
        pack_replays: Default::default(),
        event_replays: Default::default(),
        recovered: Default::default(),
        scratch: Default::default(),
    };
    let list = ctx.layout.hlist([character]);
    let mut boxed = ctx.layout.hpack(list);
    boxed.width = retained_width;
    rebox(&mut ctx, &mut boxed, target_width);
    let layout = ctx.layout.finish(boxed.list);
    (layout, boxed)
}
