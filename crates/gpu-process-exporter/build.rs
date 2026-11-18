// SPDX-License-Identifier: Apache-2.0

//! Injects the build info: VERSION, REVISION, and BRANCH from the environment, plus the rustc version.

use std::process::Command;

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string()).filter(|s| !s.is_empty())
}

fn main() {
    for var in ["VERSION", "REVISION", "BRANCH"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    let env = |var: &str| std::env::var(var).ok().filter(|v| !v.is_empty());
    let version = env("VERSION").unwrap_or_else(|| env("CARGO_PKG_VERSION").unwrap());
    let revision = env("REVISION")
        .or_else(|| output("git", &["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".into());
    let branch = env("BRANCH").unwrap_or_else(|| "unknown".into());
    let rustc = env("RUSTC").unwrap_or_else(|| "rustc".into());
    let rust_version = output(&rustc, &["--version"])
        .and_then(|v| v.split_whitespace().nth(1).map(String::from))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=BUILD_VERSION={version}");
    println!("cargo:rustc-env=BUILD_REVISION={revision}");
    println!("cargo:rustc-env=BUILD_BRANCH={branch}");
    println!("cargo:rustc-env=BUILD_RUST_VERSION={rust_version}");
}
