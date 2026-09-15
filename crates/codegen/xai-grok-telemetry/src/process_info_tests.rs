// Modified by the Bot project on 2026-09-13: Removed release-channel process tests.

use super::{
    Entrypoint, Interactivity, LeaderMode, ProcessIdentity, entrypoint, identity, set_identity,
};

#[test]
fn the_first_recorded_identity_wins_whole_and_wire_values_are_stable() {
    let first = ProcessIdentity {
        entrypoint: Entrypoint::Cli,
        leader: LeaderMode::Standalone,
        interactivity: Interactivity::Unattended,
    };
    set_identity(first);
    set_identity(ProcessIdentity {
        entrypoint: Entrypoint::Leader,
        leader: LeaderMode::Attached,
        interactivity: Interactivity::Interactive,
    });
    assert_eq!(identity(), Some(first));
    assert_eq!(entrypoint(), Some(Entrypoint::Cli));

    let labels: Vec<&str> = Entrypoint::ALL
        .iter()
        .map(|entrypoint| entrypoint.as_ref())
        .collect();
    assert_eq!(
        labels,
        [
            "embedded",
            "leader",
            "pager",
            "cli",
            "headless",
            "workspace"
        ]
    );
}
