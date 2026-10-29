//! Presentation of persisted job results. Lookup success is separate from validation.
use serde_json::{json, Value};

pub(crate) fn result_code(job: &Value) -> Option<&'static str> {
    match job["state"].as_str().unwrap_or("") {
        "cancelled" => Some("stopped"),
        "interrupted" => Some("interrupted"),
        "invalidated" => Some("invalidated"),
        "failed" => Some("process_failed"),
        "completed"
            if job["exit_code"].as_u64().is_some_and(|code| code != 0)
                || job["evidence"]["status"] == "failed" =>
        {
            Some("process_failed")
        }
        "completed"
            if job["exit_code"] != 0
                || !(matches!(
                    job["evidence"]["status"].as_str(),
                    Some("passed" | "passed_with_skips")
                ) || (job["kind"] == "build"
                    && job["evidence"]["status"] == "not_requested")) =>
        {
            Some("validation_incomplete")
        }
        _ => None,
    }
}

pub(crate) fn verdict(job: &Value) -> &'static str {
    match result_code(job) {
        Some("stopped") => "cancelled",
        Some("process_failed") => "failed",
        Some("validation_incomplete") => "incomplete",
        Some("invalidated") => "invalidated",
        Some("interrupted") => "interrupted",
        _ if job["state"] == "completed" && job["kind"] == "build" => "built",
        _ if job["state"] == "completed" && job["evidence"]["status"] == "passed_with_skips" => {
            "passed_with_skips"
        }
        _ if job["state"] == "completed" => "passed",
        _ => "pending",
    }
}

pub(crate) fn compact(job: &Value) -> Value {
    let verdict = verdict(job);
    let next = match verdict {
        "passed" | "passed_with_skips" | "built" => "complete",
        "failed" => "inspect_failure",
        "incomplete" => "verify_test_selection",
        "invalidated" => "resubmit_with_stable_inputs",
        "interrupted" => "inspect_then_resubmit",
        "cancelled" => "none",
        _ => "wait",
    };
    let mut value = json!({"verdict":verdict,"next_action":next});
    for key in [
        "job_id",
        "canonical_job_id",
        "state",
        "kind",
        "project",
        "config",
        "directory",
        "branch",
        "head",
        "created_at_ms",
        "finished_at_ms",
        "exit_code",
        "forced",
        "error",
        "timings",
    ] {
        value[key] = job[key].clone();
    }
    let mut evidence = job["evidence"].clone();
    if let Some(object) = evidence.as_object_mut() {
        object.remove("details");
        object.remove("diagnostics");
    }
    value["evidence"] = evidence;
    let mut timings = json!({});
    for key in [
        "total_ms",
        "preparation_ms",
        "soma_queue_ms",
        "resource_wait_ms",
        "maven_ms",
        "evidence_ms",
    ] {
        timings[key] = job["timings"][key].clone();
    }
    value["timings"] = timings;
    value["first_failure"] = job["evidence"]["details"]
        .as_array()
        .and_then(|details| {
            details
                .iter()
                .find(|detail| matches!(detail["outcome"].as_str(), Some("failure" | "error")))
        })
        .map(|detail| {
            let message = detail["message"]
                .as_str()
                .unwrap_or("")
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("");
            json!({"class":detail["class"],"test":detail["test"],"outcome":detail["outcome"],
            "message":message.chars().take(240).collect::<String>()})
        })
        .unwrap_or(Value::Null);
    let args = job["requested_maven_args"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut modules = Value::Null;
    let mut tests = Vec::new();
    for (index, arg) in args.iter().filter_map(Value::as_str).enumerate() {
        if arg == "-pl" || arg == "--projects" {
            modules = args.get(index + 1).cloned().unwrap_or(Value::Null);
        }
        if let Some(value) = arg
            .strip_prefix("--projects=")
            .or_else(|| arg.strip_prefix("-pl="))
        {
            modules = json!(value);
        }
        if let Some(value) = arg
            .strip_prefix("-D")
            .or_else(|| arg.strip_prefix("--define="))
        {
            if value.starts_with("test=") || value.starts_with("it.test=") {
                tests.push(value.to_owned());
            }
        }
        if arg == "-D" || arg == "--define" {
            if let Some(value) = args
                .get(index + 1)
                .and_then(Value::as_str)
                .filter(|value| value.starts_with("test=") || value.starts_with("it.test="))
            {
                tests.push(value.to_owned());
            }
        }
    }
    value["selection"] = json!({"modules":modules,"tests":tests});
    value
}

pub(crate) fn summary(mut response: Value) -> Value {
    if !response["job"].is_object() {
        return response;
    }
    let mut compact = compact(&response["job"]);
    if response["code"] == "interrupted"
        || (response["owner_unavailable"] == true && compact["verdict"] == "pending")
    {
        compact["verdict"] = json!(if response["code"] == "interrupted" {
            "interrupted"
        } else {
            "unknown"
        });
        compact["next_action"] = json!("inspect_then_resubmit");
    }
    response.as_object_mut().unwrap().remove("job");
    // Envelope fields (ok/code/participant/ownership) retain their original meaning.
    for (key, value) in response.as_object().unwrap() {
        compact[key] = value.clone();
    }
    compact
}
