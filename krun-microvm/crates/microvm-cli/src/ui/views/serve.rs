// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use crate::ui::app::App;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

pub fn render_serve(frame: &mut Frame, area: Rect, app: &App) {
    if app.serve_items.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No active AI model serving engines found.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  To serve an AI model, run: cro serve --model <model-or-path> [--engine vllm|llamacpp|uzu]",
                Style::default().fg(Color::Cyan),
            )),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" AI Model Serving Engines ")
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        frame.render_widget(empty, area);
        return;
    }

    let rows: Vec<Row> = app
        .serve_items
        .iter()
        .enumerate()
        .map(|(idx, s)| {
            let is_selected = idx == app.selected_serve;
            let (status_icon, status_color) = match s.status.as_str() {
                "RUNNING" => ("● READY", Color::Green),
                "PAUSED" => ("❚❚ PAUSED", Color::Yellow),
                _ => ("○ STOPPED", Color::DarkGray),
            };

            let row_style = if is_selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(s.vm_id.clone()),
                Cell::from(s.model_name.clone()),
                Cell::from(Span::styled(
                    s.engine.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Cell::from(Span::styled(status_icon, Style::default().fg(status_color))),
                Cell::from(s.base_url.clone()),
                Cell::from(format!("{} MiB / {} vCPUs", s.memory_mb, s.vcpus)),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(16),
        Constraint::Length(24),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(28),
        Constraint::Min(16),
    ];

    let header_row = Row::new(vec![
        Cell::from("VM ID").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("MODEL").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("ENGINE").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("STATUS").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("OPENAI ENDPOINT").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("RESOURCES").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ])
    .bottom_margin(1);

    let table = Table::new(rows, widths).header(header_row).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Active AI Model Serving Engines ")
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(table, area);
}
