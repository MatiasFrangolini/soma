use std::{
    fs::File,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_IO_INCOMPLETE},
    Storage::FileSystem::*,
    System::{
        Threading::CreateEventW,
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
    },
};

// One outstanding read per directory handle. Windows buffers changes between reads.
pub(super) struct Watch {
    directory: File,
    event: OwnedHandle,
    operation: Box<OVERLAPPED>,
    buffer: Box<[u32; 8192]>,
    pending: bool,
}
// The event and pinned allocations outlive each request. Entry serializes access.
unsafe impl Send for Watch {}

impl Watch {
    pub(super) fn new(directory: File) -> Option<Self> {
        let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if event.is_null() {
            return None;
        }
        let mut watch = Self {
            directory,
            event: unsafe { OwnedHandle::from_raw_handle(event) },
            operation: Box::default(),
            buffer: Box::new([0; 8192]),
            pending: false,
        };
        watch.arm().then_some(watch)
    }

    fn arm(&mut self) -> bool {
        *self.operation = OVERLAPPED::default();
        self.operation.hEvent = self.event.as_raw_handle();
        self.pending = unsafe {
            ReadDirectoryChangesW(
                self.directory.as_raw_handle(),
                self.buffer.as_mut_ptr().cast(),
                std::mem::size_of_val(&*self.buffer) as u32,
                1,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_DIR_NAME
                    | FILE_NOTIFY_CHANGE_ATTRIBUTES,
                ptr::null_mut(),
                &mut *self.operation,
                None,
            ) != 0
        };
        self.pending
    }

    pub(super) fn unchanged(&mut self, relevant: impl Fn(&str) -> bool) -> bool {
        // Bound work on a noisy tree. Overflow, malformed paths and other errors
        // leave this watch permanently stale so callers use a full enumeration.
        for _ in 0..8 {
            if !self.pending {
                return false;
            }
            let mut size = 0;
            if unsafe {
                GetOverlappedResult(
                    self.directory.as_raw_handle(),
                    &*self.operation,
                    &mut size,
                    0,
                )
            } == 0
            {
                return unsafe { GetLastError() } == ERROR_IO_INCOMPLETE;
            }
            self.pending = false;
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    self.buffer.as_ptr().cast::<u8>(),
                    std::mem::size_of_val(&*self.buffer),
                )
            };
            let Some(mut records) = bytes.get(..size as usize) else {
                return false;
            };
            loop {
                if records.len() < 12 {
                    return false;
                }
                let offset = u32::from_le_bytes(records[..4].try_into().unwrap()) as usize;
                let len = u32::from_le_bytes(records[8..12].try_into().unwrap()) as usize;
                if len == 0 || len % 2 != 0 || len > records.len() - 12 {
                    return false;
                }
                let name = String::from_utf16(
                    &records[12..12 + len]
                        .chunks_exact(2)
                        .map(|s| u16::from_le_bytes([s[0], s[1]]))
                        .collect::<Vec<_>>(),
                );
                if !matches!(name, Ok(ref path) if !relevant(path)) {
                    return false;
                }
                if offset == 0 {
                    break;
                }
                if offset < 12 + len || offset > records.len() {
                    return false;
                }
                records = &records[offset..];
            }
            if !self.arm() {
                return false;
            }
        }
        false
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if self.pending {
            unsafe {
                CancelIoEx(self.directory.as_raw_handle(), &*self.operation);
                GetOverlappedResult(self.directory.as_raw_handle(), &*self.operation, &mut 0, 1);
            }
        }
    }
}
