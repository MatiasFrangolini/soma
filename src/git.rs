use crate::model::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};
use windows_sys::Win32::{Foundation::*, System::Threading::*};

fn invoke(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| format!("Git no disponible: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}

pub fn root(dir: &Path) -> Option<PathBuf> {
    invoke(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .and_then(|s| std::fs::canonicalize(s.trim()).ok())
        .map(crate::model::plain)
}

pub fn same_project(base: &Path, candidate: &Path) -> bool {
    if base == candidate {
        return true;
    }
    let (Some(a), Some(b)) = (root(base), root(candidate)) else {
        return false;
    };
    if base.strip_prefix(&a).ok() != candidate.strip_prefix(&b).ok() {
        return false;
    }
    let common = |p: &Path| {
        invoke(
            p,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .ok()
        .and_then(|s| std::fs::canonicalize(s.trim()).ok())
    };
    common(base).is_some_and(|value| Some(value) == common(candidate))
}

#[derive(Clone, Serialize)]
pub struct Worktree {
    pub directory: PathBuf,
    pub branch: String,
    pub head: String,
    pub locked: bool,
    pub prunable: bool,
}

pub fn worktrees(dir: &Path) -> Vec<Worktree> {
    let offset = root(dir)
        .and_then(|r| dir.strip_prefix(r).ok().map(Path::to_path_buf))
        .unwrap_or_default();
    let Ok(raw) = invoke(dir, &["worktree", "list", "--porcelain", "-z"]) else {
        return vec![];
    };
    let mut values = Vec::new();
    for block in raw.split("\0\0") {
        let mut value = Worktree {
            directory: PathBuf::new(),
            branch: "HEAD detached".into(),
            head: String::new(),
            locked: false,
            prunable: false,
        };
        for field in block.split('\0') {
            if let Some(path) = field.strip_prefix("worktree ") {
                value.directory = PathBuf::from(path).join(&offset);
            }
            if let Some(head) = field.strip_prefix("HEAD ") {
                value.head = head.into();
            }
            if let Some(branch) = field.strip_prefix("branch refs/heads/") {
                value.branch = branch.into();
            }
            if field.starts_with("locked") {
                value.locked = true;
            }
            if field.starts_with("prunable") {
                value.prunable = true;
            }
        }
        if value.directory.is_dir() && !value.prunable {
            value.directory = crate::model::plain(
                std::fs::canonicalize(&value.directory).unwrap_or(value.directory),
            );
            values.push(value);
        }
    }
    values
}

pub fn branches(dir: &Path) -> Vec<String> {
    invoke(
        dir,
        &[
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname:short)",
            "refs/heads",
        ],
    )
    .map(|s| s.lines().map(str::to_owned).collect())
    .unwrap_or_default()
}

pub fn head(dir: &Path) -> Option<String> {
    invoke(dir, &["rev-parse", "HEAD"])
        .ok()
        .map(|s| s.trim().into())
}

pub fn switch(dir: &Path, branch: &str) -> Result<()> {
    if !branches(dir).iter().any(|b| b == branch) {
        return Err("Elegí una rama local existente".into());
    }
    if crate::model::branch(dir) == branch {
        return Ok(());
    }
    if !invoke(dir, &["status", "--porcelain", "--untracked-files=normal"])?
        .trim()
        .is_empty()
    {
        return Err("Hay cambios locales. Guardalos antes de cambiar de rama; también podés seleccionar otro worktree".into());
    }
    invoke(dir, &["switch", "--no-guess", branch]).map(|_| ())
}

// Shared by GUI owners with different data directories; checkout changes must not
// race a build or a running application in another Soma window.
pub struct CheckoutLease(HANDLE);
impl CheckoutLease {
    pub fn acquire(dir: &Path) -> Result<Self> {
        Self::named(dir, "Checkout")
    }
    pub fn runtime(dir: &Path) -> Result<Self> {
        Self::named(dir, "Runtime")
    }
    fn named(dir: &Path, kind: &str) -> Result<Self> {
        let hash = format!(
            "{:x}",
            Sha256::digest(dir.to_string_lossy().to_lowercase().as_bytes())
        );
        let name = crate::wide(&format!("Local\\Soma{kind}-{hash}"));
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if handle.is_null() {
                return Err("No se pudo proteger el directorio de ejecución".into());
            }
            match WaitForSingleObject(handle, 0) {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self(handle)),
                _ => {
                    CloseHandle(handle);
                    Err("checkout_busy: otro Soma está usando este directorio".into())
                }
            }
        }
    }
}
impl Drop for CheckoutLease {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}
