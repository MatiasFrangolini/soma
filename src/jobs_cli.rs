use crate::{ipc, jobs::Jobs, model::Store};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|a| a[0] == name && !a[1].starts_with("--"))
        .map(|a| a[1].clone())
}

// Reject malformed Soma options before contacting (or starting) the owner.
// Maven's own arguments, including --help, start only after the separator.
pub fn validate(args: &[String]) -> Result<(), String> {
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let options = &args[..split];
    let operation = options.first().map(String::as_str).unwrap_or("list");
    let (values, switches): (&[&str], &[&str]) = match operation {
        "submit" => (&["--path", "--config", "--kind", "--timeout", "--data-dir"], &["--wait", "--summary"]),
        "status" | "wait" => (&["--job", "--timeout", "--data-dir"], &["--summary"]),
        "logs" => (&["--job", "--tail", "--after", "--contains", "--data-dir"], &[]),
        "release" => (&["--job", "--participant", "--data-dir"], &[]),
        "configs" => (&["--path", "--data-dir"], &[]),
        "list" => (&["--path", "--state", "--limit", "--data-dir"], &["--summary", "--history"]),
        "doctor" => (&["--path", "--config", "--require", "--data-dir"], &[]),
        "config-clone" => (&["--path", "--from", "--to", "--env-file", "--to-data-dir", "--data-dir"], &[]),
        _ => return Err("Comando jobs desconocido; usá help".into()),
    };
    let mut i = usize::from(!options.is_empty());
    while i < options.len() {
        let name = options[i].as_str();
        if switches.contains(&name) { i += 1; continue; }
        if !values.contains(&name) { return Err(format!("Opción desconocida: {name}")); }
        if i + 1 == options.len() || options[i + 1].starts_with('-') {
            return Err(format!("Falta el valor de {name}"));
        }
        if matches!(name, "--timeout" | "--tail" | "--limit") && options[i + 1].parse::<u64>().is_err() {
            return Err(format!("{name} requiere un entero no negativo"));
        }
        i += 2;
    }
    if operation == "logs" && flag(options, "--after").is_some() && flag(options, "--tail").is_some() {
        return Err("--after consume datos nuevos; no se combina con --tail".into());
    }
    if flag(options, "--state").is_some_and(|value| !value.split(',').all(|state| matches!(state,
        "preparing" | "queued" | "waiting_checkout" | "waiting_resources" | "running" | "cancelling" |
        "completed" | "failed" | "cancelled" | "interrupted" | "invalidated"))) {
        return Err("--state requiere estados de trabajo separados por coma".into());
    }
    if flag(options, "--require").is_some_and(|value| value != "docker") {
        return Err("--require admite docker".into());
    }
    let required: &[&str] = match operation {
        "submit" | "configs" | "doctor" => &["--path"],
        "config-clone" => &["--path", "--from", "--to"],
        "status" | "wait" | "logs" => &["--job"],
        "release" => &["--job", "--participant"],
        _ => &[],
    };
    for name in required {
        if flag(options, name).is_none() { return Err(format!("Falta {name}")); }
    }
    if operation == "submit" && (split == args.len() || split + 1 == args.len()) {
        return Err("Indicá los argumentos Maven después de --".into());
    }
    if operation != "submit" && split != args.len() {
        return Err("El separador -- solo se admite en jobs submit".into());
    }
    Ok(())
}

#[cfg(test)]
mod argument_tests {
    use super::validate;

    #[test]
    fn maven_options_after_separator_are_not_soma_options() {
        let args = ["submit", "--path", "checkout", "--", "--help"]
            .into_iter().map(str::to_owned).collect::<Vec<_>>();
        assert!(validate(&args).is_ok());
    }
}
fn call(store: &Store, request: &Value, open: bool) -> Result<Value, ipc::RequestError> {
    if open {
        ipc::request_or_launch(store, request)
    } else {
        ipc::request(store, request)
    }
}
fn status(store: &Store, id: &str) -> Value {
    call(store, &json!({"command":"job_status","job":id}), false)
        .unwrap_or_else(|e| if e.is_absent() { Jobs::offline(store, id, None) } else { e.response() })
}
fn outcome(response: Value) -> Value {
    if response["ok"] != true {
        return response;
    }
    let job = &response["job"];
    let code = crate::job_output::result_code(job);
    match code {
        Some(code) => json!({"ok":false,"code":code,"job_id":response["job_id"],"job":job}),
        None => response,
    }
}
fn wait(store: &Store, id: &str, timeout: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        let response = status(store, id);
        if response["ok"] != true {
            return response;
        }
        if response["owner_unavailable"] == true {
            return json!({"ok":false,"code":"interrupted","error":"El dueño no está disponible; el resultado final no fue registrado","job":response["job"]});
        }
        if matches!(
            response["job"]["state"].as_str(),
            Some("completed" | "failed" | "cancelled" | "invalidated" | "interrupted")
        ) {
            return outcome(response);
        }
        if Instant::now() >= deadline {
            return json!({"ok":false,"code":"timeout","job_id":response["job_id"],"error":"La espera terminó; la participación y el trabajo siguen activos","job":response["job"]});
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
pub fn run(store: &Store, args: &[String]) -> Value {
    let response = execute(store, args);
    let options = &args[..args.iter().position(|arg| arg == "--").unwrap_or(args.len())];
    if options.iter().any(|arg| arg == "--summary") {
        crate::job_output::summary(response)
    } else { response }
}
fn execute(store: &Store, args: &[String]) -> Value {
    if let Err(error) = validate(args) { return crate::error(error); }
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let options = &args[..split];
    let timeout = flag(options, "--timeout")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(600)
        .clamp(1, 3600);
    let operation = options.first().map(String::as_str).unwrap_or("list");
    let id = flag(options, "--job").unwrap_or_default();
    match operation {
        "submit" => {
            let Some(directory) = flag(options, "--path") else { return crate::error("Falta --path del checkout exacto"); };
            let maven_args = args.get(split+1..).unwrap_or_default();
            let request = json!({"command":"job_submit","request":{
                "directory":directory,"config":flag(options,"--config").unwrap_or_else(||"tests".into()),
                "kind":flag(options,"--kind").unwrap_or_else(||"test".into()),"args":maven_args}});
            let submitted = match call(store, &request, true) { Ok(value)=>value, Err(e)=>return e.response() };
            if submitted["ok"] != true { return submitted; }
            let Some(requested) = submitted["job_id"].as_str() else { return crate::error("Falta identidad de trabajo"); };
            let deadline = Instant::now() + Duration::from_secs(timeout);
            let prepared = loop {
                let value = status(store, requested);
                if value["ok"] != true || value["job"]["state"] != "preparing" || Instant::now() >= deadline { break value; }
                std::thread::sleep(Duration::from_millis(100));
            };
            if prepared["ok"] != true {
                let mut result = prepared;
                result["accepted_job_id"] = submitted["job_id"].clone();
                result["participant_id"] = submitted["participant_id"].clone();
                return result;
            }
            if options.iter().any(|a|a == "--wait") {
                let canonical = prepared["job_id"].as_str().unwrap_or(requested);
                let mut result = wait(store, canonical, timeout);
                result["job_id"] = canonical.into();
                result["participant_id"] = submitted["participant_id"].clone();
                return result;
            }
            if options.iter().any(|a|a == "--summary") {
                let mut result = prepared.clone();
                result["participant_id"] = submitted["participant_id"].clone();
                result["deduplicated"] = json!(prepared["job_id"].as_str().is_some_and(|id| id != requested));
                return result;
            }
            json!({"ok":true,"job_id":prepared["job_id"].as_str().unwrap_or(requested),
                "participant_id":submitted["participant_id"],"deduplicated":prepared["job_id"].as_str().is_some_and(|id|id!=requested),
                "state":prepared["job"]["state"]})
        }
        "status" if !id.is_empty() => status(store, &id),
        "wait" if !id.is_empty() => wait(store, &id, timeout),
        "logs" if !id.is_empty() => {
            let request = json!({"command":"job_logs","job":id,
                "tail":flag(options,"--tail").and_then(|s|s.parse::<u64>().ok()),
                "after":flag(options,"--after"),"contains":flag(options,"--contains")});
            call(store, &request, false)
                .unwrap_or_else(|e| if e.is_absent() { Jobs::offline_logs(store,&id,&request) } else { e.response() })
        }
        "release" if !id.is_empty() => {
            let Some(participant) = flag(options,"--participant") else { return crate::error("Falta --participant; cancelar requiere tu propia participación"); };
            call(store,&json!({"command":"job_release","job":id,"participant":participant}),false).unwrap_or_else(|e|e.response())
        }
        "configs" => {
            let Some(path) = flag(options,"--path") else { return crate::error("Falta --path"); };
            let result = store.project_at(std::path::Path::new(&path)).and_then(|id|store.load().map(|s|(id,s)));
            match result {
                Ok((id, settings)) => {
                    let project = settings.projects.iter().find(|p|p.id == id).unwrap();
                    json!({"ok":true,"project":id,"configs":project.test_configs.iter().map(|c|
                        json!({"id":c.id,"name":c.name,"jdk":c.jdk,"env_file":c.env_file,"override_names":c.env.keys().collect::<Vec<_>>()})
                        ).collect::<Vec<_>>()})
                }
                Err(error) => crate::error(error),
            }
        }
        "doctor" => crate::test_profiles::doctor(store, std::path::Path::new(&flag(options,"--path").unwrap()),
            &flag(options,"--config").unwrap_or_else(||"tests".into()), flag(options,"--require").is_some()),
        "config-clone" => crate::test_profiles::clone_config(store,
            std::path::Path::new(&flag(options,"--path").unwrap()), &flag(options,"--from").unwrap(),
            &flag(options,"--to").unwrap(),flag(options,"--env-file").as_deref(),
            flag(options,"--to-data-dir").as_deref().map(std::path::Path::new)).unwrap_or_else(crate::error),
        "list" => list(store, options),
        _ => crate::error("Usá jobs submit/configs/list/status/wait/logs/release; status/wait/logs/release requieren --job ID"),
    }
}

fn list(store: &Store, options: &[String]) -> Value {
    let summary = options.iter().any(|arg| arg == "--summary");
    let history = options.iter().any(|arg| arg == "--history");
    let mut response = call(store,&json!({"command":"job_list","summary":summary}),false)
        .unwrap_or_else(|e| if e.is_absent() { Jobs::offline_list(store) } else { e.response() });
    if response["ok"] != true { return response; }
    let mut jobs = response["jobs"].as_array().cloned().unwrap_or_default();
    if history {
        let current = jobs.iter().filter_map(|job| job["job_id"].as_str()).map(str::to_owned).collect::<std::collections::BTreeSet<_>>();
        let records = match crate::job_history::records(store) {
            Ok(records) => records,
            Err(error) => return json!({"ok":false,"code":"history_unavailable","error":error,"next_action":"repair_history_storage"}),
        };
        for record in records {
            if record.canonical_job_id.is_none() && !current.contains(&record.job_id) {
                let mut value = json!(record);
                value["archived"] = json!(true);
                value["historical"] = json!(true);
                value["details_retained"] = json!(false);
                value["logs_retained"] = json!(false);
                jobs.push(value);
            }
        }
        response["archive_limit"] = json!(crate::job_history::LIMIT);
        response["archive_max_bytes"] = json!(crate::job_history::BYTES);
        response["archive_max_age_days"] = json!(30);
    }
    if let Some(directory) = flag(options, "--path") {
        let path = std::path::Path::new(&directory);
        let absolute = std::fs::canonicalize(path).or_else(|_| std::path::absolute(path));
        let path = match absolute { Ok(path) => crate::model::plain(path).to_string_lossy().to_lowercase(),
            Err(_) => return crate::error("No se pudo resolver --path") };
        jobs.retain(|job| job["directory"].as_str().is_some_and(|directory| directory.to_lowercase() == path));
    }
    if let Some(states) = flag(options, "--state") {
        jobs.retain(|job| job["state"].as_str().is_some_and(|state| states.split(',').any(|wanted| wanted == state)));
    }
    jobs.sort_by(|a,b| b["created_at_ms"].as_u64().cmp(&a["created_at_ms"].as_u64())
        .then_with(|| a["job_id"].as_str().cmp(&b["job_id"].as_str())));
    let matched = jobs.len();
    let maximum = (crate::job_history::LIMIT + crate::jobs::HISTORY + crate::jobs::PENDING) as u64;
    let limit = flag(options,"--limit").and_then(|n| n.parse::<u64>().ok()).unwrap_or(maximum).min(maximum) as usize;
    jobs.truncate(limit);
    if summary {
        jobs = jobs.into_iter().map(|job| {
            if job["verdict"].is_string() { return job; }
            let mut compact = crate::job_output::compact(&job);
            if response["owner_unavailable"] == true && compact["verdict"] == "pending" {
                compact["verdict"] = json!("unknown");
                compact["next_action"] = json!("inspect_then_resubmit");
            }
            for key in ["participants", "archived", "historical", "details_retained", "logs_retained"] {
                if let Some(value) = job.get(key) { compact[key] = value.clone(); }
            }
            compact
        }).collect();
    }
    response["jobs"] = json!(jobs);
    response["matched"] = json!(matched);
    response["limit"] = json!(limit);
    response["has_more"] = json!(matched > limit);
    response
}
