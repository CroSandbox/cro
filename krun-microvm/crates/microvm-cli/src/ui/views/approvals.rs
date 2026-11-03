// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use crate::ui::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

pub fn render_approvals(frame: &mut Frame, area: Rect, app: &App) {
    if app.approvals.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ✨ No pending Human-in-the-Loop (HITL) approval requests.",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "  When an AI agent inside a sandbox triggers an intercepted network action, it will appear here.",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Human-in-the-Loop Approvals Queue ")
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        frame.render_widget(empty, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let rows: Vec<Row> = app
        .approvals
        .iter()
        .enumerate()
        .map(|(idx, req)| {
            let is_selected = idx == app.selected_approval;

            let row_style = if is_selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(req.id.clone()),
                Cell::from(req.agent_or_vm_id.clone()),
                Cell::from(req.method.clone()),
                Cell::from(format!("{}:{}", req.host, req.port)),
                Cell::from(Span::styled(
                    "PENDING",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Cell::from(req.summary.clone()),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(14),
        Constraint::Length(14),
        Constraint::Length(8),
        Constraint::Length(24),
        Constraint::Length(10),
        Constraint::Min(20),
    ];

    let header_row = Row::new(vec![
        Cell::from("REQUEST ID").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("AGENT / VM").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("METHOD").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("DESTINATION").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("STATUS").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("SUMMARY").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ])
    .bottom_margin(1);

    let table = Table::new(rows, widths).header(header_row).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Pending Approvals ({}) ", app.approvals.len()))
            .border_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(table, chunks[0]);

    // Detail Panel for selected request
    if let Some(selected) = app.selected_approval_req() {
        let detail_lines = vec![
            Line::from(vec![
                Span::styled(
                    "Action Intercepted: ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    "Outbound {} request intercepted by security policy",
                    selected.method
                )),
            ]),
            Line::from(vec![
                Span::styled("Target Endpoint:    ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("{}:{}{}", selected.host, selected.port, selected.path),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Origin Agent/VM:    ", Style::default().fg(Color::Cyan)),
                Span::raw(&selected.agent_or_vm_id),
            ]),
            Line::from(vec![
                Span::styled("Payload Digest:     ", Style::default().fg(Color::Cyan)),
                Span::raw(&selected.payload_sha256),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    " DECISION: ",
                    Style::default()
                        .bg(Color::Cyan)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Press "),
                Span::styled(
                    "[y]",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" to Approve egress | Press "),
                Span::styled(
                    "[n]",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" to Deny & Terminate"),
            ]),
        ];

        let detail_box = Paragraph::new(detail_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Request Details: {} ", selected.id))
                .border_style(Style::default().fg(Color::Cyan)),
        );
        frame.render_widget(detail_box, chunks[1]);
    }
}
