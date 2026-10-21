use crate::engine::Status;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const FILES: [&str; 3] = ["previous-2.log", "previous-1.log", "current.log"];
const LIMIT: usize = 2 * 1024 * 1024;

pub fn logs(directory: &Path, request: &Value, live: Option<&Status>) -> Value {
    read_logs(directory, request, live)
        .unwrap_or_else(|error| json!({"ok":false,"code":"logs_unavailable","error":error}))
}

fn read_logs(directory: &Path, request: &Value, live: Option<&Status>) -> Result<Value, String> {
    let journal = load(directory).ok();
    let files = segments(directory)?;
    let base = journal.as_ref().map_or(0, |journal| journal.base);
    let total = base + files.iter().map(|(_, size)| size).sum::<u64>();
    let selected = request["operation"].as_str();
    let operation = selected.and_then(|id| {
        journal
            .as_ref()?
            .operations
            .iter()
            .find(|operation| operation.status.operation_id.as_deref() == Some(id))
    });
    if selected.is_some() && operation.is_none() {
        return Ok(
            json!({"ok":false,"code":"operation_logs_unavailable","error":"No hay metadatos retenidos verificables para esa ejecución"}),
        );
    }
    let scope_start = operation.map_or(base, |operation| operation.start);
    let scope_end = operation
        .and_then(|operation| operation.end)
        .unwrap_or(total)
        .min(total);
    let after = request["after"].as_str();
    let mut start = scope_start.max(base).min(scope_end);
    let mut end = scope_end;
    if let Some(cursor) = after {
        let parsed = cursor
            .rsplit_once(':')
            .and_then(|(stream, offset)| Some((stream, offset.parse::<u64>().ok()?)));
        let Some((stream, offset)) = parsed else {
            return Ok(json!({"ok":false,"code":"invalid_cursor","error":"Cursor inválido"}));
        };
        let expected_stream = journal
            .as_ref()
            .map(|journal| format!("{}@{}", journal.stream, selected.unwrap_or("all")));
        if expected_stream.as_deref() != Some(stream) {
            return Ok(
                json!({"ok":false,"code":"cursor_expired","error":"El cursor pertenece a otro historial; consultá logs otra vez"}),
            );
        }
        if offset < base || offset < scope_start || offset > scope_end {
            return Ok(
                json!({"ok":false,"code":"cursor_expired","error":"El cursor quedó fuera del historial retenido o de la ejecución seleccionada"}),
            );
        }
        start = offset;
        end = end.min(start.saturating_add(LIMIT as u64));
    } else {
        start = start.max(end.saturating_sub(LIMIT as u64));
    }
    let mut bytes = Vec::with_capacity((end.saturating_sub(start)) as usize);
    let mut position = base;
    for (path, size) in files {
        let lower = start.max(position);
        let upper = end.min(position + size);
        if lower < upper {
            let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
            file.seek(SeekFrom::Start(lower - position))
                .map_err(|error| error.to_string())?;
            file.take(upper - lower)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
        }
        position += size;
    }
    if after.is_some() && end < scope_end {
        while std::str::from_utf8(&bytes)
            .err()
            .is_some_and(|error| error.error_len().is_none())
        {
            bytes.pop();
            end -= 1;
        }
    }
    if after.is_none() && start > scope_start {
        let boundary = bytes
            .iter()
            .take_while(|byte| **byte & 0xc0 == 0x80)
            .count();
        bytes.drain(..boundary);
    }
    let text = String::from_utf8_lossy(&bytes);
    let contains = request["contains"].as_str();
    let lines: Vec<_> = text
        .lines()
        .filter(|line| contains.is_none_or(|needle| line.contains(needle)))
        .collect();
    let tail = request["tail"].as_u64().unwrap_or(200).min(20000) as usize;
    let skip = if after.is_some() {
        0
    } else {
        lines.len().saturating_sub(tail)
    };
    let latest = journal
        .as_ref()
        .and_then(|journal| journal.operations.last())
        .map(|operation| {
            live.filter(|status| status.operation_id == operation.status.operation_id)
                .unwrap_or(&operation.status)
        });
    let active = operation.is_some_and(|operation| {
        live.is_some_and(|status| {
            status.busy() && status.operation_id == operation.status.operation_id
        })
    });
    let status = operation.map(|operation| {
        live.filter(|status| status.operation_id == operation.status.operation_id)
            .unwrap_or(&operation.status)
    });
    let output = if after.is_some() && contains.is_none() {
        text.into_owned()
    } else {
        lines[skip..].join("\n")
    };
    Ok(
        json!({"ok":true,"text":output,"source":if live.is_some(){"live"}else{"disk"},
        "directory":directory,"observed_at_ms":timestamp(),"historical":!active,
        "attribution":if operation.is_some(){"operation"}else{"mixed_or_unknown"},
        "operation":status,"latest_operation":latest,
        "last_write_at_ms":fs::metadata(directory.join("current.log")).ok().and_then(|metadata| metadata.modified().ok()).and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|duration| duration.as_millis() as u64),
        "cursor":journal.as_ref().map(|journal| format!("{}@{}:{}", journal.stream,selected.unwrap_or("all"),end)),
        "has_more":end < scope_end,"retention_truncated":scope_start < base,
        "window_truncated":after.is_none() && start > scope_start.max(base),"tail_truncated":skip > 0,
        "partial_line":start > scope_start || end < scope_end}),
    )
}

pub fn offline_status(directory: &Path, unavailable: &str, reason: &str) -> Value {
    let latest = load(directory)
        .ok()
        .and_then(|journal| journal.operations.last().cloned());
    json!({"ok":true,"status":{"state":"closed"},"owner":{"state":unavailable,"reason":reason},
        "observed_at_ms":timestamp(),"execution_state":"unknown","health":{"state":"unknown","checked_now":false},
        "last_known_operation":latest.map(|operation| operation.status),"historical":true,
        "log_directory":directory})
}

pub fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Serialize, Deserialize)]
struct Operation {
    start: u64,
    end: Option<u64>,
    status: Status,
}

#[derive(Serialize, Deserialize)]
pub struct Journal {
    stream: String,
    base: u64,
    operations: Vec<Operation>,
}

fn load(directory: &Path) -> Result<Journal, String> {
    if directory.join("rotation.pending").exists() {
        return Err("Rotación incompleta; la atribución de logs no es verificable".into());
    }
    let file =
        fs::File::open(directory.join("observations.json")).map_err(|error| error.to_string())?;
    serde_json::from_reader(file.take(1024 * 1024)).map_err(|error| error.to_string())
}

fn segments(directory: &Path) -> Result<Vec<(PathBuf, u64)>, String> {
    FILES
        .iter()
        .filter_map(|name| {
            let path = directory.join(name);
            match fs::metadata(&path) {
                Ok(metadata) => Some(Ok((path, metadata.len()))),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => Some(Err(error.to_string())),
            }
        })
        .collect()
}

impl Journal {
    pub fn open(directory: &Path) -> Self {
        let mut journal = load(directory).unwrap_or_else(|_| Self {
            stream: format!(
                "{}-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                std::process::id()
            ),
            base: 0,
            operations: Vec::new(),
        });
        if let Ok(position) = journal.position(directory) {
            if let Some(operation) = journal
                .operations
                .last_mut()
                .filter(|operation| operation.end.is_none())
            {
                operation.end = Some(position);
                if operation.status.busy() {
                    operation.status.state = "interrupted".into();
                }
            }
            if journal.save(directory).is_ok() {
                let _ = fs::remove_file(directory.join("rotation.pending"));
            }
        }
        journal
    }

    fn position(&self, directory: &Path) -> Result<u64, String> {
        Ok(self.base
            + segments(directory)?
                .iter()
                .map(|(_, size)| size)
                .sum::<u64>())
    }

    fn save(&self, directory: &Path) -> Result<(), String> {
        let pending = directory.join("observations.tmp");
        fs::write(
            &pending,
            serde_json::to_vec(self).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        fs::rename(pending, directory.join("observations.json")).map_err(|error| error.to_string())
    }

    pub fn begin(&mut self, directory: &Path, status: &Status) -> Result<(), String> {
        let position = self.position(directory)?;
        if let Some(previous) = self
            .operations
            .last_mut()
            .filter(|operation| operation.end.is_none())
        {
            previous.end = Some(position);
        }
        self.operations.push(Operation {
            start: position,
            end: None,
            status: status.clone(),
        });
        if self.operations.len() > 100 {
            self.operations.remove(0);
        }
        self.save(directory)?;
        if directory.join("rotation.pending").exists() {
            fs::remove_file(directory.join("rotation.pending"))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn checkpoint(&mut self, directory: &Path, status: &Status) -> Result<(), String> {
        let position = self.position(directory)?;
        if let Some(operation) = self
            .operations
            .last_mut()
            .filter(|operation| operation.status.operation_id == status.operation_id)
        {
            operation.status = status.clone();
            if !status.busy() {
                operation.end = Some(position);
            }
            self.save(directory)?;
        }
        Ok(())
    }

    pub fn rotating(&self, directory: &Path) -> std::io::Result<()> {
        fs::write(directory.join("rotation.pending"), b"pending")
    }

    pub fn rotated(&mut self, directory: &Path, discarded: u64) -> std::io::Result<()> {
        self.base += discarded;
        self.save(directory).map_err(std::io::Error::other)?;
        fs::remove_file(directory.join("rotation.pending"))
    }
}
