//! e-TeX's effective view of a live list tail.

use tex_state::NodeView;
use tex_state::node::Direction;

/// The node e-TeX treats as the tail for enquiries and destructive commands.
///
/// e-TeX 2.6 `etex.ch` blocks 99 and 253 make a generated final `\endM`
/// boundary transparent.  No other math node is transparent: in particular,
/// a real final `MathOff` remains the effective tail.
#[derive(Clone)]
pub(crate) struct EffectiveTail<'a> {
    node: NodeView<'a>,
    index: usize,
    len: usize,
    preceded_by_begin: bool,
}

impl<'a> EffectiveTail<'a> {
    #[must_use]
    pub(crate) fn find<I>(mut nodes: I) -> Option<Self>
    where
        I: DoubleEndedIterator<Item = NodeView<'a>> + ExactSizeIterator,
    {
        let len = nodes.len();
        let last = nodes.next_back()?;
        let (node, index) = if matches!(last, NodeView::Direction(Direction::EndM)) {
            (nodes.next_back()?, len - 2)
        } else {
            (last, len - 1)
        };
        let preceded_by_begin = nodes
            .next_back()
            .is_some_and(|node| matches!(node, NodeView::Direction(Direction::BeginM)));
        Some(Self {
            node,
            index,
            len,
            preceded_by_begin,
        })
    }

    #[must_use]
    pub(crate) fn node(&self) -> NodeView<'a> {
        self.node.clone()
    }

    /// Returns the range removed by block 253 after the effective node has
    /// matched a destructive command.  An adjacent generated `beginM/endM`
    /// pair is removed with it; otherwise the transparent `endM` survives.
    #[must_use]
    pub(crate) fn removal_range(&self) -> std::ops::RangeInclusive<usize> {
        let transparent_end = self.index + 1 < self.len;
        if transparent_end && self.preceded_by_begin {
            self.index - 1..=self.index + 1
        } else {
            self.index..=self.index
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_state::node::KernKind;
    use tex_state::node::Node;
    use tex_state::scaled::Scaled;

    #[test]
    fn generated_end_m_is_transparent_but_real_math_off_is_not() {
        let kern = Node::Kern {
            amount: Scaled::from_raw(1),
            kind: KernKind::Explicit,
        };
        let generated = [
            Node::Direction(Direction::BeginM),
            kern.clone(),
            Node::Direction(Direction::EndM),
        ];
        let tail = EffectiveTail::find(generated.iter().map(Into::into)).expect("effective tail");
        assert!(matches!(tail.clone().node(), NodeView::Kern { .. }));
        assert_eq!(tail.removal_range(), 0..=2);

        let math_off = [kern, Node::MathOff(Scaled::from_raw(0))];
        let tail = EffectiveTail::find(math_off.iter().map(Into::into)).expect("effective tail");
        assert!(matches!(tail.clone().node(), NodeView::MathOff(_)));
        assert_eq!(tail.removal_range(), 1..=1);
    }

    #[test]
    fn unmatched_generated_end_m_survives_effective_tail_removal() {
        let nodes = [
            Node::Penalty(1),
            Node::Kern {
                amount: Scaled::from_raw(2),
                kind: KernKind::Explicit,
            },
            Node::Direction(Direction::EndM),
        ];
        let tail = EffectiveTail::find(nodes.iter().map(Into::into)).expect("effective tail");
        assert_eq!(tail.removal_range(), 1..=1);
    }
}
