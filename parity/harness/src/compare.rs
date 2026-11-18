//! Comparator rules for one `check` (plan section 1.2).
use crate::expo::{Exposition, Family, Labels};
use std::collections::BTreeSet;

pub const BUILD_INFO: &str = "kubex_gpu_process_exporter_build_info";
const RUST_BUILD_LABELS: [&str; 6] = ["arch", "branch", "os", "revision", "rustversion", "version"];
const SHARED_BUILD_LABELS: [&str; 3] = ["version", "revision", "branch"];
const IGNORED_PREFIXES: [&str; 2] = ["go_", "promhttp_"];
const PROCESS_PREFIX: &str = "process_";
/// `process_*` families both implementations expose; Rust adds `process_threads`.
const SHARED_PROCESS: [&str; 6] = [
    "process_cpu_seconds_total",
    "process_open_fds",
    "process_max_fds",
    "process_virtual_memory_bytes",
    "process_resident_memory_bytes",
    "process_start_time_seconds",
];
const RUST_PROCESS: &str = "process_threads";
const COUNTER_TOLERANCE: f64 = 1e-9;
/// Families where Rust deliberately fixes Go (CONTRACT.md, accepted deviations). Against a Go reference they are
/// not diffed; checks assert their Rust values through `rust_values`.
const FIXED_IN_RUST: [&str; 2] = [
    "kubex_gpu_container_ofa_utilization_percent_seconds_total",
    "kubex_gpu_container_jpg_utilization_percent_seconds_total",
];

/// One difference between two expositions.
pub enum Diff {
    /// Families, HELP, TYPE, series sets, or build info.
    Structure(String),
    /// A sample present in both with different values.
    Value {
        key: (String, Labels),
        counter: bool,
        message: String,
    },
}

impl std::fmt::Display for Diff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Diff::Structure(m) | Diff::Value { message: m, .. } => f.write_str(m),
        }
    }
}

#[derive(Default)]
struct Diffs(Vec<Diff>);

impl Diffs {
    fn push(&mut self, message: String) {
        self.0.push(Diff::Structure(message));
    }
}

/// Returns human-readable differences; empty means equal. `golden` skips build info values, which depend on
/// the build that recorded the reference.
pub fn compare(reference: &Exposition, candidate: &Exposition, golden: bool) -> Vec<String> {
    diff(reference, candidate, golden)
        .iter()
        .map(ToString::to_string)
        .collect()
}

pub fn diff(reference: &Exposition, candidate: &Exposition, golden: bool) -> Vec<Diff> {
    let mut diffs = Diffs::default();
    let rust_vs_go = is_rust(candidate) && !is_rust(reference);
    let names: BTreeSet<&String> = reference
        .families
        .keys()
        .chain(candidate.families.keys())
        .collect();
    for name in names {
        if IGNORED_PREFIXES.iter().any(|p| name.starts_with(p))
            || name.starts_with(PROCESS_PREFIX)
            || rust_vs_go && FIXED_IN_RUST.contains(&name.as_str())
        {
            continue;
        }
        match (reference.families.get(name), candidate.families.get(name)) {
            (Some(r), Some(c)) if name == BUILD_INFO => {
                compare_build_info(r, c, golden, &mut diffs)
            }
            (Some(r), Some(c)) => compare_family(name, r, c, &mut diffs),
            (Some(_), None) => diffs.push(format!("{name}: missing in candidate")),
            (None, Some(_)) => diffs.push(format!("{name}: unexpected in candidate")),
            (None, None) => unreachable!(),
        }
    }
    let mut process: Vec<&str> = SHARED_PROCESS.to_vec();
    if is_rust(candidate) {
        process.push(RUST_PROCESS);
    }
    for name in process {
        if !candidate.families.contains_key(name) {
            diffs.push(format!("{name}: missing in candidate"));
        }
    }
    diffs.0
}

pub fn is_rust(expo: &Exposition) -> bool {
    build_labels(expo.families.get(BUILD_INFO)).is_some_and(|l| l.contains_key("rustversion"))
}

fn build_labels(family: Option<&Family>) -> Option<&Labels> {
    family?.samples.keys().next().map(|(_, l)| l)
}

fn compare_build_info(reference: &Family, candidate: &Family, golden: bool, diffs: &mut Diffs) {
    let (Some(r), Some(c)) = (build_labels(Some(reference)), build_labels(Some(candidate))) else {
        diffs.push(format!("{BUILD_INFO}: no sample"));
        return;
    };
    let same_labels = r.keys().eq(c.keys());
    if !same_labels && !c.keys().map(String::as_str).eq(RUST_BUILD_LABELS) {
        diffs.push(format!(
            "{BUILD_INFO}: label names {:?}, want {RUST_BUILD_LABELS:?}",
            c.keys()
        ));
    }
    // Goldens were recorded by another build, so only live runs compare label values.
    if !golden {
        let compared: Vec<&str> = if same_labels {
            r.keys().map(String::as_str).collect()
        } else {
            SHARED_BUILD_LABELS.to_vec()
        };
        for label in compared {
            if r.get(label) != c.get(label) {
                diffs.push(format!(
                    "{BUILD_INFO}: {label} {:?} != {:?}",
                    r.get(label),
                    c.get(label)
                ));
            }
        }
    }
    if reference.kind != candidate.kind || candidate.samples.values().any(|v| *v != 1.0) {
        diffs.push(format!("{BUILD_INFO}: want one gauge sample with value 1"));
    }
}

fn compare_family(name: &str, reference: &Family, candidate: &Family, diffs: &mut Diffs) {
    if reference.help != candidate.help {
        diffs.push(format!(
            "{name}: HELP {:?} != {:?}",
            reference.help, candidate.help
        ));
    }
    if reference.kind != candidate.kind {
        diffs.push(format!(
            "{name}: TYPE {:?} != {:?}",
            reference.kind, candidate.kind
        ));
    }
    let counter = reference.kind.as_deref() == Some("counter");
    for (key, r) in &reference.samples {
        match candidate.samples.get(key) {
            None => diffs.push(format!(
                "{}{:?}: missing in candidate (reference {r})",
                key.0, key.1
            )),
            Some(c) if !values_equal(*r, *c, counter) => diffs.0.push(Diff::Value {
                key: key.clone(),
                counter,
                message: format!("{}{:?}: reference {r} != candidate {c}", key.0, key.1),
            }),
            Some(_) => {}
        }
    }
    for (key, c) in &candidate.samples {
        if !reference.samples.contains_key(key) {
            diffs.push(format!(
                "{}{:?}: unexpected in candidate ({c})",
                key.0, key.1
            ));
        }
    }
}

/// Drops the families the comparator ignores, so recorded goldens only hold compared content.
pub fn strip_ignored(text: &str) -> String {
    text.lines()
        .filter(|l| {
            let name = l
                .strip_prefix("# HELP ")
                .or_else(|| l.strip_prefix("# TYPE "))
                .unwrap_or(l);
            !IGNORED_PREFIXES
                .iter()
                .chain([&PROCESS_PREFIX])
                .any(|p| name.starts_with(p))
        })
        .map(|l| format!("{l}\n"))
        .collect()
}

pub fn values_equal(a: f64, b: f64, counter: bool) -> bool {
    if a.is_nan() || b.is_nan() {
        return a.is_nan() && b.is_nan();
    }
    if a == b {
        return true;
    }
    counter && (a - b).abs() <= COUNTER_TOLERANCE * a.abs().max(b.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GO: &str = "# HELP kubex_gpu_process_exporter_build_info b\n# TYPE kubex_gpu_process_exporter_build_info gauge\n\
        kubex_gpu_process_exporter_build_info{branch=\"b\",goarch=\"amd64\",goos=\"linux\",goversion=\"go\",revision=\"r\",tags=\"\",version=\"v\"} 1\n\
        # HELP c_total h\n# TYPE c_total counter\nc_total{a=\"1\"} 3.0000000001\n# HELP g h\n# TYPE g gauge\ng NaN\n\
        go_goroutines 7\nprocess_cpu_seconds_total 1\nprocess_open_fds 1\nprocess_max_fds 1\nprocess_virtual_memory_bytes 1\n\
        process_resident_memory_bytes 1\nprocess_start_time_seconds 1\nprocess_network_receive_bytes_total 1\n";
    const RUST: &str = "# HELP kubex_gpu_process_exporter_build_info other\n# TYPE kubex_gpu_process_exporter_build_info gauge\n\
        kubex_gpu_process_exporter_build_info{arch=\"x86_64\",branch=\"b\",os=\"linux\",revision=\"r\",rustversion=\"1\",version=\"v\"} 1\n\
        # HELP c_total h\n# TYPE c_total counter\nc_total{a=\"1\"} 3\n# HELP g h\n# TYPE g gauge\ng NaN\n\
        process_cpu_seconds_total 1\nprocess_open_fds 1\nprocess_max_fds 1\nprocess_virtual_memory_bytes 1\n\
        process_resident_memory_bytes 1\nprocess_start_time_seconds 1\nprocess_threads 1\n";

    fn parse(s: &str) -> Exposition {
        Exposition::parse(s).unwrap()
    }

    #[test]
    fn go_and_rust_outputs_compare_equal() {
        assert_eq!(
            compare(&parse(GO), &parse(RUST), false),
            Vec::<String>::new()
        );
        assert_eq!(compare(&parse(GO), &parse(GO), false), Vec::<String>::new());
    }

    #[test]
    fn detects_value_label_and_build_info_differences() {
        let rust = RUST
            .replace("g NaN", "g 1")
            .replace("revision=\"r\"", "revision=\"x\"")
            .replace("process_threads 1\n", "");
        let diffs = compare(&parse(GO), &parse(&rust), false);
        assert_eq!(diffs.len(), 3, "{diffs:?}");
        assert!(
            compare(
                &parse(GO),
                &parse(&RUST.replace("revision=\"r\"", "revision=\"x\"")),
                true
            )
            .is_empty()
        );
        let extra = RUST.replace(
            "c_total{a=\"1\"} 3",
            "c_total{a=\"1\"} 3\nc_total{a=\"2\"} 1",
        );
        assert_eq!(compare(&parse(GO), &parse(&extra), false).len(), 1);
    }
}
