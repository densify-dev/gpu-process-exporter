//! Scenario files (`parity/harness/scenarios/*.yaml`).
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    #[serde(skip)]
    pub name: String,
    #[serde(default)]
    #[serde(rename = "description")]
    pub _description: String,
    /// Runs alone, after the parallel batch (it restarts the apiserver).
    #[serde(default)]
    pub exclusive: bool,
    /// Extra exporter environment, applied to both binaries.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Handed to the fake NVML as `scenario.json`.
    #[serde(default = "default_nvml")]
    pub nvml: Value,
    #[serde(default)]
    pub steps: Vec<Step>,
    /// When set, the scenario only compares startup behavior.
    #[serde(default)]
    pub startup_cases: Vec<StartupCase>,
    #[serde(default = "default_signal")]
    pub shutdown_signal: String,
    /// YAML anchors live here so frames can share device definitions.
    #[serde(default, rename = "definitions")]
    pub _definitions: Value,
}

fn default_nvml() -> Value {
    serde_json::json!({ "frames": [{}] })
}

fn default_signal() -> String {
    "TERM".into()
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// Starts both exporters; steps before it run against an idle cluster.
    Start,
    Kube(KubeStep),
    Proc(ProcStep),
    Frame(usize),
    Sleep(String),
    Check(Check),
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum KubeStep {
    /// A Pod manifest; namespace, `spec.nodeName`, and container images are defaulted. An embedded `status`
    /// is written through the status subresource.
    Create(Value),
    Status {
        pod: String,
        status: Value,
    },
    Patch {
        pod: String,
        patch: Value,
    },
    Delete {
        pod: String,
        grace: Option<i64>,
    },
    /// Restarts the apiserver, deleting the pods from etcd while it is down.
    RestartApiserver {
        delete: Vec<String>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcStep {
    pub pid: u32,
    /// Pod name (scenario namespace) whose UID goes into the generated cgroup line.
    pub pod: Option<String>,
    pub cid: Option<String>,
    /// Raw cgroup file content instead of a generated one.
    pub cgroup: Option<String>,
    #[serde(default)]
    pub remove: bool,
    /// Replace the `/proc/<pid>` directory so it gets a new inode (PID reuse).
    #[serde(default)]
    pub recreate: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub name: String,
    /// Families that must have at least one sample in both outputs.
    #[serde(default)]
    pub non_empty: Vec<String>,
    /// Selectors that must match no sample.
    #[serde(default)]
    pub absent: Vec<String>,
    /// Hand-computed values: selector -> value (exactly one matching sample).
    #[serde(default)]
    pub values: BTreeMap<String, f64>,
    /// Hand-computed values only a Rust candidate must have, for its deliberate fixes of Go.
    #[serde(default)]
    pub rust_values: BTreeMap<String, f64>,
    /// HTTP requests compared instead of the metrics endpoint.
    #[serde(default)]
    pub http: Vec<HttpRequest>,
    /// Sends a partial request line, waits this long, and compares whether the server closed the connection.
    pub stall_headers: Option<String>,
    pub timeout: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpRequest {
    pub name: String,
    pub path: Option<String>,
    #[serde(default = "get")]
    pub method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupCase {
    pub name: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub unset: Vec<String>,
    /// `exporter` (default), `nobody`, or `none`.
    pub kubeconfig: Option<String>,
    /// Port expected to open, when not the scenario port (e.g. the 9494 default).
    pub port: Option<u16>,
    #[serde(default = "yes")]
    pub check_port: bool,
    #[serde(default)]
    pub nvml_returns: BTreeMap<String, u32>,
}

fn get() -> String {
    "GET".into()
}

fn yes() -> bool {
    true
}

pub fn load_all(dir: &Path) -> Result<Vec<Scenario>> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    paths.retain(|p| p.extension().is_some_and(|e| e == "yaml"));
    paths.sort();
    paths
        .iter()
        .map(|p| load(p).with_context(|| format!("load {}", p.display())))
        .collect()
}

fn load(path: &Path) -> Result<Scenario> {
    let mut scenario: Scenario = serde_saphyr::from_str(&std::fs::read_to_string(path)?)?;
    scenario.name = path
        .file_stem()
        .context("scenario file name")?
        .to_string_lossy()
        .into();
    Ok(scenario)
}
