//! Executes one scenario against the reference and the candidate.
use crate::compare::{compare, strip_ignored};
use crate::env::{ApiServer, Kubeconfigs};
use crate::expo::{Exposition, Selector};
use crate::exporter::{ExitClass, Exporter, STOP_DEADLINE, parse_signal, port_open};
use crate::scenario::{Check, HttpRequest, KubeStep, ProcStep, Scenario, StartupCase, Step};
use anyhow::{Context, Result, bail};
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::Client;
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const POLL_INTERVAL: &str = "200ms";
const SCRAPE_INTERVAL: &str = "2s";
const DEFAULT_ENDPOINT: &str = "/metrics";
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const START_TIMEOUT: Duration = Duration::from_secs(30);
const ACK_TIMEOUT: Duration = Duration::from_secs(10);
const RETRY_DELAY: Duration = Duration::from_millis(200);
const BASE_PORT: u16 = 21000;

pub enum Reference {
    Go(PathBuf),
    Golden,
}

pub struct Settings {
    pub reference: Reference,
    pub candidate: PathBuf,
    pub record: bool,
    pub golden_dir: PathBuf,
    pub work_dir: PathBuf,
    pub lib_dir: PathBuf,
    pub kubeconfigs: Kubeconfigs,
}

pub struct Shared {
    pub settings: Settings,
    pub apiserver: tokio::sync::Mutex<ApiServer>,
    pub admin: Client,
}

struct Run<'a> {
    shared: &'a Shared,
    scenario: &'a Scenario,
    node: String,
    ns: String,
    dir: PathBuf,
    golden: PathBuf,
    port: u16,
    reference: Option<Exporter>,
    candidate: Option<Exporter>,
    uids: HashMap<String, String>,
}

pub async fn run(shared: &Shared, scenario: &Scenario, index: usize) -> Result<()> {
    let s = &shared.settings;
    let dir = s.work_dir.join("run").join(&scenario.name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    for sub in ["proc", "nvml", "home"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    let mut run = Run {
        shared,
        scenario,
        node: format!("node-{}", scenario.name),
        ns: format!("ns-{}", scenario.name),
        golden: s.golden_dir.join(&scenario.name),
        port: BASE_PORT + 10 * index as u16,
        dir,
        reference: None,
        candidate: None,
        uids: HashMap::new(),
    };
    if s.record {
        if run.golden.exists() {
            std::fs::remove_dir_all(&run.golden)?;
        }
        std::fs::create_dir_all(&run.golden)?;
    }
    run.write_nvml(&scenario.nvml)?;
    run.cleanup().await?;
    let result = run.execute().await;
    run.reference.take();
    run.candidate.take();
    run.cleanup().await?;
    result
}

impl Run<'_> {
    async fn execute(&mut self) -> Result<()> {
        if !self.scenario.startup_cases.is_empty() {
            for case in &self.scenario.startup_cases {
                self.startup_case(case)
                    .await
                    .with_context(|| format!("startup case {}", case.name))?;
            }
            return Ok(());
        }
        ensure_namespace(&self.shared.admin, &self.ns).await?;
        let explicit_start = self.scenario.steps.iter().any(|s| matches!(s, Step::Start));
        if !explicit_start {
            self.start().await?;
        }
        for (i, step) in self.scenario.steps.iter().enumerate() {
            self.step(step)
                .await
                .with_context(|| format!("step {}", i + 1))?;
        }
        self.shutdown().await
    }

    async fn step(&mut self, step: &Step) -> Result<()> {
        match step {
            Step::Start => self.start().await,
            Step::Kube(k) => self.kube(k).await,
            Step::Proc(p) => self.proc(p),
            Step::Frame(n) => self.frame(*n).await,
            Step::Sleep(d) => {
                tokio::time::sleep(parse_duration(d)?).await;
                Ok(())
            }
            Step::Check(c) => self
                .check(c)
                .await
                .with_context(|| format!("check {}", c.name)),
        }
    }

    // ---- exporters

    fn exporter_env(&self, port: u16, kubeconfig: Option<&Path>) -> BTreeMap<String, String> {
        let d = |sub: &str| self.dir.join(sub).display().to_string();
        let mut env: BTreeMap<String, String> = [
            ("PATH", std::env::var("PATH").unwrap_or_default()),
            ("HOME", d("home")),
            ("NODE_NAME", self.node.clone()),
            ("HOST_PROC_MOUNT_POINT", d("proc")),
            ("DRIVER_POLL_INTERVAL", POLL_INTERVAL.into()),
            ("SCRAPE_INTERVAL", SCRAPE_INTERVAL.into()),
            ("EXPORTER_PORT", port.to_string()),
            (
                "LD_LIBRARY_PATH",
                self.shared.settings.lib_dir.display().to_string(),
            ),
            ("FAKE_NVML_DIR", d("nvml")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        if let Some(k) = kubeconfig {
            env.insert("KUBECONFIG".into(), k.display().to_string());
        }
        env.extend(self.scenario.env.clone());
        env
    }

    fn reference_bin(&self) -> Option<&Path> {
        match &self.shared.settings.reference {
            Reference::Go(bin) => Some(bin),
            Reference::Golden => None,
        }
    }

    async fn start(&mut self) -> Result<()> {
        let kubeconfig = self.shared.settings.kubeconfigs.exporter.clone();
        if let Some(bin) = self.reference_bin() {
            let env = self.exporter_env(self.port, Some(&kubeconfig));
            self.reference = Some(Exporter::spawn(
                "reference",
                bin,
                &env,
                self.port,
                &self.dir,
            )?);
        }
        let env = self.exporter_env(self.port + 1, Some(&kubeconfig));
        self.candidate = Some(Exporter::spawn(
            "candidate",
            &self.shared.settings.candidate,
            &env,
            self.port + 1,
            &self.dir,
        )?);
        for e in self.exporters_mut() {
            e.wait_started(START_TIMEOUT).await?;
        }
        Ok(())
    }

    fn exporters_mut(&mut self) -> impl Iterator<Item = &mut Exporter> {
        self.reference.iter_mut().chain(self.candidate.iter_mut())
    }

    async fn shutdown(&mut self) -> Result<()> {
        let signal = parse_signal(&self.scenario.shutdown_signal)?;
        let mut results = Vec::new();
        for e in [self.reference.take(), self.candidate.take()]
            .into_iter()
            .flatten()
        {
            let label = e.label;
            let (class, elapsed) = e.stop(signal).await?;
            if elapsed > STOP_DEADLINE {
                bail!(
                    "{label} took {elapsed:?} to stop after SIG{}",
                    self.scenario.shutdown_signal
                );
            }
            results.push(class.to_string());
        }
        self.compare_status("shutdown", results)
    }

    /// Compares a status line between reference and candidate, or against/into the golden file.
    fn compare_status(&self, name: &str, results: Vec<String>) -> Result<()> {
        let file = self.golden.join(format!("{name}.status"));
        let (reference, candidate) = match results.as_slice() {
            [r, c] => (r.clone(), c.clone()),
            [c] => (
                std::fs::read_to_string(&file)
                    .with_context(|| format!("read {}", file.display()))?
                    .trim()
                    .to_string(),
                c.clone(),
            ),
            _ => unreachable!(),
        };
        if reference != candidate {
            bail!("{name}: reference {reference:?} != candidate {candidate:?}");
        }
        if self.shared.settings.record {
            std::fs::write(file, format!("{reference}\n"))?;
        }
        Ok(())
    }

    async fn startup_case(&mut self, case: &StartupCase) -> Result<()> {
        let s = &self.shared.settings;
        let kubeconfig = match case.kubeconfig.as_deref().unwrap_or("exporter") {
            "exporter" => Some(s.kubeconfigs.exporter.clone()),
            "nobody" => Some(s.kubeconfigs.nobody.clone()),
            "none" => None,
            other => bail!("unknown kubeconfig {other}"),
        };
        let mut frame = self.scenario.nvml["frames"][0].clone();
        frame["returns"] = serde_json::to_value(&case.nvml_returns)?;
        self.write_nvml(&json!({ "frames": [frame] }))?;
        let bins: Vec<(&'static str, PathBuf)> = self
            .reference_bin()
            .map(|b| ("reference", b.to_path_buf()))
            .into_iter()
            .chain([("candidate", s.candidate.clone())])
            .collect();
        let mut results = Vec::new();
        for (label, bin) in bins {
            let mut env = self.exporter_env(self.port, kubeconfig.as_deref());
            env.extend(case.env.clone());
            for k in &case.unset {
                env.remove(k);
            }
            let port = case.port.unwrap_or(self.port);
            let mut e = Exporter::spawn(label, &bin, &env, port, &self.dir)?;
            let (class, opened) = observe_startup(&mut e).await?;
            if class == ExitClass::Running {
                e.stop(nix::sys::signal::Signal::SIGTERM).await?;
            }
            let mut result = class.to_string();
            if case.check_port {
                result += if opened { " port=open" } else { " port=closed" };
            }
            results.push(result);
        }
        self.compare_status(&case.name, results)
    }

    // ---- inputs

    fn write_nvml(&self, nvml: &Value) -> Result<()> {
        let mut nvml = nvml.clone();
        self.expand_value(&mut nvml);
        std::fs::write(
            self.dir.join("nvml/scenario.json"),
            serde_json::to_vec(&nvml)?,
        )?;
        write_atomic(&self.dir.join("nvml/frame"), "0")
    }

    async fn frame(&mut self, n: usize) -> Result<()> {
        write_atomic(&self.dir.join("nvml/frame"), &n.to_string())?;
        let deadline = Instant::now() + ACK_TIMEOUT;
        let pids: Vec<(&str, u32)> = self.exporters_mut().map(|e| (e.label, e.pid())).collect();
        for (label, pid) in pids {
            let ack = self.dir.join(format!("nvml/ack.{pid}"));
            while std::fs::read_to_string(&ack)
                .map(|s| s.trim().to_string())
                .ok()
                .as_deref()
                != Some(&n.to_string())
            {
                if Instant::now() > deadline {
                    bail!("{label} did not ack frame {n}");
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        Ok(())
    }

    fn proc(&self, p: &ProcStep) -> Result<()> {
        let root = self.dir.join("proc");
        let dir = root.join(p.pid.to_string());
        if p.remove {
            std::fs::remove_dir_all(&dir)?;
            return Ok(());
        }
        let content = match (&p.cgroup, &p.pod, &p.cid) {
            (Some(raw), _, _) => self.expand(raw),
            (None, Some(pod), Some(cid)) => {
                let uid = self
                    .uids
                    .get(pod)
                    .with_context(|| format!("unknown pod {pod}"))?
                    .replace('-', "_");
                format!(
                    "0::/kubepods.slice/kubepods-besteffort.slice/kubepods-besteffort-pod{uid}.slice/cri-containerd-{}.scope\n",
                    container_id(cid)
                )
            }
            _ => bail!("proc step needs cgroup, or pod and cid"),
        };
        if p.recreate {
            // Create the replacement first so the new directory cannot reuse the old inode.
            let tmp = root.join(format!("{}.new", p.pid));
            std::fs::create_dir_all(&tmp)?;
            std::fs::write(tmp.join("cgroup"), content)?;
            std::fs::remove_dir_all(&dir)?;
            std::fs::rename(tmp, dir)?;
        } else {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("cgroup"), content)?;
        }
        Ok(())
    }

    async fn kube(&mut self, step: &KubeStep) -> Result<()> {
        let admin = &self.shared.admin;
        match step {
            KubeStep::Create(manifest) => {
                let mut manifest = manifest.clone();
                self.expand_value(&mut manifest);
                let status = manifest
                    .as_object_mut()
                    .context("manifest")?
                    .remove("status");
                let meta = manifest["metadata"].as_object_mut().context("metadata")?;
                let ns = meta
                    .entry("namespace")
                    .or_insert(self.ns.clone().into())
                    .as_str()
                    .context("namespace")?
                    .to_string();
                let name = meta["name"].as_str().context("metadata.name")?.to_string();
                manifest["spec"]["nodeName"] = self.node.clone().into();
                for list in ["containers", "initContainers"] {
                    for c in manifest["spec"]
                        .get_mut(list)
                        .and_then(Value::as_array_mut)
                        .into_iter()
                        .flatten()
                    {
                        c.as_object_mut()
                            .context("container")?
                            .entry("image")
                            .or_insert("x".into());
                    }
                }
                ensure_namespace(admin, &ns).await?;
                let pod: Pod = serde_json::from_value(manifest)?;
                let created = Api::<Pod>::namespaced(admin.clone(), &ns)
                    .create(&PostParams::default(), &pod)
                    .await?;
                let key = if ns == self.ns {
                    name
                } else {
                    format!("{ns}/{name}")
                };
                self.uids
                    .insert(key.clone(), created.metadata.uid.context("uid")?);
                if let Some(status) = status {
                    self.patch_status(&key, status).await?;
                }
            }
            KubeStep::Status { pod, status } => {
                let mut status = status.clone();
                self.expand_value(&mut status);
                self.patch_status(pod, status).await?;
            }
            KubeStep::Patch { pod, patch } => {
                let mut patch = patch.clone();
                self.expand_value(&mut patch);
                let (api, name) = self.pod_api(pod);
                api.patch(name, &PatchParams::default(), &Patch::Merge(patch))
                    .await?;
            }
            KubeStep::Delete { pod, grace } => {
                let (api, name) = self.pod_api(pod);
                let params = DeleteParams {
                    grace_period_seconds: grace.map(|g| g as u32),
                    ..Default::default()
                };
                api.delete(name, &params).await?;
            }
            KubeStep::RestartApiserver { delete } => {
                let pods: Vec<String> = delete
                    .iter()
                    .map(|p| {
                        if p.contains('/') {
                            p.clone()
                        } else {
                            format!("{}/{p}", self.ns)
                        }
                    })
                    .collect();
                let admin_path = self.shared.settings.kubeconfigs.admin.clone();
                self.shared
                    .apiserver
                    .lock()
                    .await
                    .restart_deleting(&admin_path, &pods)
                    .await?;
            }
        }
        Ok(())
    }

    /// Merge-patches pod status, defaulting the required container status fields.
    async fn patch_status(&self, pod: &str, mut status: Value) -> Result<()> {
        for list in ["containerStatuses", "initContainerStatuses"] {
            for c in status
                .get_mut(list)
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                let c = c.as_object_mut().context("container status")?;
                for (k, v) in [
                    ("image", json!("x")),
                    ("imageID", json!("")),
                    ("ready", json!(false)),
                    ("restartCount", json!(0)),
                ] {
                    c.entry(k).or_insert(v);
                }
            }
        }
        let (api, name) = self.pod_api(pod);
        api.patch_status(
            name,
            &PatchParams::default(),
            &Patch::Merge(json!({ "status": status })),
        )
        .await?;
        Ok(())
    }

    fn pod_api<'p>(&self, pod: &'p str) -> (Api<Pod>, &'p str) {
        let (ns, name) = pod.split_once('/').unwrap_or((&self.ns, pod));
        (Api::namespaced(self.shared.admin.clone(), ns), name)
    }

    async fn cleanup(&self) -> Result<()> {
        let api = Api::<Pod>::all(self.shared.admin.clone());
        let pods = api
            .list(&ListParams::default().fields(&format!("spec.nodeName={}", self.node)))
            .await?;
        let params = DeleteParams {
            grace_period_seconds: Some(0),
            ..Default::default()
        };
        for pod in pods {
            let ns = pod.metadata.namespace.unwrap_or_default();
            let name = pod.metadata.name.unwrap_or_default();
            match Api::<Pod>::namespaced(self.shared.admin.clone(), &ns)
                .delete(&name, &params)
                .await
            {
                Ok(_) => {}
                Err(kube::Error::Api(e)) if e.code == 404 => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    // ---- checks

    fn endpoint(&self) -> &str {
        self.scenario
            .env
            .get("EXPORTER_ENDPOINT")
            .map_or(DEFAULT_ENDPOINT, String::as_str)
    }

    async fn check(&mut self, check: &Check) -> Result<()> {
        if let Some(wait) = &check.stall_headers {
            let wait = parse_duration(wait)?;
            let ports: Vec<u16> = self.exporters_mut().map(|e| e.port).collect();
            let results = match ports.as_slice() {
                [r, c] => {
                    let (r, c) = tokio::join!(stall_headers(*r, wait), stall_headers(*c, wait));
                    vec![r?, c?]
                }
                [c] => vec![stall_headers(*c, wait).await?],
                _ => unreachable!(),
            };
            return self.compare_status(&check.name, results);
        }
        let timeout = check
            .timeout
            .as_deref()
            .map(parse_duration)
            .transpose()?
            .unwrap_or(CHECK_TIMEOUT);
        let deadline = Instant::now() + timeout;
        let golden = self.reference.is_none();
        loop {
            let (diffs, outputs) = if check.http.is_empty() {
                self.compare_metrics(check, golden).await?
            } else {
                self.compare_http(check, golden).await?
            };
            if diffs.is_empty() {
                if self.shared.settings.record {
                    for (file, text) in outputs {
                        std::fs::write(self.golden.join(file), text)?;
                    }
                }
                return Ok(());
            }
            if Instant::now() > deadline {
                bail!("not equal after {timeout:?}:\n  {}", diffs.join("\n  "));
            }
            tokio::time::sleep(RETRY_DELAY).await;
        }
    }

    /// Returns the differences and the reference outputs to record.
    async fn compare_metrics(
        &self,
        check: &Check,
        golden: bool,
    ) -> Result<(Vec<String>, Vec<(String, String)>)> {
        let file = format!("{}.prom", check.name);
        let path = self.endpoint().to_string();
        let reference = match &self.reference {
            Some(e) => scrape(e.port, "GET", &path, &BTreeMap::new()).await?.body,
            None => read_golden(&self.golden.join(&file))?,
        };
        let candidate = scrape(
            self.candidate.as_ref().context("candidate")?.port,
            "GET",
            &path,
            &BTreeMap::new(),
        )
        .await?
        .body;
        let (r, c) = (
            Exposition::parse(&reference)?,
            Exposition::parse(&candidate)?,
        );
        let mut diffs = compare(&r, &c, golden);
        diffs.extend(self.expectations(check, &r, "reference")?);
        diffs.extend(self.expectations(check, &c, "candidate")?);
        Ok((diffs, vec![(file, strip_ignored(&reference))]))
    }

    async fn compare_http(
        &self,
        check: &Check,
        golden: bool,
    ) -> Result<(Vec<String>, Vec<(String, String)>)> {
        let mut diffs = Vec::new();
        let mut outputs = Vec::new();
        for req in &check.http {
            let file = format!("{}.{}.prom", check.name, req.name);
            let path = req.path.clone().unwrap_or_else(|| self.endpoint().into());
            let reference = match &self.reference {
                Some(e) => scrape(e.port, &req.method, &path, &req.headers).await?,
                None => Response::from_golden(&read_golden(&self.golden.join(&file))?)?,
            };
            let candidate = scrape(
                self.candidate.as_ref().context("candidate")?.port,
                &req.method,
                &path,
                &req.headers,
            )
            .await?;
            diffs.extend(compare_responses(req, &reference, &candidate, golden)?);
            for (label, resp) in [("reference", &reference), ("candidate", &candidate)] {
                if resp.has_metrics(req) {
                    let expo = Exposition::parse(&resp.body)?;
                    diffs.extend(self.expectations(
                        check,
                        &expo,
                        &format!("{}: {label}", req.name),
                    )?);
                }
            }
            outputs.push((file, reference.to_golden()));
        }
        Ok((diffs, outputs))
    }

    fn expectations(&self, check: &Check, expo: &Exposition, label: &str) -> Result<Vec<String>> {
        let mut diffs = Vec::new();
        for family in &check.non_empty {
            if expo
                .families
                .get(family)
                .is_none_or(|f| f.samples.is_empty())
            {
                diffs.push(format!("{label}: {family} is empty"));
            }
        }
        for selector in &check.absent {
            let sel: Selector = self.expand(selector).parse()?;
            if !expo.select(&sel).is_empty() {
                diffs.push(format!("{label}: {sel} should be absent"));
            }
        }
        for (selector, want) in &check.values {
            let sel: Selector = self.expand(selector).parse()?;
            match expo.select(&sel).as_slice() {
                [(_, got)] if crate::compare::values_equal(*want, *got, true) => {}
                [(_, got)] => diffs.push(format!("{label}: {sel} = {got}, want {want}")),
                matches => diffs.push(format!(
                    "{label}: {sel} matched {} samples, want 1",
                    matches.len()
                )),
            }
        }
        Ok(diffs)
    }

    // ---- substitutions

    fn expand_value(&self, v: &mut Value) {
        match v {
            Value::String(s) => *s = self.expand(s),
            Value::Array(a) => a.iter_mut().for_each(|v| self.expand_value(v)),
            Value::Object(o) => o.values_mut().for_each(|v| self.expand_value(v)),
            _ => {}
        }
    }

    /// Expands `${now}`, `${now+5s}`, `${now-1h}`, `${cid:name}`, `${puid:pod}` (cgroup-style pod UID), `${node}`,
    /// `${ns}`.
    fn expand(&self, s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(start) = rest.find("${") {
            out.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}') else {
                break;
            };
            let token = &rest[start + 2..start + end];
            out.push_str(&match token {
                "node" => self.node.clone(),
                "ns" => self.ns.clone(),
                t if t.starts_with("cid:") => container_id(&t[4..]),
                t if t.starts_with("puid:") => self
                    .uids
                    .get(&t[5..])
                    .map_or_else(|| format!("${{{t}}}"), |u| u.replace('-', "_")),
                t if t.starts_with("now") => rfc3339(now_offset(&t[3..])),
                t => format!("${{{t}}}"),
            });
            rest = &rest[start + end + 1..];
        }
        out.push_str(rest);
        out
    }
}

async fn ensure_namespace(admin: &Client, name: &str) -> Result<()> {
    let ns: Namespace = serde_json::from_value(json!({ "metadata": { "name": name } }))?;
    match Api::<Namespace>::all(admin.clone())
        .create(&PostParams::default(), &ns)
        .await
    {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(e)) if e.code == 409 => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Polls a starting exporter until it exits, or has kept its port open for a while.
async fn observe_startup(e: &mut Exporter) -> Result<(ExitClass, bool)> {
    const SETTLE: Duration = Duration::from_secs(2);
    let deadline = Instant::now() + START_TIMEOUT;
    let mut opened_at = None;
    loop {
        if e.port_open() {
            opened_at.get_or_insert_with(Instant::now);
        }
        if let Some(status) = e.exited()? {
            return Ok((status.into(), opened_at.is_some()));
        }
        if opened_at.is_some_and(|t| t.elapsed() > SETTLE) || Instant::now() > deadline {
            return Ok((ExitClass::Running, opened_at.is_some()));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ---- HTTP

struct Response {
    status: u16,
    content_type: String,
    content_encoding: String,
    body: String,
}

impl Response {
    /// Whether the body is a text exposition to compare.
    fn has_metrics(&self, req: &HttpRequest) -> bool {
        self.status == 200 && req.method != "HEAD" && self.content_type.starts_with("text/plain")
    }

    fn to_golden(&self) -> String {
        format!(
            "# HTTP status={} content-type={:?} content-encoding={:?}\n{}",
            self.status,
            self.content_type,
            self.content_encoding,
            strip_ignored(&self.body)
        )
    }

    fn from_golden(text: &str) -> Result<Response> {
        let (head, body) = text.split_once('\n').unwrap_or((text, ""));
        let head = head
            .strip_prefix("# HTTP status=")
            .context("golden HTTP header")?;
        let (status, head) = head
            .split_once(" content-type=")
            .context("golden content-type")?;
        let (content_type, content_encoding) = head
            .split_once(" content-encoding=")
            .context("golden content-encoding")?;
        Ok(Response {
            status: status.parse()?,
            content_type: serde_json::from_str(content_type)?,
            content_encoding: serde_json::from_str(content_encoding)?,
            body: body.into(),
        })
    }
}

async fn scrape(
    port: u16,
    method: &str,
    path: &str,
    headers: &BTreeMap<String, String>,
) -> Result<Response> {
    if !port_open(port) {
        bail!("port {port} is closed");
    }
    let mut req =
        reqwest::Client::new().request(method.parse()?, format!("http://127.0.0.1:{port}{path}"));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.send().await?;
    let header = |name| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let (status, content_type, content_encoding) = (
        resp.status().as_u16(),
        header("content-type"),
        header("content-encoding"),
    );
    let raw = resp.bytes().await?;
    // Binary bodies (protobuf) hold timing-dependent runtime metrics, so only their headers are compared.
    let body = if !content_type.starts_with("text/plain") {
        String::new()
    } else if content_encoding == "gzip" {
        let mut s = String::new();
        flate2::read::GzDecoder::new(&raw[..]).read_to_string(&mut s)?;
        s
    } else {
        String::from_utf8(raw.to_vec())?
    };
    Ok(Response {
        status,
        content_type,
        content_encoding,
        body,
    })
}

fn compare_responses(
    req: &HttpRequest,
    r: &Response,
    c: &Response,
    golden: bool,
) -> Result<Vec<String>> {
    let mut diffs = Vec::new();
    for (what, a, b) in [
        ("status", r.status.to_string(), c.status.to_string()),
        (
            "content-type",
            r.content_type.clone(),
            c.content_type.clone(),
        ),
        (
            "content-encoding",
            r.content_encoding.clone(),
            c.content_encoding.clone(),
        ),
    ] {
        if a != b {
            diffs.push(format!("{}: {what} {a:?} != {b:?}", req.name));
        }
    }
    if r.has_metrics(req) && c.has_metrics(req) {
        let d = compare(
            &Exposition::parse(&r.body)?,
            &Exposition::parse(&c.body)?,
            golden,
        );
        diffs.extend(d.into_iter().map(|d| format!("{}: {d}", req.name)));
    }
    Ok(diffs)
}

/// Opens a connection, sends half a request, and reports what the server did after `wait`.
async fn stall_headers(port: u16, wait: Duration) -> Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut conn = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    conn.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n").await?;
    tokio::time::sleep(wait).await;
    let mut buf = [0u8; 256];
    Ok(
        match tokio::time::timeout(Duration::from_millis(500), conn.read(&mut buf)).await {
            Err(_) => "open".into(),
            Ok(Ok(0) | Err(_)) => "closed".into(),
            Ok(Ok(n)) => format!(
                "response {:?}",
                String::from_utf8_lossy(&buf[..n])
                    .lines()
                    .next()
                    .unwrap_or_default()
            ),
        },
    )
}

// ---- helpers

fn read_golden(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("read golden {}", path.display()))
}

fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// Deterministic 64-hex container ID for a short scenario name.
pub fn container_id(name: &str) -> String {
    (0u8..4)
        .map(|seed| {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in std::iter::once(seed).chain(name.bytes()) {
                h = (h ^ b as u64).wrapping_mul(0x100000001b3);
            }
            format!("{h:016x}")
        })
        .collect()
}

pub fn parse_duration(s: &str) -> Result<Duration> {
    let (num, unit) = s.split_at(
        s.find(|c: char| !c.is_ascii_digit() && c != '.')
            .context("duration unit")?,
    );
    let n: f64 = num.parse()?;
    Ok(Duration::from_secs_f64(match unit {
        "ms" => n / 1000.0,
        "s" => n,
        "m" => n * 60.0,
        "h" => n * 3600.0,
        u => bail!("unknown duration unit {u}"),
    }))
}

fn now_offset(offset: &str) -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64;
    let Some(sign) = offset.chars().next() else {
        return now;
    };
    let delta = parse_duration(&offset[1..])
        .expect("bad ${now} offset")
        .as_secs() as i64;
    if sign == '-' {
        now - delta
    } else {
        now + delta
    }
}

fn rfc3339(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil-from-days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_rfc3339() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_795_000_000), "2026-11-18T11:06:40Z");
    }

    #[test]
    fn container_ids_are_64_hex() {
        let id = container_id("c1");
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(id, container_id("c2"));
    }
}
