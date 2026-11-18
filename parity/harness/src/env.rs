//! Hermetic kube-apiserver + etcd started the way envtest does, with static tokens and RBAC.
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
};
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding};
use kube::api::{Api, PostParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const ETCD_PORT: u16 = 23790;
const ETCD_PEER_PORT: u16 = 23800;
const APISERVER_PORT: u16 = 16443;
const ADMIN_TOKEN: &str = "parity-admin";
const EXPORTER_TOKEN: &str = "parity-exporter";
const NOBODY_TOKEN: &str = "parity-nobody";
const EXPORTER_USER: &str = "exporter";

pub struct ApiServer {
    dir: PathBuf,
    bin: PathBuf,
    etcd: Child,
    apiserver: Child,
}

/// Kubeconfigs handed to exporters.
pub struct Kubeconfigs {
    pub admin: PathBuf,
    pub exporter: PathBuf,
    pub nobody: PathBuf,
}

impl ApiServer {
    /// Starts a fresh cluster in `dir`, deleting any previous state.
    pub async fn start(bin: &Path, dir: &Path) -> Result<(ApiServer, Kubeconfigs)> {
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        std::fs::create_dir_all(dir)?;
        let dir = dir.canonicalize()?;
        write_certs(&dir)?;
        std::fs::write(
            dir.join("tokens.csv"),
            format!(
                "{ADMIN_TOKEN},admin,admin,system:masters\n{EXPORTER_TOKEN},{EXPORTER_USER},{EXPORTER_USER}\n\
                 {NOBODY_TOKEN},nobody,nobody\n"
            ),
        )?;
        let etcd = spawn(
            Command::new(bin.join("etcd")).args([
                "--data-dir",
                &dir.join("etcd").display().to_string(),
                "--listen-client-urls",
                &format!("http://127.0.0.1:{ETCD_PORT}"),
                "--advertise-client-urls",
                &format!("http://127.0.0.1:{ETCD_PORT}"),
                "--listen-peer-urls",
                &format!("http://127.0.0.1:{ETCD_PEER_PORT}"),
            ]),
            &dir.join("etcd.log"),
        )?;
        let apiserver = spawn_apiserver(bin, &dir)?;
        let mut server = ApiServer {
            dir: dir.clone(),
            bin: bin.into(),
            etcd,
            apiserver,
        };
        let kubeconfigs = Kubeconfigs {
            admin: server.kubeconfig("admin", ADMIN_TOKEN)?,
            exporter: server.kubeconfig("exporter", EXPORTER_TOKEN)?,
            nobody: server.kubeconfig("nobody", NOBODY_TOKEN)?,
        };
        server.wait_ready(&kubeconfigs.admin).await?;
        grant_exporter(&client(&kubeconfigs.admin).await?).await?;
        wait_rbac(&kubeconfigs.exporter).await?;
        Ok((server, kubeconfigs))
    }

    fn kubeconfig(&self, name: &str, token: &str) -> Result<PathBuf> {
        let ca = B64.encode(std::fs::read(self.dir.join("ca.crt"))?);
        let path = self.dir.join(format!("{name}.kubeconfig"));
        std::fs::write(
            &path,
            format!(
                "apiVersion: v1\nkind: Config\nclusters:\n- name: parity\n  cluster:\n    server: https://127.0.0.1:{APISERVER_PORT}\n    \
                 certificate-authority-data: {ca}\nusers:\n- name: {name}\n  user:\n    token: {token}\ncontexts:\n- name: parity\n  \
                 context:\n    cluster: parity\n    user: {name}\ncurrent-context: parity\n"
            ),
        )?;
        Ok(path)
    }

    async fn wait_ready(&mut self, admin: &Path) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = self.apiserver.try_wait()? {
                bail!(
                    "kube-apiserver exited with {status}; see {}",
                    self.dir.join("apiserver.log").display()
                );
            }
            if let Ok(client) = client(admin).await
                && client.apiserver_version().await.is_ok()
            {
                return Ok(());
            }
            if Instant::now() > deadline {
                bail!("kube-apiserver not ready after 60s");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Stops the apiserver, deletes `pods` (`namespace/name`) straight from etcd and compacts it, then starts
    /// the apiserver again. Watchers can only learn about the deletes through a relist.
    pub async fn restart_deleting(&mut self, admin: &Path, pods: &[String]) -> Result<()> {
        self.apiserver.kill()?;
        self.apiserver.wait()?;
        let http = reqwest::Client::new();
        let etcd = format!("http://127.0.0.1:{ETCD_PORT}/v3/kv");
        let mut revision = String::new();
        for pod in pods {
            let key = B64.encode(format!("/registry/pods/{pod}"));
            let resp: serde_json::Value = http
                .post(format!("{etcd}/deleterange"))
                .json(&serde_json::json!({ "key": key }))
                .send()
                .await?
                .json()
                .await?;
            if resp["deleted"].as_str() != Some("1") {
                bail!("etcd delete of {pod} returned {resp}");
            }
            revision = resp["header"]["revision"]
                .as_str()
                .context("etcd revision")?
                .into();
        }
        http.post(format!("{etcd}/compaction"))
            .json(&serde_json::json!({ "revision": revision, "physical": true }))
            .send()
            .await?
            .error_for_status()?;
        self.apiserver = spawn_apiserver(&self.bin, &self.dir)?;
        self.wait_ready(admin).await
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) {
        for child in [&mut self.apiserver, &mut self.etcd] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub async fn client(kubeconfig: &Path) -> Result<Client> {
    let config = Config::from_custom_kubeconfig(
        Kubeconfig::read_from(kubeconfig)?,
        &KubeConfigOptions::default(),
    )
    .await?;
    Ok(Client::try_from(config)?)
}

fn spawn(cmd: &mut Command, log: &Path) -> Result<Child> {
    let log = std::fs::File::create(log)?;
    Ok(cmd
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?)
}

fn spawn_apiserver(bin: &Path, dir: &Path) -> Result<Child> {
    let p = |name: &str| dir.join(name).display().to_string();
    spawn(
        Command::new(bin.join("kube-apiserver")).args([
            format!("--etcd-servers=http://127.0.0.1:{ETCD_PORT}"),
            format!("--secure-port={APISERVER_PORT}"),
            "--bind-address=127.0.0.1".into(),
            "--advertise-address=127.0.0.1".into(),
            format!("--tls-cert-file={}", p("serving.crt")),
            format!("--tls-private-key-file={}", p("serving.key")),
            format!("--service-account-key-file={}", p("sa.pub")),
            format!("--service-account-signing-key-file={}", p("sa.key")),
            "--service-account-issuer=https://kubernetes.default.svc".into(),
            "--service-cluster-ip-range=10.0.0.0/24".into(),
            "--allow-privileged=true".into(),
            "--disable-admission-plugins=ServiceAccount".into(),
            "--authorization-mode=RBAC".into(),
            format!("--token-auth-file={}", p("tokens.csv")),
        ]),
        &dir.join("apiserver.log"),
    )
}

fn write_certs(dir: &Path) -> Result<()> {
    let run = |args: &[&str]| -> Result<()> {
        let out = Command::new("openssl")
            .args(args)
            .current_dir(dir)
            .output()
            .context("run openssl")?;
        if !out.status.success() {
            bail!("openssl {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(())
    };
    run(&[
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        "ca.key",
        "-out",
        "ca.crt",
        "-days",
        "7",
        "-subj",
        "/CN=parity-ca",
    ])?;
    run(&[
        "req",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        "serving.key",
        "-out",
        "serving.csr",
        "-subj",
        "/CN=127.0.0.1",
    ])?;
    std::fs::write(
        dir.join("san.ext"),
        "subjectAltName=IP:127.0.0.1,DNS:localhost\n",
    )?;
    run(&[
        "x509",
        "-req",
        "-in",
        "serving.csr",
        "-CA",
        "ca.crt",
        "-CAkey",
        "ca.key",
        "-CAcreateserial",
        "-out",
        "serving.crt",
        "-days",
        "7",
        "-extfile",
        "san.ext",
    ])?;
    run(&["genrsa", "-out", "sa.key", "2048"])?;
    run(&["rsa", "-in", "sa.key", "-pubout", "-out", "sa.pub"])
}

async fn grant_exporter(admin: &Client) -> Result<()> {
    let role: ClusterRole = serde_json::from_value(serde_json::json!({
        "metadata": { "name": "gpu-process-exporter" },
        "rules": [{ "apiGroups": [""], "resources": ["pods"], "verbs": ["get", "list", "watch"] }],
    }))?;
    let binding: ClusterRoleBinding = serde_json::from_value(serde_json::json!({
        "metadata": { "name": "gpu-process-exporter" },
        "roleRef": { "apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": "gpu-process-exporter" },
        "subjects": [{ "apiGroup": "rbac.authorization.k8s.io", "kind": "User", "name": EXPORTER_USER }],
    }))?;
    Api::<ClusterRole>::all(admin.clone())
        .create(&PostParams::default(), &role)
        .await?;
    Api::<ClusterRoleBinding>::all(admin.clone())
        .create(&PostParams::default(), &binding)
        .await?;
    Ok(())
}

/// Waits until RBAC bootstrap and the exporter binding are effective.
async fn wait_rbac(exporter: &Path) -> Result<()> {
    let api = Api::<SelfSubjectAccessReview>::all(client(exporter).await?);
    let review = SelfSubjectAccessReview {
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                resource: Some("pods".into()),
                verb: Some("watch".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(r) = api.create(&PostParams::default(), &review).await
            && r.status.is_some_and(|s| s.allowed)
        {
            return Ok(());
        }
        if Instant::now() > deadline {
            bail!("exporter RBAC not effective after 60s");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
