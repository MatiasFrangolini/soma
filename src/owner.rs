use crate::{engine::Engine, ipc, jobs::Jobs, model::Store};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use windows_sys::Win32::{Foundation::*, System::Threading::CreateMutexW};

// IPC dispatch and idle retirement share this lock: an accepted command cannot
// race a temporary owner's decision to exit. Queries do not prolong its life.
pub(crate) struct Control {
    pub interactive: bool,
    pub closing: bool,
    pub last_busy: Instant,
}
struct Ownership(HANDLE);
impl Drop for Ownership {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn run() {
    let temporary = std::env::args().any(|arg| arg == "--cli-owner");
    let store = Store::from_args();
    // Take ownership before initializing history. A denied mutex is not absence.
    let name = crate::wide(&format!("Local\\{}", ipc::name(&store).replace('\\', "-")));
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return;
    }
    let _ownership = Ownership(handle);
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        drop(_ownership);
        if !temporary {
            let _ = ipc::request_or_launch(&store, &serde_json::json!({"command":"show"}));
        }
        return;
    }
    if let Err(error) = store.initialize() {
        crate::ui::startup_error(&error);
        return;
    }
    let jobs = match Jobs::new(store.clone()) {
        Ok(jobs) => jobs,
        Err(error) => {
            crate::ui::startup_error(&error);
            return;
        }
    };
    let engine = Engine::new(store);
    let control = Arc::new(Mutex::new(Control {
        interactive: !temporary,
        closing: false,
        last_busy: Instant::now(),
    }));
    ipc::serve(engine.clone(), jobs.clone(), control.clone());
    loop {
        {
            let mut state = control.lock().unwrap();
            if state.closing {
                if !engine.status().busy() && !jobs.busy() {
                    break;
                }
            } else if state.interactive {
                drop(state);
                // The same engine, queue, IPC channel and process become visible
                // only on an explicit show / manual GUI launch.
                crate::ui::run(engine.clone(), jobs.clone(), control.clone());
                break;
            } else if engine.status().busy() || jobs.busy() {
                state.last_busy = Instant::now();
            } else if state.last_busy.elapsed() >= Duration::from_secs(2) {
                // Keep the live result available if disk storage failed. Retry
                // before retiring rather than losing a completed operation.
                if let Err(error) = engine.status().persist(&engine.store) {
                    engine.note(format!(
                        "Cierre temporal pendiente: no se pudo guardar el resultado: {error}"
                    ));
                    state.last_busy = Instant::now();
                } else {
                    state.closing = true;
                    jobs.shutdown();
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
