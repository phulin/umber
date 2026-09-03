//! Opaque handles retained by storage families outside the deleted runtime-value
//! ownership substrate.

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u32);

        impl $name {
            #[allow(dead_code)]
            #[allow(unused_comparisons)]
            pub(crate) const fn new(raw: u32) -> Self {
                Self(raw)
            }

            /// Creates a placeholder id for tests that cover raw Env storage.
            #[cfg(any(test, feature = "testing"))]
            #[must_use]
            pub const fn testing_new(raw: u32) -> Self {
                Self(raw)
            }

            #[must_use]
            pub const fn raw(self) -> u32 {
                self.0
            }
        }
    };
}

opaque_id!(SnapshotId);

macro_rules! semantic_id {
    ($name:ident, $namespace:expr, $builtin_slots:expr) => {
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(crate::identity::HandleIdentity);

        #[allow(dead_code, unused_comparisons)]
        impl $name {
            pub(crate) const fn new(raw: u32) -> Self {
                if raw < $builtin_slots {
                    Self(crate::identity::HandleIdentity::builtin(raw))
                } else {
                    Self(crate::identity::HandleIdentity::reserved(
                        $namespace,
                        core::num::NonZeroU32::MIN,
                        raw,
                    ))
                }
            }

            pub(crate) const fn from_identity(identity: crate::identity::HandleIdentity) -> Self {
                Self(identity)
            }

            pub(crate) const fn builtin(slot: u32) -> Self {
                Self(crate::identity::HandleIdentity::builtin(slot))
            }

            pub(crate) const fn identity(self) -> crate::identity::HandleIdentity {
                self.0
            }

            pub(crate) const fn from_words(words: [u32; 4]) -> Option<Self> {
                match crate::identity::HandleIdentity::from_words(words) {
                    Some(identity) => Some(Self(identity)),
                    None => None,
                }
            }

            pub(crate) const fn words(self) -> [u32; 4] {
                self.0.words()
            }

            /// Creates a placeholder id for tests that cover compact stored words.
            #[cfg(any(test, feature = "testing"))]
            #[must_use]
            pub const fn testing_new(raw: u32) -> Self {
                Self::new(raw)
            }

            /// Creates a test-only identity with an explicit runtime owner.
            #[cfg(feature = "testing")]
            #[must_use]
            pub fn testing_from_words(words: [u32; 4]) -> Option<Self> {
                crate::identity::HandleIdentity::from_words(words).map(Self)
            }

            /// Returns the dense store slot used by semantic DTOs and packed words.
            #[must_use]
            pub const fn raw(self) -> u32 {
                self.0.slot()
            }
        }

        // Semantic projections use the dense slot, never the rollback owner
        // namespace/generation embedded in the live capability.
        impl core::hash::Hash for $name {
            fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
                core::hash::Hash::hash(&self.raw(), state);
            }
        }
    };
}

semantic_id!(FontId, 13, 1);
