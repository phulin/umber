use std::fmt;

/// File-owned limits shared by native and WebAssembly VFS users.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VfsLimits {
    pub user_files: usize,
    pub resolved_files: usize,
    pub one_file_bytes: usize,
    pub user_bytes: usize,
    pub resolved_bytes: usize,
}

impl VfsLimits {
    pub const HARD_MAX: Self = Self {
        user_files: 4096,
        resolved_files: 4096,
        one_file_bytes: 128 * 1024 * 1024,
        user_bytes: 64 * 1024 * 1024,
        resolved_bytes: 256 * 1024 * 1024,
    };

    pub fn validate(self) -> Result<Self, VfsLimitError> {
        for (kind, attempted, hard) in [
            (
                VfsLimitKind::UserFiles,
                self.user_files,
                Self::HARD_MAX.user_files,
            ),
            (
                VfsLimitKind::ResolvedFiles,
                self.resolved_files,
                Self::HARD_MAX.resolved_files,
            ),
            (
                VfsLimitKind::OneFileBytes,
                self.one_file_bytes,
                Self::HARD_MAX.one_file_bytes,
            ),
            (
                VfsLimitKind::UserBytes,
                self.user_bytes,
                Self::HARD_MAX.user_bytes,
            ),
            (
                VfsLimitKind::ResolvedBytes,
                self.resolved_bytes,
                Self::HARD_MAX.resolved_bytes,
            ),
        ] {
            if attempted > hard {
                return Err(VfsLimitError::HardLimitExceeded {
                    kind,
                    hard,
                    attempted,
                });
            }
        }
        Ok(self)
    }

    pub fn check(&self, kind: VfsLimitKind, attempted: usize) -> Result<(), VfsLimitError> {
        let limit = match kind {
            VfsLimitKind::UserFiles => self.user_files,
            VfsLimitKind::ResolvedFiles => self.resolved_files,
            VfsLimitKind::OneFileBytes => self.one_file_bytes,
            VfsLimitKind::UserBytes => self.user_bytes,
            VfsLimitKind::ResolvedBytes => self.resolved_bytes,
        };
        if attempted > limit {
            return Err(VfsLimitError::LimitExceeded {
                kind,
                limit,
                attempted,
            });
        }
        Ok(())
    }

    pub fn checked_replacement_total(
        &self,
        kind: VfsLimitKind,
        current: usize,
        replaced: usize,
        incoming: usize,
    ) -> Result<usize, VfsLimitError> {
        let attempted = current
            .checked_sub(replaced)
            .and_then(|value| value.checked_add(incoming))
            .ok_or_else(|| VfsLimitError::LimitExceeded {
                kind,
                limit: self.limit(kind),
                attempted: usize::MAX,
            })?;
        self.check(kind, attempted)?;
        Ok(attempted)
    }

    const fn limit(&self, kind: VfsLimitKind) -> usize {
        match kind {
            VfsLimitKind::UserFiles => self.user_files,
            VfsLimitKind::ResolvedFiles => self.resolved_files,
            VfsLimitKind::OneFileBytes => self.one_file_bytes,
            VfsLimitKind::UserBytes => self.user_bytes,
            VfsLimitKind::ResolvedBytes => self.resolved_bytes,
        }
    }
}

impl Default for VfsLimits {
    fn default() -> Self {
        Self {
            user_files: 512,
            resolved_files: 512,
            one_file_bytes: 96 * 1024 * 1024,
            user_bytes: 16 * 1024 * 1024,
            resolved_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VfsLimitKind {
    UserFiles,
    ResolvedFiles,
    OneFileBytes,
    UserBytes,
    ResolvedBytes,
}

impl VfsLimitKind {
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::UserFiles => "user files",
            Self::ResolvedFiles => "resolved files",
            Self::OneFileBytes => "one file bytes",
            Self::UserBytes => "user source bytes",
            Self::ResolvedBytes => "cached file bytes",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VfsLimitError {
    HardLimitExceeded {
        kind: VfsLimitKind,
        hard: usize,
        attempted: usize,
    },
    LimitExceeded {
        kind: VfsLimitKind,
        limit: usize,
        attempted: usize,
    },
}

impl fmt::Display for VfsLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HardLimitExceeded {
                kind,
                hard,
                attempted,
            } => write!(
                f,
                "{} setting {attempted} exceeds hard ceiling {hard}",
                kind.description()
            ),
            Self::LimitExceeded {
                kind,
                limit,
                attempted,
            } => write!(
                f,
                "{} requires {attempted}, exceeding limit {limit}",
                kind.description()
            ),
        }
    }
}

impl std::error::Error for VfsLimitError {}
