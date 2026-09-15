// Modified by the Bot project on 2026-09-13: Removed provider-specific announcements.
//! Top bar component: renders cwd and git info.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use std::path::PathBuf;

use crate::git_info;
use crate::render::line_utils::truncate_line;
use crate::theme::Theme;
use crate::views::location::{location_parts, worktree_badge};

pub fn render_top_bar(area: Rect, buf: &mut Buffer, theme: &Theme) {
    let line = truncate_line(location_line(theme), area.width as usize);
    let line_width = line.width() as u16;
    buf.set_line(area.x, area.y, &line, line_width.min(area.width));
}

/// Build the `{git branch} {worktree} {cwd}` line for the welcome top bar, reading the live process cwd.
/// The caller width-truncates the returned line.
pub(crate) fn location_line(theme: &Theme) -> Line<'static> {
    let info_style = Style::default().fg(theme.gray);
    let parts = location_parts(&process_cwd());

    let mut spans: Vec<Span> = Vec::new();
    if let Some(branch) = parts.branch.as_deref() {
        let icon = git_info::branch_icon();
        let git_style = Style::default()
            .fg(theme.text_primary)
            .add_modifier(Modifier::DIM);
        spans.push(Span::styled(format!("{icon} {branch}"), git_style));
        spans.push(Span::styled(" ", info_style));
    }
    if parts.is_worktree {
        spans.push(worktree_badge(theme));
    }
    let cwd_style = Style::default().fg(theme.gray_dim);
    spans.push(Span::styled(parts.cwd_display, cwd_style));
    Line::from(spans)
}

fn process_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
