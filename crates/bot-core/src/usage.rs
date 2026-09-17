use serde::{Deserialize, Serialize};

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
}

impl ProviderUsage {
    pub fn longest_window(&self) -> Option<(&UsageLimit, &UsageLimitWindow)> {
        self.limits
            .iter()
            .flat_map(|limit| limit.windows.iter().map(move |window| (limit, window)))
            .max_by_key(|(_, window)| window.duration_minutes.unwrap_or_default())
    }
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
        };

        let (_, selected) = usage.longest_window().expect("quota window");
        assert_eq!(selected.label, "Secondary");
        assert_eq!(selected.remaining_percent(), 60.0);
    }

    #[test]
    fn clamps_remaining_quota() {
        assert_eq!(window("Primary", -5.0, None).remaining_percent(), 100.0);
        assert_eq!(window("Primary", 125.0, None).remaining_percent(), 0.0);
    }

    #[test]
    fn serializes_provider_usage_for_adapter_boundaries() {
        let usage = ProviderUsage {
            provider: ProviderId::Codex,
            lifetime_tokens: None,
            limits: Vec::new(),
        };

        let value = serde_json::to_value(&usage).expect("serialize usage");
        assert_eq!(value["provider"], "codex");
        assert!(value.get("lifetimeTokens").is_none());
        assert!(value.get("limits").is_none());
    }
}
