//! Manual-reset notifications never hold the owner's request pipe while a CLI waits.
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    time::Duration,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, OpenEventW, SetEvent, WaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE,
};

fn name(owner: u64, job: &str, phase: &str) -> Vec<u16> {
    crate::wide(&format!("Local\\Soma-{owner}-{job}-{phase}"))
}

pub(crate) struct Signal(OwnedHandle);
impl Signal {
    fn new(job: &str, phase: &str) -> Option<Self> {
        let name = name(std::process::id().into(), job, phase);
        let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, name.as_ptr()) };
        (!handle.is_null()).then(|| Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }
    pub fn set(&self) {
        unsafe {
            SetEvent(self.0.as_raw_handle());
        }
    }
}

pub(crate) struct Signals {
    pub prepared: Signal,
    pub finished: Signal,
}
impl Signals {
    pub fn new(job: &str) -> Option<Self> {
        Some(Self {
            prepared: Signal::new(job, "prepared")?,
            finished: Signal::new(job, "finished")?,
        })
    }
}
pub(crate) fn wait(owner: u64, job: &str, phase: &str, timeout: Duration) -> bool {
    if !crate::jobs::valid_id(job) {
        return false;
    }
    let handle = unsafe {
        OpenEventW(
            SYNCHRONIZATION_SYNCHRONIZE,
            0,
            name(owner, job, phase).as_ptr(),
        )
    };
    if handle.is_null() {
        return false;
    }
    let event = unsafe { OwnedHandle::from_raw_handle(handle) };
    unsafe {
        WaitForSingleObject(event.as_raw_handle(), timeout.as_millis() as u32)
            != windows_sys::Win32::Foundation::WAIT_FAILED
    }
}
