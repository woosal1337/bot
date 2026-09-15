// Modified by the Bot project on 2026-09-13: Removed release-channel process state.

//! First-call-wins process identity labels carried on every product event.

use std::sync::OnceLock;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, strum::EnumCount, strum::AsRefStr, strum::IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum Entrypoint {
    /// Agent inside the interactive client, or the dedicated stdio agent.
    Embedded,
    /// Shared leader agent process serving many sessions.
    Leader,
    /// Interactive client process whose agent lives in a leader.
    Pager,
    /// One-shot command.
    Cli,
    /// Headless agent session, no TUI (scripts, CI, SDK harnesses).
    Headless,
    /// Remote agent server process.
    Workspace,
}

impl Entrypoint {
    pub(crate) const ALL: [Entrypoint; 6] = [
        Entrypoint::Embedded,
        Entrypoint::Leader,
        Entrypoint::Pager,
        Entrypoint::Cli,
        Entrypoint::Headless,
        Entrypoint::Workspace,
    ];
}

const _: () = assert!(Entrypoint::ALL.len() == <Entrypoint as strum::EnumCount>::COUNT);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaderMode {
    Attached,
    Standalone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interactivity {
    Interactive,
    Unattended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub entrypoint: Entrypoint,
    pub leader: LeaderMode,
    pub interactivity: Interactivity,
}

static IDENTITY: OnceLock<ProcessIdentity> = OnceLock::new();

pub fn set_identity(identity: ProcessIdentity) {
    let _ = IDENTITY.set(identity);
}

pub(crate) fn identity() -> Option<ProcessIdentity> {
    IDENTITY.get().copied()
}

pub(crate) fn entrypoint() -> Option<Entrypoint> {
    identity().map(|i| i.entrypoint)
}

#[cfg(test)]
#[path = "process_info_tests.rs"]
mod tests;
