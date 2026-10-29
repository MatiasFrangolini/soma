pub mod cli;
pub mod engine;
pub mod git;
pub mod ipc;
pub mod jobs;
pub mod jobs_cli;
mod job_output;
mod test_profiles;
mod job_history;
pub mod model;
pub mod observation;
pub mod owner;
pub mod readiness;
pub mod reports;
pub mod ui;
pub mod worker;

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
pub fn error(text: impl ToString) -> serde_json::Value {
    serde_json::json!({"ok":false,"error":text.to_string()})
}

pub fn launch_windowed(program: &std::path::Path, arguments: &str) -> std::io::Result<()> {
    unsafe {
        use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::*};
        let mut command = wide(&format!("\"{}\" {}", program.display(), arguments));
        let startup = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..std::mem::zeroed()
        };
        let mut process: PROCESS_INFORMATION = std::mem::zeroed();
        // Interactive windows must not retain the caller's captured CLI or execution pipes.
        if CreateProcessW(
            std::ptr::null(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            0,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut process,
        ) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
        Ok(())
    }
}
