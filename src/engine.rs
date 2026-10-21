use crate::{
    model::{self, CommandSpec, Config, Project, Result, RunSpec, Store},
    worker::Launch,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::SECURITY_ATTRIBUTES,
    System::{JobObjects::*, Pipes::CreatePipe, Threading::*},
};

const VISIBLE: usize = 2 * 1024 * 1024;
const DISK: u64 = 10 * 1024 * 1024;
#[cfg(test)]
#[path = "observation_tests.rs"]
mod observation_tests;
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub state: String,
    pub operation_id: Option<String>,
    pub project: Option<String>,
    pub config: Option<String>,
    pub phase: String,
    pub worker_pid: Option<u32>,
    pub exit_code: Option<u32>,
    pub error: Option<String>,
    pub forced: bool,
    pub started_at: Option<u64>,
    pub directory: Option<PathBuf>,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub readiness: String,
    pub readiness_checked_at_ms: Option<u64>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            operation_id: None,
            project: None,
            config: None,
            phase: "".into(),
            worker_pid: None,
            exit_code: None,
            error: None,
            forced: false,
            started_at: None,
            directory: None,
            branch: None,
            head: None,
            readiness: "not_configured".into(),
            readiness_checked_at_ms: None,
        }
    }
}
impl Status {
    pub fn busy(&self) -> bool {
        matches!(
            self.state.as_str(),
            "checking" | "building" | "starting" | "running" | "stopping"
        )
    }
    pub fn completed(store: &Store, id: &str) -> Option<Self> {
        if id.is_empty() || id.len() > 100 || !id.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
            return None;
        }
        let file = File::open(store.home.join("operations").join(format!("{id}.json"))).ok()?;
        let mut bytes = Vec::new();
        file.take(32769).read_to_end(&mut bytes).ok()?;
        if bytes.len() > 32768 { return None; }
        let status: Self = serde_json::from_slice(&bytes).ok()?;
        (status.operation_id.as_deref() == Some(id)
            && matches!(status.state.as_str(), "exited" | "failed" | "stopped"))
            .then_some(status)
    }
    pub(crate) fn persist(&self, store: &Store) -> std::io::Result<()> {
        let Some(id) = self.operation_id.as_deref() else { return Ok(()); };
        let directory = store.home.join("operations");
        fs::create_dir_all(&directory)?;
        let temp = directory.join(format!("{id}.tmp"));
        fs::write(&temp, serde_json::to_vec(self)?)?;
        fs::rename(temp, directory.join(format!("{id}.json")))?;
        let mut results = fs::read_dir(directory)?.flatten().map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json")
                && path.file_stem().is_some_and(|stem| stem.to_string_lossy().bytes().all(|b| b.is_ascii_digit() || b == b'-')))
            .collect::<Vec<_>>();
        results.sort();
        for path in results.iter().take(results.len().saturating_sub(100)) { fs::remove_file(path)?; }
        Ok(())
    }
}
pub struct LogBook {
    pub text: VecDeque<String>,
    pub bytes: usize,
    pub generation: u64,
    dir: PathBuf,
    file: Option<File>,
    size: u64,
    visible_limit: usize,
    segment_limit: u64,
    journal: crate::observation::Journal,
    write_error: Option<String>,
}
impl LogBook {
    pub fn new(dir: PathBuf) -> Self {
        let size = fs::metadata(dir.join("current.log"))
            .map(|m| m.len())
            .unwrap_or(0);
        let mut book = Self {
            text: VecDeque::new(),
            bytes: 0,
            generation: 0,
            dir: dir.clone(),
            file: None,
            size,
            visible_limit: VISIBLE,
            segment_limit: DISK,
            journal: crate::observation::Journal::open(&dir),
            write_error: None,
        };
        if let Ok(mut file) = File::open(book.dir.join("current.log")) {
            let _ = file.seek(SeekFrom::Start(size.saturating_sub((VISIBLE - 512) as u64)));
            let mut bytes = Vec::with_capacity(VISIBLE - 512);
            if file
                .take((VISIBLE - 512) as u64)
                .read_to_end(&mut bytes)
                .is_ok()
                && !bytes.is_empty()
            {
                book.push("[Soma] Historial anterior; no hay una ejecución activa.\n".into());
                book.push(String::from_utf8_lossy(&bytes).replace('\0', ""));
            }
        }
        book
    }
    fn write_disk(&mut self, mut bytes: &[u8]) -> std::io::Result<()> {
        while !bytes.is_empty() {
            if self.size >= self.segment_limit {
                self.journal.rotating(&self.dir)?;
                let discarded = fs::metadata(self.dir.join("previous-2.log")).map(|metadata| metadata.len()).unwrap_or(0);
                self.file.take();
                let _ = fs::remove_file(self.dir.join("previous-2.log"));
                if self.dir.join("previous-1.log").exists() {
                    fs::rename(
                        self.dir.join("previous-1.log"),
                        self.dir.join("previous-2.log"),
                    )?;
                }
                if self.dir.join("current.log").exists() {
                    fs::rename(
                        self.dir.join("current.log"),
                        self.dir.join("previous-1.log"),
                    )?;
                }
                self.size = 0;
                self.journal.rotated(&self.dir, discarded)?;
            }
            if self.file.is_none() {
                self.file = Some(
                    fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(self.dir.join("current.log"))?,
                );
            }
            let n = bytes.len().min((self.segment_limit - self.size) as usize);
            self.file.as_mut().unwrap().write_all(&bytes[..n])?;
            self.size += n as u64;
            bytes = &bytes[n..];
        }
        Ok(())
    }
    pub fn append(&mut self, text: String) {
        if let Err(e) = self.write_disk(text.as_bytes()) {
            self.write_error = Some(e.to_string());
            self.file.take();
            self.push(format!("\n[Soma] Error escribiendo logs: {e}\n"));
        }
        self.push(text);
    }
    fn push(&mut self, text: String) {
        self.bytes += text.len();
        self.text.push_back(text);
        while self.bytes > self.visible_limit {
            if let Some(s) = self.text.pop_front() {
                self.bytes -= s.len();
            } else {
                break;
            }
        }
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn snapshot(&self) -> String {
        self.text.iter().map(String::as_str).collect()
    }
    pub fn begin(&mut self, status: &Status) -> Result<()> {
        self.journal.begin(&self.dir, status)
    }
    pub fn checkpoint(&mut self, status: &Status) {
        if let Err(error) = self.journal.checkpoint(&self.dir, status) {
            self.push(format!("\n[Soma] No se pudo guardar la observación: {error}\n"));
        }
    }
    pub fn observe(&self, request: &Value, status: &Status) -> Value {
        let mut result = crate::observation::logs(&self.dir, request, Some(status));
        result["generation"] = self.generation.into();
        result["visible_bytes"] = self.bytes.into();
        result["write_error"] = json!(self.write_error);
        result
    }
    pub fn clear(&mut self) {
        self.text.clear();
        self.bytes = 0;
        self.generation = self.generation.wrapping_add(1);
    }
}
pub struct Engine {
    pub store: Store,
    pub state: Mutex<Status>,
    pub logs: Arc<Mutex<LogBook>>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    health: Mutex<Option<HealthTarget>>,
}
struct HealthTarget {
    spec: crate::readiness::Readiness,
    job: isize,
    operation_id: Option<String>,
}
struct HealthLease<'a>(&'a Mutex<Option<HealthTarget>>);
impl Drop for HealthLease<'_> {
    fn drop(&mut self) {
        self.0.lock().unwrap().take();
    }
}
#[derive(Serialize, Deserialize)]
struct Stamp {
    fingerprint: String,
    artifact: Option<String>,
    artifact_hash: Option<String>,
    built_at: u64,
}
struct SnapshotFile(PathBuf);
impl Drop for SnapshotFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        if let Some(directory) = self.0.parent() { let _ = fs::remove_dir(directory); }
    }
}
impl Engine {
    pub fn new(store: Store) -> Arc<Self> {
        // Called only by the owner after acquiring its data-directory mutex.
        if let Ok(entries) = fs::read_dir(store.home.join("runs")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.is_empty() && name.bytes().all(|c|c.is_ascii_digit() || c == b'-') {
                    let _ = fs::remove_file(entry.path().join("application.jar"));
                    let _ = fs::remove_dir(entry.path());
                }
            }
        }
        Arc::new(Self {
            logs: Arc::new(Mutex::new(LogBook::new(store.home.join("logs")))),
            store,
            state: Mutex::new(Status::default()),
            cancel: Mutex::new(None),
            health: Mutex::new(None),
        })
    }
    pub(crate) fn finite(store: Store) -> Arc<Self> {
        let mut logs = LogBook::new(store.home.join("logs"));
        logs.visible_limit = 64 * 1024;
        logs.segment_limit = 1024 * 1024;
        Arc::new(Self { logs: Arc::new(Mutex::new(logs)), store,
            state: Mutex::new(Status::default()), cancel: Mutex::new(None), health: Mutex::new(None) })
    }
    pub(crate) fn execute_finite(&self, command: CommandSpec, env: &BTreeMap<String, String>,
        cancel: &Arc<AtomicBool>) -> Result<u32> {
        self.execute(command, env, cancel, None)
    }
    pub fn status(&self) -> Status {
        self.state.lock().unwrap().clone()
    }
    pub fn observe_status(&self, request: &Value) -> Value {
        let status = self.status();
        if request["operation"].as_str().is_some_and(|expected| status.operation_id.as_deref() != Some(expected)) {
            return json!({"ok":false,"code":"operation_replaced","error":"El ID no corresponde a la ejecución actual","status":status});
        }
        let mut health = json!({"state":"not_checked","checked_now":false});
        if request["check_health"] == true {
            let target = self.health.lock().unwrap();
            health = match target.as_ref().filter(|target| target.operation_id == status.operation_id && status.state == "running") {
                Some(target) => {
                    let healthy = target.spec.target().is_ok_and(|endpoint| endpoint.probe(&target.spec, target.job as HANDLE));
                    json!({"state":if healthy {"healthy"} else {"unhealthy"},"checked_now":true,
                        "checked_at_ms":crate::observation::timestamp(),"operation_id":status.operation_id})
                }
                None => json!({"state":if status.state == "running" {"not_configured"} else {"not_running"},"checked_now":false}),
            };
            drop(target);
            let current = self.status();
            if current.operation_id != status.operation_id || current.state != status.state {
                return json!({"ok":false,"code":"operation_replaced","error":"La ejecución cambió durante la observación","status":current});
            }
        }
        json!({"ok":true,"status":status,"owner":{"state":"reachable"},"execution_state":status.state,
            "observed_at_ms":crate::observation::timestamp(),"historical":false,"health":health,
            "log_directory":self.store.home.join("logs"),"config_file":self.store.path()})
    }
    pub fn note(&self, text: impl AsRef<str>) {
        self.logs
            .lock()
            .unwrap()
            .append(format!("\n[Soma] {}\n", text.as_ref()));
    }
    fn phase(&self, phase: &str, state: &str) {
        let mut s = self.state.lock().unwrap();
        s.phase = phase.into();
        s.state = state.into();
    }
    pub fn stop(&self) -> Value {
        self.stop_expected(None)
    }
    pub fn stop_expected(&self, expected: Option<&str>) -> Value {
        let mut s = self.state.lock().unwrap();
        if expected.is_some() && expected != s.operation_id.as_deref() {
            return json!({"ok":false,"code":"operation_replaced","error":"El ID no corresponde a la ejecución actual","status":*s});
        }
        if !s.busy() {
            return json!({"ok":true,"state":s.state});
        }
        if let Some(flag) = self.cancel.lock().unwrap().as_ref() {
            flag.store(true, Ordering::SeqCst);
        }
        s.state = "stopping".into();
        json!({"ok":true,"operation_id":s.operation_id,"state":"stopping"})
    }
    pub fn start(self: &Arc<Self>, project: &str, config: Option<&str>, mode: &str) -> Value {
        self.start_at(project, config, mode, None, None)
    }
    pub fn start_at(
        self: &Arc<Self>,
        project: &str,
        config: Option<&str>,
        mode: &str,
        directory: Option<&Path>,
        readiness: Option<crate::readiness::Readiness>,
    ) -> Value {
        if !matches!(mode, "build" | "run" | "reuse") {
            return crate::error("Modo inválido");
        }
        let (p, mut c) = match self.store.target(project, config, directory) {
            Ok(v) => v,
            Err(e) => return crate::error(e),
        };
        if let Some(value) = readiness {
            c.readiness = Some(value);
        }
        if let Some(value) = &c.readiness {
            if let Err(error) = value.target() {
                return crate::error(error);
            }
        }
        let mut s = self.state.lock().unwrap();
        if s.busy() {
            return json!({"ok":false,"code":"busy","error":"Ya hay una ejecución activa"});
        }
        let id = format!(
            "{}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            std::process::id()
        );
        *s = Status {
            state: "checking".into(),
            operation_id: Some(id.clone()),
            project: Some(p.id.clone()),
            config: Some(c.id.clone()),
            started_at: Some(now()),
            directory: Some(p.directory.clone()),
            ..Default::default()
        };
        if let Err(error) = self.logs.lock().unwrap().begin(&s) {
            s.state = "failed".into();
            s.error = Some(error.clone());
            return crate::error(error);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.lock().unwrap() = Some(cancel.clone());
        let own = self.clone();
        let mode = mode.to_owned();
        std::thread::spawn(move || {
            own.note(format!("{} / {} · {}", p.name, c.name, mode));
            let result = own.operation(&p, &c, &mode, &cancel);
            let mut s = own.state.lock().unwrap();
            s.worker_pid = None;
            match result {
                Ok(code) => {
                    s.exit_code = Some(code);
                    s.state = if cancel.load(Ordering::SeqCst) {
                        "stopped"
                    } else if code == 0 {
                        "exited"
                    } else {
                        "failed"
                    }
                    .into();
                }
                Err(e) => {
                    own.note(&e);
                    s.error = Some(e);
                    s.state = if cancel.load(Ordering::SeqCst) {
                        "stopped"
                    } else {
                        "failed"
                    }
                    .into();
                }
            }
            s.phase = "finalizado".into();
            own.logs.lock().unwrap().checkpoint(&s);
            *own.cancel.lock().unwrap() = None;
            if let Err(error) = s.persist(&own.store) {
                own.note(format!("No se pudo guardar el resultado de ejecución: {error}"));
            }
        });
        json!({"ok":true,"operation_id":id,"state":"checking"})
    }
    fn operation(
        &self,
        p: &Project,
        c: &Config,
        mode: &str,
        cancel: &Arc<AtomicBool>,
    ) -> Result<u32> {
        let _runtime = if mode != "build" {
            Some(crate::git::CheckoutLease::runtime(&p.directory)?)
        } else { None };
        let mut checkout = Some(crate::git::CheckoutLease::acquire(&p.directory)?);
        // Snapshot the checkout only after other Soma owners can no longer switch it.
        let branch = model::branch(&p.directory);
        let head = crate::git::head(&p.directory);
        {
            let mut state = self.state.lock().unwrap();
            state.branch = Some(branch);
            state.head = head;
            self.logs.lock().unwrap().checkpoint(&state);
        }
        let env = model::environment(p, c)?;
        use sha2::{Digest, Sha256};
        let target_hash = format!(
            "{:x}",
            Sha256::digest(p.directory.to_string_lossy().as_bytes())
        );
        let stamp_path = self.store.home.join("builds").join(format!(
            "{}--{}--{}.json",
            p.id,
            c.id,
            &target_hash[..16]
        ));
        let needs_stamp = c.build.is_some() || matches!(c.run, RunSpec::JavaJar { .. });
        let fingerprint = if needs_stamp {
            Some(model::fingerprint(p, c, &env)?)
        } else {
            None
        };
        if cancel.load(Ordering::SeqCst) {
            return Ok(0);
        }
        if mode == "reuse" && needs_stamp {
            let stamp: Stamp = fs::read(&stamp_path)
                .ok()
                .and_then(|v| serde_json::from_slice(&v).ok())
                .ok_or("build_stale: primero usá Compilar y ejecutar")?;
            if Some(&stamp.fingerprint) != fingerprint.as_ref() {
                return Err(
                    "build_stale: cambió la rama, configuración o entradas; recompilá".into(),
                );
            }
            if let Some(artifact) = &stamp.artifact {
                if model::hash_file(Path::new(artifact)).ok() != stamp.artifact_hash {
                    return Err("build_stale: el artefacto cambió o no existe".into());
                }
            }
            self.note(format!("Build vigente · {}", stamp.built_at));
        }
        if mode != "reuse" {
            if let Some(build) = &c.build {
                let _ = fs::remove_file(&stamp_path);
                self.phase("compilando", "building");
                let mut command = build.clone();
                command.directory = Some(
                    command
                        .directory
                        .as_ref()
                        .map(|d| model::resolve(p, d))
                        .unwrap_or_else(|| p.directory.clone())
                        .to_string_lossy()
                        .into(),
                );
                let code = self.execute(command, &env, cancel, None)?;
                if code != 0 || cancel.load(Ordering::SeqCst) {
                    return Ok(code);
                }
                let after = model::fingerprint(p, c, &env)?;
                if fingerprint.as_ref() != Some(&after) {
                    return Err("Las entradas cambiaron durante la compilación; recompilá antes de ejecutar".into());
                }
                let artifact = match &c.run {
                    RunSpec::JavaJar { artifact, .. } => Some(model::resolve(p, artifact)),
                    _ => None,
                };
                let hash = artifact.as_ref().map(|p| model::hash_file(p)).transpose()?;
                let stamp = Stamp {
                    fingerprint: after,
                    artifact: artifact.map(|p| p.to_string_lossy().into()),
                    artifact_hash: hash,
                    built_at: now(),
                };
                fs::write(&stamp_path, serde_json::to_vec_pretty(&stamp).unwrap())
                    .map_err(|e| e.to_string())?;
            } else if mode == "build" || matches!(c.run, RunSpec::JavaJar { .. }) {
                return Err(
                    "Falta configurar el comando de compilación; abrí projects.json".into(),
                );
            }
            if mode == "build" {
                return Ok(0);
            }
        }
        if cancel.load(Ordering::SeqCst) {
            return Ok(0);
        }
        let mut snapshot_file = None;
        let run = match &c.run {
            RunSpec::JavaJar {
                artifact,
                snapshot,
                jvm_args,
                args,
            } => {
                let mut path = model::resolve(p, artifact);
                if !path.is_file() {
                    return Err("El JAR configurado no existe".into());
                }
                if *snapshot {
                    let id = self.status().operation_id.ok_or("Falta identidad de ejecución")?;
                    let directory = self.store.home.join("runs").join(id);
                    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
                    let copy = directory.join("application.jar");
                    snapshot_file = Some(SnapshotFile(copy.clone()));
                    fs::copy(&path, &copy).map_err(|e| e.to_string())?;
                    if model::hash_file(&path)? != model::hash_file(&copy)? {
                        let _ = fs::remove_file(&copy);
                        let _ = fs::remove_dir(&directory);
                        return Err("El JAR cambió al preparar la copia de ejecución".into());
                    }
                    path = copy.clone();
                    // The runtime lease still protects Git and other application starts.
                    checkout.take();
                    self.note("JAR autocontenido aislado; los trabajos pueden usar el checkout");
                }
                let java = if let Some(jdk) = &c.jdk {
                    model::resolve(p, jdk)
                        .join("bin/java.exe")
                        .to_string_lossy()
                        .into()
                } else {
                    "java.exe".into()
                };
                let mut arguments = vec![
                    "-Dstdout.encoding=UTF-8".into(),
                    "-Dstderr.encoding=UTF-8".into(),
                ];
                arguments.extend(jvm_args.clone());
                arguments.push("-jar".into());
                arguments.push(path.to_string_lossy().into());
                arguments.extend(args.clone());
                CommandSpec {
                    executable: java,
                    args: arguments,
                    directory: Some(p.directory.to_string_lossy().into()),
                }
            }
            RunSpec::Command { executable, args } => CommandSpec {
                executable: executable.clone(),
                args: args.clone(),
                directory: Some(p.directory.to_string_lossy().into()),
            },
        };
        self.phase("aplicación", "starting");
        self.state.lock().unwrap().readiness = if c.readiness.is_some() {
            "pending"
        } else {
            "not_configured"
        }
        .into();
        let result = self.execute(run, &env, cancel, c.readiness.as_ref());
        drop(snapshot_file);
        result
    }
    fn execute(
        &self,
        command: CommandSpec,
        env: &BTreeMap<String, String>,
        cancel: &Arc<AtomicBool>,
        readiness: Option<&crate::readiness::Readiness>,
    ) -> Result<u32> {
        let exe = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .unwrap()
            .join("soma-cli.exe");
        let mut worker = NativeWorker::spawn(&exe)?;
        let _health_lease = HealthLease(&self.health);
        if let Some(spec) = readiness {
            let operation_id = self.status().operation_id;
            *self.health.lock().unwrap() = Some(HealthTarget {
                spec: spec.clone(), job: worker.job.0, operation_id,
            });
        }
        let launch = Launch {
            command,
            environment: env.clone(),
        };
        writeln!(worker.input, "{}", serde_json::to_string(&launch).unwrap())
            .map_err(|e| e.to_string())?;
        worker.input.flush().map_err(|e| e.to_string())?;
        {
            let mut s = self.state.lock().unwrap();
            s.worker_pid = Some(worker.pid);
            if s.state == "starting" {
                s.state = "running".into();
            }
        }
        let out = worker.output.take().unwrap();
        let err = worker.error.take().unwrap();
        let readers = [out, err]
            .into_iter()
            .map(|mut file| {
                let logs = self.logs.clone();
                std::thread::spawn(move || {
                    let mut buf = [0; 8192];
                    let mut pending = Vec::with_capacity(8196);
                    loop {
                        match file.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => pending.extend_from_slice(&buf[..n]),
                        }
                        loop {
                            match std::str::from_utf8(&pending) {
                                Ok(s) => {
                                    if !s.is_empty() {
                                        logs.lock().unwrap().append(s.replace('\0', ""));
                                    }
                                    pending.clear();
                                    break;
                                }
                                Err(e) => {
                                    let valid = e.valid_up_to();
                                    if valid > 0 {
                                        logs.lock().unwrap().append(
                                            String::from_utf8_lossy(&pending[..valid])
                                                .replace('\0', ""),
                                        );
                                    }
                                    let skip = valid + e.error_len().unwrap_or(0);
                                    pending.drain(..skip);
                                    if e.error_len().is_some() {
                                        logs.lock().unwrap().append("�".into());
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if !pending.is_empty() {
                        logs.lock()
                            .unwrap()
                            .append(String::from_utf8_lossy(&pending).into());
                    }
                })
            })
            .collect::<Vec<_>>();
        let mut stop_at = None;
        let mut forced = false;
        let ready_target = readiness.map(|spec| spec.target()).transpose()?;
        let startup_at = Instant::now();
        let mut next_probe = Instant::now();
        let mut checking_ready = readiness.is_some();
        loop {
            let done = unsafe { WaitForSingleObject(worker.process.0 as HANDLE, 75) };
            if done == WAIT_OBJECT_0 {
                break;
            }
            if done == WAIT_FAILED {
                return Err("Falló la espera del proceso".into());
            }
            if cancel.load(Ordering::SeqCst) && stop_at.is_none() {
                let _ = writeln!(worker.input, "stop");
                let _ = worker.input.flush();
                stop_at = Some(Instant::now());
                self.note("Cierre ordenado solicitado; límite 15 segundos");
            }
            if checking_ready && stop_at.is_none() && Instant::now() >= next_probe {
                let spec = readiness.unwrap();
                if ready_target
                    .as_ref()
                    .unwrap()
                    .probe(spec, worker.job.0 as HANDLE)
                {
                    let mut state = self.state.lock().unwrap();
                    state.readiness = "ready".into();
                    state.readiness_checked_at_ms = Some(crate::observation::timestamp());
                    self.logs.lock().unwrap().checkpoint(&state);
                    drop(state);
                    self.note("Aplicación lista: readiness confirmado en un proceso propio");
                    checking_ready = false;
                } else if startup_at.elapsed() >= Duration::from_secs(spec.timeout_seconds) {
                    let mut state = self.state.lock().unwrap();
                    state.readiness = "timed_out".into();
                    state.readiness_checked_at_ms = Some(crate::observation::timestamp());
                    self.logs.lock().unwrap().checkpoint(&state);
                    drop(state);
                    self.note(
                        "Readiness agotó su plazo; el proceso sigue activo. Consultá sus logs",
                    );
                    checking_ready = false;
                }
                next_probe = Instant::now() + Duration::from_millis(750);
            }
            if stop_at
                .map(|t| t.elapsed() >= Duration::from_secs(15))
                .unwrap_or(false)
            {
                unsafe {
                    TerminateJobObject(worker.job.0 as HANDLE, 130);
                }
                forced = true;
                self.note("Se terminó el árbol propio después de 15 segundos");
                break;
            }
        }
        let mut code = 1;
        unsafe {
            WaitForSingleObject(worker.process.0 as HANDLE, 2000);
            GetExitCodeProcess(worker.process.0 as HANDLE, &mut code);
        }
        drop(worker);
        for reader in readers {
            let _ = reader.join();
        }
        {
            let mut s = self.state.lock().unwrap();
            s.worker_pid = None;
            s.forced |= forced;
        }
        self.note(format!("Proceso finalizado · código {code}"));
        Ok(code)
    }
    pub fn switch_branch(&self, project: &str, directory: Option<&Path>, branch: &str) -> Value {
        let state = self.state.lock().unwrap();
        if state.busy() {
            return json!({"ok":false,"code":"busy","error":"Detené la ejecución antes de cambiar de rama"});
        }
        let result = self.store.target(project, None, directory).and_then(|(p,_)| {
            let _runtime = crate::git::CheckoutLease::runtime(&p.directory)?;
            let _lease = crate::git::CheckoutLease::acquire(&p.directory)?;
            crate::git::switch(&p.directory, branch)?;
            Ok(json!({"ok":true,"directory":p.directory,"branch":model::branch(&p.directory),"head":crate::git::head(&p.directory)}))
        });
        result.unwrap_or_else(crate::error)
    }
}
struct Owned(isize);
impl Drop for Owned {
    fn drop(&mut self) {
        if self.0 != 0 && self.0 != -1 {
            unsafe {
                CloseHandle(self.0 as HANDLE);
            }
        }
    }
}
struct NativeWorker {
    job: Owned,
    process: Owned,
    pid: u32,
    input: File,
    output: Option<File>,
    error: Option<File>,
}
impl NativeWorker {
    fn spawn(exe: &Path) -> Result<Self> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let job = Owned(job as isize);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.0 as HANDLE,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            ) == 0
            {
                return Err("No se pudo limitar el árbol de procesos".into());
            }
            let (input_read, input_write) = pipe()?;
            let (output_read, output_write) = pipe()?;
            let (error_read, error_write) = pipe()?;
            for handle in [&input_write, &output_read, &error_read] {
                if SetHandleInformation(handle.as_raw_handle() as HANDLE, HANDLE_FLAG_INHERIT, 0)
                    == 0
                {
                    return Err("No se pudo aislar el pipe".into());
                }
            }
            let mut startup: STARTUPINFOEXW = std::mem::zeroed();
            startup.StartupInfo.cb = std::mem::size_of_val(&startup) as u32;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
            startup.StartupInfo.wShowWindow = 0;
            startup.StartupInfo.hStdInput = input_read.as_raw_handle() as HANDLE;
            startup.StartupInfo.hStdOutput = output_write.as_raw_handle() as HANDLE;
            startup.StartupInfo.hStdError = error_write.as_raw_handle() as HANDLE;
            let mut attribute_size = 0;
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attribute_size);
            let mut attributes =
                vec![0usize; attribute_size.div_ceil(std::mem::size_of::<usize>())];
            startup.lpAttributeList = attributes.as_mut_ptr() as _;
            if InitializeProcThreadAttributeList(startup.lpAttributeList, 1, 0, &mut attribute_size)
                == 0
            {
                return Err("No se pudo aislar la herencia de handles".into());
            }
            let handles = [
                startup.StartupInfo.hStdInput,
                startup.StartupInfo.hStdOutput,
                startup.StartupInfo.hStdError,
            ];
            if UpdateProcThreadAttribute(
                startup.lpAttributeList,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr() as _,
                std::mem::size_of_val(&handles),
                std::ptr::null_mut(),
                std::ptr::null(),
            ) == 0
            {
                DeleteProcThreadAttributeList(startup.lpAttributeList);
                return Err("No se pudo limitar la herencia de handles".into());
            }
            let mut process: PROCESS_INFORMATION = std::mem::zeroed();
            let exe_w = crate::wide(&exe.to_string_lossy());
            let mut args = crate::wide(&format!("\"{}\" __worker", exe.display()));
            let created = CreateProcessW(
                exe_w.as_ptr(),
                args.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                CREATE_NEW_CONSOLE | CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
                std::ptr::null(),
                std::ptr::null(),
                &startup.StartupInfo,
                &mut process,
            );
            let error = std::io::Error::last_os_error();
            DeleteProcThreadAttributeList(startup.lpAttributeList);
            if created == 0 {
                return Err(error.to_string());
            }
            let process_handle = Owned(process.hProcess as isize);
            let thread = Owned(process.hThread as isize);
            if AssignProcessToJobObject(job.0 as HANDLE, process.hProcess) == 0 {
                TerminateProcess(process.hProcess, 125);
                return Err("No se pudo asignar el proceso al Job Object".into());
            }
            if ResumeThread(thread.0 as HANDLE) == u32::MAX {
                return Err("No se pudo reanudar el worker".into());
            }
            drop(input_read);
            drop(output_write);
            drop(error_write);
            Ok(Self {
                job,
                process: process_handle,
                pid: process.dwProcessId,
                input: input_write,
                output: Some(output_read),
                error: Some(error_read),
            })
        }
    }
}
unsafe fn pipe() -> Result<(File, File)> {
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    let security = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    if CreatePipe(&mut read, &mut write, &security, 0) == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok((File::from_raw_handle(read), File::from_raw_handle(write)))
}
