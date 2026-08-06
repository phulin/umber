use std::collections::BTreeMap;
use std::fmt;

/// Policy attached to an outstanding semantic resource request.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RequestIntent {
    PrefetchHint,
    Probe,
    Required,
}

impl RequestIntent {
    #[must_use]
    pub const fn is_blocking(self) -> bool {
        matches!(self, Self::Probe | Self::Required)
    }
}

/// Canonical lifecycle state for one semantic resource key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdmissionState<V> {
    Outstanding(RequestIntent),
    Admitted(V),
    Unavailable,
}

/// A conflict in the canonical resource lifecycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdmissionError<K> {
    Unexpected(K),
    AvailabilityConflict(K),
    BindingConflict(K),
    NegativeHint(K),
}

impl<K: fmt::Debug> fmt::Display for AdmissionError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unexpected(key) => write!(
                formatter,
                "resource response for {key:?} was not authorized"
            ),
            Self::AvailabilityConflict(key) => write!(
                formatter,
                "resource {key:?} was rebound between available and unavailable"
            ),
            Self::BindingConflict(key) => write!(
                formatter,
                "resource {key:?} was rebound to different content"
            ),
            Self::NegativeHint(key) => write!(
                formatter,
                "prefetch hint {key:?} cannot create an unavailable binding"
            ),
        }
    }
}

impl<K: fmt::Debug> std::error::Error for AdmissionError<K> {}

/// Ordered immutable bindings plus candidate-local response authorizations.
///
/// Starting another batch drops only outstanding authorizations. Positive and
/// authoritative-negative bindings are immutable session history.
#[derive(Clone, Debug)]
pub struct ResourceLifecycle<K, V> {
    states: BTreeMap<K, AdmissionState<V>>,
}

impl<K, V> Default for ResourceLifecycle<K, V> {
    fn default() -> Self {
        Self {
            states: BTreeMap::new(),
        }
    }
}

impl<K: Clone + Ord, V: Eq> ResourceLifecycle<K, V> {
    pub fn begin_batch(
        &mut self,
        required: impl IntoIterator<Item = K>,
        probes: impl IntoIterator<Item = K>,
        hints: impl IntoIterator<Item = K>,
    ) {
        self.cancel_outstanding();
        for (intent, keys) in [
            (
                RequestIntent::PrefetchHint,
                hints.into_iter().collect::<Vec<_>>(),
            ),
            (RequestIntent::Probe, probes.into_iter().collect()),
            (RequestIntent::Required, required.into_iter().collect()),
        ] {
            for key in keys {
                match self.states.entry(key) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(AdmissionState::Outstanding(intent));
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        if let AdmissionState::Outstanding(existing) = entry.get()
                            && intent > *existing
                        {
                            entry.insert(AdmissionState::Outstanding(intent));
                        }
                    }
                }
            }
        }
    }

    pub fn admit(&mut self, key: K, value: V) -> Result<bool, AdmissionError<K>> {
        match self.states.get(&key) {
            Some(AdmissionState::Outstanding(_)) => {
                self.states.insert(key, AdmissionState::Admitted(value));
                Ok(true)
            }
            Some(AdmissionState::Admitted(existing)) if existing == &value => Ok(false),
            Some(AdmissionState::Admitted(_)) => Err(AdmissionError::BindingConflict(key)),
            Some(AdmissionState::Unavailable) => Err(AdmissionError::AvailabilityConflict(key)),
            None => Err(AdmissionError::Unexpected(key)),
        }
    }

    pub fn admit_unavailable(&mut self, key: K) -> Result<bool, AdmissionError<K>> {
        match self.states.get(&key) {
            Some(AdmissionState::Outstanding(intent)) if intent.is_blocking() => {
                self.states.insert(key, AdmissionState::Unavailable);
                Ok(true)
            }
            Some(AdmissionState::Outstanding(_)) => Err(AdmissionError::NegativeHint(key)),
            Some(AdmissionState::Unavailable) => Ok(false),
            Some(AdmissionState::Admitted(_)) => Err(AdmissionError::AvailabilityConflict(key)),
            None => Err(AdmissionError::Unexpected(key)),
        }
    }

    /// Installs an already-validated immutable binding while restoring a
    /// retained session or constructing a parent multipass capability.
    pub fn restore(&mut self, key: K, value: V) -> Result<bool, AdmissionError<K>> {
        match self.states.get(&key) {
            Some(AdmissionState::Admitted(existing)) if existing == &value => Ok(false),
            Some(AdmissionState::Admitted(_)) => Err(AdmissionError::BindingConflict(key)),
            Some(AdmissionState::Unavailable) => Err(AdmissionError::AvailabilityConflict(key)),
            Some(AdmissionState::Outstanding(_)) | None => {
                self.states.insert(key, AdmissionState::Admitted(value));
                Ok(true)
            }
        }
    }

    #[must_use]
    pub fn admitted(&self, key: &K) -> Option<&V> {
        match self.states.get(key) {
            Some(AdmissionState::Admitted(value)) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_unavailable(&self, key: &K) -> bool {
        matches!(self.states.get(key), Some(AdmissionState::Unavailable))
    }

    #[must_use]
    pub fn is_bound(&self, key: &K) -> bool {
        matches!(
            self.states.get(key),
            Some(AdmissionState::Admitted(_) | AdmissionState::Unavailable)
        )
    }

    #[must_use]
    pub fn is_outstanding(&self, key: &K) -> bool {
        matches!(self.states.get(key), Some(AdmissionState::Outstanding(_)))
    }

    pub fn outstanding(&self) -> impl Iterator<Item = (&K, RequestIntent)> {
        self.states.iter().filter_map(|(key, state)| match state {
            AdmissionState::Outstanding(intent) => Some((key, *intent)),
            AdmissionState::Admitted(_) | AdmissionState::Unavailable => None,
        })
    }

    pub fn admitted_entries(&self) -> impl Iterator<Item = (&K, &V)> {
        self.states.iter().filter_map(|(key, state)| match state {
            AdmissionState::Admitted(value) => Some((key, value)),
            AdmissionState::Outstanding(_) | AdmissionState::Unavailable => None,
        })
    }

    pub fn unavailable_keys(&self) -> impl Iterator<Item = &K> {
        self.states
            .iter()
            .filter_map(|(key, state)| matches!(state, AdmissionState::Unavailable).then_some(key))
    }

    #[must_use]
    pub fn binding_count(&self) -> usize {
        self.states
            .values()
            .filter(|state| {
                matches!(
                    state,
                    AdmissionState::Admitted(_) | AdmissionState::Unavailable
                )
            })
            .count()
    }

    pub fn cancel_outstanding(&mut self) {
        self.states
            .retain(|_, state| !matches!(state, AdmissionState::Outstanding(_)));
    }

    pub fn clear(&mut self) {
        self.states.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_promotes_orders_and_preserves_bindings_across_batches() {
        let mut lifecycle = ResourceLifecycle::default();
        lifecycle.begin_batch(["required"], ["promoted", "probe"], ["promoted", "hint"]);
        assert_eq!(
            lifecycle.outstanding().collect::<Vec<_>>(),
            vec![
                (&"hint", RequestIntent::PrefetchHint),
                (&"probe", RequestIntent::Probe),
                (&"promoted", RequestIntent::Probe),
                (&"required", RequestIntent::Required),
            ]
        );
        assert!(lifecycle.admit("required", 7).expect("admit"));
        assert!(lifecycle.admit_unavailable("probe").expect("negative"));
        lifecycle.begin_batch(["next"], [], []);
        assert_eq!(lifecycle.admitted(&"required"), Some(&7));
        assert!(lifecycle.is_unavailable(&"probe"));
        assert!(!lifecycle.is_outstanding(&"hint"));
        assert_eq!(
            lifecycle.admit("hint", 9),
            Err(AdmissionError::Unexpected("hint"))
        );
    }

    #[test]
    fn lifecycle_rejects_conflicts_unexpected_and_negative_hints() {
        let mut lifecycle = ResourceLifecycle::default();
        lifecycle.begin_batch([], [], ["hint"]);
        assert_eq!(
            lifecycle.admit_unavailable("hint"),
            Err(AdmissionError::NegativeHint("hint"))
        );
        assert_eq!(
            lifecycle.admit("other", 1),
            Err(AdmissionError::Unexpected("other"))
        );
        lifecycle.begin_batch(["key"], [], []);
        lifecycle.admit("key", 1).expect("initial binding");
        assert!(!lifecycle.admit("key", 1).expect("duplicate"));
        assert_eq!(
            lifecycle.admit("key", 2),
            Err(AdmissionError::BindingConflict("key"))
        );
    }
}
