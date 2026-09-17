use std::ffi::OsString;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::app_view::{AppView, AuthState, TrustState};

const SOURCE: &str = "herdr:bot";
const AGENT: &str = "bot";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LifecycleState {
    Idle,
    Working,
    Blocked,
}

impl LifecycleState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    state: LifecycleState,
    session_id: Option<String>,
}

enum WorkerMessage {
    Report(Snapshot),
    Release,
}

struct Config {
    binary: OsString,
    pane_id: String,
}

pub(crate) struct Reporter {
    sender: Option<Sender<WorkerMessage>>,
    worker: Option<JoinHandle<()>>,
    last_snapshot: Option<Snapshot>,
}

impl Reporter {
    pub(crate) fn from_env() -> Self {
        let Some(config) = Config::from_env() else {
            return Self {
                sender: None,
                worker: None,
                last_snapshot: None,
            };
        };
        let starting = Snapshot::starting();
        let sequence = sequence_seed();
        report(&config, sequence, &starting);
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut sequence = sequence;
            let mut session_id = None;
            while let Ok(message) = receiver.recv() {
                match message {
                    WorkerMessage::Report(snapshot) => {
                        if snapshot.session_id != session_id {
                            if let Some(next_session_id) = snapshot.session_id.as_deref() {
                                sequence = sequence.saturating_add(1);
                                report_session(&config, sequence, next_session_id);
                            }
                            session_id.clone_from(&snapshot.session_id);
                        }
                        sequence = sequence.saturating_add(1);
                        report(&config, sequence, &snapshot);
                    }
                    WorkerMessage::Release => {
                        sequence = sequence.saturating_add(1);
                        release(&config, sequence);
                        break;
                    }
                }
            }
        });
        Self {
            sender: Some(sender),
            worker: Some(worker),
            last_snapshot: Some(starting),
        }
    }

    pub(crate) fn sync(&mut self, app: &AppView) {
        let snapshot = Snapshot::from_app(app);
        if self.last_snapshot.as_ref() == Some(&snapshot) {
            return;
        }
        let Some(sender) = self.sender.as_ref() else {
            return;
        };
        if sender.send(WorkerMessage::Report(snapshot.clone())).is_ok() {
            self.last_snapshot = Some(snapshot);
        }
    }
}

fn sequence_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .try_into()
        .unwrap_or(u64::MAX - 1)
}

impl Drop for Reporter {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(WorkerMessage::Release);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Config {
    fn from_env() -> Option<Self> {
        if std::env::var("HERDR_ENV").as_deref() != Ok("1") {
            return None;
        }
        let pane_id = std::env::var("HERDR_PANE_ID").ok()?;
        if pane_id.trim().is_empty() {
            return None;
        }
        let binary = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| OsString::from("herdr"));
        Some(Self { binary, pane_id })
    }
}

impl Snapshot {
    fn starting() -> Self {
        Self {
            state: LifecycleState::Working,
            session_id: None,
        }
    }

    fn from_app(app: &AppView) -> Self {
        let blocked = !matches!(app.auth_state, AuthState::Done)
            || matches!(app.trust_state, TrustState::Pending { .. })
            || app.agents.values().any(agent_is_blocked);
        let working = app.reconnect_pending || app.agents.values().any(agent_is_working);
        Self {
            state: resolve_state(blocked, working),
            session_id: app.active_session_id().map(str::to_owned),
        }
    }
}

fn resolve_state(blocked: bool, working: bool) -> LifecycleState {
    if blocked {
        LifecycleState::Blocked
    } else if working {
        LifecycleState::Working
    } else {
        LifecycleState::Idle
    }
}

fn agent_is_blocked(agent: &super::agent_view::AgentView) -> bool {
    !agent.permission_queue.is_empty()
        || agent.question_view.is_some()
        || agent.elicitation_view.is_some()
        || agent.pending_elicitation.is_some()
        || agent.plan_approval_view.is_some()
        || agent.cancel_turn_view.is_some()
}

fn agent_is_working(agent: &super::agent_view::AgentView) -> bool {
    agent.session.state.is_busy()
        || agent.session.loading_replay
        || agent.session.model_switch_pending
        || agent.running_wake_turn.is_some()
}

fn report(config: &Config, sequence: u64, snapshot: &Snapshot) {
    let mut command = Command::new(&config.binary);
    command.args([
        "pane",
        "report-agent",
        &config.pane_id,
        "--source",
        SOURCE,
        "--agent",
        AGENT,
        "--state",
        snapshot.state.as_str(),
        "--seq",
        &sequence.to_string(),
    ]);
    if let Some(session_id) = snapshot.session_id.as_deref() {
        command.args(["--agent-session-id", session_id]);
    }
    run(command);
}

fn report_session(config: &Config, sequence: u64, session_id: &str) {
    let mut command = Command::new(&config.binary);
    command.args([
        "pane",
        "report-agent-session",
        &config.pane_id,
        "--source",
        SOURCE,
        "--agent",
        AGENT,
        "--seq",
        &sequence.to_string(),
        "--agent-session-id",
        session_id,
        "--session-start-source",
        "startup",
    ]);
    run(command);
}

fn release(config: &Config, sequence: u64) {
    let mut command = Command::new(&config.binary);
    command.args([
        "pane",
        "release-agent",
        &config.pane_id,
        "--source",
        SOURCE,
        "--agent",
        AGENT,
        "--seq",
        &sequence.to_string(),
    ]);
    run(command);
}

fn run(mut command: Command) {
    xai_tty_utils::detach_std_command(&mut command);
    #[allow(
        clippy::disallowed_methods,
        reason = "the child is enrolled immediately after spawn"
    )]
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return;
    };
    let Ok(group) = xai_tty_utils::global_process_scope().enroll_std(&child) else {
        let _ = child.kill();
        let _ = xai_tty_utils::wait_child_bounded(&mut child, xai_tty_utils::KILL_REAP_TIMEOUT);
        return;
    };
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                drop(group);
                return;
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                if group.kill().is_err() {
                    let _ = child.kill();
                }
                let _ =
                    xai_tty_utils::wait_child_bounded(&mut child, xai_tty_utils::KILL_REAP_TIMEOUT);
                return;
            }
            Err(_) => {
                if group.kill().is_err() {
                    let _ = child.kill();
                }
                let _ =
                    xai_tty_utils::wait_child_bounded(&mut child, xai_tty_utils::KILL_REAP_TIMEOUT);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::agent::AgentState;

    #[test]
    fn blocked_state_has_priority() {
        assert_eq!(resolve_state(true, true), LifecycleState::Blocked);
        assert_eq!(resolve_state(false, true), LifecycleState::Working);
        assert_eq!(resolve_state(false, false), LifecycleState::Idle);
    }

    #[test]
    fn startup_claim_precedes_provider_session_identity() {
        assert_eq!(
            Snapshot::starting(),
            Snapshot {
                state: LifecycleState::Working,
                session_id: None,
            }
        );
    }

    #[test]
    fn snapshot_tracks_the_active_session() {
        let app = crate::app::app_view::tests::test_app_with_agent();
        let snapshot = Snapshot::from_app(&app);
        assert_eq!(snapshot.state, LifecycleState::Idle);
        assert_eq!(snapshot.session_id.as_deref(), Some("test-session"));
    }

    #[test]
    fn running_turn_reports_working() {
        let mut app = crate::app::app_view::tests::test_app_with_agent();
        let agent_id = app.active_view.agent_id().unwrap();
        app.agents.get_mut(&agent_id).unwrap().session.state = AgentState::TurnRunning;
        assert_eq!(Snapshot::from_app(&app).state, LifecycleState::Working);
    }

    #[test]
    fn authentication_reports_blocked() {
        let mut app = crate::app::app_view::tests::test_app_with_agent();
        let agent_id = app.active_view.agent_id().unwrap();
        app.agents.get_mut(&agent_id).unwrap().session.state = AgentState::TurnRunning;
        app.auth_state = AuthState::Pending { error: None };
        assert_eq!(Snapshot::from_app(&app).state, LifecycleState::Blocked);
    }
}
