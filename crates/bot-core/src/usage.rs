use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ProviderId;

pub const PROVIDER_USAGE_UPDATED_METHOD: &str = "bot/usage/updated";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageUpdate {
    pub session_id: String,
    pub usage: ProviderUsage,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsage {
    pub provider: ProviderId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<UsageLimit>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl ProviderUsage {
    pub fn unavailable(provider: ProviderId) -> Self {
        Self {
            provider,
            lifetime_tokens: None,
            limits: Vec::new(),
            extensions: BTreeMap::new(),
        }
    }

    pub fn longest_window(&self) -> Option<(&UsageLimit, &UsageLimitWindow)> {
        self.limits
            .iter()
            .flat_map(|limit| limit.windows.iter().map(move |window| (limit, window)))
            .max_by(|(_, left), (_, right)| {
                left.duration_minutes
                    .unwrap_or_default()
                    .cmp(&right.duration_minutes.unwrap_or_default())
                    .then_with(|| left.used_percent.total_cmp(&right.used_percent))
            })
    }
}

pub fn format_reset_countdown(resets_at: i64, now: i64) -> String {
    let reset_seconds = if resets_at.unsigned_abs() >= 10_000_000_000 {
        resets_at / 1_000
    } else {
        resets_at
    };
    let remaining = reset_seconds.saturating_sub(now);
    if remaining <= 0 {
        return "due now".to_owned();
    }
    let days = remaining / 86_400;
    let hours = remaining % 86_400 / 3_600;
    let minutes = remaining % 3_600 / 60;
    if days > 0 {
        return format_countdown_parts(days, "day", hours, "hour");
    }
    if hours > 0 {
        return format_countdown_parts(hours, "hour", minutes, "minute");
    }
    if minutes > 0 {
        return format!("in {}", plural(minutes, "minute"));
    }
    "in less than 1 minute".to_owned()
}

fn format_countdown_parts(
    primary: i64,
    primary_unit: &str,
    secondary: i64,
    secondary_unit: &str,
) -> String {
    let primary = plural(primary, primary_unit);
    if secondary > 0 {
        format!("in {primary} and {}", plural(secondary, secondary_unit))
    } else {
        format!("in {primary}")
    }
}

fn plural(value: i64, unit: &str) -> String {
    format!("{value} {unit}{}", if value == 1 { "" } else { "s" })
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<UsageLimitWindow>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimitWindow {
    pub label: String,
    pub used_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_minutes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

impl UsageLimitWindow {
    pub fn remaining_percent(&self) -> f64 {
        (100.0 - self.used_percent).clamp(0.0, 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(label: &str, used_percent: f64, duration_minutes: Option<u64>) -> UsageLimitWindow {
        UsageLimitWindow {
            label: label.to_owned(),
            used_percent,
            duration_minutes,
            resets_at: None,
        }
    }

    #[test]
    fn selects_the_longest_quota_window() {
        let usage = ProviderUsage {
            provider: ProviderId::Codex,
            lifetime_tokens: Some(1_234),
            limits: vec![UsageLimit {
                id: Some("codex".to_owned()),
                name: "Codex".to_owned(),
                model: None,
                windows: vec![
                    window("Primary", 25.0, Some(300)),
                    window("Secondary", 40.0, Some(10_080)),
                ],
            }],
            extensions: BTreeMap::new(),
        };

        let (_, selected) = usage.longest_window().expect("quota window");
        assert_eq!(selected.label, "Secondary");
        assert_eq!(selected.remaining_percent(), 60.0);
    }

    #[test]
    fn selects_the_most_used_window_when_durations_match() {
        let usage = ProviderUsage {
            provider: ProviderId::Codex,
            lifetime_tokens: None,
            limits: vec![
                UsageLimit {
                    id: Some("codex".to_owned()),
                    name: "Codex".to_owned(),
                    model: None,
                    windows: vec![window("7 days", 90.0, Some(10_080))],
                },
                UsageLimit {
                    id: Some("spark".to_owned()),
                    name: "Spark".to_owned(),
                    model: Some("gpt-5.3-codex-spark".to_owned()),
                    windows: vec![window("7 days", 0.0, Some(10_080))],
                },
            ],
            extensions: BTreeMap::new(),
        };

        let (_, selected) = usage.longest_window().expect("quota window");
        assert_eq!(selected.used_percent, 90.0);
    }

    #[test]
    fn formats_reset_countdowns_for_days_hours_and_minutes() {
        let now = 1_000_000;
        assert_eq!(
            format_reset_countdown(now + 176_400, now),
            "in 2 days and 1 hour"
        );
        assert_eq!(
            format_reset_countdown(now + 7_380, now),
            "in 2 hours and 3 minutes"
        );
        assert_eq!(format_reset_countdown(now + 180, now), "in 3 minutes");
        assert_eq!(
            format_reset_countdown(now + 30, now),
            "in less than 1 minute"
        );
        assert_eq!(format_reset_countdown(now - 1, now), "due now");
    }

    #[test]
    fn clamps_remaining_quota() {
        assert_eq!(window("Primary", -5.0, None).remaining_percent(), 100.0);
        assert_eq!(window("Primary", 125.0, None).remaining_percent(), 0.0);
    }

    #[test]
    fn serializes_provider_usage_for_adapter_boundaries() {
        let usage = ProviderUsage::unavailable(ProviderId::Codex);

        let value = serde_json::to_value(&usage).expect("serialize usage");
        assert_eq!(value["provider"], "codex");
        assert!(value.get("lifetimeTokens").is_none());
        assert!(value.get("limits").is_none());
    }

    #[test]
    fn preserves_provider_specific_extensions() {
        let usage: ProviderUsage = serde_json::from_value(serde_json::json!({
            "provider": "codex",
            "providerMeter": {"remaining": 42}
        }))
        .expect("provider usage");

        assert_eq!(usage.extensions["providerMeter"]["remaining"], 42);
        let value = serde_json::to_value(usage).expect("serialize provider usage");
        assert_eq!(value["providerMeter"]["remaining"], 42);
    }
}
