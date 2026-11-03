// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use crate::ui::app::App;
use crate::utils::format_bytes;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Sparkline, Table};
use ratatui::Frame;

pub fn render_instances(frame: &mut Frame, area: Rect, app: &App) {
    if app.instances.is_empty() {
        let empty_msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No microVM instances found. Run `cro run <image>` or `cro serve` to launch one.",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Instances ")
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        frame.render_widget(empty_msg, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    // Left Pane: MicroVM Table
    let rows: Vec<Row> = app
        .instances
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let is_selected = idx == app.selected_instance;
            let (status_icon, status_color) = match item.status_text.as_str() {
                "RUNNING" => ("● RUNNING", Color::Green),
                "PAUSED" => ("❚❚ PAUSED", Color::Yellow),
                _ => ("○ STOPPED", Color::DarkGray),
            };

            let port_str = item
                .state
                .port_forwards
                .iter()
                .map(|p| format!("{}:{}", p.host, p.guest))
                .collect::<Vec<_>>()
                .join(", ");

            let id_display = if item.state.id.len() > 14 {
                format!("{}...", &item.state.id[..11])
            } else {
                item.state.id.clone()
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
                Cell::from(id_display),
                Cell::from(item.state.pid.to_string()),
                Cell::from(Span::styled(status_icon, Style::default().fg(status_color))),
                Cell::from(format!("{:.1}%", item.cpu_pct)),
                Cell::from(format_bytes(item.rss_bytes)),
                Cell::from(if port_str.is_empty() {
                    "-".to_string()
                } else {
                    port_str
                }),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(16),
        Constraint::Length(8),
        Constraint::Length(12),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Min(12),
    ];

    let header_row = Row::new(vec![
        Cell::from("ID").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("PID").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("STATUS").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("CPU").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("MEM").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("PORTS").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ])
    .bottom_margin(1);

    let table = Table::new(rows, widths).header(header_row).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" MicroVMs ")
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(table, chunks[0]);

    // Right Pane: Detail Panel for selected VM
    if let Some(selected) = app.selected_vm() {
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), // Sparkline CPU
                Constraint::Length(4), // Memory Gauge
                Constraint::Min(6),    // Metadata details
            ])
            .split(chunks[1]);

        // 1. CPU Sparkline
        let default_history = std::collections::VecDeque::new();
        let history = app
            .cpu_history
            .get(&selected.state.id)
            .unwrap_or(&default_history);
        let history_slice: Vec<u64> = history.iter().copied().collect();

        let sparkline = Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" CPU History ({:.1}%) ", selected.cpu_pct))
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .data(&history_slice)
            .style(Style::default().fg(Color::Green));
        frame.render_widget(sparkline, right_chunks[0]);

        // 2. Memory Gauge
        let mem_mib = selected.state.memory_mib.unwrap_or(1024) as u64;
        let max_bytes = mem_mib * 1024 * 1024;
        let ratio = if max_bytes > 0 {
            (selected.rss_bytes as f64 / max_bytes as f64).min(1.0)
        } else {
            0.0
        };

        let gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Memory Usage ")
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .gauge_style(
                Style::default()
                    .fg(if ratio > 0.85 {
                        Color::Red
                    } else if ratio > 0.6 {
                        Color::Yellow
                    } else {
                        Color::Cyan
                    })
                    .bg(Color::DarkGray),
            )
            .ratio(ratio)
            .label(format!(
                "{} / {} MiB ({:.0}%)",
                format_bytes(selected.rss_bytes),
                mem_mib,
                ratio * 100.0
            ));
        frame.render_widget(gauge, right_chunks[1]);

        // 3. Metadata details
        let created_at_str = if selected.state.created_at > 0 {
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!(
                "{}s ago",
                now_secs.saturating_sub(selected.state.created_at)
            )
        } else {
            "Unknown".to_string()
        };

        let info_lines = vec![
            Line::from(vec![
                Span::styled("ID:        ", Style::default().fg(Color::Cyan)),
                Span::raw(&selected.state.id),
            ]),
            Line::from(vec![
                Span::styled("Image:     ", Style::default().fg(Color::Cyan)),
                Span::raw(&selected.state.image),
            ]),
            Line::from(vec![
                Span::styled("vCPUs:     ", Style::default().fg(Color::Cyan)),
                Span::raw(format!("{}", selected.state.vcpus.unwrap_or(1))),
            ]),
            Line::from(vec![
                Span::styled("Uptime:    ", Style::default().fg(Color::Cyan)),
                Span::raw(created_at_str),
            ]),
            Line::from(vec![
                Span::styled("Rootfs:    ", Style::default().fg(Color::Cyan)),
                Span::raw(selected.state.instance_dir.display().to_string()),
            ]),
        ];

        let detail_box = Paragraph::new(info_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Metadata ")
                .border_style(Style::default().fg(Color::Cyan)),
        );
        frame.render_widget(detail_box, right_chunks[2]);
    }
}
