use std::env;
use std::path::{Path, PathBuf};

use bot_core::ProviderId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderProtocol {
    CodexAppServer,
    AgentClientProtocol,
    PolicyGated,
}

impl ProviderProtocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::CodexAppServer => "app-server",
            Self::AgentClientProtocol => "ACP",
            Self::PolicyGated => "policy gate",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportLevel {
    Ready,
    Next,
    Planned,
    Deferred,
}

impl SupportLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Next => "next",
            Self::Planned => "planned",
            Self::Deferred => "deferred",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDescriptor {
    id: ProviderId,
    executable: &'static str,
    executable_path: Option<PathBuf>,
    protocol: ProviderProtocol,
    support: SupportLevel,
}

impl ProviderDescriptor {
    pub fn new(
        id: ProviderId,
        executable: &'static str,
        executable_path: Option<PathBuf>,
        protocol: ProviderProtocol,
        support: SupportLevel,
    ) -> Self {
        Self {
            id,
            executable,
            executable_path,
            protocol,
            support,
        }
    }

    pub fn id(&self) -> &ProviderId {
        &self.id
    }

    pub fn executable(&self) -> &str {
        self.executable
    }

    pub fn executable_path(&self) -> Option<&Path> {
        self.executable_path.as_deref()
    }

    pub fn protocol(&self) -> ProviderProtocol {
        self.protocol
    }

    pub fn support(&self) -> SupportLevel {
        self.support
    }

    pub fn is_installed(&self) -> bool {
        self.executable_path.is_some()
    }
}

pub fn discover_providers() -> Vec<ProviderDescriptor> {
    [
        (
            ProviderId::Grok,
            "grok",
            ProviderProtocol::AgentClientProtocol,
            SupportLevel::Ready,
        ),
        (
            ProviderId::Codex,
            "codex",
            ProviderProtocol::CodexAppServer,
            SupportLevel::Next,
        ),
        (
            ProviderId::Gemini,
            "gemini",
            ProviderProtocol::AgentClientProtocol,
            SupportLevel::Planned,
        ),
        (
            ProviderId::Claude,
            "claude",
            ProviderProtocol::PolicyGated,
            SupportLevel::Deferred,
        ),
    ]
    .into_iter()
    .map(|(id, executable, protocol, support)| {
        ProviderDescriptor::new(
            id,
            executable,
            find_executable(executable),
            protocol,
            support,
        )
    })
    .collect()
}

fn find_executable(executable: &str) -> Option<PathBuf> {
    let value = env::var_os("PATH")?;
    find_executable_in(executable, env::split_paths(&value))
}

fn find_executable_in(
    executable: &str,
    paths: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    let names = executable_names(executable);
    paths
        .into_iter()
        .flat_map(|path| names.iter().map(move |name| path.join(name)))
        .find(|candidate| is_executable(candidate))
}

fn executable_names(executable: &str) -> Vec<String> {
    let suffix = env::consts::EXE_SUFFIX;
    if suffix.is_empty() || executable.ends_with(suffix) {
        vec![executable.to_owned()]
    } else {
        vec![executable.to_owned(), format!("{executable}{suffix}")]
    }
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    executable_permissions(path)
}

#[cfg(unix)]
fn executable_permissions(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable_permissions(_: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_an_executable_in_a_known_path() {
        let current = env::current_exe().expect("test executable path");
        let directory = current.parent().expect("test executable directory");
        let name = current
            .file_name()
            .and_then(|value| value.to_str())
            .expect("test executable name");
        let found = find_executable_in(name, [directory.to_path_buf()]);
        assert_eq!(found.as_deref(), Some(current.as_path()));
    }

    #[test]
    fn reports_a_missing_executable() {
        let found = find_executable_in("bot-provider-that-does-not-exist", []);
        assert_eq!(found, None);
    }

    #[test]
    fn keeps_provider_order_stable() {
        let providers = discover_providers();
        assert_eq!(
            providers.first().map(ProviderDescriptor::id),
            Some(&ProviderId::Grok)
        );
        assert_eq!(providers.len(), 4);
    }
}
