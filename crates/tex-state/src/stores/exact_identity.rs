use crate::state_hash::exact_identity_bytes;
use std::sync::Arc;

const EMPTY_ENV_DOMAIN: &[u8] = b"umber-exact-env-empty-v1";
const ENV_NODE_DOMAIN: &[u8] = b"umber-exact-env-node-v1";

/// Persistent deterministic treap over canonical environment-cell identities.
///
/// The shape depends only on the key identities, not insertion order. Updating
/// one cell path-copies logarithmically many nodes, so snapshots retain one
/// root while exact checkpoint identity follows only journal-dirty cells.
#[derive(Clone, Debug, Default)]
pub(super) struct ExactEnvIdentity {
    root: Option<Arc<Node>>,
    #[cfg(test)]
    updates: usize,
}

impl ExactEnvIdentity {
    pub(super) fn identity(&self) -> u64 {
        self.root.as_ref().map_or_else(
            || exact_identity_bytes(EMPTY_ENV_DOMAIN, &[]),
            |root| root.identity,
        )
    }

    pub(super) fn update(&mut self, key: u64, value: Option<u64>) {
        #[cfg(test)]
        {
            self.updates += 1;
        }
        self.root = match value {
            Some(value) => insert(self.root.take(), key, value),
            None => remove(self.root.take(), key),
        };
    }

    #[cfg(test)]
    pub(super) const fn testing_updates(&self) -> usize {
        self.updates
    }
}

#[derive(Debug)]
struct Node {
    key: u64,
    value: u64,
    priority: u64,
    left: Option<Arc<Node>>,
    right: Option<Arc<Node>>,
    len: usize,
    identity: u64,
}

impl Node {
    fn new(key: u64, value: u64, left: Option<Arc<Self>>, right: Option<Arc<Self>>) -> Arc<Self> {
        let priority = priority(key);
        let len = 1 + node_len(&left) + node_len(&right);
        let mut bytes = [0_u8; 192];
        let mut offset = 0;
        for part in [
            ENV_NODE_DOMAIN,
            key.to_le_bytes().as_slice(),
            value.to_le_bytes().as_slice(),
            node_identity(&left).to_le_bytes().as_slice(),
            node_identity(&right).to_le_bytes().as_slice(),
            (len as u64).to_le_bytes().as_slice(),
        ] {
            bytes[offset..offset + part.len()].copy_from_slice(part);
            offset += part.len();
        }
        Arc::new(Self {
            key,
            value,
            priority,
            left,
            right,
            len,
            identity: exact_identity_bytes(ENV_NODE_DOMAIN, &bytes[..offset]),
        })
    }
}

fn node_len(node: &Option<Arc<Node>>) -> usize {
    node.as_ref().map_or(0, |node| node.len)
}

fn node_identity(node: &Option<Arc<Node>>) -> u64 {
    node.as_ref().map_or(0, |node| node.identity)
}

fn higher_priority_key(key: u64, right: &Node) -> bool {
    let key_priority = priority(key);
    key_priority < right.priority || (key_priority == right.priority && key < right.key)
}

fn priority(mut key: u64) -> u64 {
    key ^= key >> 30;
    key = key.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    key ^= key >> 27;
    key = key.wrapping_mul(0x94d0_49bb_1331_11eb);
    key ^ (key >> 31)
}

fn higher_priority(left: &Node, right: &Node) -> bool {
    higher_priority_key(left.key, right)
}

fn insert(root: Option<Arc<Node>>, key: u64, value: u64) -> Option<Arc<Node>> {
    let Some(root) = root else {
        return Some(Node::new(key, value, None, None));
    };
    if key == root.key {
        if value == root.value {
            return Some(root);
        }
        return Some(Node::new(key, value, root.left.clone(), root.right.clone()));
    }
    if higher_priority_key(key, &root) {
        let (left, right) = split(Some(root), key);
        return Some(Node::new(key, value, left, right));
    }
    if key < root.key {
        Some(Node::new(
            root.key,
            root.value,
            insert(root.left.clone(), key, value),
            root.right.clone(),
        ))
    } else {
        Some(Node::new(
            root.key,
            root.value,
            root.left.clone(),
            insert(root.right.clone(), key, value),
        ))
    }
}

fn remove(root: Option<Arc<Node>>, key: u64) -> Option<Arc<Node>> {
    let root = root?;
    if key == root.key {
        return merge(root.left.clone(), root.right.clone());
    }
    if key < root.key {
        Some(Node::new(
            root.key,
            root.value,
            remove(root.left.clone(), key),
            root.right.clone(),
        ))
    } else {
        Some(Node::new(
            root.key,
            root.value,
            root.left.clone(),
            remove(root.right.clone(), key),
        ))
    }
}

fn split(root: Option<Arc<Node>>, key: u64) -> (Option<Arc<Node>>, Option<Arc<Node>>) {
    let Some(root) = root else {
        return (None, None);
    };
    if root.key < key {
        let (middle, right) = split(root.right.clone(), key);
        (
            Some(Node::new(root.key, root.value, root.left.clone(), middle)),
            right,
        )
    } else {
        let (left, middle) = split(root.left.clone(), key);
        (
            left,
            Some(Node::new(root.key, root.value, middle, root.right.clone())),
        )
    }
}

fn merge(left: Option<Arc<Node>>, right: Option<Arc<Node>>) -> Option<Arc<Node>> {
    match (left, right) {
        (None, right) => right,
        (left, None) => left,
        (Some(left), Some(right)) if higher_priority(&left, &right) => Some(Node::new(
            left.key,
            left.value,
            left.left.clone(),
            merge(left.right.clone(), Some(right)),
        )),
        (Some(left), Some(right)) => Some(Node::new(
            right.key,
            right.value,
            merge(Some(left), right.left.clone()),
            right.right.clone(),
        )),
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
        let mut forward = ExactEnvIdentity::default();
        let mut reverse = ExactEnvIdentity::default();
        for value in 1..=32 {
            forward.update(hash(value), Some(hash(value + 64)));
        }
        for value in (1..=32).rev() {
            reverse.update(hash(value), Some(hash(value + 64)));
        }
        assert_eq!(forward.identity(), reverse.identity());
    }

    #[test]
    fn replacement_and_removal_restore_identity() {
        let mut identity = ExactEnvIdentity::default();
        let empty = identity.identity();
        identity.update(hash(1), Some(hash(2)));
        let original = identity.identity();
        identity.update(hash(1), Some(hash(3)));
        assert_ne!(identity.identity(), original);
        identity.update(hash(1), Some(hash(2)));
        assert_eq!(identity.identity(), original);
        identity.update(hash(1), None);
        assert_eq!(identity.identity(), empty);
    }
}
