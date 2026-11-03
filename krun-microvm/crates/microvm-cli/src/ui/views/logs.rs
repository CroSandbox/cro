// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use crate::ui::app::App;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub fn render_logs(frame: &mut Frame, area: Rect, app: &App) {
    let vm_label = app.log_vm_id.as_deref().unwrap_or("No microVM selected");

    let follow_label = if app.log_follow {
        " [FOLLOW: ON] "
    } else {
        " [FOLLOW: PAUSED] "
    };

    let title = format!(
        " Console Logs: {} {} ({}/{} lines) ",
        vm_label,
        follow_label,
        app.log_scroll_offset + 1,
        app.log_buffer.len()
    );

    let border_color = if app.log_follow {
        Color::Green
    } else {
        Color::Yellow
    };

    if app.log_buffer.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  No log output recorded yet for {}", vm_label),
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(border_color)),
        );
        frame.render_widget(empty, area);
        return;
    }

    // Determine slice to display based on available height
    let available_height = area.height.saturating_sub(2) as usize;
    let end_idx = (app.log_scroll_offset + 1).min(app.log_buffer.len());
    let start_idx = end_idx.saturating_sub(available_height);

    let lines: Vec<Line> = app.log_buffer[start_idx..end_idx]
        .iter()
        .map(|l| Line::from(Span::raw(l.clone())))
        .collect();

    let logs_widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(Style::default().add_modifier(Modifier::BOLD))
            .border_style(Style::default().fg(border_color)),
    );

    frame.render_widget(logs_widget, area);
}
