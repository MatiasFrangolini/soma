//! Cache only sorted names. Every verification still reads and hashes every byte.
mod notifications;
use crate::model::{Config, Project, Result};
use notifications::Watch;
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};
use windows_sys::Win32::Storage::FileSystem::*;
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
    watch: Mutex<Option<Watch>>,
    files: Arc<Vec<PathBuf>>,
}
impl Entry {
    fn new(p: &Project, c: &Config) -> Option<Self> {
        // Restrict the index to direct local roots. Other layouts use a full walk.
        if c.inputs.iter().any(|s| {
            let mut parts = Path::new(s).components();
            !matches!(parts.next(), Some(Component::Normal(name)) if name == s.as_str())
                || parts.next().is_some()
                || s.ends_with(['.', ' '])
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
        let root = OpenOptions::new()
            .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED)
            .open(&p.directory)
            .ok()?;
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
            || String::from_utf16_lossy(&fs_name[..4]) != "NTFS"
            || fs_name[4] != 0
        {
            return None;
        }
        let watch = Watch::new(root.try_clone().ok()?)?;
        Some(Self {
            path: p.directory.clone(),
            inputs: c.inputs.clone(),
            excludes: c.input_excludes.clone(),
            root,
            watch: Mutex::new(Some(watch)),
            files: Arc::default(),
        })
    }
    fn current(&self) -> bool {
        let mut watch = self.watch.lock().unwrap();
        let current = identity(&self.root)
            .is_some_and(|id| directory(&self.path).and_then(|f| identity(&f)) == Some(id))
            && watch
                .as_mut()
                .is_some_and(|watch| watch.unchanged(|name| self.relevant(name)));
        if !current {
            *watch = None;
        }
        current
    }

    fn relevant(&self, name: &str) -> bool {
        // Short names and non-ASCII case folding can alias configured inputs.
        if name.contains('~')
            || !name.is_ascii()
            || self.inputs.iter().any(|s| !s.is_ascii() || s.contains('~'))
        {
            return true;
        }
        let path = Path::new(name);
        let Some(Component::Normal(first)) = path.components().next() else {
            return true;
        };
        self.inputs
            .iter()
            .any(|s| first.eq_ignore_ascii_case(s.as_str()))
            && !path
                .ancestors()
                .any(|p| crate::model::ignored_relative(p, &self.excludes))
    }
}

#[derive(Default)]
pub(crate) struct Inventory(Mutex<VecDeque<Arc<Entry>>>);

pub(crate) struct Selection {
    pub names: Arc<Vec<PathBuf>>,
    pub hit: bool,
    witness: Option<Arc<Entry>>,
}
impl Selection {
    pub fn uncached(names: Vec<PathBuf>) -> Self {
        Self {
            names: Arc::new(names),
            hit: false,
            witness: None,
        }
    }
    pub fn changed(&self) -> bool {
        self.witness.as_ref().is_some_and(|w| !w.current())
    }
    pub fn observed(&self) -> bool {
        self.witness.is_some()
    }
}

impl Inventory {
    pub fn files(
        &self,
        p: &Project,
        c: &Config,
        scan: impl FnOnce() -> Result<(Vec<PathBuf>, bool)>,
    ) -> Result<Selection> {
        {
            let mut cache = self.0.lock().unwrap();
            if let Some(index) = cache.iter().position(|e| {
                e.path == p.directory && e.inputs == c.inputs && e.excludes == c.input_excludes
            }) {
                if cache[index].current() {
                    let entry = cache[index].clone();
                    return Ok(Selection {
                        names: entry.files.clone(),
                        hit: true,
                        witness: Some(entry),
                    });
                }
                cache.remove(index);
            }
        }
        self.scan(p, c, scan)
    }

    pub fn scan(
        &self,
        p: &Project,
        c: &Config,
        scan: impl FnOnce() -> Result<(Vec<PathBuf>, bool)>,
    ) -> Result<Selection> {
        let entry = Entry::new(p, c);
        let (files, cacheable) = scan()?;
        let files = Arc::new(files);
        let witness = entry.filter(|_| cacheable).map(|mut entry| {
            entry.files = files.clone();
            Arc::new(entry)
        });
        if let Some(entry) = witness.as_ref().filter(|e| {
            e.current()
                && files
                    .iter()
                    .map(|p| p.as_os_str().len() + std::mem::size_of::<PathBuf>())
                    .sum::<usize>()
                    <= 2 * 1024 * 1024
        }) {
            let mut cache = self.0.lock().unwrap();
            cache.retain(|other| other.path != entry.path);
            if cache.len() >= 4 {
                cache.pop_front();
            }
            cache.push_back(entry.clone());
        }
        Ok(Selection {
            names: files,
            hit: false,
            witness,
        })
    }
}
