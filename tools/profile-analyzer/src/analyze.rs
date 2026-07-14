use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::model::{Library, Profile, Thread};
use crate::symbols::Symbolizer;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct Frame {
    pub(crate) library: String,
    pub(crate) function: String,
    pub(crate) raw: bool,
}

#[derive(Clone, Debug)]
struct Sample {
    weight: f64,
    frames: Vec<Frame>,
}

#[derive(Clone, Debug)]
pub struct AnalysisOptions {
    pub thread_filter: Option<String>,
    pub app_filter: Option<String>,
    pub subtree: Option<String>,
    pub top: usize,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub total_weight: f64,
    pub selected_weight: f64,
    pub selected_percent: f64,
    pub threads: Vec<String>,
    pub application_library: String,
    pub unresolved_frames: usize,
    pub subtree: Option<String>,
    pub self_time: Vec<Entry>,
    pub inclusive_time: Vec<Entry>,
    pub immediate_callees: Vec<Entry>,
    pub immediate_callers: Vec<Entry>,
    pub application_callers: Vec<Entry>,
    pub runtime_by_library: Vec<Entry>,
    pub runtime_callers: Vec<Entry>,
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub library: String,
    pub function: String,
    pub weight: f64,
    pub percent_selected: f64,
    pub percent_total: f64,
}

pub(crate) fn analyze(
    profile: &Profile,
    symbolizer: Option<&Symbolizer>,
    options: &AnalysisOptions,
) -> Result<Report, String> {
    if options.top == 0 {
        return Err("--top must be greater than zero".to_owned());
    }
    let selected_threads = profile
        .threads
        .iter()
        .filter(|thread| {
            options.thread_filter.as_ref().is_none_or(|pattern| {
                thread.name.contains(pattern) || thread.process_name.contains(pattern)
            })
        })
        .collect::<Vec<_>>();
    if selected_threads.is_empty() {
        return Err("no profile threads matched the requested filter".to_owned());
    }
    let mut samples = Vec::new();
    let mut unresolved_frames = 0;
    for thread in &selected_threads {
        reconstruct_thread(
            profile,
            thread,
            symbolizer,
            &mut samples,
            &mut unresolved_frames,
        )?;
    }
    let total_weight = samples.iter().map(|sample| sample.weight).sum::<f64>();
    if total_weight == 0.0 {
        return Err("selected threads contain no weighted samples".to_owned());
    }
    let app = detect_application(
        profile,
        &selected_threads,
        &samples,
        options.app_filter.as_deref(),
    )?;

    let mut self_counts = BTreeMap::new();
    let mut inclusive_counts = BTreeMap::new();
    let mut immediate_counts = BTreeMap::new();
    let mut caller_counts = BTreeMap::new();
    let mut application_caller_counts = BTreeMap::new();
    let mut runtime_libraries = BTreeMap::new();
    let mut runtime_callers = BTreeMap::new();
    let mut selected_weight = 0.0;
    for sample in &samples {
        let Some(limit) = subtree_limit(sample, options.subtree.as_deref()) else {
            continue;
        };
        selected_weight += sample.weight;
        let selected = &sample.frames[..limit];
        if let Some(leaf) = selected.first() {
            *self_counts.entry(leaf.clone()).or_insert(0.0) += sample.weight;
        }
        let mut seen = BTreeSet::new();
        for frame in selected {
            if seen.insert(frame.clone()) {
                *inclusive_counts.entry(frame.clone()).or_insert(0.0) += sample.weight;
            }
        }
        if options.subtree.is_some() {
            let immediate = selected
                .get(selected.len().saturating_sub(2))
                .or_else(|| selected.last());
            if let Some(immediate) = immediate {
                *immediate_counts.entry(immediate.clone()).or_insert(0.0) += sample.weight;
            }
            if let Some(caller) = sample.frames.get(limit) {
                *caller_counts.entry(caller.clone()).or_insert(0.0) += sample.weight;
            }
            if let Some(caller) = sample
                .frames
                .iter()
                .skip(limit)
                .find(|frame| frame.library == app && !is_runtime_function(&frame.function))
            {
                *application_caller_counts
                    .entry(caller.clone())
                    .or_insert(0.0) += sample.weight;
            }
        }
        if let Some(leaf) = selected.first()
            && leaf.library != app
        {
            *runtime_libraries.entry(leaf.library.clone()).or_insert(0.0) += sample.weight;
            if let Some(caller) = selected.iter().skip(1).find(|frame| frame.library == app) {
                *runtime_callers.entry(caller.clone()).or_insert(0.0) += sample.weight;
            }
        }
    }
    if selected_weight == 0.0 {
        return Err(format!(
            "subtree {:?} matched no sampled stacks",
            options.subtree.as_deref().unwrap_or_default()
        ));
    }

    Ok(Report {
        total_weight,
        selected_weight,
        selected_percent: percent(selected_weight, total_weight),
        threads: selected_threads
            .iter()
            .map(|thread| thread.name.clone())
            .collect(),
        application_library: app,
        unresolved_frames,
        subtree: options.subtree.clone(),
        self_time: entries(self_counts, selected_weight, total_weight, options.top),
        inclusive_time: entries(inclusive_counts, selected_weight, total_weight, options.top),
        immediate_callees: entries(immediate_counts, selected_weight, total_weight, options.top),
        immediate_callers: entries(caller_counts, selected_weight, total_weight, options.top),
        application_callers: entries(
            application_caller_counts,
            selected_weight,
            total_weight,
            options.top,
        ),
        runtime_by_library: library_entries(
            runtime_libraries,
            selected_weight,
            total_weight,
            options.top,
        ),
        runtime_callers: entries(runtime_callers, selected_weight, total_weight, options.top),
    })
}

fn reconstruct_thread(
    profile: &Profile,
    thread: &Thread,
    symbolizer: Option<&Symbolizer>,
    out: &mut Vec<Sample>,
    unresolved_frames: &mut usize,
) -> Result<(), String> {
    for (sample_index, stack) in thread.samples.stack.iter().enumerate() {
        let Some(mut stack) = *stack else {
            continue;
        };
        let weight = thread
            .samples
            .weight
            .as_ref()
            .and_then(|weights| weights.get(sample_index))
            .and_then(|weight| *weight)
            .unwrap_or(1.0);
        let mut frames = Vec::new();
        let mut depth = 0;
        loop {
            depth += 1;
            if depth > thread.stack_table.frame.len() {
                return Err(format!("stack cycle in thread {:?}", thread.name));
            }
            let frame_index = *thread
                .stack_table
                .frame
                .get(stack)
                .ok_or_else(|| format!("stack frame index {stack} is out of range"))?;
            append_frame(
                profile,
                thread,
                frame_index,
                symbolizer,
                &mut frames,
                unresolved_frames,
            )?;
            match thread.stack_table.prefix.get(stack).copied().flatten() {
                Some(prefix) => stack = prefix,
                None => break,
            }
        }
        out.push(Sample { weight, frames });
    }
    Ok(())
}

fn append_frame(
    profile: &Profile,
    thread: &Thread,
    frame_index: usize,
    symbolizer: Option<&Symbolizer>,
    frames: &mut Vec<Frame>,
    unresolved_frames: &mut usize,
) -> Result<(), String> {
    let function_index = *thread
        .frame_table
        .func
        .get(frame_index)
        .ok_or_else(|| format!("frame index {frame_index} is out of range"))?;
    let name_index = *thread
        .func_table
        .name
        .get(function_index)
        .ok_or_else(|| format!("function index {function_index} is out of range"))?;
    let raw_name = thread
        .string_array
        .get(name_index)
        .ok_or_else(|| format!("string index {name_index} is out of range"))?;
    let library = function_library(profile, thread, function_index);
    let address = thread
        .frame_table
        .address
        .get(frame_index)
        .copied()
        .flatten();
    let resolved = library
        .and_then(|library| address.and_then(|address| symbolizer?.resolve(library, address)));
    let library_name = library.map_or("unknown", |library| library.name.as_str());
    if let Some(resolved) = resolved {
        for function in resolved {
            push_distinct(
                frames,
                Frame {
                    library: library_name.to_owned(),
                    function: normalize_function(&function),
                    raw: false,
                },
            );
        }
    } else {
        let raw = raw_name.starts_with("0x");
        *unresolved_frames += usize::from(raw);
        push_distinct(
            frames,
            Frame {
                library: library_name.to_owned(),
                function: normalize_function(raw_name),
                raw,
            },
        );
    }
    Ok(())
}

fn function_library<'a>(
    profile: &'a Profile,
    thread: &Thread,
    function: usize,
) -> Option<&'a Library> {
    let resource = thread
        .func_table
        .resource
        .get(function)
        .copied()
        .flatten()?;
    let library = thread.resource_table.lib.get(resource).copied().flatten()?;
    profile.libs.get(library)
}

fn push_distinct(frames: &mut Vec<Frame>, frame: Frame) {
    if frames.last() != Some(&frame) {
        frames.push(frame);
    }
}

fn normalize_function(name: &str) -> String {
    let mut name = name.split(" (in ").next().unwrap_or(name).to_owned();
    if let Some(index) = name.rfind("::h") {
        let hash = &name[index + 3..];
        if hash.len() >= 16 && hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
            name.truncate(index);
        }
    }
    name
}

fn detect_application(
    profile: &Profile,
    threads: &[&Thread],
    samples: &[Sample],
    filter: Option<&str>,
) -> Result<String, String> {
    if let Some(filter) = filter {
        return profile
            .libs
            .iter()
            .find(|library| library.name.contains(filter))
            .map(|library| library.name.clone())
            .ok_or_else(|| format!("no application library matched {filter:?}"));
    }
    for thread in threads {
        if let Some(library) = profile
            .libs
            .iter()
            .find(|library| library.name == thread.process_name)
        {
            return Ok(library.name.clone());
        }
    }
    let mut counts = BTreeMap::new();
    for sample in samples {
        for frame in &sample.frames {
            if !is_runtime_library(&frame.library) {
                *counts.entry(frame.library.clone()).or_insert(0.0) += sample.weight;
            }
        }
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(library, _)| library)
        .ok_or_else(|| "could not infer the application library; pass --app".to_owned())
}

fn is_runtime_library(library: &str) -> bool {
    library == "unknown"
        || library == "dyld"
        || library == "libdyld.dylib"
        || library.starts_with("libsystem_")
}

fn is_runtime_function(function: &str) -> bool {
    ["alloc::", "core::", "std::", "__rustc::"]
        .iter()
        .any(|prefix| function.starts_with(prefix))
}

fn subtree_limit(sample: &Sample, subtree: Option<&str>) -> Option<usize> {
    match subtree {
        None => Some(sample.frames.len()),
        Some(pattern) => sample
            .frames
            .iter()
            .rposition(|frame| frame.function.contains(pattern))
            .map(|index| index + 1),
    }
}

fn entries(counts: BTreeMap<Frame, f64>, selected: f64, total: f64, top: usize) -> Vec<Entry> {
    let mut counts = counts.into_iter().collect::<Vec<_>>();
    counts.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    counts
        .into_iter()
        .take(top)
        .map(|(frame, weight)| Entry {
            library: frame.library,
            function: frame.function,
            weight,
            percent_selected: percent(weight, selected),
            percent_total: percent(weight, total),
        })
        .collect()
}

fn library_entries(
    counts: BTreeMap<String, f64>,
    selected: f64,
    total: f64,
    top: usize,
) -> Vec<Entry> {
    entries(
        counts
            .into_iter()
            .map(|(library, weight)| {
                (
                    Frame {
                        library: library.clone(),
                        function: library,
                        raw: false,
                    },
                    weight,
                )
            })
            .collect(),
        selected,
        total,
        top,
    )
}

fn percent(value: f64, total: f64) -> f64 {
    100.0 * value / total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FrameTable, FuncTable, ResourceTable, Samples, StackTable};

    fn synthetic_profile() -> Profile {
        Profile {
            libs: vec![
                Library {
                    name: "app".into(),
                    debug_name: "app".into(),
                    code_id: None,
                },
                Library {
                    name: "libsystem_malloc.dylib".into(),
                    debug_name: "malloc".into(),
                    code_id: None,
                },
            ],
            threads: vec![Thread {
                name: "worker".into(),
                process_name: "app".into(),
                string_array: vec!["root", "subtree", "leaf", "malloc"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                resource_table: ResourceTable {
                    lib: vec![Some(0), Some(1)],
                },
                func_table: FuncTable {
                    name: vec![0, 1, 2, 3],
                    resource: vec![Some(0), Some(0), Some(0), Some(1)],
                },
                frame_table: FrameTable {
                    address: vec![None; 4],
                    func: vec![0, 1, 2, 3],
                },
                // leaf -> subtree -> subtree(recursive) -> root; malloc -> subtree -> root
                stack_table: StackTable {
                    frame: vec![0, 1, 1, 2, 3, 1],
                    prefix: vec![None, Some(0), Some(1), Some(2), Some(5), Some(0)],
                },
                samples: Samples {
                    stack: vec![Some(3), Some(4)],
                    weight: Some(vec![Some(2.0), Some(3.0)]),
                },
            }],
        }
    }

    #[test]
    fn subtree_weights_recursion_once_and_attributes_runtime_caller() {
        let report = analyze(
            &synthetic_profile(),
            None,
            &AnalysisOptions {
                thread_filter: None,
                app_filter: None,
                subtree: Some("subtree".into()),
                top: 10,
            },
        )
        .expect("analyze synthetic profile");
        assert_eq!(report.total_weight, 5.0);
        assert_eq!(report.selected_weight, 5.0);
        let subtree = report
            .inclusive_time
            .iter()
            .find(|entry| entry.function == "subtree")
            .expect("subtree entry");
        assert_eq!(subtree.weight, 5.0, "recursive frame counted once");
        assert_eq!(report.runtime_callers[0].function, "subtree");
        assert_eq!(report.runtime_callers[0].weight, 3.0);
        assert_eq!(report.immediate_callers[0].function, "root");
        assert_eq!(report.immediate_callers[0].weight, 5.0);
        assert_eq!(report.application_callers[0].function, "root");
        assert_eq!(report.application_callers[0].weight, 5.0);
    }

    #[test]
    fn thread_filter_rejects_missing_thread() {
        let error = analyze(
            &synthetic_profile(),
            None,
            &AnalysisOptions {
                thread_filter: Some("missing".into()),
                app_filter: None,
                subtree: None,
                top: 10,
            },
        )
        .expect_err("missing thread should fail");
        assert!(error.contains("no profile threads"));
    }
}
