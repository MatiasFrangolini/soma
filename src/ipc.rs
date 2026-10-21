use crate::{engine::Engine, model::Store};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::windows::io::{FromRawHandle, IntoRawHandle},
    sync::{
        atomic::{AtomicIsize, Ordering},
        Arc,
    },
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX},
    System::Pipes::*,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

pub static WINDOW: AtomicIsize = AtomicIsize::new(0);
pub const SHOW: u32 = WM_APP + 5;
pub const CLOSE: u32 = WM_APP + 6;
#[derive(Debug)]
pub struct RequestError {
    error: std::io::Error,
    absent: bool,
}
impl RequestError {
    pub fn is_absent(&self) -> bool {
        self.absent
    }
    pub fn response(&self) -> Value {
        let code = if self.error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
            "access_denied"
        } else if self.absent {
            "owner_unavailable"
        } else {
            "ipc_unavailable"
        };
        let mut response = json!({"ok":false,"code":code,"error":format!("No se pudo comunicar con Soma: {}", self.error),
            "os_error":self.error.raw_os_error()});
        if code == "access_denied" {
            response["next_action"] = json!("request_authorized_execution");
            response["hint"] = json!("Si el comando corre en un sandbox, solicitá al anfitrión la ejecución autorizada de ese comando exacto. El canal requiere escribir la solicitud incluso para status/logs. Conservá el dueño y sus ejecuciones; no cambies los permisos del canal.");
        }
        response
    }
}
impl From<std::io::Error> for RequestError {
    fn from(error: std::io::Error) -> Self {
        Self { error, absent: false }
    }
}
impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}
pub fn name(store: &Store) -> String {
    let h = Sha256::digest(store.home.to_string_lossy().to_ascii_lowercase().as_bytes());
    format!("\\\\.\\pipe\\Soma-{:x}", h)[..41].to_string()
}
pub fn request(store: &Store, request: &Value) -> Result<Value, RequestError> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut pipe = loop {
        match OpenOptions::new().read(true).write(true).open(name(store)) {
            Ok(p) => break p,
            Err(e) if e.raw_os_error() == Some(231) && std::time::Instant::now() < until => unsafe {
                WaitNamedPipeW(crate::wide(&name(store)).as_ptr(), 250);
            },
            Err(error) => return Err(RequestError {
                absent: matches!(error.raw_os_error(), Some(2 | 3)),
                error,
            }),
        }
    };
    writeln!(pipe, "{request}")?;
    pipe.flush()?;
    let mut result = String::new();
    BufReader::new(pipe)
        .take(16 * 1024 * 1024)
        .read_line(&mut result)?;
    serde_json::from_str(&result)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e).into())
}
pub fn request_or_launch(store: &Store, request: &Value) -> Result<Value, RequestError> {
    match self::request(store, request) {
        Ok(value) => return Ok(value),
        Err(error) if error.is_absent() => {},
        Err(error) => return Err(error),
    }
    let exe = std::env::current_exe()?.with_file_name("Soma.exe");
    crate::launch_windowed(&exe, &format!("--cli-owner --data-dir \"{}\"", store.home.display()))?;
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(75));
        match self::request(store, request) {
            Ok(value) => return Ok(value),
            Err(error) if error.is_absent() => {},
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Soma no pudo abrir su canal local").into())
}
pub fn serve(engine: Arc<Engine>, jobs: Arc<crate::jobs::Jobs>) {
    std::thread::spawn(move || unsafe {
        let mut descriptor = std::ptr::null_mut();
        let sddl = crate::wide("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)");
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            engine.note("No se pudo crear la seguridad del canal local");
            return;
        }
        let security = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let handle = CreateNamedPipeW(
            crate::wide(&name(&engine.store)).as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            65536,
            65536,
            0,
            &security,
        );
        LocalFree(descriptor);
        if handle == INVALID_HANDLE_VALUE {
            engine.note(format!(
                "No se pudo abrir el canal local: {}",
                std::io::Error::last_os_error()
            ));
            return;
        }
        loop {
            let connected = ConnectNamedPipe(handle, std::ptr::null_mut()) != 0
                || GetLastError() == ERROR_PIPE_CONNECTED;
            if !connected {
                break;
            }
            let file = File::from_raw_handle(handle);
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            let response = match reader.by_ref().take(65536).read_line(&mut line) {
                Ok(_) => match serde_json::from_str::<Value>(&line) {
                    Ok(value) => dispatch(&engine, &jobs, &value),
                    Err(_) => crate::error("Solicitud JSON inválida"),
                },
                Err(_) => crate::error("No se pudo leer la solicitud"),
            };
            let mut file = reader.into_inner();
            let _ = writeln!(file, "{response}");
            let _ = file.flush();
            let raw = file.into_raw_handle();
            FlushFileBuffers(raw);
            DisconnectNamedPipe(raw);
        }
        CloseHandle(handle);
    });
}
fn dispatch(engine: &Arc<Engine>, jobs: &Arc<crate::jobs::Jobs>, r: &Value) -> Value {
    match r["command"].as_str().unwrap_or("") {
        "job_submit" => match serde_json::from_value(r["request"].clone()) {
            Ok(request) => jobs.submit(request),
            Err(_) => crate::error("Solicitud de trabajo inválida"),
        },
        "job_status" => jobs.status(r["job"].as_str().unwrap_or("")),
        "job_list" => jobs.list(),
        "job_release" => jobs.release(r["job"].as_str().unwrap_or(""), r["participant"].as_str().unwrap_or("")),
        "job_logs" => jobs.logs(r["job"].as_str().unwrap_or(""), r["tail"].as_u64().unwrap_or(200).min(2000) as usize),
        "job_started" => jobs.started(r["job"].as_str().unwrap_or(""), r["token"].as_str().unwrap_or(""),
            serde_json::from_value(r["args"].clone()).unwrap_or_default()),
        "status" => {
            json!({"ok":true,"status":engine.status(),"log_directory":engine.store.home.join("logs"),"config_file":engine.store.path()})
        }
        "logs" => {
            let logs = engine.logs.lock().unwrap();
            let text = logs.snapshot();
            let count = r["tail"].as_u64().unwrap_or(200).min(20000) as usize;
            let mut lines = text.lines().rev().take(count).collect::<Vec<_>>();
            lines.reverse();
            json!({"ok":true,"text":lines.join("\n"),"generation":logs.generation,"visible_bytes":logs.bytes,"directory":engine.store.home.join("logs")})
        }
        "run" | "build" | "reuse" => engine.start(
            r["project"].as_str().unwrap_or(""),
            r["config"].as_str(),
            r["command"].as_str().unwrap(),
        ),
        "start_target" => {
            let readiness = match r.get("readiness").filter(|v| !v.is_null()) {
                Some(value) => match serde_json::from_value(value.clone()) {
                    Ok(value) => Some(value),
                    Err(_) => return crate::error("Configuración readiness inválida"),
                },
                None => None,
            };
            engine.start_at(
                r["project"].as_str().unwrap_or(""),
                r["config"].as_str(),
                r["mode"].as_str().unwrap_or("run"),
                r["directory"].as_str().map(std::path::Path::new),
                readiness,
            )
        }
        "switch" => engine.switch_branch(
            r["project"].as_str().unwrap_or(""),
            r["directory"].as_str().map(std::path::Path::new),
            r["branch"].as_str().unwrap_or(""),
        ),
        "stop" => engine.stop_expected(r["operation"].as_str()),
        "restart" => {
            let status = engine.status();
            if let Some(expected) = r["operation"].as_str() {
                if status.operation_id.as_deref() != Some(expected) {
                    return json!({"ok":false,"code":"operation_replaced","error":"La ejecución actual tiene otro ID","status":status});
                }
            }
            let directory = r["directory"]
                .as_str()
                .map(std::path::PathBuf::from)
                .or(status.directory.clone());
            let project = r["project"].as_str().map(str::to_owned).or(status.project);
            let config = r["config"].as_str().map(str::to_owned).or(status.config);
            if let Some(project) = project {
                if engine.status().busy() {
                    let stopped = engine.stop_expected(r["operation"].as_str());
                    if stopped["ok"] != true {
                        return stopped;
                    }
                    let e = engine.clone();
                    std::thread::spawn(move || {
                        for _ in 0..240 {
                            if !e.status().busy() {
                                e.start_at(
                                    &project,
                                    config.as_deref(),
                                    "run",
                                    directory.as_deref(),
                                    None,
                                );
                                return;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                        e.note("Restart no completado: la ejecución anterior no terminó");
                    });
                    json!({"ok":true,"state":"restarting"})
                } else {
                    engine.start_at(
                        &project,
                        config.as_deref(),
                        "run",
                        directory.as_deref(),
                        None,
                    )
                }
            } else {
                crate::error("Falta proyecto")
            }
        }
        "show" => {
            unsafe {
                PostMessageW(WINDOW.load(Ordering::SeqCst) as _, SHOW, 0, 0);
            }
            json!({"ok":true})
        }
        "close" => {
            unsafe {
                PostMessageW(WINDOW.load(Ordering::SeqCst) as _, CLOSE, 0, 0);
            }
            json!({"ok":true})
        }
        _ => crate::error("Comando desconocido"),
    }
}
