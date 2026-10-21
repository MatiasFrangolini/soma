use crate::{ipc, jobs::Jobs, model::Store};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|a| a[0] == name && !a[1].starts_with("--"))
        .map(|a| a[1].clone())
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
    let code = match job["state"].as_str().unwrap_or("") {
        "cancelled" => Some("stopped"),
        "interrupted" => Some("interrupted"),
        "invalidated" => Some("invalidated"),
        "failed" => Some("process_failed"),
        "completed" if job["evidence"]["status"] == "failed" => Some("process_failed"),
        "completed"
            if !matches!(
                job["evidence"]["status"].as_str(),
                Some("passed" | "passed_with_skips" | "not_requested")
            ) =>
        {
            Some("validation_incomplete")
        }
        _ => None,
    };
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
            json!({"ok":true,"job_id":prepared["job_id"].as_str().unwrap_or(requested),
                "participant_id":submitted["participant_id"],"deduplicated":prepared["job_id"].as_str().is_some_and(|id|id!=requested),
                "state":prepared["job"]["state"]})
        }
        "status" if !id.is_empty() => status(store, &id),
        "wait" if !id.is_empty() => wait(store, &id, timeout),
        "logs" if !id.is_empty() => {
            let tail = flag(options,"--tail").and_then(|s|s.parse::<usize>().ok()).unwrap_or(200).min(2000);
            call(store, &json!({"command":"job_logs","job":id,"tail":tail}), false)
                .unwrap_or_else(|e| if e.is_absent() { Jobs::offline(store,&id,Some(tail)) } else { e.response() })
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
        "list" => call(store,&json!({"command":"job_list"}),false).unwrap_or_else(|e| if e.is_absent() { Jobs::offline_list(store) } else { e.response() }),
        _ => crate::error("Usá jobs submit/configs/list/status/wait/logs/release; status/wait/logs/release requieren --job ID"),
    }
}
