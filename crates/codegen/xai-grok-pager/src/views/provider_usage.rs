use bot_core::{ProviderUsage, UsageLimitWindow};
use chrono::{DateTime, Utc};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

pub fn status_line(usage: &ProviderUsage, hovered: bool, theme: &Theme) -> Option<Line<'static>> {
    let quota = usage.longest_window().map(|(_, window)| window);
    if quota.is_none() && usage.lifetime_tokens.is_none() {
        return None;
    }
    let modifier = if hovered {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };
    let mut spans = Vec::new();
    if let Some(window) = quota {
        let remaining = window.remaining_percent();
        let color = if remaining <= 5.0 {
            theme.accent_error
        } else if remaining <= 20.0 {
            theme.warning
        } else {
            theme.accent_success
        };
        spans.push(Span::styled(
            format!(
                "{} {}% left",
                compact_duration(window.duration_minutes),
                format_percent(remaining)
            ),
            Style::default()
                .fg(color)
                .bg(theme.bg_base)
                .add_modifier(modifier),
        ));
    }
    if let Some(tokens) = usage.lifetime_tokens {
        if !spans.is_empty() {
            spans.push(Span::styled(
                " · ",
                Style::default().fg(theme.gray_dim).bg(theme.bg_base),
            ));
        }
        spans.push(Span::styled(
            format!("{} tokens", compact_tokens(tokens)),
            Style::default()
                .fg(theme.text_secondary)
                .bg(theme.bg_base)
                .add_modifier(modifier),
        ));
    }
    Some(Line::from(spans))
}

pub fn detail_lines(usage: &ProviderUsage, theme: &Theme) -> Vec<Line<'static>> {
    let header = Style::default()
        .fg(theme.text_primary)
        .add_modifier(Modifier::BOLD);
    let label = Style::default().fg(theme.text_secondary);
    let value = Style::default().fg(theme.text_primary);
    let mut lines = vec![Line::styled("Account usage", header)];
    lines.push(Line::from(vec![
        Span::styled("Provider: ", label),
        Span::styled(usage.provider.label().to_owned(), value),
    ]));
    if let Some(tokens) = usage.lifetime_tokens {
        lines.push(Line::from(vec![
            Span::styled("Lifetime tokens: ", label),
            Span::styled(format_integer(tokens), value),
        ]));
    }
    for limit in &usage.limits {
        lines.push(Line::default());
        lines.push(Line::styled(limit.name.clone(), header));
        if let Some(model) = &limit.model {
            lines.push(Line::from(vec![
                Span::styled("Model: ", label),
                Span::styled(model.clone(), value),
            ]));
        }
        for window in &limit.windows {
            lines.push(window_line(window, label, value));
        }
    }
    if usage.lifetime_tokens.is_none() && usage.limits.is_empty() {
        lines.push(Line::styled(
            "The provider did not return account usage.",
            Style::default().fg(theme.gray_dim),
        ));
    }
    lines
}

pub fn detail_text(usage: &ProviderUsage, theme: &Theme) -> String {
    detail_lines(usage, theme)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn window_line(window: &UsageLimitWindow, label_style: Style, value_style: Style) -> Line<'static> {
    let reset = window
        .resets_at
        .and_then(timestamp)
        .map(|time| format!(" · resets {}", time.format("%Y-%m-%d %H:%M UTC")))
        .unwrap_or_default();
    Line::from(vec![
        Span::styled(
            format!(
                "{}: ",
                detailed_duration(window.duration_minutes, &window.label)
            ),
            label_style,
        ),
        Span::styled(
            format!(
                "{}% left · {}% used{reset}",
                format_percent(window.remaining_percent()),
                format_percent(window.used_percent)
            ),
            value_style,
        ),
    ])
}

fn timestamp(value: i64) -> Option<DateTime<Utc>> {
    let seconds = if value.abs() >= 10_000_000_000 {
        value / 1_000
    } else {
        value
    };
    DateTime::from_timestamp(seconds, 0)
}

fn compact_duration(minutes: Option<u64>) -> String {
    match minutes {
        Some(value) if value > 0 && value % 1_440 == 0 => format!("{}d", value / 1_440),
        Some(value) if value > 0 && value % 60 == 0 => format!("{}h", value / 60),
        Some(value) => format!("{value}m"),
        None => "quota".to_owned(),
    }
}

fn detailed_duration(minutes: Option<u64>, fallback: &str) -> String {
    match minutes {
        Some(value) if value > 0 && value % 1_440 == 0 => plural(value / 1_440, "day"),
        Some(value) if value > 0 && value % 60 == 0 => plural(value / 60, "hour"),
        Some(value) => plural(value, "minute"),
        None => fallback.to_owned(),
    }
}

fn plural(value: u64, unit: &str) -> String {
    format!("{value} {unit}{}", if value == 1 { "" } else { "s" })
}

fn format_percent(value: f64) -> String {
    if value.fract().abs() < 0.05 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn compact_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000_000 {
        compact_decimal(tokens, 1_000_000_000, "B")
    } else if tokens >= 1_000_000 {
        compact_decimal(tokens, 1_000_000, "M")
    } else if tokens >= 1_000 {
        compact_decimal(tokens, 1_000, "k")
    } else {
        tokens.to_string()
    }
}

fn compact_decimal(value: u64, divisor: u64, suffix: &str) -> String {
    let scaled = value as f64 / divisor as f64;
    format!("{scaled:.1}{suffix}").replace(&format!(".0{suffix}"), suffix)
}

fn format_integer(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use bot_core::{ProviderId, UsageLimit};

    fn usage() -> ProviderUsage {
        ProviderUsage {
            provider: ProviderId::Codex,
            lifetime_tokens: Some(1_234_567),
            limits: vec![UsageLimit {
                id: Some("codex".to_owned()),
                name: "Codex".to_owned(),
                model: Some("gpt-5.6-sol".to_owned()),
                windows: vec![
                    UsageLimitWindow {
                        label: "Primary".to_owned(),
                        used_percent: 37.0,
                        duration_minutes: Some(300),
                        resets_at: Some(1_789_238_400),
                    },
                    UsageLimitWindow {
                        label: "Secondary".to_owned(),
                        used_percent: 8.0,
                        duration_minutes: Some(10_080),
                        resets_at: Some(1_789_843_200),
                    },
                ],
            }],
        }
    }

    #[test]
    fn snapshots_compact_status_content() {
        let line = status_line(&usage(), false, &Theme::groknight()).expect("status line");
        insta::assert_snapshot!(line.to_string(), @"7d 92% left · 1.2M tokens");
    }

    #[test]
    fn snapshots_account_usage_details() {
        let text = detail_text(&usage(), &Theme::groknight());
        insta::assert_snapshot!(text, @r###"
        Account usage
        Provider: Codex
        Lifetime tokens: 1,234,567

        Codex
        Model: gpt-5.6-sol
        5 hours: 63% left · 37% used · resets 2026-09-12 18:40 UTC
        7 days: 92% left · 8% used · resets 2026-09-19 18:40 UTC
        "###);
    }

    #[test]
    fn omits_a_status_when_the_provider_returned_no_metrics() {
        let usage = ProviderUsage {
            provider: ProviderId::Claude,
            lifetime_tokens: None,
            limits: Vec::new(),
        };
        assert!(status_line(&usage, false, &Theme::groknight()).is_none());
    }
}
