use std::path::{Path, PathBuf};

use tex_state::{FileContent, InputReadState};

/// Ordered host-side policy for resolving TeX `\input` files.
///
/// TeX first tries the user area and, for names without an explicit area,
/// retries a configured system area. Umber models the user area as the
/// principal input's directory and accepts an ordered list of system areas
/// from the driver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TexInputSearchPath {
    user_area: PathBuf,
    system_areas: Vec<PathBuf>,
}

impl TexInputSearchPath {
    #[must_use]
    pub fn new(
        user_area: impl Into<PathBuf>,
        system_areas: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        Self {
            user_area: user_area.into(),
            system_areas: system_areas.into_iter().collect(),
        }
    }

    /// Resolves and records an input through the narrow World-backed input
    /// capability. Failed probes are not input records; the successful read is.
    pub fn read<C: InputReadState>(
        &self,
        input: &mut C,
        name: &str,
    ) -> Result<FileContent, String> {
        let requested = with_default_extension(Path::new(name));
        let candidates = self.candidates(&requested);
        let mut failures = Vec::with_capacity(candidates.len());
        for path in candidates {
            match input.read_input_file(&path) {
                Ok(content) => return Ok(content),
                Err(err) => failures.push(format!("{} ({err})", path.display())),
            }
        }
        Err(failures.join("; "))
    }

    fn candidates(&self, requested: &Path) -> Vec<PathBuf> {
        if requested.is_absolute() {
            return vec![requested.to_owned()];
        }

        let user_path = self.user_area.join(requested);
        if has_explicit_area(requested) {
            return vec![user_path];
        }

        let mut candidates = Vec::with_capacity(1 + self.system_areas.len());
        candidates.push(user_path);
        for area in &self.system_areas {
            let candidate = area.join(requested);
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        candidates
    }
}

fn with_default_extension(path: &Path) -> PathBuf {
    if path.extension().is_some() {
        return path.to_owned();
    }
    let mut path = path.to_owned();
    path.set_extension("tex");
    path
}

fn has_explicit_area(path: &Path) -> bool {
    path.parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
}

#[cfg(test)]
mod tests;
