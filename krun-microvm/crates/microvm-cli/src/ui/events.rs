// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Asynchronous terminal event stream reader.

use crossterm::event::{Event as CrosstermEvent, KeyEvent};
use futures::StreamExt;
use std::time::Duration;
use tokio::time::interval;

/// Events handled by the cockpit event loop.
#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Resize(u16, u16),
    Tick,
}

pub struct EventHandler {
    receiver: tokio::sync::mpsc::UnboundedReceiver<Event>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let _sender = sender.clone();

        tokio::spawn(async move {
            let mut reader = crossterm::event::EventStream::new();
            let mut tick = interval(tick_rate);

            loop {
                let tick_delay = tick.tick();
                let crossterm_event = reader.next();

                tokio::select! {
                    _ = tick_delay => {
                        if sender.send(Event::Tick).is_err() {
                            break;
                        }
                    }
                    Some(Ok(evt)) = crossterm_event => {
                        match evt {
                            CrosstermEvent::Key(key) => {
                                if sender.send(Event::Key(key)).is_err() {
                                    break;
                                }
                            }
                            CrosstermEvent::Resize(w, h)
                                if sender.send(Event::Resize(w, h)).is_err() =>
                            {
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        Self { receiver }
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.receiver.recv().await
    }
}
