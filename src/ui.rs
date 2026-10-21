use crate::{
    engine::Engine,
    ipc,
    model::{Settings, Store},
    wide,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
    System::{Com::*, LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

const PROJECT: i32 = 101;
const CONFIG: i32 = 102;
const FOLDER: i32 = 103;
const REFRESH: i32 = 104;
const EDIT_CONFIG: i32 = 105;
const WORKTREE: i32 = 106;
const BUILD: i32 = 110;
const RUN: i32 = 111;
const REUSE: i32 = 112;
const STOP: i32 = 113;
const TERMINAL: i32 = 114;
const LOGS: i32 = 115;
const LOG_VIEW: i32 = 120;
const SEARCH: i32 = 121;
const FIND: i32 = 122;
const PAUSE: i32 = 123;
const CLEAR: i32 = 124;
const TITLE: i32 = 130;
const DIR: i32 = 131;
const BRANCH: i32 = 132;
const STATUS: i32 = 133;
const HINT: i32 = 134;
const P_LABEL: i32 = 135;
const C_LABEL: i32 = 136;
const W_LABEL: i32 = 137;
const B_LABEL: i32 = 138;
fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
unsafe fn window_icons(window: HWND) {
    let instance = GetModuleHandleW(std::ptr::null());
    let dpi = GetDpiForWindow(window);
    for (kind, x, y) in [
        (ICON_BIG, SM_CXICON, SM_CYICON),
        (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
    ] {
        let icon = LoadImageW(
            instance,
            101usize as _,
            IMAGE_ICON,
            GetSystemMetricsForDpi(x, dpi),
            GetSystemMetricsForDpi(y, dpi),
            LR_SHARED,
        );
        if !icon.is_null() {
            SendMessageW(window, WM_SETICON, kind as usize, icon as isize);
        }
    }
}
struct Ui {
    engine: Arc<Engine>,
    jobs: Arc<crate::jobs::Jobs>,
    settings: Settings,
    controls: BTreeMap<i32, HWND>,
    font: HFONT,
    heading: HFONT,
    background: HBRUSH,
    edit_bg: HBRUSH,
    scale: f64,
    paused: bool,
    closing: bool,
    generation: u64,
    displayed: String,
    search_at: usize,
    last_status: String,
    refreshing: bool,
    worktrees: Vec<crate::git::Worktree>,
    branches: Vec<String>,
}
impl Ui {
    fn new(engine: Arc<Engine>, jobs: Arc<crate::jobs::Jobs>) -> Self {
        unsafe {
            Self {
                engine,
                jobs,
                settings: Settings::default(),
                controls: BTreeMap::new(),
                font: std::ptr::null_mut(),
                heading: std::ptr::null_mut(),
                background: CreateSolidBrush(rgb(22, 26, 32)),
                edit_bg: CreateSolidBrush(rgb(13, 17, 22)),
                scale: GetDpiForSystem() as f64 / 96.0,
                paused: false,
                closing: false,
                generation: u64::MAX,
                displayed: String::new(),
                search_at: 0,
                last_status: String::new(),
                refreshing: false,
                worktrees: Vec::new(),
                branches: Vec::new(),
            }
        }
    }
    fn h(&self, id: i32) -> HWND {
        self.controls
            .get(&id)
            .copied()
            .unwrap_or(std::ptr::null_mut())
    }
    fn px(&self, value: i32) -> i32 {
        (value as f64 * self.scale).round() as i32
    }
    unsafe fn create(&mut self, parent: HWND, id: i32, class: &str, text: &str, style: u32) {
        let h = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            10,
            10,
            parent,
            id as usize as _,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        self.controls.insert(id, h);
    }
    unsafe fn fonts(&mut self) {
        let old = self.font;
        let old_heading = self.heading;
        self.font = CreateFontW(
            -self.px(14),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            FIXED_PITCH as u32,
            wide("Consolas").as_ptr(),
        );
        self.heading = CreateFontW(
            -self.px(26),
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            wide("Segoe UI").as_ptr(),
        );
        for (id, h) in &self.controls {
            SendMessageW(
                *h,
                WM_SETFONT,
                if *id == TITLE {
                    self.heading
                } else {
                    self.font
                } as usize,
                1,
            );
        }
        if !old.is_null() {
            DeleteObject(old);
        }
        if !old_heading.is_null() {
            DeleteObject(old_heading);
        }
    }
    unsafe fn initialize(&mut self, h: HWND) {
        self.create(h, TITLE, "STATIC", "Soma", 0);
        self.create(
            h,
            HINT,
            "STATIC",
            "Un proyecto. Una ejecución. Control desde CLI.",
            0,
        );
        self.create(h, P_LABEL, "STATIC", "Proyecto", 0);
        self.create(h, C_LABEL, "STATIC", "Config", 0);
        self.create(h, W_LABEL, "STATIC", "Worktree", 0);
        self.create(h, B_LABEL, "STATIC", "Rama", 0);
        self.create(
            h,
            PROJECT,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL,
        );
        self.create(
            h,
            CONFIG,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL,
        );
        for id in [WORKTREE, BRANCH] {
            self.create(
                h,
                id,
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL,
            );
        }
        for (id, text) in [
            (FOLDER, "&Carpeta..."),
            (REFRESH, "&Actualizar"),
            (EDIT_CONFIG, "Abrir &config"),
            (BUILD, "&Build"),
            (RUN, "Compilar y &Run"),
            (REUSE, "Run build &vigente"),
            (STOP, "&Stop"),
            (TERMINAL, "&Terminal"),
            (LOGS, "Abrir &logs"),
            (FIND, "Buscar"),
            (PAUSE, "Pausar visor"),
            (CLEAR, "Limpiar visor"),
        ] {
            self.create(h, id, "BUTTON", text, BS_OWNERDRAW as u32 | WS_TABSTOP);
        }
        for id in [DIR, STATUS] {
            self.create(h, id, "STATIC", "", 0x0000000C);
        }
        self.create(h,LOG_VIEW,"EDIT","Elegí un proyecto y una configuración para comenzar.\r\n\r\nLos agentes pueden usar soma-cli.exe help.\r\n",ES_MULTILINE as u32|ES_READONLY as u32|ES_AUTOVSCROLL as u32|ES_AUTOHSCROLL as u32|WS_VSCROLL|WS_HSCROLL|WS_TABSTOP);
        SendMessageW(
            self.h(LOG_VIEW),
            EM_SETLIMITTEXT,
            (4 * 1024 * 1024) as usize,
            0,
        );
        self.create(
            h,
            SEARCH,
            "EDIT",
            "",
            ES_AUTOHSCROLL as u32 | WS_TABSTOP | WS_BORDER,
        );
        SendMessageW(
            self.h(SEARCH),
            EM_SETCUEBANNER,
            0,
            wide("Buscar texto (distingue mayúsculas)").as_ptr() as isize,
        );
        self.fonts();
        self.reload(None);
        SetTimer(h, 1, 300, None);
        self.layout(h);
    }
    unsafe fn text(&self, id: i32, text: &str) {
        SetWindowTextW(self.h(id), wide(text).as_ptr());
    }
    unsafe fn reload(&mut self, select: Option<String>) {
        let previous = select.or_else(|| self.selected().map(|(p, _)| p));
        match self.engine.store.load() {
            Ok(s) => {
                self.settings = s;
                SendMessageW(self.h(PROJECT), CB_RESETCONTENT, 0, 0);
                for p in &self.settings.projects {
                    SendMessageW(
                        self.h(PROJECT),
                        CB_ADDSTRING,
                        0,
                        wide(&p.name).as_ptr() as isize,
                    );
                }
                let index = previous
                    .and_then(|id| self.settings.projects.iter().position(|p| p.id == id))
                    .unwrap_or(0);
                SendMessageW(self.h(PROJECT), CB_SETCURSEL, index, 0);
                self.project_changed();
            }
            Err(e) => self.engine.note(e),
        }
    }
    unsafe fn selected(&self) -> Option<(String, Option<String>)> {
        let i = SendMessageW(self.h(PROJECT), CB_GETCURSEL, 0, 0);
        let p = self.settings.projects.get(i as usize)?;
        let j = SendMessageW(self.h(CONFIG), CB_GETCURSEL, 0, 0);
        Some((
            p.id.clone(),
            p.configs.get(j as usize).map(|c| c.id.clone()),
        ))
    }
    unsafe fn project_changed(&mut self) {
        SendMessageW(self.h(CONFIG), CB_RESETCONTENT, 0, 0);
        let index = SendMessageW(self.h(PROJECT), CB_GETCURSEL, 0, 0);
        if let Some(p) = self.settings.projects.get(index as usize) {
            for c in &p.configs {
                SendMessageW(
                    self.h(CONFIG),
                    CB_ADDSTRING,
                    0,
                    wide(&c.name).as_ptr() as isize,
                );
            }
            SendMessageW(self.h(CONFIG), CB_SETCURSEL, 0, 0);
            self.worktrees = crate::git::worktrees(&p.directory);
            if self.worktrees.is_empty() {
                self.worktrees.push(crate::git::Worktree {
                    directory: p.directory.clone(),
                    branch: crate::model::branch(&p.directory),
                    head: crate::git::head(&p.directory).unwrap_or_default(),
                    locked: false,
                    prunable: false,
                });
            }
            SendMessageW(self.h(WORKTREE), CB_RESETCONTENT, 0, 0);
            for item in &self.worktrees {
                let label = format!("{} · {}", item.branch, item.directory.display());
                SendMessageW(
                    self.h(WORKTREE),
                    CB_ADDSTRING,
                    0,
                    wide(&label).as_ptr() as isize,
                );
            }
            let chosen = p.selected_worktree.as_ref().unwrap_or(&p.directory);
            let chosen = self
                .worktrees
                .iter()
                .position(|item| &item.directory == chosen)
                .unwrap_or(0);
            SendMessageW(self.h(WORKTREE), CB_SETCURSEL, chosen, 0);
            self.checkout_changed();
        } else {
            self.text(
                DIR,
                "Sin proyectos. Elegí una carpeta o usá la CLI para registrar uno.",
            );
            SendMessageW(self.h(WORKTREE), CB_RESETCONTENT, 0, 0);
            SendMessageW(self.h(BRANCH), CB_RESETCONTENT, 0, 0);
        }
    }
    unsafe fn selected_directory(&self) -> Option<PathBuf> {
        let index = SendMessageW(self.h(WORKTREE), CB_GETCURSEL, 0, 0);
        self.worktrees
            .get(index as usize)
            .map(|item| item.directory.clone())
    }
    unsafe fn checkout_changed(&mut self) {
        if let Some(directory) = self.selected_directory() {
            self.text(DIR, &directory.to_string_lossy());
            self.branches = crate::git::branches(&directory);
            let current = crate::model::branch(&directory);
            if current == "HEAD detached" {
                self.branches.insert(0, current.clone());
            }
            SendMessageW(self.h(BRANCH), CB_RESETCONTENT, 0, 0);
            for branch in &self.branches {
                SendMessageW(
                    self.h(BRANCH),
                    CB_ADDSTRING,
                    0,
                    wide(branch).as_ptr() as isize,
                );
            }
            if let Some(index) = self.branches.iter().position(|branch| branch == &current) {
                SendMessageW(self.h(BRANCH), CB_SETCURSEL, index, 0);
            }
            EnableWindow(
                self.h(BRANCH),
                (!self.engine.status().busy() && !self.branches.is_empty()) as i32,
            );
        }
    }
    unsafe fn layout(&self, h: HWND) {
        if self.controls.is_empty() {
            return;
        }
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(h, &mut rect);
        let w = (rect.right as f64 / self.scale) as i32;
        let height = (rect.bottom as f64 / self.scale) as i32;
        let pad = 22;
        let avail = w - 2 * pad;
        let place = |id: i32, x: i32, y: i32, width: i32, height: i32| {
            MoveWindow(
                self.h(id),
                self.px(x),
                self.px(y),
                self.px(width.max(1)),
                self.px(height.max(1)),
                1,
            );
        };
        place(TITLE, pad, 14, 220, 38);
        place(HINT, pad + 230, 27, (avail - 230).max(100), 22);
        place(P_LABEL, pad, 73, 82, 24);
        place(PROJECT, pad + 86, 68, (avail - 370).max(180), 300);
        place(FOLDER, w - pad - 268, 68, 142, 31);
        place(REFRESH, w - pad - 114, 68, 114, 31);
        place(W_LABEL, pad, 111, 82, 24);
        place(WORKTREE, pad + 86, 106, avail - 86, 300);
        place(DIR, pad, 147, avail, 24);
        place(C_LABEL, pad, 187, 82, 24);
        place(CONFIG, pad + 86, 181, 220, 300);
        place(EDIT_CONFIG, pad + 318, 181, 142, 31);
        place(B_LABEL, pad + 478, 187, 44, 24);
        place(BRANCH, pad + 528, 181, (avail - 528).max(50), 300);
        let mut x = pad;
        let mut y = 233;
        for (id, bw) in [
            (BUILD, 86),
            (RUN, 164),
            (REUSE, 192),
            (STOP, 80),
            (TERMINAL, 116),
            (LOGS, 116),
        ] {
            if x + bw > w - pad {
                x = pad;
                y += 41;
            }
            place(id, x, y, bw, 32);
            x += bw + 10;
        }
        y += 48;
        place(STATUS, pad, y, avail, 24);
        y += 33;
        place(LOG_VIEW, pad, y, avail, (height - y - 68).max(60));
        let footer = height - 43;
        place(SEARCH, pad, footer, (avail - 440).max(130), 28);
        place(FIND, w - pad - 428, footer, 78, 28);
        place(PAUSE, w - pad - 338, footer, 164, 28);
        place(CLEAR, w - pad - 162, footer, 162, 28);
    }
    unsafe fn refresh(&mut self, h: HWND) {
        // SetWindowText and selection messages synchronously notify the parent.
        // A continuously changing log must never recursively refresh the view.
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        let state = self.engine.status();
        let busy = state.busy();
        if busy {
            if let (Some(project), Some(directory)) = (&state.project, &state.directory) {
                if self.selected().map(|(p, _)| p).as_ref() != Some(project)
                    || self.selected_directory().as_ref() != Some(directory)
                    || self.selected().and_then(|(_, c)| c).as_ref() != state.config.as_ref()
                {
                    self.reload(Some(project.clone()));
                    if let Some(index) = self
                        .worktrees
                        .iter()
                        .position(|item| &item.directory == directory)
                    {
                        SendMessageW(self.h(WORKTREE), CB_SETCURSEL, index, 0);
                        self.checkout_changed();
                    }
                    let project_index = SendMessageW(self.h(PROJECT), CB_GETCURSEL, 0, 0);
                    if let Some(p) = self.settings.projects.get(project_index as usize) {
                        if let Some(index) = p
                            .configs
                            .iter()
                            .position(|c| Some(&c.id) == state.config.as_ref())
                        {
                            SendMessageW(self.h(CONFIG), CB_SETCURSEL, index, 0);
                        }
                    }
                }
            }
        }
        let title = format!(
            "{}  {} · {}{}",
            if self.closing {
                "Cerrando"
            } else {
                &state.state
            },
            state.project.as_deref().unwrap_or(""),
            match state.readiness.as_str() {
                "ready" => "API lista",
                "pending" => "Esperando API",
                "timed_out" => "API sin confirmar",
                _ => &state.phase,
            },
            state
                .error
                .as_ref()
                .map(|e| format!(" · {e}"))
                .unwrap_or_else(|| format!(" · {}", state.phase))
        );
        if title != self.last_status {
            self.text(STATUS, &title);
            self.last_status = title;
            for id in [BUILD, RUN, REUSE, PROJECT, CONFIG, WORKTREE, FOLDER] {
                EnableWindow(self.h(id), (!busy && !self.closing) as i32);
            }
            EnableWindow(
                self.h(BRANCH),
                (!busy && !self.closing && !self.branches.is_empty()) as i32,
            );
            EnableWindow(self.h(STOP), busy as i32);
        }
        if !self.paused {
            let logs = self.engine.logs.lock().unwrap();
            if logs.generation != self.generation && logs.generation != 0 {
                self.generation = logs.generation;
                self.displayed = logs.snapshot();
                drop(logs);
                let shown = self
                    .displayed
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .replace('\n', "\r\n");
                self.text(LOG_VIEW, &shown);
                let end = shown.encode_utf16().count();
                SendMessageW(self.h(LOG_VIEW), EM_SETSEL, end, end as isize);
                SendMessageW(self.h(LOG_VIEW), EM_SCROLLCARET, 0, 0);
                SendMessageW(self.h(LOG_VIEW), EM_EMPTYUNDOBUFFER, 0, 0);
                InvalidateRect(self.h(LOG_VIEW), std::ptr::null(), 1);
            }
        }
        self.refreshing = false;
        if self.closing && !busy && !self.jobs.busy() {
            DestroyWindow(h);
        }
    }
    unsafe fn open(&self, path: &std::path::Path) {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(&path.to_string_lossy()).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
    unsafe fn command(&mut self, h: HWND, id: i32, notification: u32) {
        if self.refreshing {
            return;
        }
        if id == PROJECT && notification == CBN_SELCHANGE {
            self.project_changed();
            return;
        }
        if id == WORKTREE && notification == CBN_SELCHANGE {
            if !self.engine.status().busy() {
                if let (Some((project, _)), Some(directory)) =
                    (self.selected(), self.selected_directory())
                {
                    if let Err(error) = self.engine.store.select_worktree(&project, &directory) {
                        self.engine.note(error);
                    }
                }
                self.checkout_changed();
            }
            return;
        }
        if id == BRANCH && notification == CBN_SELCHANGE {
            let index = SendMessageW(self.h(BRANCH), CB_GETCURSEL, 0, 0);
            if let (Some(branch), Some((project, _)), Some(directory)) = (
                self.branches.get(index as usize),
                self.selected(),
                self.selected_directory(),
            ) {
                if branch != "HEAD detached" {
                    let result = self
                        .engine
                        .switch_branch(&project, Some(&directory), branch);
                    if result["ok"] != true {
                        self.engine.note(
                            result["error"]
                                .as_str()
                                .unwrap_or("No se pudo cambiar de rama"),
                        );
                    }
                    let config_index = SendMessageW(self.h(CONFIG), CB_GETCURSEL, 0, 0);
                    self.reload(Some(project));
                    SendMessageW(self.h(CONFIG), CB_SETCURSEL, config_index as usize, 0);
                }
            }
            return;
        }
        if notification != BN_CLICKED
            || !matches!(
                id,
                FOLDER
                    | REFRESH
                    | EDIT_CONFIG
                    | BUILD
                    | RUN
                    | REUSE
                    | STOP
                    | TERMINAL
                    | LOGS
                    | FIND
                    | PAUSE
                    | CLEAR
            )
        {
            return;
        }
        match id {
            FOLDER => {
                let title = wide("Elegí el directorio del proyecto");
                let info = BROWSEINFOW {
                    hwndOwner: h,
                    lpszTitle: title.as_ptr(),
                    ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
                    ..std::mem::zeroed()
                };
                let pidl = SHBrowseForFolderW(&info);
                if !pidl.is_null() {
                    let mut path = [0u16; 32768];
                    if SHGetPathFromIDListW(pidl, path.as_mut_ptr()) != 0 {
                        let len = path.iter().position(|v| *v == 0).unwrap_or(0);
                        match self
                            .engine
                            .store
                            .add_directory(&PathBuf::from(String::from_utf16_lossy(&path[..len])))
                        {
                            Ok(id) => self.reload(Some(id)),
                            Err(e) => self.engine.note(e),
                        }
                    }
                    CoTaskMemFree(pidl as _);
                }
            }
            REFRESH => self.reload(None),
            EDIT_CONFIG => self.open(&self.engine.store.path()),
            LOGS => self.open(&self.engine.store.home.join("logs")),
            BUILD | RUN | REUSE => {
                if let Some((p, Some(c))) = self.selected() {
                    let result = self.engine.start_at(
                        &p,
                        Some(&c),
                        if id == BUILD {
                            "build"
                        } else if id == REUSE {
                            "reuse"
                        } else {
                            "run"
                        },
                        self.selected_directory().as_deref(),
                        None,
                    );
                    if result["ok"] != true {
                        self.engine
                            .note(result["error"].as_str().unwrap_or("No se pudo iniciar"));
                    }
                } else {
                    self.engine
                        .note("Creá una configuración en projects.json o mediante un agente");
                }
            }
            STOP => {
                self.engine.stop();
            }
            TERMINAL => {
                if let Some(directory) = self.selected_directory() {
                    if let Err(e) = crate::launch_windowed(
                        std::path::Path::new("wt.exe"),
                        &format!("-d \"{}\"", directory.display()),
                    ) {
                        self.engine
                            .note(format!("No se pudo abrir Windows Terminal: {e}"));
                    }
                }
            }
            PAUSE => {
                self.paused = !self.paused;
                self.text(
                    PAUSE,
                    if self.paused {
                        "Reanudar visor"
                    } else {
                        "Pausar visor"
                    },
                );
            }
            CLEAR => {
                self.engine.logs.lock().unwrap().clear();
                self.displayed.clear();
                self.text(LOG_VIEW, "");
            }
            FIND => {
                let len = GetWindowTextLengthW(self.h(SEARCH));
                let mut buf = vec![0u16; len as usize + 1];
                GetWindowTextW(self.h(SEARCH), buf.as_mut_ptr(), buf.len() as i32);
                let needle = String::from_utf16_lossy(&buf[..len as usize]);
                if !needle.is_empty() {
                    let text = self
                        .displayed
                        .replace("\r\n", "\n")
                        .replace('\r', "\n")
                        .replace('\n', "\r\n");
                    let start = self.search_at.min(text.len());
                    let start = if text.is_char_boundary(start) {
                        start
                    } else {
                        0
                    };
                    if let Some(pos) = text[start..]
                        .find(&needle)
                        .map(|n| n + start)
                        .or_else(|| text.find(&needle))
                    {
                        self.paused = true;
                        self.text(PAUSE, "Reanudar visor");
                        let a = text[..pos].encode_utf16().count();
                        let b = a + needle.encode_utf16().count();
                        SendMessageW(self.h(LOG_VIEW), EM_SETSEL, a, b as isize);
                        SendMessageW(self.h(LOG_VIEW), EM_SCROLLCARET, 0, 0);
                        self.search_at = pos + needle.len();
                        SetFocus(self.h(LOG_VIEW));
                    }
                }
            }
            _ => {}
        }
        self.refresh(h);
    }
}
impl Drop for Ui {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.font);
            DeleteObject(self.heading);
            DeleteObject(self.background);
            DeleteObject(self.edit_bg);
        }
    }
}
unsafe extern "system" fn window_proc(h: HWND, msg: u32, w: usize, l: isize) -> isize {
    if msg == WM_NCCREATE {
        let c = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(h, GWLP_USERDATA, c.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(h, GWLP_USERDATA) as *mut Ui;
    if ptr.is_null() {
        return DefWindowProcW(h, msg, w, l);
    }
    let ui = &mut *ptr;
    match msg {
        WM_CREATE => {
            ui.initialize(h);
            0
        }
        WM_SIZE => {
            ui.layout(h);
            0
        }
        WM_ACTIVATE => {
            if w & 0xffff != WA_INACTIVE as usize {
                ui.reload(None);
            }
            0
        }
        WM_TIMER => {
            ui.refresh(h);
            0
        }
        WM_COMMAND => {
            ui.command(h, (w & 0xffff) as i32, ((w >> 16) & 0xffff) as u32);
            0
        }
        WM_CLOSE | ipc::CLOSE => {
            ui.closing = true;
            ui.jobs.shutdown();
            ui.engine.stop();
            ui.refresh(h);
            0
        }
        ipc::SHOW => {
            ShowWindow(h, SW_RESTORE);
            SetForegroundWindow(h);
            0
        }
        WM_DESTROY => {
            KillTimer(h, 1);
            PostQuitMessage(0);
            0
        }
        WM_ERASEBKGND => {
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(h, &mut r);
            FillRect(w as _, &r, ui.background);
            1
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            let dc = w as HDC;
            SetTextColor(dc, rgb(206, 216, 226));
            SetBkMode(dc, OPAQUE as i32);
            let brush = if l as HWND == ui.h(LOG_VIEW) || l as HWND == ui.h(SEARCH) {
                ui.edit_bg
            } else {
                ui.background
            };
            SetBkColor(
                dc,
                if brush == ui.edit_bg {
                    rgb(13, 17, 22)
                } else {
                    rgb(22, 26, 32)
                },
            );
            brush as isize
        }
        WM_DRAWITEM => {
            let item = &*(l as *const DRAWITEMSTRUCT);
            let disabled = item.itemState & ODS_DISABLED != 0;
            let color = if disabled {
                rgb(37, 41, 47)
            } else if item.CtlID == RUN as u32 {
                rgb(29, 77, 63)
            } else if item.CtlID == STOP as u32 {
                rgb(78, 37, 43)
            } else {
                rgb(36, 43, 52)
            };
            let brush = CreateSolidBrush(color);
            FillRect(item.hDC, &item.rcItem, brush);
            DeleteObject(brush);
            let edge = CreateSolidBrush(rgb(63, 73, 85));
            FrameRect(item.hDC, &item.rcItem, edge);
            DeleteObject(edge);
            SetBkMode(item.hDC, TRANSPARENT as i32);
            SetTextColor(
                item.hDC,
                if disabled {
                    rgb(104, 112, 123)
                } else {
                    rgb(224, 232, 240)
                },
            );
            SelectObject(item.hDC, ui.font);
            let mut text = [0u16; 128];
            GetWindowTextW(item.hwndItem, text.as_mut_ptr(), 128);
            let mut rect = item.rcItem;
            DrawTextW(
                item.hDC,
                text.as_ptr(),
                -1,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            if item.itemState & ODS_FOCUS != 0 {
                InflateRect(&mut rect, -3, -3);
                DrawFocusRect(item.hDC, &rect);
            }
            1
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(l as *mut MINMAXINFO);
            info.ptMinTrackSize.x = ui.px(780);
            info.ptMinTrackSize.y = ui.px(560);
            0
        }
        WM_DPICHANGED => {
            ui.scale = (w & 0xffff) as f64 / 96.0;
            window_icons(h);
            ui.fonts();
            let r = &*(l as *const RECT);
            SetWindowPos(
                h,
                std::ptr::null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            ui.layout(h);
            0
        }
        _ => DefWindowProcW(h, msg, w, l),
    }
}
pub fn run() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let store = Store::from_args();
        // Check ownership before touching history or configuration. A restricted caller
        // may see the data directory but cannot open an existing owner's mutex.
        let mutex_name = wide(&format!("Local\\{}", ipc::name(&store).replace('\\', "-")));
        let mutex = CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
        if mutex.is_null() {
            return;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = ipc::request(&store, &serde_json::json!({"command":"show"}));
            CloseHandle(mutex);
            return;
        }
        if let Err(e) = store.initialize() {
            startup_error(&e);
            CloseHandle(mutex);
            return;
        }
        let jobs = match crate::jobs::Jobs::new(store.clone()) {
            Ok(jobs) => jobs,
            Err(error) => {
                startup_error(&error);
                CloseHandle(mutex);
                return;
            }
        };
        let engine = Engine::new(store);
        let ptr = Box::into_raw(Box::new(Ui::new(engine.clone(), jobs.clone())));
        let class = wide("SomaWindow");
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hIcon: LoadIconW(instance, 101usize as _),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let h = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Soma — aplicaciones locales").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            (*ptr).px(1080),
            (*ptr).px(740),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            ptr as _,
        );
        if h.is_null() {
            startup_error(&std::io::Error::last_os_error().to_string());
            drop(Box::from_raw(ptr));
            CloseHandle(mutex);
            return;
        }
        window_icons(h);
        let dark: i32 = 1;
        DwmSetWindowAttribute(h, 20, &dark as *const _ as _, 4);
        ipc::WINDOW.store(h as isize, Ordering::SeqCst);
        ipc::serve(engine, jobs);
        ShowWindow(h, SW_SHOW);
        UpdateWindow(h);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(h, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        drop(Box::from_raw(ptr));
        if !mutex.is_null() {
            CloseHandle(mutex);
        }
        CoUninitialize();
    }
}

unsafe fn startup_error(error: &str) {
    // CLI callers receive a JSON connection error; never leave them waiting on a modal dialog.
    if !std::env::args().any(|arg| arg == "--cli-owner") {
        MessageBoxW(std::ptr::null_mut(), wide(error).as_ptr(), wide("Soma").as_ptr(), MB_ICONERROR);
    }
}
