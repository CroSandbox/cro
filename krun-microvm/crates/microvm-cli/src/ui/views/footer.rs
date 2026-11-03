// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use crate::ui::app::{App, Tab};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = Vec::new();

    // If there is an active notification toast, show it on the left
    if let Some(msg) = app.active_notification() {
        spans.push(Span::styled(
            format!(" 🔔 {} ", msg),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }

    // Contextual shortcuts based on active tab
    match app.current_tab {
        Tab::Instances => {
            spans.extend(vec![
                Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
                Span::raw(" Next Tab  "),
                Span::styled("[↑/↓]", Style::default().fg(Color::Cyan)),
                Span::raw(" Select  "),
                Span::styled("[p]", Style::default().fg(Color::Yellow)),
                Span::raw(" Pause  "),
                Span::styled("[r]", Style::default().fg(Color::Green)),
                Span::raw(" Resume  "),
                Span::styled("[s]", Style::default().fg(Color::Red)),
                Span::raw(" Stop  "),
                Span::styled("[d]", Style::default().fg(Color::DarkGray)),
                Span::raw(" Delete  "),
                Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
                Span::raw(" View Logs  "),
                Span::styled("[q]", Style::default().fg(Color::Cyan)),
                Span::raw(" Quit"),
            ]);
        }
        Tab::Logs => {
            spans.extend(vec![
                Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
                Span::raw(" Next Tab  "),
                Span::styled("[Space]", Style::default().fg(Color::Yellow)),
                Span::raw(if app.log_follow {
                    " Pause Auto-Scroll  "
                } else {
                    " Resume Auto-Scroll  "
                }),
                Span::styled("[↑/↓]", Style::default().fg(Color::Cyan)),
                Span::raw(" Scroll  "),
                Span::styled("[PgUp/PgDn]", Style::default().fg(Color::Cyan)),
                Span::raw(" Page Scroll  "),
                Span::styled("[q]", Style::default().fg(Color::Cyan)),
                Span::raw(" Quit"),
            ]);
        }
        Tab::Serve => {
            spans.extend(vec![
                Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
                Span::raw(" Next Tab  "),
                Span::styled("[↑/↓]", Style::default().fg(Color::Cyan)),
                Span::raw(" Select  "),
                Span::styled("[w]", Style::default().fg(Color::Cyan)),
                Span::raw(" Toggle Pane  "),
                Span::styled("[d]", Style::default().fg(Color::Yellow)),
                Span::raw(" Rescan  "),
                Span::styled("[s/Enter]", Style::default().fg(Color::Green)),
                Span::raw(" Serve  "),
                Span::styled("[q]", Style::default().fg(Color::Cyan)),
                Span::raw(" Quit"),
            ]);
        }
        Tab::Approvals => {
            spans.extend(vec![
                Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
                Span::raw(" Next Tab  "),
                Span::styled("[↑/↓]", Style::default().fg(Color::Cyan)),
                Span::raw(" Select  "),
                Span::styled("[y]", Style::default().fg(Color::Green)),
                Span::raw(" Approve  "),
                Span::styled("[n]", Style::default().fg(Color::Red)),
                Span::raw(" Reject  "),
                Span::styled("[q]", Style::default().fg(Color::Cyan)),
                Span::raw(" Quit"),
            ]);
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
