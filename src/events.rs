use crate::store::RunStore;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub run_id: Uuid,
    pub event_type: String,
    pub phase: Option<String>,
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    pub timestamp_ms: u128,
}

pub struct Emitter {
    json: bool,
    run_id: Uuid,
    store: Option<RunStore>,
}

impl Emitter {
    pub fn new(json: bool, run_id: Uuid, store: Option<RunStore>) -> Self {
        Self {
            json,
            run_id,
            store,
        }
    }

    pub fn json_mode(&self) -> bool {
        self.json
    }

    pub async fn emit(
        &mut self,
        event_type: &str,
        phase: Option<&str>,
        message: Option<String>,
        data: Option<Value>,
    ) -> Result<()> {
        let event = Event {
            run_id: self.run_id,
            event_type: event_type.to_string(),
            phase: phase.map(str::to_string),
            message: message.clone(),
            data,
            timestamp_ms: now_ms(),
        };
        if let Some(store) = &self.store {
            store.append_event(&event)?;
        }
        if self.json {
            println!("{}", serde_json::to_string(&event)?);
        } else {
            render_human(&event);
        }
        Ok(())
    }
}

fn render_human(event: &Event) {
    match event.event_type.as_str() {
        "git_stdout" => {
            if let Some(message) = &event.message {
                println!("{message}");
            }
        }
        "git_stderr" => {
            if let Some(message) = &event.message {
                eprintln!("{message}");
            }
        }
        _ => {
            let phase = event.phase.as_deref().unwrap_or("run");
            let message = event
                .message
                .as_deref()
                .unwrap_or(event.event_type.as_str());
            eprintln!("[{phase}] {message}");
        }
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_millis()
}
