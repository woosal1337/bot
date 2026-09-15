//! `FEATURES` is the source of truth and the operator tables are hand-maintained mirrors with no compile-time check of their own.
//! This test is that check.

use xai_grok_shell::agent::config::FEATURES;

const CONFIG_REFERENCE: &str = include_str!("../docs/user-guide/26-config-reference.md");
const PUBLIC_DOCS: &str = concat!(
    include_str!("../docs/user-guide/26-config-reference.md"),
    include_str!("../docs/user-guide/05-configuration.md")
);

#[test]
fn every_registered_feature_reaches_the_operator() {
    for spec in FEATURES {
        assert!(
            CONFIG_REFERENCE.contains(&format!("`{}`", spec.path)),
            "{} has no row in the public configuration reference",
            spec.path,
        );
        assert!(
            PUBLIC_DOCS.contains(&format!("`{}`", spec.env)),
            "{} is missing from the public configuration documentation",
            spec.env,
        );
    }
}
