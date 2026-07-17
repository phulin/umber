use crate::state_hash::exact_identity_bytes;
use std::sync::Arc;

const EMPTY_DOMAIN: &[u8] = b"umber-exact-canonical-collection-empty-v1";
const NODE_DOMAIN: &[u8] = b"umber-exact-canonical-collection-node-v1";

/// Persistent deterministic treap over canonical content identities.
///
/// Its shape depends only on the identities, never their insertion order.
/// Cloning a root is O(1), and inserting one new identity path-copies an
/// expected logarithmic number of nodes.
#[derive(Clone, Debug, Default)]
pub(super) struct CanonicalCollectionRoot(Option<Arc<Node>>);

impl CanonicalCollectionRoot {
    pub(super) fn insert(&mut self, key: u64) {
        self.0 = insert(self.0.take(), key);
    }

    pub(super) fn identity(&self) -> u64 {
        self.0.as_ref().map_or_else(
            || exact_identity_bytes(EMPTY_DOMAIN, &[]),
            |root| root.identity,
        )
    }
}

#[derive(Debug)]
struct Node {
    key: u64,
    priority: u64,
    left: Option<Arc<Self>>,
    right: Option<Arc<Self>>,
    len: usize,
    identity: u64,
}

impl Node {
    fn new(key: u64, left: Option<Arc<Self>>, right: Option<Arc<Self>>) -> Arc<Self> {
        let len = 1 + node_len(&left) + node_len(&right);
        let priority = priority(key);
        let mut framed = Vec::with_capacity(144);
        framed.extend_from_slice(NODE_DOMAIN);
        framed.extend_from_slice(&key.to_le_bytes());
        framed.extend_from_slice(&node_identity(&left).to_le_bytes());
        framed.extend_from_slice(&node_identity(&right).to_le_bytes());
        framed.extend_from_slice(&(len as u64).to_le_bytes());
        Arc::new(Self {
            key,
            priority,
            left,
            right,
            len,
            identity: exact_identity_bytes(NODE_DOMAIN, &framed),
        })
    }
}

fn priority(key: u64) -> u64 {
    let mut priority = key;
    priority ^= priority >> 30;
    priority = priority.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    priority ^= priority >> 27;
    priority = priority.wrapping_mul(0x94d0_49bb_1331_11eb);
    priority ^ (priority >> 31)
}

fn node_len(node: &Option<Arc<Node>>) -> usize {
    node.as_ref().map_or(0, |node| node.len)
}

fn node_identity(node: &Option<Arc<Node>>) -> u64 {
    node.as_ref().map_or(0, |node| node.identity)
}

fn insert(root: Option<Arc<Node>>, key: u64) -> Option<Arc<Node>> {
    let Some(root) = root else {
        return Some(Node::new(key, None, None));
    };
    if key == root.key {
        return Some(root);
    }
    let priority = priority(key);
    if priority < root.priority || (priority == root.priority && key < root.key) {
        let (left, right) = split(Some(root), key);
        return Some(Node::new(key, left, right));
    }
    if key < root.key {
        Some(Node::new(
            root.key,
            insert(root.left.clone(), key),
            root.right.clone(),
        ))
    } else {
        Some(Node::new(
            root.key,
            root.left.clone(),
            insert(root.right.clone(), key),
        ))
    }
}

fn split(root: Option<Arc<Node>>, key: u64) -> (Option<Arc<Node>>, Option<Arc<Node>>) {
    let Some(root) = root else {
        return (None, None);
    };
    if root.key < key {
        let (middle, right) = split(root.right.clone(), key);
        (Some(Node::new(root.key, root.left.clone(), middle)), right)
    } else {
        let (left, middle) = split(root.left.clone(), key);
        (left, Some(Node::new(root.key, middle, root.right.clone())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: u8) -> u64 {
        exact_identity_bytes(b"test", &[value])
    }

    #[test]
    fn insertion_order_does_not_change_identity() {
        let mut forward = CanonicalCollectionRoot::default();
        let mut reverse = CanonicalCollectionRoot::default();
        for value in 1..=32 {
            forward.insert(hash(value));
        }
        for value in (1..=32).rev() {
            reverse.insert(hash(value));
        }
        assert_eq!(forward.identity(), reverse.identity());
    }

    #[test]
    fn duplicate_content_does_not_change_set_identity() {
        let mut identity = CanonicalCollectionRoot::default();
        identity.insert(hash(1));
        let once = identity.identity();
        identity.insert(hash(1));
        assert_eq!(identity.identity(), once);
    }
}
