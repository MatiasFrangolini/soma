use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "version")]
    pub schema_version: u32,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub maven_adapter: Option<String>,
}
fn version() -> u32 {
    1
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
    pub configs: Vec<Config>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub env_file: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub jdk: Option<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub config_files: Vec<String>,
    #[serde(default)]
    pub build: Option<CommandSpec>,
    pub run: RunSpec,
    #[serde(default = "default_inputs")]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub input_excludes: Vec<String>,
}
fn default_inputs() -> Vec<String> {
    vec![".".into()]
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub directory: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunSpec {
    JavaJar {
        artifact: String,
        #[serde(default)]
        jvm_args: Vec<String>,
        #[serde(default)]
        args: Vec<String>,
    },
    Command {
        executable: String,
        #[serde(default)]
        args: Vec<String>,
    },
}
#[derive(Clone)]
pub struct Store {
    pub home: PathBuf,
}
impl Store {
    pub fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let supplied = args
            .windows(2)
            .find(|a| a[0] == "--data-dir")
            .map(|a| PathBuf::from(&a[1]));
        let home = supplied
            .or_else(|| std::env::var_os("SOMA_HOME").map(PathBuf::from))
            .or_else(|| std::env::var_os("RUNLOCAL_HOME").map(PathBuf::from))
            .unwrap_or_else(|| {
                let exe = std::env::current_exe().unwrap();
                let directory = exe.parent().unwrap();
                let home = directory.join("Soma-data");
                let legacy = directory.join("RunLocal-data");
                if home.exists() {
                    home
                } else if legacy.join("projects.json").is_file() {
                    legacy
                } else {
                    std::env::var_os("LOCALAPPDATA")
                        .map(PathBuf::from)
                        .map(|p| p.join("Soma"))
                        .unwrap_or(home)
                }
            });
        let home = if home.is_absolute() {
            home
        } else {
            std::env::current_dir().unwrap().join(home)
        };
        Self {
            home: plain(fs::canonicalize(&home).unwrap_or(home)),
        }
    }
    pub fn initialize(&self) -> Result<()> {
        fs::create_dir_all(self.home.join("logs")).map_err(|e| e.to_string())?;
        fs::create_dir_all(self.home.join("builds")).map_err(|e| e.to_string())?;
        if !self.path().exists() {
            self.save(&Settings {
                schema_version: 1,
                ..Default::default()
            })?;
        }
        Ok(())
    }
    pub fn path(&self) -> PathBuf {
        self.home.join("projects.json")
    }
    pub fn load(&self) -> Result<Settings> {
        let raw = fs::read(self.path()).map_err(|e| e.to_string())?;
        let value: Settings =
            serde_json::from_slice(&raw).map_err(|e| format!("projects.json inválido: {e}"))?;
        if value.schema_version != 1 {
            return Err("schema_version debe ser 1".into());
        }
        let mut ids = std::collections::HashSet::new();
        for p in &value.projects {
            validate_id(&p.id)?;
            if !ids.insert(p.id.clone()) {
                return Err("ID de proyecto duplicado".into());
            }
            if !p.directory.is_absolute() {
                return Err(format!("{}: directory debe ser absoluto", p.id));
            }
            let mut configs = std::collections::HashSet::new();
            for c in &p.configs {
                validate_id(&c.id)?;
                if !configs.insert(c.id.clone()) {
                    return Err("ID de configuración duplicado".into());
                }
            }
        }
        Ok(value)
    }
    pub fn save(&self, value: &Settings) -> Result<()> {
        let temp = self.home.join("projects.json.tmp");
        fs::write(&temp, serde_json::to_vec_pretty(value).unwrap()).map_err(|e| e.to_string())?;
        fs::rename(temp, self.path()).map_err(|e| e.to_string())
    }
    pub fn find(&self, project: &str, config: Option<&str>) -> Result<(Project, Config)> {
        let s = self.load()?;
        let p = s
            .projects
            .into_iter()
            .find(|p| p.id == project)
            .ok_or("Proyecto no encontrado")?;
        let c = p
            .configs
            .iter()
            .find(|c| config.map(|v| c.id == v).unwrap_or(true))
            .cloned()
            .ok_or("Configuración no encontrada")?;
        if !p.directory.is_dir() {
            return Err("El directorio del proyecto no existe".into());
        }
        Ok((p, c))
    }
    pub fn add_directory(&self, path: &Path) -> Result<String> {
        let dir = plain(fs::canonicalize(path).map_err(|e| e.to_string())?);
        let mut s = self.load()?;
        if let Some(p) = s.projects.iter().find(|p| p.directory == dir) {
            return Ok(p.id.clone());
        }
        let name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let base: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let mut id = base.clone();
        let mut n = 2;
        while s.projects.iter().any(|p| p.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        let mut configs = Vec::new();
        if dir.join("pom.xml").exists() {
            let module = if dir.join("application/pom.xml").exists() {
                "application"
            } else {
                ""
            };
            let pom = dir.join(module).join("pom.xml");
            let xml = fs::read_to_string(pom).map_err(|e| e.to_string())?;
            let own = xml.split("</parent>").nth(1).unwrap_or(&xml);
            let artifact = xml_value(own, "artifactId").unwrap_or(name.clone());
            let ver = xml_value(own, "version")
                .or_else(|| xml_value(&xml, "version"))
                .unwrap_or("0.0.1-SNAPSHOT".into());
            let jar = if module.is_empty() {
                format!("target/{artifact}-{ver}.jar")
            } else {
                format!("{module}/target/{artifact}-{ver}.jar")
            };
            let build = s
                .maven_adapter
                .as_ref()
                .map(|adapter| CommandSpec {
                    executable: "pwsh".into(),
                    args: vec![
                        "-NoProfile".into(),
                        "-File".into(),
                        adapter.clone(),
                        "-ProjectDir".into(),
                        dir.to_string_lossy().into(),
                        "-Module".into(),
                        module.into(),
                    ],
                    directory: None,
                })
                .or_else(|| {
                    let wrapper = dir.join("mvnw.cmd");
                    let mut args = Vec::new();
                    if !module.is_empty() {
                        args.extend(["-pl".into(), module.into(), "-am".into()]);
                    }
                    args.push("package".into());
                    Some(CommandSpec {
                        executable: if wrapper.is_file() {
                            wrapper.to_string_lossy().into()
                        } else {
                            "mvn.cmd".into()
                        },
                        args,
                        directory: None,
                    })
                });
            let jdk = std::env::var_os("JAVA_HOME")
                .map(PathBuf::from)
                .filter(|p| p.join("bin/java.exe").is_file())
                .or_else(|| {
                    std::env::var_os("USERPROFILE")
                        .map(PathBuf::from)
                        .and_then(|h| fs::read_dir(h.join(".jdks")).ok())
                        .into_iter()
                        .flatten()
                        .flatten()
                        .map(|e| e.path())
                        .find(|p| {
                            p.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .starts_with("ms-21")
                        })
                });
            configs.push(Config {
                id: "local".into(),
                name: "Local".into(),
                env_file: dir.join(".env").exists().then_some(".env".into()),
                env: BTreeMap::new(),
                jdk: jdk.map(|p| p.to_string_lossy().into()),
                profiles: vec![],
                config_files: vec![],
                build,
                run: RunSpec::JavaJar {
                    artifact: jar,
                    jvm_args: vec![],
                    args: vec![],
                },
                inputs: default_inputs(),
                input_excludes: vec![],
            });
        }
        s.projects.push(Project {
            id: id.clone(),
            name,
            directory: dir,
            configs,
        });
        self.save(&s)?;
        Ok(id)
    }
}
pub fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Err("ID inválido: use letras ASCII, números, - o _".into())
    } else {
        Ok(())
    }
}
fn plain(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(v) = s.strip_prefix("\\\\?\\UNC\\") {
        PathBuf::from(format!("\\\\{v}"))
    } else if let Some(v) = s.strip_prefix("\\\\?\\") {
        PathBuf::from(v)
    } else {
        path
    }
}
fn xml_value(xml: &str, key: &str) -> Option<String> {
    xml.split(&format!("<{key}>"))
        .nth(1)?
        .split(&format!("</{key}>"))
        .next()
        .map(str::to_owned)
}
pub fn resolve(p: &Project, path: &str) -> PathBuf {
    let v = PathBuf::from(path);
    if v.is_absolute() {
        v
    } else {
        p.directory.join(v)
    }
}
pub fn environment(p: &Project, c: &Config) -> Result<BTreeMap<String, String>> {
    let mut env: BTreeMap<String, String> = std::env::vars()
        .map(|(k, v)| (k.to_ascii_uppercase(), v))
        .collect();
    if let Some(file) = &c.env_file {
        let path = resolve(p, file);
        let entries = dotenvy::from_path_iter(&path)
            .map_err(|_| format!("No se puede leer .env: {}", path.display()))?;
        for entry in entries {
            let (k, v) = entry.map_err(|_| "Sintaxis inválida en .env (contenido omitido)")?;
            env.insert(k.to_ascii_uppercase(), v);
        }
    }
    env.extend(
        c.env
            .iter()
            .map(|(k, v)| (k.to_ascii_uppercase(), v.clone())),
    );
    if let Some(jdk) = &c.jdk {
        let path = resolve(p, jdk);
        if !path.join("bin/java.exe").is_file() {
            return Err("El JDK configurado no contiene bin/java.exe".into());
        }
        env.insert("JAVA_HOME".into(), path.to_string_lossy().into());
        let existing = env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        env.retain(|k, _| !k.eq_ignore_ascii_case("PATH"));
        env.insert(
            "PATH".into(),
            format!("{};{}", path.join("bin").display(), existing),
        );
    }
    if !c.profiles.is_empty() {
        env.insert("SPRING_PROFILES_ACTIVE".into(), c.profiles.join(","));
    }
    if !c.config_files.is_empty() {
        let files: Vec<String> = c
            .config_files
            .iter()
            .map(|f| {
                format!(
                    "file:{}",
                    resolve(p, f).to_string_lossy().replace('\\', "/")
                )
            })
            .collect();
        env.insert("SPRING_CONFIG_ADDITIONAL_LOCATION".into(), files.join(","));
    }
    env.remove("CODEX_HEAVY_GATE_ACTIVE");
    Ok(env)
}
pub fn branch(dir: &Path) -> String {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "branch", "--show-current"])
        .creation_flags(0x08000000)
        .output();
    if let Ok(o) = out {
        if o.status.success() {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            return if s.is_empty() {
                "HEAD detached".into()
            } else {
                s
            };
        }
    }
    "Sin Git / no disponible".into()
}
fn ignored(path: &Path, c: &Config, root: &Path) -> bool {
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    if c.input_excludes
        .iter()
        .any(|x| rel == *x || rel.starts_with(&format!("{x}/")))
    {
        return true;
    }
    path.file_name()
        .map(|n| {
            matches!(
                n.to_string_lossy().as_ref(),
                ".git"
                    | ".idea"
                    | ".vscode"
                    | "target"
                    | "node_modules"
                    | ".next"
                    | "dist"
                    | "build"
                    | "out"
                    | "output"
                    | "logs"
                    | ".gradle"
                    | ".local"
                    | ".diagnostics"
                    | ".reviews"
                    | ".codex-worktrees"
                    | ".worktrees"
            )
        })
        .unwrap_or(false)
}
fn walk(path: &Path, c: &Config, root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if ignored(path, c, root) {
        return Ok(());
    }
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err(format!("Entrada symlink no soportada: {}", path.display()));
    }
    if meta.is_dir() {
        for item in fs::read_dir(path).map_err(|e| e.to_string())? {
            walk(&item.map_err(|e| e.to_string())?.path(), c, root, files)?;
        }
    } else if meta.is_file() {
        files.push(path.to_path_buf());
    }
    Ok(())
}
pub fn hash_file(path: &Path) -> Result<String> {
    let mut h = Sha256::new();
    feed_file(path, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}
fn feed_file(path: &Path, h: &mut Sha256) -> Result<()> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut b = [0; 65536];
    loop {
        let n = f.read(&mut b).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(())
}
pub fn fingerprint(p: &Project, c: &Config, env: &BTreeMap<String, String>) -> Result<String> {
    let mut files = Vec::new();
    for input in &c.inputs {
        walk(&resolve(p, input), c, &p.directory, &mut files)?;
    }
    files.sort();
    files.dedup();
    let mut h = Sha256::new();
    h.update(serde_json::to_vec(c).unwrap());
    // Build tools may interpolate any environment value. Persist only the digest.
    h.update(serde_json::to_vec(env).unwrap());
    h.update(branch(&p.directory));
    use std::os::windows::process::CommandExt;
    if let Ok(o) = std::process::Command::new("git")
        .args(["-C", &p.directory.to_string_lossy(), "rev-parse", "HEAD"])
        .creation_flags(0x08000000)
        .output()
    {
        if o.status.success() {
            h.update(o.stdout);
        }
    }
    for path in files {
        h.update(
            path.strip_prefix(&p.directory)
                .unwrap_or(&path)
                .to_string_lossy()
                .as_bytes(),
        );
        h.update([0]);
        feed_file(&path, &mut h)?;
        h.update([0]);
    }
    Ok(format!("{:x}", h.finalize()))
}
