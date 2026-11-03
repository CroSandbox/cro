// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use crate::ui::app::{App, Tab};
use crate::utils::{format_bytes, host_memory_bytes};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};
use ratatui::Frame;

pub fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(3)])
        .split(area);

    // Host status info line
    let cpus = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);
    let mem_str = host_memory_bytes()
        .map(format_bytes)
        .unwrap_or_else(|| "Unknown".to_string());
    let hypervisor = if cfg!(target_os = "macos") {
        "Apple Silicon (HVF)"
    } else {
        "Linux KVM"
    };

    let running_count = app.instances.iter().filter(|i| i.is_alive).count();
    let paused_count = app
        .instances
        .iter()
        .filter(|i| i.status_text == "PAUSED")
        .count();
    let stopped_count = app
        .instances
        .len()
        .saturating_sub(running_count + paused_count);

    let host_line = Line::from(vec![
        Span::styled(
            " cro cockpit ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("Host: {} vCPUs | {} RAM | {}", cpus, mem_str, hypervisor),
            Style::default().fg(Color::Gray),
        ),
        Span::raw(" | "),
        Span::styled(
            format!("● {} Running", running_count),
            Style::default().fg(Color::Green),
        ),
        Span::raw(" "),
        Span::styled(
            format!("❚❚ {} Paused", paused_count),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(" "),
        Span::styled(
            format!("○ {} Stopped", stopped_count),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(host_line), chunks[0]);

    // Navigation Tabs
    let tab_titles: Vec<Line> = Tab::all()
        .iter()
        .map(|t| {
            let (fg, bg) = if *t == app.current_tab {
                (Color::Cyan, Color::Reset)
            } else {
                (Color::DarkGray, Color::Reset)
            };
            Line::from(vec![Span::styled(
                format!(" {} ", t.title()),
                Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
            )])
        })
        .collect();

    let tabs_widget = Tabs::new(tab_titles)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .select(match app.current_tab {
            Tab::Instances => 0,
            Tab::Logs => 1,
            Tab::Serve => 2,
            Tab::Approvals => 3,
        })
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );

    frame.render_widget(tabs_widget, chunks[1]);
}
