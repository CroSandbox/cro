// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use crate::ui::app::{App, ServeFocus};
use crate::utils::{format_bytes, truncate_str};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

pub fn render_serve(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(45), // Active Serves
            Constraint::Percentage(55), // Discovered Local Models
        ])
        .split(area);

    render_active_serves(frame, chunks[0], app);
    render_discovered_models(frame, chunks[1], app);
}

fn render_active_serves(frame: &mut Frame, area: Rect, app: &App) {
    let is_focused = app.serve_focus == ServeFocus::Active;
    let border_color = if is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let title = format!(
        " Active AI Serving MicroVMs ({}) {} ",
        app.serve_items.len(),
        if is_focused { "◀ [FOCUSED]" } else { "" }
    );

    if app.serve_items.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No active AI model serving microVMs currently running.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  Select a discovered model below and press [s] or [Enter] to launch an isolated server.",
                Style::default().fg(Color::Cyan),
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

    let rows: Vec<Row> = app
        .serve_items
        .iter()
        .enumerate()
        .map(|(idx, s)| {
            let is_selected = is_focused && idx == app.selected_serve;
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
                Cell::from(truncate_str(&s.model_name, 22)),
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
        Constraint::Length(14),
        Constraint::Length(24),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(26),
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
            .title(title)
            .border_style(Style::default().fg(border_color)),
    );

    frame.render_widget(table, area);
}

fn render_discovered_models(frame: &mut Frame, area: Rect, app: &App) {
    let is_focused = app.serve_focus == ServeFocus::Discovered;
    let border_color = if is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let title = format!(
        " Discovered Local Models ({}) - [s/Enter] Serve | [d] Rescan | [w] Toggle Focus {} ",
        app.discovered_models.len(),
        if is_focused { "◀ [FOCUSED]" } else { "" }
    );

    if app.discovered_models.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No local models detected across standard directories (Hugging Face, LM Studio, Ollama, cache).",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  Press [d] to rescan or specify directories with: cro model discover --path <dir>",
                Style::default().fg(Color::Yellow),
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

    let rows: Vec<Row> = app
        .discovered_models
        .iter()
        .enumerate()
        .map(|(idx, m)| {
            let is_selected = is_focused && idx == app.selected_discovered;
            let row_style = if is_selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let rec_mb =
                ((m.size_bytes + (4096 * 256 * 1024) + (1024 * 1024 * 1024) + (512 * 1024 * 1024))
                    / (1024 * 1024)) as u32;

            Row::new(vec![
                Cell::from(truncate_str(&m.name, 26)),
                Cell::from(Span::styled(
                    m.format.to_string(),
                    Style::default().fg(Color::Cyan),
                )),
                Cell::from(format_bytes(m.size_bytes)),
                Cell::from(m.source.to_string()),
                Cell::from(Span::styled(
                    format!("~{} MiB", rec_mb),
                    Style::default().fg(Color::Green),
                )),
                Cell::from(truncate_str(&m.path.to_string_lossy(), 45)),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(28),
        Constraint::Length(14),
        Constraint::Length(12),
        Constraint::Length(18),
        Constraint::Length(14),
        Constraint::Min(20),
    ];

    let header_row = Row::new(vec![
        Cell::from("NAME").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("FORMAT").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("SIZE").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("ECOSYSTEM").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("REC RAM").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("PATH").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ])
    .bottom_margin(1);

    let table = Table::new(rows, widths).header(header_row).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(border_color)),
    );

    frame.render_widget(table, area);
}
