use bot_core::{ProviderUsage, UsageLimitWindow, format_reset_countdown};
use chrono::Utc;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

pub fn status_line(usage: &ProviderUsage, hovered: bool, theme: &Theme) -> Option<Line<'static>> {
    status_line_at(usage, hovered, theme, Utc::now().timestamp())
}

fn status_line_at(
    usage: &ProviderUsage,
    hovered: bool,
    theme: &Theme,
    now: i64,
) -> Option<Line<'static>> {
    let (_, window) = usage.longest_window()?;
    let modifier = if hovered {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };
    let mut spans = Vec::new();
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
    if let Some(reset) = window.resets_at {
        spans.push(Span::styled(
            format!(" · resets {}", format_reset_countdown(reset, now)),
            Style::default().fg(theme.gray_dim).bg(theme.bg_base),
        ));
    }
    Some(Line::from(spans))
}

pub fn detail_lines(usage: &ProviderUsage, theme: &Theme) -> Vec<Line<'static>> {
    detail_lines_at(usage, theme, Utc::now().timestamp())
}

fn detail_lines_at(usage: &ProviderUsage, theme: &Theme, now: i64) -> Vec<Line<'static>> {
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
            lines.push(window_line(window, label, value, now));
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

#[cfg(test)]
fn detail_text_at(usage: &ProviderUsage, theme: &Theme, now: i64) -> String {
    detail_lines_at(usage, theme, now)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn window_line(
    window: &UsageLimitWindow,
    label_style: Style,
    value_style: Style,
    now: i64,
) -> Line<'static> {
    let reset = window
        .resets_at
        .map(|time| format!(" · resets {}", format_reset_countdown(time, now)))
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
            extensions: Default::default(),
        }
    }

    #[test]
    fn snapshots_compact_status_content() {
        let line = status_line_at(&usage(), false, &Theme::groknight(), 1_789_152_000)
            .expect("status line");
        insta::assert_snapshot!(line.to_string(), @"7d 92% left · resets in 8 days");
    }

    #[test]
    fn snapshots_account_usage_details() {
        let text = detail_text_at(&usage(), &Theme::groknight(), 1_789_152_000);
        insta::assert_snapshot!(text, @r###"
        Account usage
        Provider: Codex
        Lifetime tokens: 1,234,567

        Codex
        Model: gpt-5.6-sol
        5 hours: 63% left · 37% used · resets in 1 day
        7 days: 92% left · 8% used · resets in 8 days
        "###);
    }

    #[test]
    fn omits_a_status_when_the_provider_returned_no_metrics() {
        let usage = ProviderUsage::unavailable(ProviderId::Claude);
        assert!(status_line(&usage, false, &Theme::groknight()).is_none());
    }

    #[test]
    fn keeps_lifetime_tokens_in_details_without_using_header_space() {
        let mut usage = usage();
        usage.limits.clear();

        assert!(status_line(&usage, false, &Theme::groknight()).is_none());
        assert!(detail_text(&usage, &Theme::groknight()).contains("Lifetime tokens: 1,234,567"));
    }
}
