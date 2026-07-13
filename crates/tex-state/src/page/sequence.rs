use crate::node::Node;
use std::sync::Arc;

pub(super) const PAGE_NODE_CHUNK_LEN: usize = 64;

/// Canonical persistent sequence for the growing current page.
///
/// Full 64-node leaves form a binary forest whose shape is the binary
/// decomposition of the full-leaf count. Appending a full leaf merges only the
/// carry path, while snapshots share every unaffected subtree. A bounded tail
/// holds fewer than 64 nodes. Shape depends only on content position, and the
/// representation carries no mutation-maintained hash state.
#[derive(Clone, Debug, Default)]
pub(super) struct PageNodeSequence {
    pub(super) forest: Arc<Vec<Arc<PageNodeTree>>>,
    pub(super) tail: Arc<Vec<Node>>,
    pub(super) len: usize,
}

#[derive(Debug)]
pub(super) enum PageNodeTree {
    Leaf(Vec<Node>),
    Branch {
        height: u8,
        len: usize,
        left: Arc<PageNodeTree>,
        right: Arc<PageNodeTree>,
    },
}

impl PageNodeTree {
    pub(super) fn height(&self) -> u8 {
        match self {
            Self::Leaf(_) => 0,
            Self::Branch { height, .. } => *height,
        }
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Leaf(nodes) => nodes.len(),
            Self::Branch { len, .. } => *len,
        }
    }

    fn get(&self, index: usize) -> Option<&Node> {
        match self {
            Self::Leaf(nodes) => nodes.get(index),
            Self::Branch { left, right, .. } => {
                let left_len = left.len();
                if index < left_len {
                    left.get(index)
                } else {
                    right.get(index - left_len)
                }
            }
        }
    }
}

pub(super) struct PageNodeIter<'a> {
    nodes: &'a PageNodeSequence,
    front: usize,
    back: usize,
}

impl<'a> Iterator for PageNodeIter<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        let node = self.nodes.get(self.front);
        self.front += 1;
        node
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.back - self.front;
        (remaining, Some(remaining))
    }
}

impl DoubleEndedIterator for PageNodeIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        self.nodes.get(self.back)
    }
}

impl ExactSizeIterator for PageNodeIter<'_> {}

impl PartialEq for PageNodeSequence {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().eq(other.iter())
    }
}

impl PageNodeSequence {
    pub(super) fn iter(&self) -> PageNodeIter<'_> {
        PageNodeIter {
            nodes: self,
            front: 0,
            back: self.len,
        }
    }

    pub(super) fn last(&self) -> Option<&Node> {
        self.get(self.len.checked_sub(1)?)
    }

    pub(super) const fn len(&self) -> usize {
        self.len
    }

    pub(super) fn push(&mut self, node: Node) {
        let tail = Arc::make_mut(&mut self.tail);
        tail.push(node);
        self.len += 1;
        if tail.len() != PAGE_NODE_CHUNK_LEN {
            return;
        }

        let leaf = Arc::new(PageNodeTree::Leaf(std::mem::take(tail)));
        let forest = Arc::make_mut(&mut self.forest);
        let mut carry = leaf;
        while forest
            .last()
            .is_some_and(|root| root.height() == carry.height())
        {
            let left = forest.pop().expect("equal-height forest root exists");
            carry = Arc::new(PageNodeTree::Branch {
                height: carry.height() + 1,
                len: left.len() + carry.len(),
                left,
                right: carry,
            });
        }
        forest.push(carry);
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn take_prefix(&mut self, split_index: usize) -> (Vec<Node>, Vec<Node>) {
        let split_index = split_index.min(self.len);
        let mut nodes = self.iter().cloned().collect::<Vec<_>>();
        let after = nodes.split_off(split_index);
        self.clear();
        (nodes, after)
    }

    fn get(&self, mut index: usize) -> Option<&Node> {
        if index >= self.len {
            return None;
        }
        for root in self.forest.iter() {
            if index < root.len() {
                return root.get(index);
            }
            index -= root.len();
        }
        self.tail.get(index)
    }
}
