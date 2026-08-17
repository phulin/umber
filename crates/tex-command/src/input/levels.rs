//! Dense source and token-list input-level ownership.
#![allow(dead_code)] // consumed by the ordered raw-delivery implementation issues

use std::sync::Arc;

use tex_state::provenance::{OriginListRef, OriginRef};
use tex_state::token::{RootedTracedTokenWord, TracedTokenWord};
use tex_state::token_store::TokenListRef;

use crate::macro_call::{MacroActivationId, MacroArgumentRange};

use super::{
    lines::SourceProvenance,
    source::{SourceCursor, SourceNameClass},
};

/// Stable identity for one live input level.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct InputLevelId(pub(crate) u64);

/// One future-relevant input level.
///
/// Conditions, caches, scanner policy, and paragraph transitions cannot be
/// represented here. Both character profiles use this same level structure.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum InputLevel {
    Source(Box<SourceLevel>),
    Tokens(TokenCursor),
}

/// One registered-source level and its exact delivery identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceLevel {
    pub(crate) identity: InputLevelId,
    pub(crate) cursor: SourceCursor,
    /// tex.web §303's `name` classification for this level. A token-list
    /// level has no counterpart: §307 reuses `name` there as the eqtb address
    /// of the macro being expanded, which is why this lives on `SourceLevel`
    /// and not on [`InputLevel`].
    pub(crate) name_class: SourceNameClass,
    pub(crate) retirement: SourceRetirement,
    /// e-TeX §24.362's once-only token list, pushed above this source when
    /// natural EOF is first observed and before `end_file_reading`.
    pub(crate) every_eof: Option<tex_state::TracedTokenList>,
    /// e-TeX 2.6 [23.328]'s `grp_stack[in_open]`/`if_stack[in_open]`: the
    /// live group and conditional boundary ancestry recorded when this
    /// level's `begin_file_reading` ran, compared against the current stacks
    /// at `end_file_reading` to drive `\tracingnesting`'s `file_warning`.
    /// `None` until the opener records it (this crate has no `Universe`
    /// access at construction time; see `CommandState::record_source_open_depths`).
    pub(crate) open_depths: Option<Box<SourceOpenDepths>>,
}

/// e-TeX 2.6's `grp_stack`/`if_stack` entry for one open source level.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceOpenDepths {
    pub(crate) group_lineages: Box<[u64]>,
    pub(crate) conditional_identities: Box<[u64]>,
}

/// What exhausting a source level does, per tex.web §360.
///
/// §360 branches on `name`, the level's file identity: `if name>17 then
/// <read the next line, or end the file>` and otherwise, for a `\read`
/// pseudo-file, `if not terminal_input then {\read line has ended} begin
/// cur_cmd:=0; cur_chr:=0; return; end`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum SourceRetirement {
    /// §362's `name>17`: `end_file_reading` and resume the enclosing level.
    #[default]
    Pop,
    /// §483's `name:=m+1`: one acquired line, whose exhaustion is §360's
    /// `cur_tok=0` and ends the `\read` collection rather than falling
    /// through to whatever was being read before.
    EndReadLine,
}

/// One token-list cursor.
///
/// The four classified fields deliberately keep storage ownership, delivery
/// semantics, end-of-level handling, and diagnostic explanation independent.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TokenCursor {
    pub(crate) payload: TokenPayload,
    pub(crate) behavior: TokenBehavior,
    pub(crate) retirement: RetirementBehavior,
    pub(crate) trace: ReplayTrace,
    pub(crate) index: usize,
    pub(crate) identity: InputLevelId,
}

/// Storage owning the tokens delivered by a token-list level.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TokenPayload {
    /// Immutable semantic tokens and their parallel immutable origins.
    Stored {
        tokens: TokenListRef,
        origins: OriginListRef,
    },
    /// Tokens materialized for a bounded insertion or scanner operation.
    Transient(SharedTokenBuffer),
    /// One transient token stored directly in its input level.
    InlineTransient(RootedTracedTokenWord),
    /// One command restored by TeX's `back_input`, retaining the committed
    /// physical spelling range if it originally came from registered source.
    BackedUp(SharedBackedUpBuffer),
    /// One backed-up command stored directly in its input level.
    InlineBackedUp(RootedBackedUpToken),
    /// One already materialized macro argument, replayed literally by range.
    ArgumentRange {
        buffer: SharedTokenBuffer,
        range: MacroArgumentRange,
    },
}

impl TokenPayload {
    /// Selects inline storage for one transient token and shared storage for
    /// empty or multi-token payloads. The fixed two-token case constructs its
    /// shared slice directly from an array; longer iterators materialize the
    /// unbounded owned buffer once.
    pub(crate) fn transient(tokens: impl IntoIterator<Item = TracedTokenWord>) -> Self {
        let mut tokens = tokens.into_iter();
        let Some(first) = tokens.next() else {
            return Self::Transient(SharedTokenBuffer::default());
        };
        let Some(second) = tokens.next() else {
            return Self::InlineTransient(RootedTracedTokenWord::unowned(first));
        };
        let Some(third) = tokens.next() else {
            return Self::Transient(SharedTokenBuffer::new([first, second]));
        };
        let (lower, _) = tokens.size_hint();
        let mut shared = Vec::with_capacity(lower.saturating_add(3));
        shared.extend([first, second, third]);
        shared.extend(tokens);
        Self::Transient(SharedTokenBuffer::new(shared))
    }

    /// Selects shared structural storage for rooted transient positions.
    /// Rooted singletons deliberately do not use the raw inline form.
    pub(crate) fn transient_rooted(
        tokens: impl IntoIterator<Item = RootedTracedTokenWord>,
    ) -> Self {
        let mut tokens = tokens.into_iter();
        let Some(first) = tokens.next() else {
            return Self::Transient(SharedTokenBuffer::default());
        };
        let Some(second) = tokens.next() else {
            return Self::InlineTransient(first);
        };
        Self::Transient(SharedTokenBuffer::new_rooted(
            [first, second].into_iter().chain(tokens),
        ))
    }

    /// Selects inline storage for one backed-up command and shared storage for
    /// empty or multi-token payloads.
    pub(crate) fn backed_up(tokens: impl IntoIterator<Item = BackedUpToken>) -> Self {
        let mut tokens = tokens.into_iter();
        let Some(first) = tokens.next() else {
            return Self::BackedUp(SharedBackedUpBuffer::default());
        };
        let Some(second) = tokens.next() else {
            return Self::InlineBackedUp(RootedBackedUpToken::unowned(first));
        };
        let (lower, _) = tokens.size_hint();
        let mut shared = Vec::with_capacity(lower.saturating_add(2));
        shared.extend([first, second]);
        shared.extend(tokens);
        Self::BackedUp(SharedBackedUpBuffer::new(shared))
    }

    pub(crate) fn backed_up_rooted(tokens: impl IntoIterator<Item = RootedBackedUpToken>) -> Self {
        let mut tokens = tokens.into_iter();
        let Some(first) = tokens.next() else {
            return Self::BackedUp(SharedBackedUpBuffer::default());
        };
        let Some(second) = tokens.next() else {
            return Self::InlineBackedUp(first);
        };
        Self::BackedUp(SharedBackedUpBuffer::new_rooted(
            [first, second].into_iter().chain(tokens),
        ))
    }

    pub(crate) fn transient_words(&self) -> Option<&[TracedTokenWord]> {
        match self {
            Self::Transient(words) => Some(words.words()),
            Self::InlineTransient(_) => None,
            _ => None,
        }
    }

    pub(crate) fn transient_len(&self) -> Option<usize> {
        match self {
            Self::Transient(words) => Some(words.len()),
            Self::InlineTransient(_) => Some(1),
            _ => None,
        }
    }

    pub(crate) fn backed_up_words(&self) -> Option<&[BackedUpToken]> {
        match self {
            Self::BackedUp(words) => Some(words.words()),
            Self::InlineBackedUp(_) => None,
            _ => None,
        }
    }

    pub(crate) fn backed_up_len(&self) -> Option<usize> {
        match self {
            Self::BackedUp(words) => Some(words.words().len()),
            Self::InlineBackedUp(_) => Some(1),
            _ => None,
        }
    }

    pub(crate) fn backed_up_get(&self, index: usize) -> Option<BackedUpToken> {
        match self {
            Self::BackedUp(words) => words.get(index),
            Self::InlineBackedUp(word) => (index == 0).then(|| word.token()),
            _ => None,
        }
    }

    pub(crate) fn is_backed_up(&self) -> bool {
        matches!(self, Self::BackedUp(_) | Self::InlineBackedUp(_))
    }

    /// Prepends e-TeX aftergroup tokens, promoting inline storage when the
    /// resulting backed-up level contains multiple commands.
    pub(crate) fn prepend_backed_up(
        &mut self,
        prefix: impl IntoIterator<Item = RootedBackedUpToken>,
    ) -> Option<()> {
        let mut prefix = prefix.into_iter().collect::<Vec<_>>();
        match self {
            Self::BackedUp(words) => words.prepend(prefix),
            Self::InlineBackedUp(word) => {
                prefix.push(word.clone());
                *self = Self::backed_up_rooted(prefix);
            }
            _ => return None,
        }
        Some(())
    }

    pub(crate) fn rehome_backed_up_source(
        &mut self,
        source: tex_state::SourceId,
        byte_delta: i64,
    ) -> Option<()> {
        match self {
            Self::BackedUp(words) => words.rehome_source(source, byte_delta),
            Self::InlineBackedUp(word) => {
                if let Some(provenance) = &mut word.token.source_provenance {
                    provenance.rehome(source, byte_delta)?;
                }
                Some(())
            }
            _ => None,
        }
    }

    pub(crate) fn adopt_matching_origins(&mut self, live: &Self) -> Option<()> {
        if let (Self::InlineTransient(recorded), Self::InlineTransient(live)) = (&*self, live) {
            if recorded.word().token() != live.word().token() {
                return None;
            }
            *self = Self::InlineTransient(live.clone());
            return Some(());
        }
        if let (Self::InlineBackedUp(recorded), Self::InlineBackedUp(live)) = (&*self, live) {
            if recorded.token().spelling.token() != live.token().spelling.token()
                || recorded.token().source_provenance != live.token().source_provenance
            {
                return None;
            }
            *self = Self::InlineBackedUp(live.clone());
            return Some(());
        }
        if let (Some(recorded), Some(live_words)) = (self.transient_words(), live.transient_words())
        {
            if recorded.len() != live_words.len()
                || recorded
                    .iter()
                    .zip(live_words)
                    .any(|(recorded, live)| recorded.token() != live.token())
            {
                return None;
            }
            *self = live.clone();
            return Some(());
        }
        let (Some(recorded), Some(live_words)) = (self.backed_up_words(), live.backed_up_words())
        else {
            return None;
        };
        if recorded.len() != live_words.len()
            || recorded.iter().zip(live_words).any(|(recorded, live)| {
                recorded.spelling.token() != live.spelling.token()
                    || recorded.source_provenance != live.source_provenance
            })
        {
            return None;
        }
        *self = live.clone();
        Some(())
    }
}

/// Shared ownership of a contiguous traced-token allocation.
///
/// Cloning a cursor or snapshot retains the allocation rather than copying its
/// tokens. A macro activation and its parameter cursors may share this value.
#[derive(Debug, Eq, Hash, PartialEq)]
struct SharedTokenBufferValue {
    words: Box<[TracedTokenWord]>,
    /// Sorted, distinct structural owners. Direct, fallback, and unknown
    /// positions are represented by their packed words alone.
    roots: Box<[OriginRef]>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SharedTokenBuffer(Arc<SharedTokenBufferValue>);

#[cfg(test)]
pub(crate) struct SharedTokenBufferWeak(std::sync::Weak<SharedTokenBufferValue>);

#[cfg(test)]
impl SharedTokenBufferWeak {
    pub(crate) fn is_live(&self) -> bool {
        self.0.upgrade().is_some()
    }
}

impl Default for SharedTokenBuffer {
    fn default() -> Self {
        Self::new([])
    }
}

impl SharedTokenBuffer {
    #[cfg(test)]
    pub(crate) fn downgrade(&self) -> SharedTokenBufferWeak {
        SharedTokenBufferWeak(Arc::downgrade(&self.0))
    }

    /// Builds a rootless buffer. Arena-backed words are rejected because a
    /// raw id cannot confer ownership on its provenance atom.
    pub(crate) fn new(tokens: impl AsRef<[TracedTokenWord]>) -> Self {
        Self::new_rooted(
            tokens
                .as_ref()
                .iter()
                .copied()
                .map(RootedTracedTokenWord::unowned),
        )
    }

    pub(crate) fn new_rooted(tokens: impl IntoIterator<Item = RootedTracedTokenWord>) -> Self {
        let mut words = Vec::new();
        let mut roots = Vec::new();
        for token in tokens {
            let (word, root) = token.into_parts();
            words.push(word);
            if root.record().is_some() {
                match roots.binary_search_by_key(&root.id(), OriginRef::id) {
                    Ok(_) => {}
                    Err(index) => roots.insert(index, root),
                }
            }
        }
        Self(Arc::new(SharedTokenBufferValue {
            words: words.into_boxed_slice(),
            roots: roots.into_boxed_slice(),
        }))
    }

    /// Freezes a scanner-owned buffer by transferring its paired word and
    /// provenance-root allocations. Macro matching is the hot caller: it has
    /// already established the same sorted-root invariant while collecting
    /// arguments, so replay need not iterate, clone roots, and rebuild words.
    pub(crate) fn from_rooted_buffer(buffer: tex_state::token::RootedTracedTokenBuffer) -> Self {
        let (words, roots) = buffer.into_storage_parts();
        Self(Arc::new(SharedTokenBufferValue {
            words: words.into_boxed_slice(),
            roots: roots.into_boxed_slice(),
        }))
    }

    pub(crate) fn len(&self) -> usize {
        self.0.words.len()
    }

    pub(crate) fn get(&self, index: usize) -> Option<TracedTokenWord> {
        self.0.words.get(index).copied()
    }

    pub(crate) fn get_rooted(&self, index: usize) -> Option<RootedTracedTokenWord> {
        let word = self.get(index)?;
        let root = self
            .0
            .roots
            .binary_search_by_key(&word.origin(), OriginRef::id)
            .map_or_else(
                |_| OriginRef::direct(word.origin()),
                |index| self.0.roots[index].clone(),
            );
        Some(RootedTracedTokenWord::from_word(word, root))
    }

    pub(crate) fn words(&self) -> &[TracedTokenWord] {
        &self.0.words
    }

    pub(crate) fn rooted_words(&self) -> impl ExactSizeIterator<Item = RootedTracedTokenWord> + '_ {
        (0..self.len()).map(|index| {
            self.get_rooted(index)
                .expect("index from the exact shared-buffer length")
        })
    }

    pub(crate) fn adopt_matching_origins(&mut self, live: &Self) -> Option<()> {
        if self.0.words.len() != live.0.words.len()
            || self
                .0
                .words
                .iter()
                .zip(live.0.words.iter())
                .any(|(recorded, live)| recorded.token() != live.token())
        {
            return None;
        }
        self.0 = Arc::clone(&live.0);
        Some(())
    }
}

/// Shared ownership of commands restored by `back_input` or scanner replay.
///
/// The range is delivery metadata, separate from the token's semantic and
/// opaque-origin identity, so reusing it cannot affect fixture identities.
#[derive(Debug, Eq, Hash, PartialEq)]
struct SharedBackedUpBufferValue {
    tokens: Box<[BackedUpToken]>,
    roots: Box<[OriginRef]>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SharedBackedUpBuffer(Arc<SharedBackedUpBufferValue>);

impl Default for SharedBackedUpBuffer {
    fn default() -> Self {
        Self::new([])
    }
}

impl SharedBackedUpBuffer {
    pub(crate) fn new(tokens: impl AsRef<[BackedUpToken]>) -> Self {
        Self::new_rooted(
            tokens
                .as_ref()
                .iter()
                .copied()
                .map(RootedBackedUpToken::unowned),
        )
    }

    pub(crate) fn new_rooted(tokens: impl IntoIterator<Item = RootedBackedUpToken>) -> Self {
        let mut words = Vec::new();
        let mut roots = Vec::new();
        for rooted in tokens {
            let (token, root) = rooted.into_parts();
            words.push(token);
            if root.record().is_some() {
                match roots.binary_search_by_key(&root.id(), OriginRef::id) {
                    Ok(_) => {}
                    Err(index) => roots.insert(index, root),
                }
            }
        }
        Self(Arc::new(SharedBackedUpBufferValue {
            tokens: words.into_boxed_slice(),
            roots: roots.into_boxed_slice(),
        }))
    }

    pub(crate) fn get(&self, index: usize) -> Option<BackedUpToken> {
        self.0.tokens.get(index).copied()
    }

    pub(crate) fn get_rooted(&self, index: usize) -> Option<RootedBackedUpToken> {
        let token = self.get(index)?;
        let root = self
            .0
            .roots
            .binary_search_by_key(&token.spelling.origin(), OriginRef::id)
            .map_or_else(
                |_| OriginRef::direct(token.spelling.origin()),
                |index| self.0.roots[index].clone(),
            );
        Some(RootedBackedUpToken { token, root })
    }

    pub(crate) fn words(&self) -> &[BackedUpToken] {
        &self.0.tokens
    }

    /// Prepends tokens to e-TeX's active optimized `backed_up` list.
    pub(crate) fn prepend(&mut self, prefix: impl IntoIterator<Item = RootedBackedUpToken>) {
        let mut tokens = prefix.into_iter().collect::<Vec<_>>();
        tokens.extend((0..self.0.tokens.len()).map(|index| {
            self.get_rooted(index)
                .expect("index from exact backed-up-buffer length")
        }));
        *self = Self::new_rooted(tokens);
    }

    pub(crate) fn rehome_source(
        &mut self,
        source: tex_state::SourceId,
        byte_delta: i64,
    ) -> Option<()> {
        let mut tokens = self.0.tokens.to_vec();
        for token in &mut tokens {
            if let Some(provenance) = &mut token.source_provenance {
                provenance.rehome(source, byte_delta)?;
            }
        }
        let rooted = tokens.into_iter().enumerate().map(|(index, token)| {
            let old = self
                .get_rooted(index)
                .expect("rehome preserves backed-up-buffer length");
            RootedBackedUpToken::new(token, old.origin_ref().clone())
        });
        *self = Self::new_rooted(rooted);
        Some(())
    }

    pub(crate) fn adopt_matching_origins(&mut self, live: &Self) -> Option<()> {
        if self.0.tokens.len() != live.0.tokens.len() {
            return None;
        }
        let mut tokens = self.0.tokens.to_vec();
        for (recorded, live) in tokens.iter_mut().zip(live.0.tokens.iter()) {
            if recorded.spelling.token() != live.spelling.token()
                || recorded.source_provenance != live.source_provenance
            {
                return None;
            }
            recorded.spelling = tex_state::token::TracedTokenWord::pack(
                live.spelling.token()?,
                live.spelling.origin(),
            );
        }
        let rooted = tokens.into_iter().enumerate().map(|(index, token)| {
            RootedBackedUpToken::new(
                token,
                live.get_rooted(index)
                    .expect("matching buffers have equal lengths")
                    .origin_ref()
                    .clone(),
            )
        });
        *self = Self::new_rooted(rooted);
        Some(())
    }
}

/// One restored command plus the source range committed at its first delivery.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BackedUpToken {
    pub(crate) spelling: TracedTokenWord,
    pub(crate) source_provenance: Option<SourceProvenance>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RootedBackedUpToken {
    token: BackedUpToken,
    root: OriginRef,
}

impl RootedBackedUpToken {
    pub(crate) fn new(token: BackedUpToken, root: OriginRef) -> Self {
        assert_eq!(token.spelling.origin(), root.id());
        Self { token, root }
    }

    pub(crate) fn unowned(token: BackedUpToken) -> Self {
        let rooted = RootedTracedTokenWord::unowned(token.spelling);
        Self {
            token,
            root: rooted.into_parts().1,
        }
    }

    pub(crate) const fn token(&self) -> BackedUpToken {
        self.token
    }

    pub(crate) fn origin_ref(&self) -> &OriginRef {
        &self.root
    }

    pub(crate) fn into_parts(self) -> (BackedUpToken, OriginRef) {
        (self.token, self.root)
    }
}

/// Semantic treatment applied while a token level delivers its payload.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TokenBehavior {
    Ordinary,
    /// A TeX recovery insertion that must retire before a scanner backs its
    /// consumed token up for ordinary replay.
    Recovery,
    /// Replacement text associated with the sole activation owner.
    MacroBody(MacroActivationId),
    /// Literal replay of an already substituted macro argument.
    Parameter,
    BackedUp(BackupTreatment),
    UTemplate,
    VTemplate,
}

/// One-delivery handling attached to explicitly backed-up input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum BackupTreatment {
    Ordinary,
    SuppressExpandableControlSequence,
}

/// Action selected only when a token payload is exhausted.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RetirementBehavior {
    Pop,
    StopAtEnd,
    RetainExhaustedVTemplate,
    /// The exhausted v-template has reported its frozen `end_template`
    /// boundary. tex.web §§325/390 still refuse to drain it for stack
    /// conservation and §1131's `do_endv` still expects to find it, but
    /// §357's `end_token_list` pops it as soon as `get_next` reaches it.
    AwaitingVTemplateRetirement,
}

/// Non-semantic explanation for why a token payload is being replayed.
///
/// This value is diagnostic/provenance state. It cannot select expansion,
/// parameter substitution, backup treatment, or retirement.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ReplayTrace {
    /// TeX82 §307's `inserted` token type: the level TeX82 §323's `ins_list`
    /// installs.
    ///
    /// This is a token _type_, not a storage strategy, so it is independent of
    /// whether the payload is a fresh transient buffer (§470's `conv_toks`
    /// renders one) or an immutable stored list (§467's `ins_the_toks` shares
    /// §465's copy). Nesting it under [`ReplayTrace::Transient`] conflated the
    /// two and let §467's inserted level be installed as an ordinary stored
    /// token list.
    Inserted,
    Stored(StoredReplayReason),
    Transient(TransientReplayReason),
    MacroReplacement,
    MacroParameter {
        slot: u8,
    },
    BackedUp,
    UTemplate,
    VTemplate,
    /// tex.web §789's `begin_token_list(omit_template,v_template)`: an
    /// `\omit` entry installs the shared constant list `omit_template`
    /// instead of the column's ⟨v_j⟩ part. Both are `token_type=v_template`
    /// (§307), so this is a trace distinction only -- exactly the one the
    /// pinned observer makes with `start=omit_template` when it names a
    /// retiring level.
    OmitTemplate,
}

/// Canonical explanations for immutable stored token-list replay.
///
/// The first block is one tex.web §307 `token_type` each -- the token lists
/// TeX82 installs with `begin_token_list` and names in its own input trace.
/// The second block is Umber's own: replay levels the command state owns for
/// material tex.web reads live, which therefore have no §307 identity to
/// borrow.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum StoredReplayReason {
    /// §307 `output_text=6`.
    OutputRoutine,
    /// §307 `every_par_text=7`.
    EveryPar,
    /// §307 `every_math_text=8`.
    EveryMath,
    /// §307 `every_display_text=9`.
    EveryDisplay,
    /// §307 `every_hbox_text=10`.
    EveryHBox,
    /// §307 `every_vbox_text=11`.
    EveryVBox,
    /// §307 `every_job_text=12`.
    EveryJob,
    /// §307 `every_cr_text=13`.
    EveryCr,
    /// e-TeX §22.307's `every_eof_text`.
    EveryEof,
    /// §307 `mark_text=14`.
    Mark,
    /// §307 `write_text=15`.
    Write,
    Discretionary,
}

/// Canonical explanations for a materialized transient insertion that is not
/// TeX82 §307's `inserted` token type (which is [`ReplayTrace::Inserted`]).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TransientReplayReason {
    ExpandedTokenList,
}

#[cfg(test)]
mod tests;
