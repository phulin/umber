//! Immutable, path-copied Patricia index for checkpointed PDF version events.

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(super) struct PdfVersionRoot(pub(super) Option<u32>);

#[derive(Clone, Copy, Debug)]
enum Node {
    Leaf { key: u64, value: u32 },
    Branch { bit: u8, children: [u32; 2] },
}

#[derive(Debug, Default)]
pub(super) struct PdfVersionIndex {
    accepted: Vec<Node>,
    candidate: Vec<Node>,
}

impl PdfVersionIndex {
    /// Physical vector allocation retained by this ledger, including spare
    /// capacity. The number of initialized historical nodes is `len`, while
    /// heap attribution must count the reserved allocations.
    pub(super) fn allocated_bytes(&self) -> usize {
        (self.accepted.capacity() + self.candidate.capacity()) * std::mem::size_of::<Node>()
    }

    fn node(&self, index: u32) -> Node {
        let index = index as usize;
        if index < self.accepted.len() {
            self.accepted[index]
        } else {
            self.candidate[index - self.accepted.len()]
        }
    }

    pub(super) fn get(&self, root: PdfVersionRoot, key: u64) -> Option<u32> {
        let mut index = root.0?;
        loop {
            match self.node(index) {
                Node::Leaf { key: found, value } => return (found == key).then_some(value),
                Node::Branch { bit, children } => index = children[branch(key, bit)],
            }
        }
    }

    #[cfg(all(feature = "profiling", feature = "testing"))]
    pub(super) fn lookup_probes(&self, root: PdfVersionRoot, key: u64) -> u32 {
        let Some(mut index) = root.0 else { return 0 };
        let mut probes = 0;
        loop {
            probes += 1;
            match self.node(index) {
                Node::Leaf { .. } => return probes,
                Node::Branch { bit, children } => index = children[branch(key, bit)],
            }
        }
    }

    pub(super) fn insert(
        &mut self,
        root: PdfVersionRoot,
        key: u64,
        value: u32,
        candidate: bool,
    ) -> PdfVersionRoot {
        let Some(mut index) = root.0 else {
            return PdfVersionRoot(Some(self.push(Node::Leaf { key, value }, candidate)));
        };

        // First find a representative leaf. Its highest differing bit is the
        // only point where a new branch may enter the compressed tree.
        let mut ancestors = [0; u64::BITS as usize];
        let mut depth = 0;
        let old_key = loop {
            match self.node(index) {
                Node::Leaf { key: found, .. } => break found,
                Node::Branch { bit, children } => {
                    ancestors[depth] = index;
                    depth += 1;
                    index = children[branch(key, bit)];
                }
            }
        };
        if old_key == key {
            let leaf = self.push(Node::Leaf { key, value }, candidate);
            return PdfVersionRoot(Some(self.copy_ancestors(
                &ancestors[..depth],
                key,
                leaf,
                candidate,
            )));
        }

        let differing_bit = (u64::BITS - 1 - (old_key ^ key).leading_zeros()) as u8;
        // Descend only through branch bits above the new split. All keys below
        // that point share the representative leaf's bit at the split.
        index = root.0.expect("nonempty root");
        depth = 0;
        while let Node::Branch { bit, children } = self.node(index) {
            if bit < differing_bit {
                break;
            }
            debug_assert_ne!(bit, differing_bit);
            ancestors[depth] = index;
            depth += 1;
            index = children[branch(key, bit)];
        }
        let leaf = self.push(Node::Leaf { key, value }, candidate);
        let mut children = [index; 2];
        children[branch(key, differing_bit)] = leaf;
        let split = self.push(
            Node::Branch {
                bit: differing_bit,
                children,
            },
            candidate,
        );
        PdfVersionRoot(Some(self.copy_ancestors(
            &ancestors[..depth],
            key,
            split,
            candidate,
        )))
    }

    fn copy_ancestors(
        &mut self,
        ancestors: &[u32],
        key: u64,
        mut child: u32,
        candidate: bool,
    ) -> u32 {
        for &index in ancestors.iter().rev() {
            let Node::Branch { bit, mut children } = self.node(index) else {
                unreachable!("only branches precede a leaf")
            };
            children[branch(key, bit)] = child;
            child = self.push(Node::Branch { bit, children }, candidate);
        }
        child
    }

    fn push(&mut self, node: Node, candidate: bool) -> u32 {
        let absolute = self.accepted.len() + self.candidate.len();
        let absolute = u32::try_from(absolute).expect("PDF version-index capacity");
        if candidate {
            self.candidate.push(node);
        } else {
            debug_assert!(self.candidate.is_empty());
            self.accepted.push(node);
        }
        absolute
    }

    pub(super) fn reject_candidate(&mut self) {
        self.candidate.clear();
    }

    pub(super) fn accept_candidate(&mut self) {
        self.accepted.append(&mut self.candidate);
    }
}

const fn branch(key: u64, bit: u8) -> usize {
    ((key >> bit) & 1) as usize
}

#[cfg(test)]
mod tests;
