//! Test environment inspection and transactional profile creation; never starts an owner.
use crate::model::{self, Project, Result, Store};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn executable(name: &str, environment: &BTreeMap<String, String>) -> bool {
    let path = Path::new(name);
    if path.is_absolute() {
        return path.is_file();
    }
    environment.get("PATH").is_some_and(|value| {
        std::env::split_paths(value).any(|dir| {
            [
                name.to_owned(),
                format!("{name}.exe"),
                format!("{name}.cmd"),
            ]
            .iter()
            .any(|name| dir.join(name).is_file())
        })
    })
}

fn docker_available(environment: &BTreeMap<String, String>) -> bool {
    use std::os::windows::process::CommandExt;
    let Ok(mut child) = Command::new("docker.exe")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

pub(crate) fn doctor(store: &Store, directory: &Path, config: &str, docker: bool) -> Value {
    let result = (|| -> Result<Value> {
        let (settings, project, test) = store.test_target(directory, config)?;
        let context = test.context();
        // dotenv parser errors can contain the original line, including secrets.
        let environment = model::environment(&project, &context).map_err(|_| {
            "El entorno de tests es inválido; revisá el JDK y el archivo de variables configurados"
        })?;
        let mut checks = Vec::new();
        let mut check = |name: &str, ready: bool, message: &str| {
            checks.push(json!({"check":name,"ok":ready,"message":message}));
        };
        check(
            "pom",
            project.directory.join("pom.xml").is_file(),
            "pom.xml del checkout seleccionado",
        );
        let wrapper = project.directory.join("mvnw.cmd").is_file();
        check(
            "maven",
            if wrapper {
                project
                    .directory
                    .join(".mvn/wrapper/maven-wrapper.properties")
                    .is_file()
            } else {
                executable("mvn.cmd", &environment)
            },
            "Wrapper con sus propiedades, o Maven disponible",
        );
        let jdk = environment.get("JAVA_HOME").map(PathBuf::from);
        check(
            "jdk",
            jdk.as_ref()
                .map(|jdk| {
                    jdk.join("bin/java.exe").is_file() && jdk.join("bin/javac.exe").is_file()
                })
                .unwrap_or_else(|| {
                    executable("java.exe", &environment) && executable("javac.exe", &environment)
                }),
            "Java y javac disponibles; no ejecuta una compilación",
        );
        if let Some(adapter) = &settings.job_adapter {
            let adapter_exists = executable(&adapter.executable, &environment)
                && adapter
                    .args
                    .iter()
                    .enumerate()
                    .filter(|(_, value)| value.eq_ignore_ascii_case("-File"))
                    .all(|(index, _)| {
                        adapter
                            .args
                            .get(index + 1)
                            .is_some_and(|file| model::resolve(&project, file).is_file())
                    });
            check(
                "job_adapter",
                adapter_exists,
                "Adaptador configurado presente; no lo ejecuta",
            );
        } else {
            check(
                "job_adapter",
                true,
                "Sin adaptador configurado: Maven directo, sujeto a la política local",
            );
        }
        if docker {
            check(
                "docker",
                docker_available(&environment),
                "Consulta opcional del daemon; no valida compatibilidad de Testcontainers",
            );
        }
        let ok = checks.iter().all(|check| check["ok"] == true);
        Ok(
            json!({"ok":ok,"code":if ok {Value::Null}else{json!("preflight_failed")},
            "project":project.id,"config":config,"directory":project.directory,
            "env_file":test.env_file,"override_names":test.env.keys().collect::<Vec<_>>(),
            "checks":checks,"next_action":if ok {"submit_job"}else{"fix_test_environment"},
            "limits":["No ejecuta Maven ni tests", "No hereda el entorno del runtime", "No prueba bases, credenciales ni compatibilidad de proveedores"]}),
        )
    })();
    result.unwrap_or_else(|error| json!({"ok":false,"code":"preflight_failed","error":error,"next_action":"fix_test_environment"}))
}

pub(crate) fn clone_config(
    store: &Store,
    directory: &Path,
    from: &str,
    to: &str,
    env_file: Option<&str>,
    destination: Option<&Path>,
) -> Result<Value> {
    model::validate_id(to)?;
    let (source_settings, project, mut test) = store.test_target(directory, from)?;
    let registered = source_settings
        .projects
        .iter()
        .find(|p| p.id == project.id)
        .ok_or("Proyecto no encontrado")?;
    // Shared settings retain the registered project's bindings, not the paths
    // materialized for this particular checkout during validation.
    let mut shared = registered
        .test_configs
        .iter()
        .find(|c| c.id == from)
        .ok_or("Configuración de pruebas no encontrada")?
        .clone();
    test.id = to.into();
    test.name = format!("{} ({to})", test.name);
    shared.id = test.id.clone();
    shared.name = test.name.clone();
    if let Some(file) = env_file {
        let selected = model::resolve(&project, file);
        let binding = selected
            .strip_prefix(&project.directory)
            .map(|relative| registered.directory.join(relative))
            .unwrap_or_else(|_| selected.clone());
        shared.env_file = Some(binding.to_string_lossy().into());
        test.env_file = Some(selected.to_string_lossy().into());
    }
    model::environment(&project, &test.context())
        .map_err(|_| "El perfil de tests es inválido; revisá el JDK y el archivo de variables")?;
    let target = if let Some(path) = destination {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path)
        };
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        Store {
            home: model::plain(fs::canonicalize(path).map_err(|e| e.to_string())?),
        }
    } else {
        store.clone()
    };
    target.initialize()?;
    let stored = if target.home == store.home {
        shared
    } else {
        test
    };
    target.edit(|settings| {
        if destination.is_some()
            && settings.projects.is_empty()
            && settings.job_adapter.is_none()
            && settings.maven_adapter.is_none()
        {
            settings.job_adapter = source_settings.job_adapter.clone();
            settings.maven_adapter = source_settings.maven_adapter.clone();
            settings.job_concurrency = source_settings.job_concurrency;
        } else if target.home != store.home
            && serde_json::to_value(&settings.job_adapter).ok()
                != serde_json::to_value(&source_settings.job_adapter).ok()
        {
            return Err(
                "El destino tiene otro job_adapter; no se reemplazó su configuración".into(),
            );
        }
        let index = if let Some(index) = settings.projects.iter().position(|p| p.id == project.id) {
            if target.home != store.home && settings.projects[index].directory != project.directory
            {
                return Err("El proyecto de destino corresponde a otro checkout".into());
            }
            index
        } else {
            settings.projects.push(Project {
                id: project.id.clone(),
                name: project.name.clone(),
                directory: project.directory.clone(),
                selected_worktree: None,
                configs: vec![],
                test_configs: vec![],
            });
            settings.projects.len() - 1
        };
        let configs = &mut settings.projects[index].test_configs;
        if configs.iter().any(|c| c.id == to) {
            return Err("El perfil de destino ya existe; no se sobrescribió".into());
        }
        configs.push(stored.clone());
        Ok(())
    })?;
    Ok(
        json!({"ok":true,"project":project.id,"directory":project.directory,"config":to,"from":from,
        "config_file":target.path(),"isolated_store":target.home != store.home,
        "env_file":stored.env_file,"override_names":stored.env.keys().collect::<Vec<_>>()}),
    )
}
