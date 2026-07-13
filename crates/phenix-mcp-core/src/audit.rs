use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub audit_id: String,
    pub timestamp: String,
    pub tool: String,
    pub input: serde_json::Value,
    pub mutation_level: String,
    pub success: bool,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

pub struct AuditSink {
    entries: Mutex<Vec<AuditEntry>>,
    log_dir: Option<PathBuf>,
}

impl AuditSink {
    pub fn new(log_dir: Option<PathBuf>) -> Self {
        if let Some(ref dir) = log_dir {
            let _ = fs::create_dir_all(dir);
        }
        Self {
            entries: Mutex::new(Vec::new()),
            log_dir,
        }
    }

    pub fn record(&self, entry: AuditEntry) {
        if let Ok(mut entries) = self.entries.lock() {
            if let Some(ref dir) = self.log_dir {
                let path = dir.join(format!("{}.json", entry.audit_id));
                if let Ok(json) = serde_json::to_string_pretty(&entry) {
                    let _ = fs::write(&path, &json);
                }
            }
            entries.push(entry);
        }
    }

    pub fn generate_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub fn create_entry(
        &self,
        audit_id: String,
        tool: &str,
        input: serde_json::Value,
        mutation_level: &str,
    ) -> AuditEntryBuilder<'_> {
        AuditEntryBuilder {
            audit_id,
            timestamp: Utc::now().to_rfc3339(),
            tool: tool.to_string(),
            input,
            mutation_level: mutation_level.to_string(),
            success: false,
            summary: String::new(),
            command: None,
            exit_code: None,
            duration_ms: None,
            sink: self,
        }
    }
}

pub struct AuditEntryBuilder<'a> {
    audit_id: String,
    timestamp: String,
    tool: String,
    input: serde_json::Value,
    mutation_level: String,
    success: bool,
    summary: String,
    command: Option<Vec<String>>,
    exit_code: Option<i32>,
    duration_ms: Option<u64>,
    sink: &'a AuditSink,
}

impl<'a> AuditEntryBuilder<'a> {
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    pub fn with_command(mut self, command: Vec<String>) -> Self {
        self.command = Some(command);
        self
    }

    pub fn with_exit_code(mut self, code: i32) -> Self {
        self.exit_code = Some(code);
        self
    }

    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    pub fn with_success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    pub fn record(self) -> String {
        let entry = AuditEntry {
            audit_id: self.audit_id.clone(),
            timestamp: self.timestamp,
            tool: self.tool,
            input: self.input,
            mutation_level: self.mutation_level,
            success: self.success,
            summary: self.summary,
            command: self.command,
            exit_code: self.exit_code,
            duration_ms: self.duration_ms,
        };
        self.sink.record(entry);
        self.audit_id
    }
}
