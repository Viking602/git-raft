use crate::events::Event;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: Uuid,
    pub command: String,
    pub status: RunStatus,
    pub started_at_ms: u128,
    pub finished_at_ms: Option<u128>,
    pub backup_ref: Option<String>,
    pub conflicts: Vec<String>,
    pub verify_passed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone)]
pub struct RunStore {
    run_dir: PathBuf,
    record: Arc<Mutex<RunRecord>>,
}

impl RunStore {
    pub fn create(git_dir: PathBuf, run_id: Uuid, command: &str) -> Result<Self> {
        let runs_dir = git_dir.join("git-raft").join("runs");
        fs::create_dir_all(&runs_dir)?;
        let run_dir = runs_dir.join(run_id.to_string());
        fs::create_dir_all(&run_dir)?;
        let record = RunRecord {
            run_id,
            command: command.to_string(),
            status: RunStatus::Running,
            started_at_ms: now_ms(),
            finished_at_ms: None,
            backup_ref: None,
            conflicts: Vec::new(),
            verify_passed: None,
        };
        let store = Self {
            run_dir,
            record: Arc::new(Mutex::new(record)),
        };
        store.write_record()?;
        Ok(store)
    }

    pub fn run_id(&self) -> Uuid {
        self.record.lock().expect("lock").run_id
    }

    pub fn append_event(&self, event: &Event) -> Result<()> {
        let path = self.run_dir.join("events.ndjson");
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{}", serde_json::to_string(event)?)?;
        Ok(())
    }

    pub fn set_backup_ref(&self, backup_ref: Option<String>) -> Result<()> {
        self.record.lock().expect("lock").backup_ref = backup_ref;
        self.write_record()
    }

    pub fn set_conflicts(&self, conflicts: Vec<String>) -> Result<()> {
        self.record.lock().expect("lock").conflicts = conflicts;
        self.write_record()
    }

    pub fn finish(
        &self,
        status: RunStatus,
        backup_ref: Option<String>,
        verify_passed: Option<bool>,
    ) -> Result<()> {
        let mut record = self.record.lock().expect("lock");
        record.status = status;
        if backup_ref.is_some() {
            record.backup_ref = backup_ref;
        }
        if verify_passed.is_some() {
            record.verify_passed = verify_passed;
        }
        record.finished_at_ms = Some(now_ms());
        drop(record);
        self.write_record()
    }

    pub fn write_json<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        fs::write(self.run_dir.join(name), serde_json::to_vec_pretty(value)?)?;
        Ok(())
    }

    pub fn list(git_dir: PathBuf) -> Result<Vec<RunRecord>> {
        let runs_dir = git_dir.join("git-raft").join("runs");
        if !runs_dir.exists() {
            return Ok(Vec::new());
        }
        let mut records = fs::read_dir(runs_dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path().join("run.json"))
            .filter(|path| path.exists())
            .map(|path| {
                let bytes = fs::read(path)?;
                Ok(serde_json::from_slice::<RunRecord>(&bytes)?)
            })
            .collect::<Result<Vec<_>>>()?;
        records.sort_by(|left, right| right.started_at_ms.cmp(&left.started_at_ms));
        Ok(records)
    }

    pub fn load(git_dir: PathBuf, run_id: &str) -> Result<RunRecord> {
        let path = git_dir
            .join("git-raft")
            .join("runs")
            .join(run_id)
            .join("run.json");
        let bytes = fs::read(&path).with_context(|| format!("missing run.json for {run_id}"))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn read_events(git_dir: PathBuf, run_id: &str) -> Result<Vec<String>> {
        let path = git_dir
            .join("git-raft")
            .join("runs")
            .join(run_id)
            .join("events.ndjson");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(path)?;
        Ok(content.lines().map(str::to_string).collect())
    }

    fn write_record(&self) -> Result<()> {
        let record = self.record.lock().expect("lock").clone();
        fs::write(
            self.run_dir.join("run.json"),
            serde_json::to_vec_pretty(&record)?,
        )?;
        Ok(())
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_millis()
}
