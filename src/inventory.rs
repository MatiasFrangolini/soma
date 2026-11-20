//! Cache only sorted names. Every verification still reads and hashes every byte.
use crate::model::{Config, Project, Result};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT},
    Storage::FileSystem::*,
    System::Threading::WaitForSingleObject,
};

struct Watch(isize);
impl Drop for Watch {
    fn drop(&mut self) {
        unsafe {
            FindCloseChangeNotification(self.0 as HANDLE);
        }
    }
}
fn directory(path: &Path) -> Option<File> {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .ok()
}
fn identity(file: &File) -> Option<(u32, u32, u32)> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    (unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0).then_some((
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ))
}
struct Entry {
    path: PathBuf,
    inputs: Vec<String>,
    excludes: Vec<String>,
    root: File,
    watch: Watch,
    files: Arc<Vec<PathBuf>>,
}
impl Entry {
    fn new(p: &Project, c: &Config) -> Option<Self> {
        // Restrict the index to direct local roots. Other layouts use a full walk.
        if c.inputs.iter().any(|s| {
            let mut parts = Path::new(s).components();
            !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some()
        }) {
            return None;
        }
        let Some(Component::Prefix(prefix)) = p.directory.components().next() else {
            return None;
        };
        let std::path::Prefix::Disk(drive) = prefix.kind() else {
            return None;
        };
        const DRIVE_FIXED: u32 = 3;
        let drive_root = [drive as u16, b':' as u16, b'\\' as u16, 0];
        if unsafe { GetDriveTypeW(drive_root.as_ptr()) } != DRIVE_FIXED {
            return None;
        }
        let root = directory(&p.directory)?;
        let mut fs_name = [0u16; 16];
        if unsafe {
            GetVolumeInformationByHandleW(
                root.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs_name.as_mut_ptr(),
                16,
            )
        } == 0
            || String::from_utf16_lossy(&fs_name[..4]) != "NTFS" || fs_name[4] != 0
        {
            return None;
        }
        let native = std::os::windows::ffi::OsStrExt::encode_wide(p.directory.as_os_str())
            .chain(Some(0))
            .collect::<Vec<_>>();
        let handle = unsafe {
            FindFirstChangeNotificationW(
                native.as_ptr(),
                1,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_DIR_NAME
                    | FILE_NOTIFY_CHANGE_ATTRIBUTES,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        Some(Self {
            path: p.directory.clone(),
            inputs: c.inputs.clone(),
            excludes: c.input_excludes.clone(),
            root,
            watch: Watch(handle as isize),
            files: Arc::default(),
        })
    }
    fn current(&self) -> bool {
        identity(&self.root)
            .is_some_and(|id| directory(&self.path).and_then(|f| identity(&f)) == Some(id))
            && unsafe { WaitForSingleObject(self.watch.0 as HANDLE, 0) } == WAIT_TIMEOUT
    }
}

#[derive(Default)]
pub(crate) struct Inventory(Mutex<VecDeque<Entry>>);
impl Inventory {
    pub fn files(
        &self,
        p: &Project,
        c: &Config,
        scan: impl FnOnce() -> Result<(Vec<PathBuf>, bool)>,
    ) -> Result<(Arc<Vec<PathBuf>>, bool)> {
        {
            let mut cache = self.0.lock().unwrap();
            if let Some(index) = cache.iter().position(|e| {
                e.path == p.directory && e.inputs == c.inputs && e.excludes == c.input_excludes
            }) {
                if cache[index].current() {
                    return Ok((cache[index].files.clone(), true));
                }
                cache.remove(index);
            }
        }
        let entry = Entry::new(p, c);
        let (files, cacheable) = scan()?;
        let files = Arc::new(files);
        if let Some(mut entry) = entry.filter(|e| {
            cacheable
                && e.current()
                && files
                    .iter()
                    .map(|p| p.as_os_str().len() + std::mem::size_of::<PathBuf>())
                    .sum::<usize>()
                    <= 2 * 1024 * 1024
        }) {
            entry.files = files.clone();
            let mut cache = self.0.lock().unwrap();
            cache.retain(|other| other.path != entry.path);
            if cache.len() >= 4 {
                cache.pop_front();
            }
            cache.push_back(entry);
        }
        Ok((files, false))
    }
}
