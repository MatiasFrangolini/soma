//! Bounded metadata archive, independent from detailed results and rotating logs.
use crate::{
    jobs::{valid_id, Record, JSON_LIMIT},
    model::{Result, Store},
};
use std::{
    fs,
    io::{self, Read},
    path::PathBuf,
};

pub(crate) const LIMIT: usize = 1000;
pub(crate) const BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const AGE_MS: u64 = 30 * 24 * 60 * 60 * 1000;

fn path(store: &Store, id: &str) -> PathBuf {
    store.home.join("job-history").join(format!("{id}.json"))
}
fn expired(record: &Record) -> bool {
    crate::observation::timestamp().saturating_sub(record.finished_at_ms.unwrap_or(0)) > AGE_MS
}
enum ReadError {
    Missing,
    Invalid,
    Storage(io::Error),
}
impl ReadError {
    fn message(&self) -> String {
        match self {
            Self::Missing => "Metadata archivada no encontrada".into(),
            Self::Invalid => "Metadata archivada inválida; no se eliminó".into(),
            Self::Storage(error) => format!("No se pudo leer metadata archivada: {error}"),
        }
    }
}
// Storage failures must remain distinguishable from a proved retention expiry.
fn load(store: &Store, id: &str) -> std::result::Result<(Record, u64), ReadError> {
    if !valid_id(id) {
        return Err(ReadError::Invalid);
    }
    let file = fs::File::open(path(store, id)).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ReadError::Missing
        } else {
            ReadError::Storage(error)
        }
    })?;
    let size = file.metadata().map_err(ReadError::Storage)?.len();
    let mut bytes = Vec::new();
    file.take(JSON_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(ReadError::Storage)?;
    if bytes.len() > JSON_LIMIT {
        return Err(ReadError::Invalid);
    }
    let record: Record = serde_json::from_slice(&bytes).map_err(|_| ReadError::Invalid)?;
    if record.job_id != id
        || record.finished_at_ms.is_none()
        || !matches!(
            record.state.as_str(),
            "completed" | "failed" | "cancelled" | "interrupted" | "invalidated" | "joined"
        )
    {
        return Err(ReadError::Invalid);
    }
    Ok((record, size))
}
pub(crate) fn read(store: &Store, id: &str) -> Result<Record> {
    let (record, _) = load(store, id).map_err(|error| error.message())?;
    if expired(&record) {
        return Err("Metadata archivada vencida".into());
    }
    Ok(record)
}

pub(crate) fn records(store: &Store) -> Result<Vec<Record>> {
    let mut records = Vec::new();
    for entry in entries(store)? {
        match load(store, &entry) {
            Ok((record, _)) if !expired(&record) => records.push(record),
            Ok(_) | Err(ReadError::Missing) => {}
            Err(error) => return Err(error.message()),
        }
    }
    Ok(records)
}
fn entries(store: &Store) -> Result<Vec<String>> {
    let directory = match fs::read_dir(store.home.join("job-history")) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => {
            return Err(format!(
                "No se pudo inspeccionar el archivo de metadata: {error}"
            ))
        }
    };
    let mut ids = Vec::new();
    for entry in directory {
        let entry = entry.map_err(|error| error.to_string())?;
        if let Some(id) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| valid_id(id))
        {
            ids.push(id.to_owned());
        }
    }
    Ok(ids)
}
struct Entry {
    id: String,
    size: u64,
    finished: u64,
    expired: bool,
}

/// A maintenance batch reads the archive once, then reserves space before
/// retiring any detailed result. Failed eviction never discards newer evidence.
pub(crate) struct Archive<'a> {
    store: &'a Store,
    entries: Vec<Entry>,
    bytes: u64,
}
impl<'a> Archive<'a> {
    pub(crate) fn open(store: &'a Store) -> Result<Self> {
        let mut archive = Self {
            store,
            entries: Vec::new(),
            bytes: 0,
        };
        for id in entries(store)? {
            match load(store, &id) {
                Ok((record, size)) => {
                    archive.bytes += size;
                    archive.entries.push(Entry {
                        id,
                        size,
                        finished: record.finished_at_ms.unwrap(),
                        expired: expired(&record),
                    });
                }
                Err(ReadError::Missing) => {}
                Err(error) => return Err(error.message()),
            }
        }
        archive
            .entries
            .sort_by(|a, b| a.finished.cmp(&b.finished).then_with(|| a.id.cmp(&b.id)));
        Ok(archive)
    }
    fn reserve(&mut self, count: usize, bytes: u64) -> Result<()> {
        while self.entries.first().is_some_and(|entry| entry.expired)
            || self.entries.len() > count
            || self.bytes > bytes
        {
            let entry = self
                .entries
                .first()
                .ok_or("No se pudo liberar espacio para metadata")?;
            match fs::remove_file(path(self.store, &entry.id)) {
                Ok(()) => {},
                Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                Err(error) => return Err(format!("No se pudo retirar metadata antigua; se conservó la evidencia detallada: {error}")),
            }
            self.bytes = self.bytes.saturating_sub(entry.size);
            self.entries.remove(0);
        }
        Ok(())
    }
    pub(crate) fn prune(&mut self) -> Result<()> {
        self.reserve(LIMIT, BYTES)
    }

    pub(crate) fn retain(&mut self, record: &Record) -> Result<()> {
        if !valid_id(&record.job_id) || record.finished_at_ms.is_none() {
            return Err("Resultado sin identidad o finalización verificable".into());
        }
        if expired(record) {
            return Ok(());
        }
        // Final results are immutable. This also tolerates a prior detailed-file
        // removal that was delayed by a Windows sharing violation.
        if self.entries.iter().any(|entry| entry.id == record.job_id) {
            return Ok(());
        }
        let mut metadata = record.clone();
        metadata.evidence.details_truncated |=
            !metadata.evidence.details.is_empty() || !metadata.evidence.diagnostics.is_empty();
        metadata.evidence.details.clear();
        metadata.evidence.diagnostics.clear();
        let bytes = serde_json::to_vec(&metadata).map_err(|error| error.to_string())?;
        if bytes.len() > JSON_LIMIT || bytes.len() as u64 > BYTES {
            return Err("Metadata demasiado grande para archivarla de forma verificable".into());
        }
        self.reserve(LIMIT - 1, BYTES - bytes.len() as u64)?;
        fs::create_dir_all(self.store.home.join("job-history"))
            .map_err(|error| error.to_string())?;
        let target = path(self.store, &record.job_id);
        let temporary = target.with_extension("tmp");
        fs::write(&temporary, &bytes).map_err(|error| error.to_string())?;
        if let Err(error) = fs::rename(&temporary, &target) {
            let _ = fs::remove_file(temporary);
            return Err(error.to_string());
        }
        self.bytes += bytes.len() as u64;
        self.entries.push(Entry {
            id: record.job_id.clone(),
            size: bytes.len() as u64,
            finished: record.finished_at_ms.unwrap(),
            expired: false,
        });
        self.entries
            .sort_by(|a, b| a.finished.cmp(&b.finished).then_with(|| a.id.cmp(&b.id)));
        Ok(())
    }
}
