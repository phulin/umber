//! Lazy durable publication for future expansion-episode memoization.

use std::sync::Arc;

use tex_state::token::TracedTokenWord;
use tex_state::{ExpansionState, TracedTokenList};

/// Memo-owned transient token content with optional durable identity.
///
/// Construction and ordinary token access never touch the permanent token or
/// provenance stores. The memo recorder calls [`Self::publish`] only when it
/// accepts an episode for reuse. An episode belongs to that state timeline;
/// it deliberately is not `Clone`, which prevents cached live handles from
/// being copied casually into an unrelated `Universe`.
#[derive(Debug)]
pub struct TransientTokenEpisode {
    tokens: Arc<[TracedTokenWord]>,
    published: Option<TracedTokenList>,
}

impl TransientTokenEpisode {
    #[must_use]
    pub fn new(tokens: impl Into<Arc<[TracedTokenWord]>>) -> Self {
        Self {
            tokens: tokens.into(),
            published: None,
        }
    }

    #[must_use]
    pub fn tokens(&self) -> &[TracedTokenWord] {
        &self.tokens
    }

    #[must_use]
    pub const fn published(&self) -> Option<TracedTokenList> {
        self.published
    }

    /// Publishes this episode at most once on its owning state timeline.
    pub fn publish(&mut self, stores: &mut impl ExpansionState) -> TracedTokenList {
        if let Some(published) = self.published {
            return published;
        }
        let published = stores.finish_traced_token_list(&self.tokens);
        self.published = Some(published);
        published
    }
}
