//! Optional direct-child census for built-root structural copies.

use super::{PageListId, PageMaterialArena};
use crate::measurement::BoxFallbackShape;
use crate::node_view::NodeView;

impl PageMaterialArena<'_> {
    pub(super) fn profile_built_fallback_shape(&self, root: PageListId) -> BoxFallbackShape {
        let Ok(wrapper) = self.node_cursor(root) else {
            return BoxFallbackShape::Inline;
        };
        let child = match wrapper.first() {
            Some(NodeView::HList(boxed) | NodeView::VList(boxed)) => boxed.children,
            _ => return BoxFallbackShape::Inline,
        };
        let Ok(body) = self.node_cursor(child) else {
            return BoxFallbackShape::Inline;
        };
        let mut shape = BoxFallbackShape::Inline;
        for node in body.iter() {
            let candidate = match node {
                NodeView::Glue {
                    leader: Some(_), ..
                } => BoxFallbackShape::Leader,
                NodeView::HList(_) | NodeView::VList(_) | NodeView::Unset(_) => {
                    BoxFallbackShape::NestedBox
                }
                NodeView::Disc { .. } => BoxFallbackShape::Disc,
                NodeView::Ins { .. } | NodeView::Adjust(_) => BoxFallbackShape::Migration,
                NodeView::MathNoad(_)
                | NodeView::FractionNoad(_)
                | NodeView::MathChoice(_)
                | NodeView::MathList(_) => BoxFallbackShape::Math,
                NodeView::Glue { .. } => BoxFallbackShape::Glue,
                NodeView::Lig { .. } | NodeView::Mark { .. } | NodeView::Whatsit(_) => {
                    BoxFallbackShape::OtherAnnex
                }
                _ => BoxFallbackShape::Inline,
            };
            if (candidate as u8) > (shape as u8) {
                shape = candidate;
            }
        }
        shape
    }
}
