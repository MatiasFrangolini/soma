use crate::model::CommandSpec;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, BufRead},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT},
    System::Console::{
        GenerateConsoleCtrlEvent, GetStdHandle, SetConsoleCP, SetConsoleCtrlHandler,
        SetConsoleOutputCP, CTRL_C_EVENT, STD_INPUT_HANDLE,
    },
};

#[derive(Serialize, Deserialize)]
pub struct Launch {
    pub command: CommandSpec,
    pub environment: BTreeMap<String, String>,
}
unsafe extern "system" fn control_handler(event: u32) -> i32 {
    if event == CTRL_C_EVENT {
        1
    } else {
        0
    }
}
pub fn run() -> i32 {
    unsafe {
        SetConsoleCP(65001);
        SetConsoleOutputCP(65001);
        SetConsoleCtrlHandler(Some(control_handler), 1);
        SetHandleInformation(GetStdHandle(STD_INPUT_HANDLE), HANDLE_FLAG_INHERIT, 0);
    }
    let mut input = io::stdin().lock();
    let mut first = String::new();
    if input.read_line(&mut first).is_err() || first.len() > 1024 * 1024 {
        return 125;
    }
    let launch: Launch = match serde_json::from_str(&first) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("Worker: solicitud inválida");
            return 125;
        }
    };
    drop(input);
    let cmd = &launch.command;
    let mut process = Command::new(&cmd.executable);
    process
        .args(&cmd.args)
        .env_clear()
        .envs(&launch.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(dir) = &cmd.directory {
        process.current_dir(dir);
    }
    let mut child = match process.spawn() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("No se pudo iniciar {}: {e}", cmd.executable);
            return 126;
        }
    };
    let stopping = Arc::new(AtomicBool::new(false));
    let flag = stopping.clone();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if matches!(line.as_deref(), Ok("stop")) || line.is_err() {
                flag.store(true, Ordering::SeqCst);
                break;
            }
        }
    });
    let mut sent = false;
    loop {
        match child.try_wait() {
            Ok(Some(code)) => return code.code().unwrap_or(1),
            Err(_) => return 125,
            _ => {}
        }
        if stopping.load(Ordering::SeqCst) && !sent {
            unsafe {
                GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
            }
            sent = true;
        }
        std::thread::sleep(Duration::from_millis(75));
    }
}
