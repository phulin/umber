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

/// Ordered host-side policy for resolving TFM font files.
///
/// Area-less font names probe the principal input's directory first and then
/// the configured TeX font areas. Names with an explicit area are used as
/// written, matching TeX82's `read_font_info` split between `aire` and the
/// configured `TEX_font_area`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TexFontSearchPath {
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
    pub fn read<C: InputReadState + ?Sized>(
        &self,
        input: &mut C,
        name: &str,
    ) -> Result<FileContent, String> {
        let name = Path::new(name);
        let requested = with_default_extension(name, "tex");
        let mut candidates = search_candidates(&self.user_area, &self.system_areas, &requested);
        if name.extension().is_some_and(|extension| extension != "tex") {
            let fallback = append_extension(name, "tex");
            for candidate in search_candidates(&self.user_area, &self.system_areas, &fallback) {
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
        read_first(input, candidates)
    }

    /// Emulates pdfTeX's restricted `|kpsewhich NAME` pipe without launching
    /// a process. The existing deterministic search policy resolves `NAME`,
    /// and the generated input consists only of that resolved path.
    pub(crate) fn read_restricted_pipe<C: InputReadState + ?Sized>(
        &self,
        input: &mut C,
        name: &str,
    ) -> Option<Result<String, String>> {
        let command = name.trim();
        let requested = command.strip_prefix("|kpsewhich ")?;
        if requested.is_empty() || requested.chars().any(char::is_whitespace) {
            return Some(Err(
                "restricted kpsewhich pipe requires one TeX filename".to_owned()
            ));
        }
        Some(
            self.read(input, requested)
                .map(|content| format!("{}\n", content.path().display())),
        )
    }
}

impl TexFontSearchPath {
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

    /// Resolves and records a TFM through the narrow World-backed input
    /// capability. Failed probes are not input records; the successful read is.
    pub fn read<C: InputReadState + ?Sized>(
        &self,
        input: &mut C,
        path: &Path,
    ) -> Result<FileContent, String> {
        let requested = with_default_extension(path, "tfm");
        let candidates = font_candidates(&self.user_area, &self.system_areas, &requested);
        read_first(input, candidates)
    }
}

fn with_default_extension(path: &Path, extension: &str) -> PathBuf {
    if path.extension().is_some() {
        return path.to_owned();
    }
    let mut path = path.to_owned();
    path.set_extension(extension);
    path
}

fn append_extension(path: &Path, extension: &str) -> PathBuf {
    let mut path = path.as_os_str().to_os_string();
    path.push(".");
    path.push(extension);
    path.into()
}

fn search_candidates(user_area: &Path, system_areas: &[PathBuf], requested: &Path) -> Vec<PathBuf> {
    if requested.is_absolute() {
        return vec![requested.to_owned()];
    }

    let user_path = user_area.join(requested);
    if has_explicit_area(requested) {
        return vec![user_path];
    }

    let mut candidates = Vec::with_capacity(1 + system_areas.len());
    candidates.push(user_path);
    for area in system_areas {
        let candidate = area.join(requested);
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn font_candidates(user_area: &Path, system_areas: &[PathBuf], requested: &Path) -> Vec<PathBuf> {
    if requested.is_absolute() || has_explicit_area(requested) {
        return vec![requested.to_owned()];
    }
    search_candidates(user_area, system_areas, requested)
}

fn read_first<C: InputReadState + ?Sized>(
    input: &mut C,
    candidates: Vec<PathBuf>,
) -> Result<FileContent, String> {
    let mut failures = Vec::with_capacity(candidates.len());
    for path in candidates {
        match input.read_input_file(&path) {
            Ok(content) => return Ok(content),
            Err(err) => failures.push(format!("{} ({err})", path.display())),
        }
    }
    Err(failures.join("; "))
}

fn has_explicit_area(path: &Path) -> bool {
    path.parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
}

#[cfg(test)]
mod tests;
