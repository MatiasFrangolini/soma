use crate::{ipc, model::Store};
use serde_json::{json, Value};
use std::time::Duration;

fn flag(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find(|a| a[0] == name).map(|a| a[1].clone())
}
pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    // Help belongs to Soma only before the Maven argument separator. In
    // particular, it must never submit a job or initialise the data store.
    let options = &args[..args.iter().position(|arg| arg == "--").unwrap_or(args.len())];
    if matches!(cmd, "help" | "--help" | "-h")
        || options.iter().any(|arg| matches!(arg.as_str(), "--help" | "-h")) {
        println!("Soma {} — agente y ventana comparten una ejecución\n\n  projects [--json]\n  add --path DIRECTORIO\n  configs --project ID\n  presets\n  apply-preset --project ID [--preset jvm-optimized] [--from-config ID]\n  validate\n  config-path\n  status [--operation ID] [--health]\n  worktrees --path DIRECTORIO\n  branches --path DIRECTORIO\n  switch --path DIRECTORIO --branch RAMA\n  run --path DIRECTORIO [--config ID] [--reuse] [--wait-ready] [--health-url URL]\n  build --project ID [--config ID] [--wait]\n  wait --operation ID [--ready] [--timeout 600]\n  restart [--project ID] [--config ID]\n  stop [--operation ID]\n  logs [--operation ID] [--tail 200 | --after CURSOR] [--contains TEXTO]\n  jobs submit --path DIRECTORIO --config tests --kind test|build [--wait] -- ARGUMENTOS_MAVEN\n  jobs configs --path DIRECTORIO\n  jobs list\n  jobs status|wait|logs --job ID\n  jobs release --job ID --participant ID\n  show\n  close\n\nTodas las respuestas de operación son JSON. --data-dir PATH / SOMA_HOME cambia el almacenamiento.\nRun admite --project ID o --path y compila antes de iniciar. --reuse exige un build vigente.\n--wait espera la salida; --wait-ready espera la API. Stop con --operation rechaza un ID ajeno. El cierre de la ventana detiene sus procesos.", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    if cmd == "jobs" {
        if let Err(error) = crate::jobs_cli::validate(&args[1..]) {
            return print(crate::error(error));
        }
    }
    let store = Store::from_args();
    if cmd == "status" && args.iter().any(|argument| argument == "--operation")
        && flag(&args, "--operation").is_none_or(|value| value.starts_with("--")) {
        return print(crate::error("Falta el valor de --operation"));
    }
    if cmd == "logs" {
        for name in ["--operation", "--after", "--contains", "--tail"] {
            if args.iter().any(|argument| argument == name)
                && flag(&args, name).is_none_or(|value| value.starts_with("--")) {
                return print(crate::error(format!("Falta el valor de {name}")));
            }
        }
        if flag(&args, "--tail").is_some_and(|value| value.parse::<u64>().is_err()) {
            return print(crate::error("--tail requiere un entero no negativo"));
        }
        if flag(&args, "--after").is_some() && flag(&args, "--tail").is_some() {
            return print(crate::error("--after consume datos nuevos; no se combina con --tail"));
        }
    }
    if let Err(e) = store.initialize() {
        return print(crate::error(e));
    }
    if cmd == "jobs" {
        return print(crate::jobs_cli::run(&store, &args[1..]));
    }
    let timeout = flag(&args, "--timeout")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(600)
        .clamp(1, 3600);
    let path = flag(&args, "--path").map(std::path::PathBuf::from);
    let project = if let Some(id) = flag(&args, "--project") {
        Some(id)
    } else if let Some(path) = &path {
        if matches!(cmd, "run" | "build" | "restart" | "switch" | "configs") {
            match store.project_at(path) {
                Ok(id) => Some(id),
                Err(e) => return print(crate::error(e)),
            }
        } else {
            None
        }
    } else {
        None
    };
    if args.iter().any(|s| s == "--wait-ready") && args.iter().any(|s| s == "--wait") {
        return print(crate::error(
            "Elegí --wait-ready o --wait; son esperas diferentes",
        ));
    }
    if cmd == "wait" {
        return print(match flag(&args, "--operation") {
            Some(id) => wait_for(&store, &id, timeout, args.iter().any(|s| s == "--ready")),
            None => crate::error("Falta --operation ID"),
        });
    }
    if matches!(cmd, "worktrees" | "branches") {
        let directory = match path {
            Some(path) => path,
            None => match store.target(project.as_deref().unwrap_or(""), None, None) {
                Ok((p, _)) => p.directory,
                Err(e) => return print(crate::error(e)),
            },
        };
        return print(
            json!({"ok":true,"directory":directory,"worktrees":crate::git::worktrees(&directory),
            "branches":crate::git::branches(&directory)}),
        );
    }
    let result=match cmd {
        "presets"=>crate::model::JavaPreset::optimized().map(|preset|json!({"ok":true,"presets":[preset]})),
        "apply-preset"=>{
            let preset=flag(&args,"--preset").unwrap_or_else(||"jvm-optimized".into());
            store.apply_preset(project.as_deref().unwrap_or_default(),&preset,flag(&args,"--from-config").as_deref())
                .map(|added|json!({"ok":true,"project":project,"config":preset,"added":added,"config_file":store.path()}))
        },
        "projects"=>store.load().map(|s|json!({"ok":true,"projects":s.projects.iter().map(|p|json!({"id":p.id,"name":p.name,"directory":p.directory,"configs":p.configs.iter().map(|c|json!({"id":c.id,"name":c.name})).collect::<Vec<_>>()})).collect::<Vec<_>>()})),
        "configs"=>store.find(project.as_deref().unwrap_or_default(),flag(&args,"--config").as_deref()).and_then(|(p,_)|store.load().map(|s|json!({"ok":true,"project":p.id,"configs":s.projects.iter().find(|x|x.id==p.id).unwrap().configs.iter().map(|c|json!({"id":c.id,"name":c.name,"env_file":c.env_file,"override_names":c.env.keys().collect::<Vec<_>>(),"profiles":c.profiles,"jdk":c.jdk,"config_files":c.config_files})).collect::<Vec<_>>()}))),
        "add"=>store.add_directory(std::path::Path::new(&flag(&args,"--path").unwrap_or_default())).map(|id|json!({"ok":true,"project":id,"config_file":store.path()})),
        "config-path"=>Ok(json!({"ok":true,"path":store.path()})),
        "validate"=>store.load().and_then(|s|{for p in &s.projects{if !p.directory.is_dir(){return Err(format!("{}: directorio no encontrado",p.id));}for c in &p.configs {crate::model::environment(p,c)?;}for c in &p.test_configs {crate::model::environment(p,&c.context())?;}}Ok(json!({"ok":true,"projects":s.projects.len()}))}),
        "status"|"stop"|"logs"|"run"|"build"|"restart"|"show"|"close"|"switch"=>{
            let mode=if cmd=="run"&&args.iter().any(|s|s=="--reuse"){"reuse"}else{cmd};
            let command=if matches!(cmd,"run"|"build"){"start_target"}else{cmd};
            let readiness=flag(&args,"--health-url").map(|url|crate::readiness::Readiness{url,timeout_seconds:timeout,json_field:Some("status".into()),expected:"UP".into()});
            let request=json!({"command":command,"mode":mode,"project":project,"config":flag(&args,"--config"),"directory":path,
                "operation":flag(&args,"--operation"),"branch":flag(&args,"--branch"),"readiness":readiness,
                "after":flag(&args,"--after"),"contains":flag(&args,"--contains"),"check_health":args.iter().any(|argument| argument == "--health"),
                "tail":flag(&args,"--tail").and_then(|s|s.parse::<u64>().ok())});
            match ipc::request(&store,&request) {
                Ok(v) if v["code"]=="owner_closing" && matches!(cmd,"run"|"build"|"restart"|"show"|"switch")=>Ok(ipc::request_or_launch(&store,&request).unwrap_or_else(|e|e.response())),
                Ok(v)=>Ok(v),
                Err(e) if !e.is_absent()=>Ok(e.response()),
                Err(_) if matches!(cmd,"run"|"build"|"restart"|"show"|"switch")=>{
                    Ok(ipc::request_or_launch(&store,&request).unwrap_or_else(|e|e.response()))
                },Err(error) if cmd=="status"=>{
                    let mut value=crate::observation::offline_status(&store.home.join("logs"),"absent",&error.to_string());
                    value["config_file"]=json!(store.path());
                    if request["check_health"]==true || request["operation"].is_string() {
                        value["ok"]=json!(false);value["code"]=json!("owner_unavailable");
                        value["error"]=json!("No se puede verificar la ejecución sin comunicación con su instancia Soma");
                    }
                    Ok(value)
                },
                Err(_) if matches!(cmd,"stop"|"close")=>Ok(json!({"ok":true,"state":"closed"})),
                Err(_) if cmd=="logs"=>Ok(crate::observation::logs(&store.home.join("logs"),&request,None)),
                Err(e)=>Err(format!("La ventana está cerrada: {e}")),
            }
        },
        _=>Err("Comando desconocido; usá help".into()),
    };
    let result = result.unwrap_or_else(crate::error);
    if result["ok"] == true
        && args
            .iter()
            .any(|s| matches!(s.as_str(), "--wait" | "--wait-ready"))
    {
        if let Some(id) = result["operation_id"].as_str() {
            return print(wait_for(
                &store,
                id,
                timeout,
                args.iter().any(|s| s == "--wait-ready"),
            ));
        }
    }
    print(result)
}
fn wait_for(store: &Store, id: &str, timeout: u64, ready: bool) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let mut historical = false;
        let response = match ipc::request(store, &json!({"command":"status"})) {
            Ok(v) => {
                if v["status"]["operation_id"].as_str() != Some(id) {
                    if let Some(status) = crate::engine::Status::completed(store, id) {
                        historical = true;
                        json!({"ok":true,"status":status})
                    } else { v }
                } else { v }
            },
            Err(e) if !e.is_absent() => return e.response(),
            Err(_) => {
                match crate::engine::Status::completed(store, id) {
                    Some(status) => { historical = true; json!({"ok":true,"status":status}) },
                    None => return json!({"ok":false,"code":"stopped","error":"Soma se cerró sin un resultado final disponible","operation_id":id}),
                }
            }
        };
        let state = &response["status"];
        let source = if historical { "disk" } else { "live" };
        if state["operation_id"].as_str() != Some(id) {
            return json!({"ok":false,"code":"operation_replaced","error":"Otra operación reemplazó el estado consultado","status":state});
        }
        match state["state"].as_str().unwrap_or("") {
            "exited" => {
                return if ready {
                    json!({"ok":false,"code":"process_failed","error":"El proceso terminó antes de confirmar readiness","status":state,"source":source})
                } else {
                    json!({"ok":true,"status":state,"source":source})
                }
            }
            "failed" => {
                let error = state["error"].as_str().unwrap_or("");
                let code = if error.starts_with("build_stale:") {
                    "build_stale"
                } else if error.starts_with("checkout_busy:") {
                    "busy"
                } else {
                    "process_failed"
                };
                return json!({"ok":false,"code":code,"status":state,"source":source});
            }
            "stopped" => return json!({"ok":false,"code":"stopped","status":state,"source":source}),
            _ => {}
        }
        if ready && state["state"] == "running" {
            match state["readiness"].as_str() {
                Some("ready") => return json!({"ok":true,"status":state}),
                Some("not_configured") => {
                    return json!({"ok":false,"code":"readiness_not_configured","error":"Configurá readiness o usá --health-url","status":state})
                }
                Some("timed_out") => {
                    return json!({"ok":false,"code":"readiness_timeout","error":"La API no quedó lista; el proceso sigue activo","status":state})
                }
                _ => {}
            }
        }
        if std::time::Instant::now() >= deadline {
            return json!({"ok":false,"code":"timeout","error":"La espera terminó; la operación sigue activa","status":state});
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
fn print(result: Value) -> i32 {
    println!("{result}");
    if result["ok"].as_bool() == Some(true) {
        0
    } else {
        match result["code"].as_str() {
            Some("busy") => 3,
            Some("build_stale") => 4,
            Some("process_failed") => 5,
            Some("timeout") => 6,
            Some("stopped") => 130,
            Some("operation_replaced") => 7,
            Some("readiness_not_configured") => 8,
            Some("readiness_timeout") => 9,
            Some("invalidated") => 10,
            Some("validation_incomplete") => 11,
            Some("interrupted") => 12,
            _ => 2,
        }
    }
}
