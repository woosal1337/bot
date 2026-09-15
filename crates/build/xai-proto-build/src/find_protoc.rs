// Modified by the Bot project on 2026-09-16: use PATH before Unix wrappers on Windows.
use anyhow::{Context, bail};
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

fn check_protoc_good(protoc: &Path) -> anyhow::Result<()> {
    let output = Command::new(protoc)
        .arg("--version")
        .output()
        .context("Failed to execute protoc")?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "protoc --version failed, likely dotslash is missing; \
             try `cargo install dotslash`; stdout: {stdout:?}, stderr: {stderr:?}"
        );
    }
    Ok(())
}

fn is_github_actions() -> bool {
    env::var_os("GITHUB_ACTIONS").is_some()
}

fn find_protoc_in_path(path: &OsStr) -> Option<PathBuf> {
    env::split_paths(path)
        .map(|directory| directory.join(format!("protoc{}", env::consts::EXE_SUFFIX)))
        .filter(|candidate| candidate.is_file())
        .find(|candidate| check_protoc_good(candidate).is_ok())
        .and_then(|candidate| std::path::absolute(candidate).ok())
}

/// Find `protoc` command.
///
/// Search order:
/// 1. `$PROTOC` environment variable (set by Bazel `build_script_env` or user override)
/// 2. `bin/protoc` walking up parent directories (dotslash wrapper for local dev)
/// 3. `protoc` on `$PATH` (system install or other tooling)
///
/// When `bin/protoc` exists but fails to execute (e.g. the dotslash wrapper running
/// in Bazel remote execution where `dotslash` is not installed), the error is not fatal —
/// we fall through to the PATH-based lookup instead.
///
/// Returns `Ok(None)` if not found and not in a strict environment (GitHub Actions).
pub fn find_protoc() -> anyhow::Result<Option<PathBuf>> {
    println!("cargo:rerun-if-env-changed=PROTOC");
    println!("cargo:rerun-if-env-changed=PATH");
    // 1. Check the PROTOC env var first. This is the standard override used by prost-build
    //    and is set by Bazel cargo_build_script build_script_env to point at a hermetic
    //    protoc binary instead of the dotslash wrapper.
    if let Ok(protoc_env) = env::var("PROTOC") {
        let protoc = PathBuf::from(&protoc_env);
        if protoc.try_exists()? {
            check_protoc_good(&protoc)?;
            return Ok(Some(protoc));
        }
    }

    if cfg!(windows)
        && let Some(protoc) = env::var_os("PATH").and_then(|path| find_protoc_in_path(&path))
    {
        return Ok(Some(protoc));
    }

    // 2. Walk up directories looking for bin/protoc (dotslash wrapper).
    let cwd = env::current_dir()?;
    let mut dir = cwd.clone();
    let mut dir_rel = PathBuf::new();
    loop {
        // Return relative path to make build more deterministic.
        let protoc = dir_rel.join("bin/protoc");
        if protoc.try_exists()? {
            match check_protoc_good(&protoc) {
                Ok(()) => return Ok(Some(protoc)),
                Err(e) => {
                    // bin/protoc exists but can't execute — likely the dotslash wrapper
                    // in an environment without dotslash (e.g. Bazel remote execution).
                    // Fall through to PATH-based lookup below.
                    eprintln!(
                        "bin/protoc found at `{}` but failed to execute: {e:#}; \
                         trying protoc from PATH as fallback",
                        protoc.display()
                    );
                    break;
                }
            }
        }
        if !dir.pop() {
            break;
        }
        dir_rel.push("..");
    }

    // 3. Try protoc from PATH (system install or other tooling).
    if let Some(protoc) = env::var_os("PATH").and_then(|path| find_protoc_in_path(&path)) {
        return Ok(Some(protoc));
    }

    // 4. Not found anywhere.
    if is_github_actions() {
        return Err(anyhow::anyhow!(
            "`protoc` not found (checked $PROTOC env, bin/protoc, and PATH)"
        ));
    }
    eprintln!("`protoc` not found; likely it is missing in docker image");
    Ok(None)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fixture(directory: &Path, exit: u8) -> PathBuf {
        std::fs::create_dir_all(directory).unwrap();
        let executable = directory.join("protoc");
        std::fs::write(&executable, format!("#!/bin/sh\nexit {exit}\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        executable
    }

    #[test]
    fn path_lookup_returns_an_existing_absolute_file() {
        let directory = tempfile::tempdir().unwrap();
        let bin = directory.path().join("bin with spaces");
        let expected = fixture(&bin, 0);
        let resolved = find_protoc_in_path(&env::join_paths([bin]).unwrap()).unwrap();
        assert_eq!(resolved, std::path::absolute(expected).unwrap());
        assert!(resolved.is_absolute());
        assert!(resolved.is_file());
    }

    #[test]
    fn path_lookup_skips_missing_and_failed_candidates() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing");
        let failed = directory.path().join("failed");
        let working = directory.path().join("working");
        fixture(&failed, 1);
        let expected = fixture(&working, 0);
        let path = env::join_paths([&missing, &failed, &working]).unwrap();
        assert_eq!(find_protoc_in_path(&path), Some(expected));
        let path = env::join_paths([missing, failed]).unwrap();
        assert_eq!(find_protoc_in_path(&path), None);
    }
}
