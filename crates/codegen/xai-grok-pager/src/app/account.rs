use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountStatus {
    pub provider: String,
    pub signed_in: bool,
    #[serde(default)]
    pub auth_method: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub ordinary_usage_allowed: Option<bool>,
    #[serde(default)]
    pub reset_credits: Option<i64>,
    #[serde(default)]
    pub rate_limits: Vec<ProviderRateLimit>,
    #[serde(default)]
    pub rate_limits_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRateLimit {
    pub name: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub primary: Option<ProviderRateLimitWindow>,
    #[serde(default)]
    pub secondary: Option<ProviderRateLimitWindow>,
    #[serde(default)]
    pub credits: Option<ProviderCredits>,
    #[serde(default)]
    pub limit_reached: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRateLimitWindow {
    pub used_percent: i32,
    #[serde(default)]
    pub window_minutes: Option<i64>,
    #[serde(default)]
    pub resets_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCredits {
    pub has_credits: bool,
    pub unlimited: bool,
    #[serde(default)]
    pub balance: Option<String>,
}

impl ProviderAccountStatus {
    pub fn render_markdown(&self) -> String {
        let mut lines = vec![format!("# {} account", self.provider), String::new()];
        lines.push(format!(
            "**Status:** {}",
            if self.signed_in {
                "Signed in"
            } else {
                "Signed out"
            }
        ));
        if let Some(email) = &self.email {
            lines.push(format!("**Email:** {email}"));
        }
        if let Some(plan) = &self.plan {
            lines.push(format!("**Plan:** {plan}"));
        }
        if let Some(method) = &self.auth_method {
            lines.push(format!("**Authentication:** {method}"));
        }
        if !self.signed_in {
            lines.extend([
                String::new(),
                "Use `/login` or `/login device` to sign in.".to_owned(),
            ]);
            return lines.join("\n");
        }
        lines.extend([String::new(), "## Rate limits".to_owned()]);
        if self.rate_limits.is_empty() {
            lines.push(
                self.rate_limits_error
                    .clone()
                    .unwrap_or_else(|| "No rate-limit data is available.".to_owned()),
            );
        } else {
            for limit in &self.rate_limits {
                lines.extend([String::new(), format!("### {}", limit.name)]);
                if let Some(model) = &limit.model {
                    lines.push(format!("**Model:** {model}"));
                }
                if let Some(primary) = &limit.primary {
                    lines.push(format_window("Primary", primary));
                }
                if let Some(secondary) = &limit.secondary {
                    lines.push(format_window("Secondary", secondary));
                }
                if let Some(credits) = &limit.credits {
                    lines.push(format_credits(credits));
                }
                if let Some(reached) = &limit.limit_reached {
                    lines.push(format!("**Limit state:** {}", display_value(reached)));
                }
            }
        }
        if let Some(allowed) = self.ordinary_usage_allowed {
            lines.extend([
                String::new(),
                format!(
                    "**Included usage:** {}",
                    if allowed { "Available" } else { "Unavailable" }
                ),
            ]);
        }
        if let Some(credits) = self.reset_credits {
            lines.push(format!("**Reset credits:** {credits}"));
        }
        lines.join("\n")
    }
}

fn format_window(label: &str, window: &ProviderRateLimitWindow) -> String {
    let remaining = (100 - window.used_percent).clamp(0, 100);
    let duration = window
        .window_minutes
        .map(format_duration)
        .unwrap_or_else(|| label.to_owned());
    let reset = window
        .resets_at
        .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
        .map(|time| format!(" · resets {}", time.format("%Y-%m-%d %H:%M UTC")))
        .unwrap_or_default();
    format!(
        "**{duration}:** {remaining}% remaining ({}% used){reset}",
        window.used_percent
    )
}

fn format_duration(minutes: i64) -> String {
    if minutes > 0 && minutes % 1_440 == 0 {
        plural(minutes / 1_440, "day")
    } else if minutes > 0 && minutes % 60 == 0 {
        plural(minutes / 60, "hour")
    } else {
        plural(minutes, "minute")
    }
}

fn plural(value: i64, unit: &str) -> String {
    format!("{value} {unit}{}", if value == 1 { "" } else { "s" })
}

fn format_credits(credits: &ProviderCredits) -> String {
    if credits.unlimited {
        return "**Credits:** Unlimited".to_owned();
    }
    match &credits.balance {
        Some(balance) => format!("**Credits:** {balance}"),
        None if credits.has_credits => "**Credits:** Available".to_owned(),
        None => "**Credits:** None".to_owned(),
    }
}

fn display_value(value: &str) -> String {
    value.replace(['_', '-'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_account_and_rate_limit_details() {
        let text = ProviderAccountStatus {
            provider: "Codex".to_owned(),
            signed_in: true,
            auth_method: Some("ChatGPT".to_owned()),
            email: Some("bot@example.com".to_owned()),
            plan: Some("Pro".to_owned()),
            ordinary_usage_allowed: Some(true),
            reset_credits: Some(2),
            rate_limits: vec![ProviderRateLimit {
                name: "Codex".to_owned(),
                model: Some("gpt-5.6-sol".to_owned()),
                primary: Some(ProviderRateLimitWindow {
                    used_percent: 37,
                    window_minutes: Some(300),
                    resets_at: Some(1_789_238_400),
                }),
                secondary: None,
                credits: Some(ProviderCredits {
                    has_credits: true,
                    unlimited: false,
                    balance: Some("12.50".to_owned()),
                }),
                limit_reached: None,
            }],
            rate_limits_error: None,
        }
        .render_markdown();
        insta::assert_snapshot!(text, @r###"
        # Codex account

        **Status:** Signed in
        **Email:** bot@example.com
        **Plan:** Pro
        **Authentication:** ChatGPT

        ## Rate limits

        ### Codex
        **Model:** gpt-5.6-sol
        **5 hours:** 63% remaining (37% used) · resets 2026-09-12 18:40 UTC
        **Credits:** 12.50

        **Included usage:** Available
        **Reset credits:** 2
        "###);
    }

    #[test]
    fn renders_signed_out_guidance() {
        let text = ProviderAccountStatus {
            provider: "Codex".to_owned(),
            signed_in: false,
            auth_method: None,
            email: None,
            plan: None,
            ordinary_usage_allowed: None,
            reset_credits: None,
            rate_limits: Vec::new(),
            rate_limits_error: None,
        }
        .render_markdown();
        assert!(text.contains("**Status:** Signed out"));
        assert!(text.contains("`/login device`"));
    }
}
