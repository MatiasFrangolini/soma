use crate::{
    engine::Engine,
    model::{self, CommandSpec, Project, Result, Store},
    reports::{self, Evidence},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const HISTORY: usize = 100;
pub(crate) const PENDING: usize = 128;
pub(crate) const JSON_LIMIT: usize = 256 * 1024;
const LOG_BUDGET: u64 = 30 * 1024 * 1024;
const ACTIVE_LOG_BUDGET: u64 = 3 * 1024 * 1024;
const PARTICIPANTS: usize = 128;
const ALIASES: usize = 128;
fn utc_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn ms(instant: Instant) -> u64 {
    instant.elapsed().as_millis().min(u64::MAX as u128) as u64
}
fn identity(prefix: &str) -> String {
    unsafe {
        let mut guid = std::mem::zeroed();
        if windows_sys::Win32::System::Com::CoCreateGuid(&mut guid) == 0 {
            return format!(
                "{prefix}-{:08x}{:04x}{:04x}{}",
                guid.data1,
                guid.data2,
                guid.data3,
                guid.data4
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
        }
    }
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{prefix}-{}-{}-{}",
        utc_ms(),
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    use std::io::Read;
    let file = fs::File::open(path).map_err(|_| "Archivo de trabajo no encontrado")?;
    let mut bytes = Vec::new();
    file.take(JSON_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > JSON_LIMIT {
        return Err("Archivo de trabajo demasiado grande".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Archivo de trabajo inválido".into())
}
pub(crate) fn valid_id(id: &str) -> bool {
    id.starts_with("job-")
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn terminal(state: &str) -> bool {
    matches!(
        state,
        "completed" | "failed" | "cancelled" | "interrupted" | "invalidated" | "joined"
    )
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Timings {
    pub preparation_ms: Option<u64>,
    pub soma_queue_ms: Option<u64>,
    pub resource_wait_ms: Option<u64>,
    pub adapter_and_maven_ms: Option<u64>,
    pub maven_ms: Option<u64>,
    pub evidence_ms: Option<u64>,
    pub total_ms: Option<u64>,
    pub preparation: Option<PlanTimings>,
    pub pre_execution: Option<PlanTimings>,
    pub final_verification: Option<PlanTimings>,
    pub report_collection_ms: Option<u64>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct PlanTimings {
    pub reactor_ms: u64,
    pub configuration_ms: u64,
    pub environment_ms: u64,
    pub adapter_ms: u64,
    pub fingerprint: model::FingerprintTimings,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub job_id: String,
    pub canonical_job_id: Option<String>,
    pub state: String,
    pub kind: String,
    pub project: String,
    pub config: String,
    pub directory: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub fingerprint: Option<String>,
    pub adapter_digest: Option<String>,
    pub requested_maven_args: Vec<String>,
    #[serde(default)]
    pub effective_maven_args: Option<Vec<String>>,
    pub created_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub exit_code: Option<u32>,
    pub forced: bool,
    pub error: Option<String>,
    pub timings: Timings,
    pub evidence: Evidence,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub directory: PathBuf,
    pub config: String,
    pub kind: String,
    #[serde(default)]
    pub args: Vec<String>,
}
#[derive(Clone)]
struct Plan {
    project: Project,
    modules: Vec<PathBuf>,
    environment: BTreeMap<String, String>,
    wrapper: String,
    args: Vec<String>,
    adapter: Option<CommandSpec>,
    adapter_digest: String,
    key: String,
    observed: PlanTimings,
}
#[derive(Serialize, Deserialize)]
pub struct MavenRequest {
    pub executable: String,
    pub directory: PathBuf,
    pub args: Vec<String>,
    pub soma_cli: PathBuf,
    pub data_directory: PathBuf,
    pub job_id: String,
    pub event_token: String,
}
struct Entry {
    record: Record,
    request: Option<Request>,
    plan: Option<Plan>,
    participants: BTreeSet<String>,
    cancel: Arc<AtomicBool>,
    accepted: Instant,
    prepared: Option<Instant>,
    adapter_started: Option<Instant>,
    maven_started: Option<Instant>,
    event_token: String,
    runner: Option<Arc<Engine>>,
}
struct State {
    entries: BTreeMap<String, Entry>,
    preparing: VecDeque<String>,
    ready: VecDeque<String>,
    active_checkouts: BTreeSet<PathBuf>,
    closing: bool,
    history_error: Option<String>,
}
pub struct Jobs {
    store: Store,
    state: Mutex<State>,
    changed: Condvar,
    preparing: AtomicBool,
    concurrency: usize,
}
fn safe_args(args: &[String]) -> Vec<String> {
    let mut property = false;
    let mut secret = false;
    args.iter()
        .map(|a| {
            if secret {
                secret = false;
                return "<redacted>".into();
            }
            if property {
                property = false;
                let key = a.split_once('=').map(|(k, _)| k).unwrap_or(a);
                if matches!(key, "test" | "it.test") { return a.clone(); }
                return format!("{key}=<redacted>");
            }
            if matches!(a.as_str(), "-D" | "--define") {
                property = true;
            }
            if matches!(
                a.as_str(),
                "--encrypt-password" | "--encrypt-master-password" | "-ep" | "-emp"
            ) {
                secret = true;
            }
            if a.starts_with("-D") {
                if let Some((key, value)) = a.split_once('=') {
                    if matches!(key, "-Dtest" | "-Dit.test") {
                        return format!("{key}={value}");
                    }
                    return format!("{key}=<redacted>");
                }
            }
            if let Some(raw) = a.strip_prefix("--define=") {
                let (key, _) = raw.split_once('=').unwrap_or((raw, ""));
                if matches!(key, "test" | "it.test") {
                    return a.clone();
                }
                return format!("--define={key}=<redacted>");
            }
            for prefix in [
                "--encrypt-password=",
                "--encrypt-master-password=",
                "-emp",
                "-ep",
            ] {
                if a.starts_with(prefix) && a.len() > prefix.len() {
                    return format!("{prefix}<redacted>");
                }
            }
            a.clone()
        })
        .collect()
}
fn adapter_digest(adapter: &Option<CommandSpec>) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(serde_json::to_vec(adapter).map_err(|e| e.to_string())?);
    if let Some(adapter) = adapter {
        for token in std::iter::once(&adapter.executable).chain(adapter.args.iter()) {
            let path = Path::new(token);
            if path.is_file() {
                digest.update(model::hash_file(path)?);
            }
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn validate_reactor(directory: &Path) -> Result<Vec<PathBuf>> {
    // Every declared reactor module must share the checkout protected by the lease.
    use quick_xml::{events::Event, Reader};
    use std::io::{BufReader, Read};
    let mut pending = vec![directory.join("pom.xml")];
    let mut seen = BTreeSet::new();
    while let Some(pom) = pending.pop() {
        let pom = fs::canonicalize(pom).map_err(|_| "POM del reactor no encontrado")?;
        if !model::plain(pom.clone()).starts_with(directory) {
            return Err("Módulos Maven fuera del checkout no admitidos".into());
        }
        if !seen.insert(pom.clone()) {
            continue;
        }
        if seen.len() > 512 {
            return Err("El reactor excede el límite de 512 módulos".into());
        }
        let file = fs::File::open(&pom).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > 2 * 1024 * 1024 {
            return Err("POM demasiado grande".into());
        }
        let mut reader = Reader::from_reader(BufReader::new(file.take(2 * 1024 * 1024 + 1)));
        let mut buffer = Vec::new();
        let mut module = None::<String>;
        loop {
            match reader
                .read_event_into(&mut buffer)
                .map_err(|_| "POM XML inválido")?
            {
                Event::Start(e) if e.name().as_ref() == "module" => module = Some(String::new()),
                Event::Text(e) if module.is_some() => module.as_mut().unwrap().push_str(e.as_ref()),
                Event::End(e) if e.name().as_ref() == "module" => {
                    let name = module.take().unwrap_or_default();
                    let name = quick_xml::escape::unescape(name.trim())
                        .map_err(|_| "Módulo XML inválido")?;
                    if name.contains("${") {
                        return Err("Módulos con rutas calculadas requieren una configuración explícita futura".into());
                    }
                    let target = pom.parent().unwrap().join(name.as_ref());
                    pending.push(if target.is_dir() {
                        target.join("pom.xml")
                    } else {
                        target
                    });
                }
                Event::DocType(_) => return Err("El POM no admite DTD".into()),
                Event::Eof => break,
                _ => {}
            }
            buffer.clear();
        }
    }
    Ok(seen
        .into_iter()
        .map(|pom| model::plain(pom.parent().unwrap().to_path_buf()))
        .collect())
}
fn plan(store: &Store, request: &Request) -> Result<Plan> {
    let mut observed = PlanTimings::default();
    if !matches!(request.kind.as_str(), "test" | "build") {
        return Err("kind debe ser test o build".into());
    }
    if request.args.len() > 256
        || request.args.iter().any(|a| a.len() > 4096)
        || request.args.iter().map(String::len).sum::<usize>() > 48 * 1024
    {
        return Err("Demasiados argumentos Maven".into());
    }
    if request
        .args
        .iter()
        .any(|a| a.starts_with("-Dsurefire.reportNameSuffix"))
    {
        return Err(
            "Soma reserva surefire.reportNameSuffix para identificar la evidencia del trabajo"
                .into(),
        );
    }
    let directory =
        model::plain(fs::canonicalize(&request.directory).map_err(|_| "El checkout no existe")?);
    if request.args.iter().any(|a| {
        a == "-f"
            || a.starts_with("-f=")
            || a.starts_with("--file")
            || (a.starts_with("-f") && a.len() > 2)
    }) {
        return Err(
            "Esta entrega ejecuta el pom.xml del checkout elegido; no admite -f/--file".into(),
        );
    }
    let clock = Instant::now();
    let modules = validate_reactor(&directory)?;
    observed.reactor_ms = ms(clock);
    let clock = Instant::now();
    let (settings, project, test) = store.test_target(&directory, &request.config)?;
    let config = test.context();
    observed.configuration_ms = ms(clock);
    if !project.directory.join("pom.xml").is_file() {
        return Err("El checkout no contiene pom.xml".into());
    }
    let clock = Instant::now();
    let environment = model::environment(&project, &config)?;
    observed.environment_ms = ms(clock);
    let wrapper = if project.directory.join("mvnw.cmd").is_file() {
        project.directory.join("mvnw.cmd").to_string_lossy().into()
    } else {
        "mvn.cmd".into()
    };
    let args = if request.args.is_empty() {
        vec![if request.kind == "test" {
            "test"
        } else {
            "package"
        }
        .into()]
    } else {
        request.args.clone()
    };
    let clock = Instant::now();
    let adapter_digest = adapter_digest(&settings.job_adapter)?;
    let wrapper_digest = if Path::new(&wrapper).is_file() { Some(model::hash_file(Path::new(&wrapper))?) } else { None };
    observed.adapter_ms = ms(clock);
    let mut digest = Sha256::new();
    let (fingerprint, timings) = model::fingerprint_observed(&project, &config, &environment)?;
    observed.fingerprint = timings;
    digest.update(fingerprint);
    digest.update(&request.kind);
    digest.update(serde_json::to_vec(&args).map_err(|e| e.to_string())?);
    digest.update(&adapter_digest);
    if let Some(digest_value) = wrapper_digest {
        digest.update(digest_value);
    }
    Ok(Plan {
        project,
        modules,
        environment,
        wrapper,
        args,
        adapter: settings.job_adapter,
        adapter_digest,
        key: format!("{:x}", digest.finalize()),
        observed,
    })
}
impl Jobs {
    pub fn new(store: Store) -> Result<Arc<Self>> {
        let concurrency = store.load()?.job_concurrency;
        fs::create_dir_all(store.home.join("jobs")).map_err(|e| e.to_string())?;
        let mut state = State {
            entries: BTreeMap::new(),
            preparing: VecDeque::new(),
            ready: VecDeque::new(),
            active_checkouts: BTreeSet::new(),
            closing: false,
            history_error: crate::job_history::Archive::open(&store).and_then(|mut history| history.prune()).err(),
        };
        for directory in fs::read_dir(store.home.join("jobs"))
            .map_err(|e| e.to_string())?
            .flatten()
            .take(1000)
        {
            let id = directory.file_name().to_string_lossy().into_owned();
            if !valid_id(&id) {
                continue;
            }
            let Ok(mut record) = read_json::<Record>(&directory.path().join("result.json")) else {
                continue;
            };
            if record.job_id != id {
                continue;
            }
            if !terminal(&record.state) {
                record.state = "interrupted".into();
                record.finished_at_ms = Some(utc_ms());
                record.error =
                    Some("El dueño anterior terminó antes de registrar el resultado".into());
            }
            let _ = fs::remove_file(directory.path().join("request.json"));
            state.entries.insert(
                id,
                Entry {
                    record,
                    request: None,
                    plan: None,
                    participants: BTreeSet::new(),
                    cancel: Arc::new(AtomicBool::new(false)),
                    accepted: Instant::now(),
                    prepared: None,
                    adapter_started: None,
                    maven_started: None,
                    event_token: String::new(),
                    runner: None,
                },
            );
        }
        let jobs = Arc::new(Self {
            store,
            state: Mutex::new(state),
            changed: Condvar::new(),
            preparing: AtomicBool::new(false),
            concurrency,
        });
        {
            let mut state = jobs.state.lock().unwrap();
            for entry in state.entries.values() {
                jobs.persist(&entry.record)?;
            }
            jobs.prune(&mut state, LOG_BUDGET);
        }
        let own = jobs.clone();
        std::thread::spawn(move || own.prepare_loop());
        for _ in 0..concurrency {
            let own = jobs.clone();
            std::thread::spawn(move || own.execute_loop());
        }
        Ok(jobs)
    }
    fn directory(&self, id: &str) -> PathBuf {
        self.store.home.join("jobs").join(id)
    }
    fn persist(&self, record: &Record) -> Result<()> {
        let directory = self.directory(&record.job_id);
        fs::create_dir_all(directory.join("logs")).map_err(|e| e.to_string())?;
        let temp = directory.join("result.json.tmp");
        fs::write(
            &temp,
            serde_json::to_vec(record).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(temp, directory.join("result.json")).map_err(|e| e.to_string())
    }
    fn update(&self, state: &mut State, id: &str, action: impl FnOnce(&mut Entry)) {
        if let Some(entry) = state.entries.get_mut(id) {
            action(entry);
            if let Err(error) = self.persist(&entry.record) {
                entry.record.error = Some(format!("No se pudo guardar el resultado: {error}"));
                if terminal(&entry.record.state) && entry.record.state != "joined" {
                    entry.record.state = "failed".into();
                }
            }
        }
    }
    fn finish(&self, id: &str, state_name: &str, error: Option<String>) {
        let mut state = self.state.lock().unwrap();
        self.update(&mut state, id, |entry| {
            entry.record.state = state_name.into();
            entry.record.error = error;
            entry.record.finished_at_ms = Some(utc_ms());
            entry.record.timings.total_ms = Some(ms(entry.accepted));
            entry.plan = None;
            entry.request = None;
            entry.runner = None;
            entry.participants.clear();
        });
        let _ = fs::remove_file(self.directory(id).join("request.json"));
        self.prune(&mut state, LOG_BUDGET);
        self.changed.notify_all();
    }
    pub fn submit(&self, request: Request) -> Value {
        let mut state = self.state.lock().unwrap();
        if state.closing {
            return crate::error("Soma se está cerrando");
        }
        self.prune(&mut state, LOG_BUDGET);
        if let Some(error) = &state.history_error {
            return json!({"ok":false,"code":"history_unavailable","error":error,"next_action":"repair_history_storage"});
        }
        if state.entries.values().filter(|e| terminal(&e.record.state) && e.record.canonical_job_id.is_none()).count() > HISTORY
            || state.entries.values().filter(|e| e.record.canonical_job_id.is_some()).count() > ALIASES {
            return crate::error("No se pudo archivar el historial; revisá el almacenamiento antes de enviar otro trabajo");
        }
        if state
            .entries
            .values()
            .filter(|e| !terminal(&e.record.state))
            .count()
            >= PENDING
        {
            return crate::error("La cola alcanzó su límite de 128 trabajos pendientes");
        }
        let id = identity("job");
        let participant = identity("part");
        let record = Record {
            job_id: id.clone(),
            canonical_job_id: None,
            state: "preparing".into(),
            kind: request.kind.clone(),
            project: String::new(),
            config: request.config.clone(),
            directory: request.directory.clone(),
            branch: None,
            head: None,
            fingerprint: None,
            adapter_digest: None,
            requested_maven_args: safe_args(&request.args),
            effective_maven_args: None,
            created_at_ms: utc_ms(),
            finished_at_ms: None,
            exit_code: None,
            forced: false,
            error: None,
            timings: Timings::default(),
            evidence: Evidence::default(),
        };
        if let Err(error) = self.persist(&record) {
            return crate::error(error);
        }
        state.entries.insert(
            id.clone(),
            Entry {
                record,
                request: Some(request),
                plan: None,
                participants: BTreeSet::from([participant.clone()]),
                cancel: Arc::new(AtomicBool::new(false)),
                accepted: Instant::now(),
                prepared: None,
                adapter_started: None,
                maven_started: None,
                event_token: identity("event"),
                runner: None,
            },
        );
        state.preparing.push_back(id.clone());
        self.changed.notify_all();
        json!({"ok":true,"job_id":id,"participant_id":participant,"state":"preparing"})
    }
    fn resolve(state: &State, id: &str) -> Option<String> {
        let entry = state.entries.get(id)?;
        Some(
            entry
                .record
                .canonical_job_id
                .clone()
                .unwrap_or_else(|| id.into()),
        )
    }
    pub fn status(&self, id: &str) -> Value {
        let state = self.state.lock().unwrap();
        let Some(canonical) = Self::resolve(&state, id) else {
            drop(state);
            return Self::offline(&self.store, id, None);
        };
        let Some(entry) = state.entries.get(&canonical) else {
            drop(state);
            return Self::offline(&self.store, id, None);
        };
        let wait_reason = match entry.record.state.as_str() {
            "queued" if state.active_checkouts.contains(&entry.record.directory) => Some("checkout_in_use"),
            "queued" => Some("soma_queue"),
            "waiting_checkout" => Some("checkout_in_use"),
            "waiting_resources" => Some("resource_admission"),
            _ => None,
        };
        json!({"ok":true,"job_id":canonical,"requested_job_id":id,"job":entry.record,"wait_reason":wait_reason,
            "history_error":state.history_error,"participants":entry.participants.len(),"worker_pid":entry.runner.as_ref().and_then(|r|r.status().worker_pid)})
    }
    pub fn list(&self, summary: bool) -> Value {
        let state = self.state.lock().unwrap();
        let records = state
            .entries
            .values()
            .filter(|e| e.record.canonical_job_id.is_none())
            .map(|e| {
                if summary {
                    let mut value = crate::job_output::compact(&json!(e.record));
                    value["participants"] = json!(e.participants.len());
                    value["historical"] = json!(terminal(&e.record.state));
                    value
                } else { json!({"job_id":e.record.job_id,"state":e.record.state,"kind":e.record.kind,
                "project":e.record.project,"directory":e.record.directory,"config":e.record.config,
                "created_at_ms":e.record.created_at_ms,"participants":e.participants.len()}) }
            })
            .collect::<Vec<_>>();
        json!({"ok":true,"jobs":records,"concurrency":self.concurrency,"retained_limit":HISTORY,"log_budget_bytes":LOG_BUDGET,"history_error":state.history_error})
    }
    pub fn release(&self, id: &str, participant: &str) -> Value {
        let mut state = self.state.lock().unwrap();
        let Some(canonical) = Self::resolve(&state, id) else {
            return crate::error("Trabajo no encontrado");
        };
        let Some(entry) = state.entries.get_mut(&canonical) else {
            return crate::error("Trabajo vencido");
        };
        if !entry.participants.remove(participant) {
            return crate::error(
                "La participación no corresponde a este trabajo o ya fue retirada",
            );
        }
        let remaining = entry.participants.len();
        if remaining == 0 && !terminal(&entry.record.state) {
            entry.cancel.store(true, Ordering::SeqCst);
            if matches!(entry.record.state.as_str(), "preparing" | "queued") {
                entry.record.state = "cancelled".into();
                entry.record.finished_at_ms = Some(utc_ms());
                entry.record.timings.total_ms = Some(ms(entry.accepted));
                entry.plan = None;
                entry.request = None;
            } else {
                entry.record.state = "cancelling".into();
            }
        }
        let _ = self.persist(&entry.record);
        self.prune(&mut state, LOG_BUDGET);
        self.changed.notify_all();
        json!({"ok":true,"job_id":canonical,"participants":remaining})
    }
    pub fn shutdown(&self) {
        let mut state = self.state.lock().unwrap();
        state.closing = true;
        let ids = state.entries.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            self.update(&mut state, &id, |entry| {
                if !terminal(&entry.record.state) {
                    entry.cancel.store(true, Ordering::SeqCst);
                    if matches!(entry.record.state.as_str(), "preparing" | "queued") {
                        entry.record.state = "cancelled".into();
                        entry.record.finished_at_ms = Some(utc_ms());
                        entry.record.timings.total_ms = Some(ms(entry.accepted));
                        entry.plan = None;
                        entry.request = None;
                    } else {
                        entry.record.state = "cancelling".into();
                    }
                }
            });
        }
        self.changed.notify_all();
    }
    pub fn busy(&self) -> bool {
        let state = self.state.lock().unwrap();
        self.preparing.load(Ordering::SeqCst) || !state.active_checkouts.is_empty()
            || state.entries.values().any(|entry| !terminal(&entry.record.state))
    }
    fn prepare_loop(self: Arc<Self>) {
        loop {
            let (id, request, cancel) = {
                let mut state = self.state.lock().unwrap();
                while state.preparing.is_empty() && !state.closing {
                    state = self.changed.wait(state).unwrap();
                }
                if state.closing {
                    return;
                }
                let id = state.preparing.pop_front().unwrap();
                let Some(entry) = state.entries.get(&id) else {
                    continue;
                };
                if terminal(&entry.record.state) {
                    continue;
                }
                let Some(request) = entry.request.clone() else {
                    continue;
                };
                self.preparing.store(true, Ordering::SeqCst);
                (id, request, entry.cancel.clone())
            };
            let clock = Instant::now();
            let prepared = plan(&self.store, &request);
            if cancel.load(Ordering::SeqCst) {
                self.finish(&id, "cancelled", None);
            } else {
                match prepared {
                    Err(error) => self.finish(&id, "failed", Some(error)),
                    Ok(plan) => {
                        let mut state = self.state.lock().unwrap();
                        if state.closing || cancel.load(Ordering::SeqCst) {
                            self.update(&mut state, &id, |entry| {
                                entry.record.state = "cancelled".into();
                                entry.record.finished_at_ms = Some(utc_ms());
                                entry.record.timings.total_ms = Some(ms(entry.accepted));
                                entry.request = None;
                            });
                            self.preparing.store(false, Ordering::SeqCst);
                            continue;
                        }
                        let equivalent = state
                            .entries
                            .iter()
                            .find(|(other, entry)| {
                                *other != &id
                                    && !terminal(&entry.record.state)
                                    && !entry.cancel.load(Ordering::SeqCst)
                                    && entry.record.fingerprint.as_deref() == Some(&plan.key)
                            })
                            .map(|(id, _)| id.clone());
                        if let Some(canonical) = equivalent {
                            if state
                                .entries
                                .get(&canonical)
                                .is_some_and(|e| e.participants.len() >= PARTICIPANTS)
                            {
                                drop(state);
                                self.finish(
                                    &id,
                                    "failed",
                                    Some(
                                        "El trabajo alcanzó su límite de 128 participaciones"
                                            .into(),
                                    ),
                                );
                                self.preparing.store(false, Ordering::SeqCst);
                                continue;
                            }
                            let participants = state
                                .entries
                                .get_mut(&id)
                                .map(|e| std::mem::take(&mut e.participants))
                                .unwrap_or_default();
                            if let Some(entry) = state.entries.get_mut(&canonical) {
                                entry.participants.extend(participants);
                            }
                            self.update(&mut state, &id, |entry| {
                                entry.record.canonical_job_id = Some(canonical);
                                entry.record.state = "joined".into();
                                entry.record.finished_at_ms = Some(utc_ms());
                                entry.record.timings.preparation_ms = Some(ms(clock));
                                entry.record.timings.preparation = Some(plan.observed.clone());
                                entry.request = None;
                            });
                        } else {
                            self.update(&mut state, &id, |entry| {
                                entry.record.state = "queued".into();
                                entry.record.project = plan.project.id.clone();
                                entry.record.directory = plan.project.directory.clone();
                                entry.record.branch = Some(model::branch(&plan.project.directory));
                                entry.record.head = crate::git::head(&plan.project.directory);
                                entry.record.fingerprint = Some(plan.key.clone());
                                entry.record.adapter_digest = Some(plan.adapter_digest.clone());
                                entry.record.requested_maven_args = safe_args(&plan.args);
                                entry.record.timings.preparation_ms = Some(ms(clock));
                                entry.record.timings.preparation = Some(plan.observed.clone());
                                entry.prepared = Some(Instant::now());
                                entry.plan = Some(plan);
                            });
                            state.ready.push_back(id.clone());
                        }
                        self.prune(&mut state, LOG_BUDGET);
                        self.changed.notify_all();
                    }
                }
            }
            self.preparing.store(false, Ordering::SeqCst);
        }
    }
    fn execute_loop(self: Arc<Self>) {
        loop {
            let (id, plan, request, cancel) = {
                let mut state = self.state.lock().unwrap();
                loop {
                    if state.closing { return; }
                    let position = state.ready.iter().position(|id| match state.entries.get(id) {
                        None => true,
                        Some(entry) if terminal(&entry.record.state) => true,
                        Some(entry) => entry.plan.as_ref().is_none_or(|plan|
                            !state.active_checkouts.contains(&plan.project.directory)),
                    });
                    let Some(position) = position else {
                        state = self.changed.wait(state).unwrap();
                        continue;
                    };
                    let id = state.ready.remove(position).unwrap();
                    let Some(entry) = state.entries.get_mut(&id) else { continue };
                    if terminal(&entry.record.state) { continue; }
                    let (Some(plan), Some(request)) = (entry.plan.take(), entry.request.clone()) else { continue };
                    let cancel = entry.cancel.clone();
                    state.active_checkouts.insert(plan.project.directory.clone());
                    break (id, plan, request, cancel);
                }
            };
            let result = self.execute(&id, &plan, &request, &cancel);
            match result {
                Ok((code, evidence, changed)) => {
                    {
                        let mut state = self.state.lock().unwrap();
                        self.update(&mut state, &id, |entry| {
                            entry.record.exit_code = Some(code);
                            entry.record.evidence = evidence;
                            entry.record.forced =
                                entry.runner.as_ref().is_some_and(|r| r.status().forced);
                        });
                    }
                    let state = if cancel.load(Ordering::SeqCst) {
                        "cancelled"
                    } else if changed {
                        "invalidated"
                    } else if code == 0 {
                        "completed"
                    } else {
                        "failed"
                    };
                    self.finish(
                        &id,
                        state,
                        changed.then(|| {
                            "Las entradas o condiciones cambiaron durante el trabajo".into()
                        }),
                    );
                }
                Err(error) => {
                    let state = if cancel.load(Ordering::SeqCst) {
                        "cancelled"
                    } else if error.starts_with("invalidated:") {
                        "invalidated"
                    } else {
                        "failed"
                    };
                    self.finish(&id, state, Some(error));
                }
            }
            let mut state = self.state.lock().unwrap();
            state.active_checkouts.remove(&plan.project.directory);
            self.changed.notify_all();
        }
    }
    fn execute(
        &self,
        id: &str,
        plan: &Plan,
        request: &Request,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(u32, Evidence, bool)> {
        let _lease = loop {
            if cancel.load(Ordering::SeqCst) {
                return Err("Trabajo cancelado antes de iniciar Maven".into());
            }
            match crate::git::CheckoutLease::acquire(&plan.project.directory) {
                Ok(lease) => break lease,
                Err(error) if error.starts_with("checkout_busy:") => {
                    let mut state = self.state.lock().unwrap();
                    self.update(&mut state, id, |entry| {
                        entry.record.state = "waiting_checkout".into();
                    });
                    drop(state);
                    std::thread::sleep(Duration::from_millis(150));
                }
                Err(error) => return Err(error),
            }
        };
        let verified = crate::jobs::plan(&self.store, request)?;
        {
            let mut state = self.state.lock().unwrap();
            self.update(&mut state, id, |entry| entry.record.timings.pre_execution = Some(verified.observed));
        }
        if verified.key != plan.key {
            return Err("invalidated: las entradas cambiaron mientras el trabajo esperaba".into());
        }
        let directory = self.directory(id);
        let cli = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .ok_or("Directorio de Soma inválido")?
            .join("soma-cli.exe");
        let (event_token, prepared) = {
            let state = self.state.lock().unwrap();
            let entry = state.entries.get(id).ok_or("Trabajo vencido")?;
            (entry.event_token.clone(), entry.prepared)
        };
        let mut args = plan.args.clone();
        args.push(format!("-Dsurefire.reportNameSuffix={id}"));
        let payload = MavenRequest {
            executable: plan.wrapper.clone(),
            directory: plan.project.directory.clone(),
            args: args.clone(),
            soma_cli: cli.clone(),
            data_directory: self.store.home.clone(),
            job_id: id.into(),
            event_token,
        };
        let payload_path = directory.join("request.json");
        fs::write(
            &payload_path,
            serde_json::to_vec(&payload).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let command = if let Some(adapter) = &plan.adapter {
            let mut command = adapter.clone();
            command
                .args
                .extend(["-RequestFile".into(), payload_path.to_string_lossy().into()]);
            command.directory = Some(plan.project.directory.to_string_lossy().into());
            command
        } else {
            let mut command_args = vec![
                "__maven_child".into(),
                "--request".into(),
                payload_path.to_string_lossy().into(),
                "--".into(),
            ];
            command_args.extend(args);
            CommandSpec {
                executable: cli.to_string_lossy().into(),
                args: command_args,
                directory: Some(plan.project.directory.to_string_lossy().into()),
            }
        };
        let runner = Engine::finite(Store {
            home: directory.clone(),
        });
        {
            let mut state = self.state.lock().unwrap();
            self.prune(&mut state, LOG_BUDGET - ACTIVE_LOG_BUDGET * self.concurrency as u64);
            self.update(&mut state, id, |entry| {
                entry.record.state = "waiting_resources".into();
                entry.record.timings.soma_queue_ms = prepared.map(ms);
                entry.adapter_started = Some(Instant::now());
                entry.runner = Some(runner.clone());
            });
        }
        if cancel.load(Ordering::SeqCst) {
            return Err("Trabajo cancelado antes de iniciar Maven".into());
        }
        let clock = Instant::now();
        let result = runner.execute_finite(command, &plan.environment, cancel);
        {
            let mut state = self.state.lock().unwrap();
            self.update(&mut state, id, |entry| {
                entry.record.timings.adapter_and_maven_ms = Some(ms(clock));
                entry.record.timings.maven_ms = entry.maven_started.map(ms);
            });
        }
        let code = result?;
        let clock = Instant::now();
        let mut evidence = reports::collect(&plan.modules, id, request.kind == "test");
        // Environment values are never serialized; mask known credentials in bounded failure details.
        for detail in &mut evidence.details {
            for (key, value) in &plan.environment {
                if value.len() >= 4
                    && ["PASSWORD", "SECRET", "TOKEN", "PRIVATE_KEY", "ACCESS_KEY"]
                        .iter()
                        .any(|s| key.contains(s))
                {
                    detail.message = detail.message.replace(value, "<redacted>");
                }
            }
            if detail.message.len() > 2048 {
                let mut end = 2048;
                while !detail.message.is_char_boundary(end) {
                    end -= 1;
                }
                detail.message.truncate(end);
            }
        }
        let collection_ms = ms(clock);
        let verified = crate::jobs::plan(&self.store, request);
        let changed = verified.as_ref().map(|p| p.key != plan.key).unwrap_or(true);
        {
            let mut state = self.state.lock().unwrap();
            self.update(&mut state, id, |entry| {
                entry.record.timings.evidence_ms = Some(ms(clock));
                entry.record.timings.report_collection_ms = Some(collection_ms);
                entry.record.timings.final_verification = verified.ok().map(|p| p.observed);
            });
        }
        Ok((code, evidence, changed))
    }
    pub fn started(&self, id: &str, token: &str, args: Vec<String>) -> Value {
        if args.len() > 300 || args.iter().map(String::len).sum::<usize>() > 56 * 1024 {
            return crate::error("Argumentos efectivos demasiado grandes");
        }
        let mut state = self.state.lock().unwrap();
        let Some(entry) = state.entries.get(id) else {
            return crate::error("Trabajo no encontrado");
        };
        if entry.event_token != token
            || entry.maven_started.is_some()
            || entry.cancel.load(Ordering::SeqCst)
            || entry.record.state != "waiting_resources"
        {
            return crate::error("Evento de trabajo inválido");
        }
        self.update(&mut state, id, |entry| {
            entry.maven_started = Some(Instant::now());
            entry.record.effective_maven_args = Some(safe_args(&args));
            entry.record.state = "running".into();
            entry.record.timings.resource_wait_ms = entry.adapter_started.map(ms);
        });
        json!({"ok":true})
    }
    pub fn logs(&self, id: &str, request: &Value) -> Value {
        let response = self.status(id);
        job_logs(&self.store, response, request)
    }
    fn prune(&self, state: &mut State, log_budget: u64) {
        let retry_history = state.history_error.take().is_some();
        let mut aliases = state
            .entries
            .iter()
            .filter(|(_, e)| e.record.canonical_job_id.is_some())
            .map(|(id, e)| (e.record.created_at_ms, id.clone()))
            .collect::<Vec<_>>();
        aliases.sort();
        let needs_history = retry_history || aliases.len() > ALIASES
            || state.entries.values().filter(|e| terminal(&e.record.state) && e.record.canonical_job_id.is_none()).count() > HISTORY;
        let mut history = if needs_history {
            match crate::job_history::Archive::open(&self.store).and_then(|mut history| { history.prune()?; Ok(history) }) {
                Ok(history) => Some(history),
                Err(error) => { state.history_error = Some(error); None }
            }
        } else { None };
        for (_, id) in aliases.iter().take(aliases.len().saturating_sub(ALIASES)) {
            let Some(history) = history.as_mut() else { break; };
            if let Some(entry) = state.entries.get(id) {
                if let Err(error) = history.retain(&entry.record) { state.history_error = Some(error); break; }
            }
            state.entries.remove(id);
            self.remove_files(id, true);
        }
        let mut finished = state
            .entries
            .iter()
            .filter(|(_, e)| terminal(&e.record.state) && e.record.canonical_job_id.is_none())
            .map(|(id, e)| (e.record.created_at_ms, id.clone()))
            .collect::<Vec<_>>();
        finished.sort();
        for (_, id) in finished.iter().take(finished.len().saturating_sub(HISTORY)) {
            if state.history_error.is_some() { break; }
            let Some(history) = history.as_mut() else { break; };
            let aliases = state
                .entries
                .iter()
                .filter(|(_, e)| e.record.canonical_job_id.as_deref() == Some(id))
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for alias in aliases.into_iter().chain(std::iter::once(id.clone())) {
                if let Some(entry) = state.entries.get(&alias) {
                    if let Err(error) = history.retain(&entry.record) { state.history_error = Some(error); break; }
                }
                state.entries.remove(&alias);
                self.remove_files(&alias, true);
            }
        }
        let total = || {
            state
                .entries
                .keys()
                .map(|id| log_size(&self.directory(id)))
                .sum::<u64>()
        };
        let mut bytes = total();
        for (_, id) in &finished {
            if bytes <= log_budget {
                break;
            }
            let size = log_size(&self.directory(id));
            self.remove_files(id, false);
            bytes = bytes.saturating_sub(size);
        }
        state.preparing.retain(|id| {
            state
                .entries
                .get(id)
                .is_some_and(|e| !terminal(&e.record.state))
        });
        state.ready.retain(|id| {
            state
                .entries
                .get(id)
                .is_some_and(|e| !terminal(&e.record.state))
        });
    }
    fn remove_files(&self, id: &str, metadata: bool) {
        if !valid_id(id) {
            return;
        }
        let directory = self.directory(id);
        for name in ["current.log", "previous-1.log", "previous-2.log"] {
            let _ = fs::remove_file(directory.join("logs").join(name));
        }
        let _ = fs::remove_dir(directory.join("logs"));
        if metadata {
            for name in ["result.json", "result.json.tmp", "request.json"] {
                let _ = fs::remove_file(directory.join(name));
            }
            for name in ["observations.json", "observations.tmp", "rotation.pending"] {
                let _ = fs::remove_file(directory.join("logs").join(name));
            }
            let _ = fs::remove_dir(directory.join("logs"));
            let _ = fs::remove_dir(directory);
        }
    }
    pub fn offline(store: &Store, id: &str, tail: Option<usize>) -> Value {
        if !valid_id(id) {
            return crate::error("ID de trabajo inválido");
        }
        let read = |id: &str| -> Result<Record> {
            read_json::<Record>(&store.home.join("jobs").join(id).join("result.json"))
                .or_else(|_| crate::job_history::read(store, id))
                .and_then(|record| if record.job_id == id { Ok(record) } else { Err("Identidad de trabajo inválida".into()) })
        };
        let mut record = match read(id) {
            Ok(r) => r,
            Err(e) => return crate::error(e),
        };
        if let Some(canonical) = record.canonical_job_id.clone() {
            if !valid_id(&canonical) {
                return crate::error("ID compartido inválido");
            }
            record = match read(&canonical) {
                Ok(r) => r,
                Err(e) => return crate::error(e),
            };
        }
        if let Some(tail) = tail {
            return job_logs(store, Self::offline(store, &record.job_id, None), &json!({"tail":tail}));
        }
        let owner_unavailable = !terminal(&record.state);
        let archived = !store.home.join("jobs").join(&record.job_id).join("result.json").is_file();
        let logs_retained = ["current.log", "previous-1.log", "previous-2.log"].iter()
            .any(|name| store.home.join("jobs").join(&record.job_id).join("logs").join(name).is_file());
        json!({"ok":true,"job_id":record.job_id,"job":record,"owner_unavailable":owner_unavailable,
            "archived":archived,"historical":true,"details_retained":!archived,"logs_retained":logs_retained})
    }
    pub fn offline_list(store: &Store) -> Value {
        let mut records = Vec::new();
        if let Ok(entries) = fs::read_dir(store.home.join("jobs")) {
            for entry in entries.flatten().take(1000) {
                let id = entry.file_name().to_string_lossy().into_owned();
                if !valid_id(&id) {
                    continue;
                }
                let value = Self::offline(store, &id, None);
                if value["ok"] == true
                    && value["job"]["canonical_job_id"].is_null()
                    && value["job_id"] == id
                {
                    let mut record = value["job"].clone();
                    for key in ["archived", "historical", "details_retained", "logs_retained"] {
                        record[key] = value[key].clone();
                    }
                    records.push(record);
                }
            }
        }
        let concurrency = store.load().map(|settings| settings.job_concurrency).unwrap_or(1);
        json!({"ok":true,"jobs":records,"owner_unavailable":true,"concurrency":concurrency,
            "retained_limit":HISTORY,"log_budget_bytes":LOG_BUDGET})
    }
    pub fn offline_logs(store: &Store, id: &str, request: &Value) -> Value {
        job_logs(store, Self::offline(store, id, None), request)
    }
}
fn log_size(directory: &Path) -> u64 {
    ["current.log", "previous-1.log", "previous-2.log"]
        .iter()
        .filter_map(|name| fs::metadata(directory.join("logs").join(name)).ok())
        .map(|m| m.len())
        .sum()
}
fn job_logs(store: &Store, response: Value, request: &Value) -> Value {
    if response["ok"] != true { return response; }
    let id = response["job_id"].as_str().unwrap_or("");
    if !valid_id(id) { return crate::error("ID de trabajo inválido"); }
    let directory = store.home.join("jobs").join(id).join("logs");
    let finished = terminal(response["job"]["state"].as_str().unwrap_or(""));
    let has_logs = ["current.log", "previous-1.log", "previous-2.log"].iter().any(|name| directory.join(name).is_file());
    if response["archived"] == true || (finished && !has_logs) {
        return json!({"ok":false,"code":"job_logs_unavailable","job_id":id,
            "error":"Los logs ya no están retenidos; la metadata del resultado sigue disponible",
            "next_action":"inspect_job_result"});
    }
    let mut scoped = request.clone();
    scoped["operation"] = Value::Null;
    if scoped["after"].is_null() {
        scoped["tail"] = json!(request["tail"].as_u64().unwrap_or(200).min(2000));
    }
    let mut value = crate::observation::logs(&directory, &scoped, None);
    value["job_id"] = json!(id);
    value["attribution"] = json!("job");
    value["historical"] = json!(finished || response["owner_unavailable"] == true);
    value
}
pub fn maven_child() -> i32 {
    unsafe extern "system" fn control(event: u32) -> i32 {
        if event == windows_sys::Win32::System::Console::CTRL_C_EVENT {
            1
        } else {
            0
        }
    }
    let args = std::env::args().skip(2).collect::<Vec<_>>();
    let Some(index) = args.iter().position(|s| s == "--request") else {
        return 125;
    };
    let Some(path) = args.get(index + 1) else {
        return 125;
    };
    let Ok(request) = read_json::<MavenRequest>(Path::new(path)) else {
        return 125;
    };
    let Some(split) = args.iter().position(|s| s == "--") else {
        return 125;
    };
    let event = crate::ipc::request(
        &Store {
            home: request.data_directory,
        },
        &json!({"command":"job_started","job":request.job_id,"token":request.event_token,"args":args[split+1..]}),
    );
    if event.as_ref().map(|e| e["ok"] != true).unwrap_or(true) {
        return 130;
    }
    // Wait for Maven's own console shutdown instead of exiting the proxy first.
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(control), 1);
    }
    match std::process::Command::new(request.executable)
        .args(&args[split + 1..])
        .current_dir(request.directory)
        .status()
    {
        Ok(code) => code.code().unwrap_or(1),
        Err(_) => {
            eprintln!("No se pudo iniciar el Maven Wrapper del checkout");
            126
        }
    }
}
