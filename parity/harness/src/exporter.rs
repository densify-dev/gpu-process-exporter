//! One exporter process under test.
use anyhow::{Context, Result, bail};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use std::collections::BTreeMap;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub const STOP_DEADLINE: Duration = Duration::from_secs(5);

pub struct Exporter {
    pub label: &'static str,
    pub port: u16,
    child: Child,
    log: PathBuf,
}

/// How a process ended, compared between reference and candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitClass {
    Zero,
    NonZero,
    Running,
}

impl std::fmt::Display for ExitClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ExitClass::Zero => "exit=0",
            ExitClass::NonZero => "exit=nonzero",
            ExitClass::Running => "running",
        })
    }
}

impl From<ExitStatus> for ExitClass {
    fn from(status: ExitStatus) -> ExitClass {
        if status.success() {
            ExitClass::Zero
        } else {
            ExitClass::NonZero
        }
    }
}

impl Exporter {
    pub fn spawn(
        label: &'static str,
        bin: &Path,
        env: &BTreeMap<String, String>,
        port: u16,
        dir: &Path,
    ) -> Result<Exporter> {
        let log = dir.join(format!("{label}.log"));
        let out = std::fs::File::create(&log)?;
        let child = Command::new(bin)
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .stdout(out.try_clone()?)
            .stderr(out)
            .spawn()
            .with_context(|| format!("spawn {}", bin.display()))?;
        Ok(Exporter {
            label,
            port,
            child,
            log,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn port_open(&self) -> bool {
        port_open(self.port)
    }

    pub fn exited(&mut self) -> Result<Option<ExitStatus>> {
        Ok(self.child.try_wait()?)
    }

    pub async fn wait_started(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        while !self.port_open() {
            if let Some(status) = self.exited()? {
                bail!(
                    "{} exited during startup with {status}:\n{}",
                    self.label,
                    self.log_tail()
                );
            }
            if Instant::now() > deadline {
                bail!(
                    "{} did not open port {} within {timeout:?}:\n{}",
                    self.label,
                    self.port,
                    self.log_tail()
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(())
    }

    /// Sends `signal` and waits for exit, killing the process after twice the shutdown deadline.
    pub async fn stop(mut self, signal: Signal) -> Result<(ExitClass, Duration)> {
        let start = Instant::now();
        if self.exited()?.is_none() {
            kill(Pid::from_raw(self.child.id() as i32), signal)?;
        }
        loop {
            if let Some(status) = self.exited()? {
                return Ok((status.into(), start.elapsed()));
            }
            if start.elapsed() > 2 * STOP_DEADLINE {
                self.child.kill()?;
                self.child.wait()?;
                return Ok((ExitClass::Running, start.elapsed()));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(20)..].join("\n")
    }
}

impl Drop for Exporter {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(100)).is_ok()
}

pub fn parse_signal(name: &str) -> Result<Signal> {
    Ok(match name {
        "TERM" => Signal::SIGTERM,
        "INT" => Signal::SIGINT,
        other => bail!("unsupported signal {other}"),
    })
}
