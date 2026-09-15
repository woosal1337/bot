// Modified by the Bot project on 2026-09-13: Removed obsolete updater launch flags.

use std::ffi::{OsStr, OsString};
use std::io::{self, Write};

use crate::provider::ProviderId;

pub(crate) fn build_provider_relaunch_args(
    current_args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    provider: &ProviderId,
) -> Vec<OsString> {
    let mut iter = current_args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .peekable();
    let _ = iter.next();
    let mut output = Vec::new();
    while let Some(arg) = iter.next() {
        let value = arg.to_string_lossy();
        if value == "--" {
            break;
        }
        if value.starts_with("--provider=")
            || value.starts_with("--resume=")
            || value.starts_with("--load=")
            || value.starts_with("--session-id=")
            || value.starts_with("-s=")
            || value.starts_with("--worktree=")
            || value.starts_with("--worktree-ref=")
            || value.starts_with("--ref=")
        {
            continue;
        }
        if matches!(
            value.as_ref(),
            "--continue" | "-c" | "--fork-session" | "--restore-code"
        ) {
            continue;
        }
        if matches!(
            value.as_ref(),
            "--provider"
                | "--resume"
                | "-r"
                | "--load"
                | "--session-id"
                | "-s"
                | "--worktree"
                | "-w"
                | "--worktree-ref"
                | "--ref"
        ) {
            if iter
                .peek()
                .is_some_and(|next| !next.to_string_lossy().starts_with('-'))
            {
                let _ = iter.next();
            }
            continue;
        }
        if value.starts_with('-') {
            let takes_value = !value.contains('=')
                && super::screen_mode_relaunch::flag_takes_value(value.as_ref());
            output.push(arg);
            if takes_value
                && iter
                    .peek()
                    .is_some_and(|next| !next.to_string_lossy().starts_with('-'))
            {
                output.push(iter.next().expect("peeked value present"));
            }
        }
    }
    output.push(OsString::from("--provider"));
    output.push(OsString::from(provider.key()));
    output
}

pub(crate) fn exec_provider_relaunch(provider: &ProviderId) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let args = build_provider_relaunch_args(std::env::args_os(), provider);
    let mut command = std::process::Command::new(executable);
    command.args(args);
    eprintln!("Starting {}…", provider.label());
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(io::Error::other(format!(
            "failed to exec provider: {error}"
        )))
    }
    #[cfg(windows)]
    {
        let status = command.status()?;
        std::process::exit(status.code().unwrap_or(0));
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(io::Error::other("provider relaunch is not supported"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_provider_and_drops_session_state() {
        let args = [
            "bot",
            "--provider",
            "grok",
            "--resume",
            "session-1",
            "old prompt",
        ];
        assert_eq!(
            build_provider_relaunch_args(args, &ProviderId::Codex),
            ["--provider", "codex"].map(OsString::from).to_vec()
        );
    }

    #[test]
    fn preserves_value_flags_and_screen_mode() {
        let args = [
            "bot",
            "--cwd",
            "/tmp/project",
            "--minimal",
            "--provider=grok",
        ];
        assert_eq!(
            build_provider_relaunch_args(args, &ProviderId::Codex),
            ["--cwd", "/tmp/project", "--minimal", "--provider", "codex",]
                .map(OsString::from)
                .to_vec()
        );
    }
}
