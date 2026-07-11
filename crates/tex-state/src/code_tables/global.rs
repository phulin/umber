use std::sync::Arc;

use super::{Catcode, DelCode, LcCode, MathCode, SfCode, UcCode};

#[derive(Clone, Copy, Debug)]
pub(super) enum GlobalCodeTableWrite {
    Catcode(char, Catcode),
    LcCode(char, LcCode),
    UcCode(char, UcCode),
    SfCode(char, SfCode),
    MathCode(char, MathCode),
    DelCode(char, DelCode),
}

#[derive(Clone, Debug, Default)]
pub(super) struct GlobalWriteHistory {
    head: Option<Arc<GlobalWriteNode>>,
    len: usize,
}

#[derive(Debug)]
struct GlobalWriteNode {
    write: GlobalCodeTableWrite,
    previous: Option<Arc<Self>>,
}

impl GlobalWriteHistory {
    pub(super) fn push(&mut self, write: GlobalCodeTableWrite) {
        self.head = Some(Arc::new(GlobalWriteNode {
            write,
            previous: self.head.take(),
        }));
        self.len = self
            .len
            .checked_add(1)
            .expect("global code-table history overflow");
    }

    pub(super) fn writes_since(&self, base: &Self) -> Vec<GlobalCodeTableWrite> {
        let count = self
            .len
            .checked_sub(base.len)
            .expect("global code-table history is older than group frame");
        let mut writes = Vec::with_capacity(count);
        let mut cursor = self.head.as_ref();
        for _ in 0..count {
            let node = cursor.expect("global code-table history is truncated");
            writes.push(node.write);
            cursor = node.previous.as_ref();
        }
        assert!(
            heads_match(cursor, base.head.as_ref()),
            "global code-table history belongs to a different branch"
        );
        writes.reverse();
        writes
    }

    #[cfg(test)]
    pub(super) const fn len(&self) -> usize {
        self.len
    }
}

impl Drop for GlobalWriteHistory {
    fn drop(&mut self) {
        let mut cursor = self.head.take();
        while let Some(node) = cursor {
            match Arc::try_unwrap(node) {
                Ok(mut node) => cursor = node.previous.take(),
                Err(shared) => {
                    drop(shared);
                    break;
                }
            }
        }
    }
}

fn heads_match(left: Option<&Arc<GlobalWriteNode>>, right: Option<&Arc<GlobalWriteNode>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}
