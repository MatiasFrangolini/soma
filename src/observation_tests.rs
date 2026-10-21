use super::{LogBook, Status};
use crate::observation;
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "soma-observation-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
    fn read(&self, request: Value) -> Value {
        observation::logs(&self.0, &request, None)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.starts_with(std::env::temp_dir()));
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn status(id: &str) -> Status {
    Status {
        state: "running".into(),
        operation_id: Some(id.into()),
        directory: Some(PathBuf::from("C:/disposable checkout")),
        head: Some("fixture-head".into()),
        ..Status::default()
    }
}

#[test]
fn execution_scope_and_cursor_survive_offline_reads_without_mixing_runs() {
    let fixture = Fixture::new();
    let mut book = LogBook::new(fixture.0.clone());
    let mut first = status("first");
    book.begin(&first).unwrap();
    book.append("first-only\n".into());
    let first_cursor = book.observe(&json!({"operation":"first"}), &first)["cursor"].clone();
    first.state = "exited".into();
    book.checkpoint(&first);
    let second = status("second");
    book.begin(&second).unwrap();
    book.append("second-only\n".into());
    let first_logs = fixture.read(json!({"operation":"first"}));
    assert_eq!(first_logs["text"], "first-only");
    assert_eq!(
        first_logs["operation"]["directory"],
        "C:/disposable checkout"
    );
    assert_eq!(first_logs["operation"]["state"], "exited");
    assert_eq!(first_logs["historical"], true);
    assert_eq!(
        fixture.read(json!({"operation":"second","after":first_cursor}))["code"],
        "cursor_expired"
    );
    let baseline = book.observe(&json!({"operation":"second"}), &second);
    assert_eq!(baseline["historical"], false);
    book.append("nuevo ñ✓\nERROR literal\n".into());
    let delta = fixture.read(json!({"operation":"second","after":baseline["cursor"]}));
    assert_eq!(delta["text"], "nuevo ñ✓\nERROR literal\n");
    assert_eq!(delta["source"], "disk");
    assert_eq!(
        fixture.read(json!({"operation":"second","after":delta["cursor"]}))["text"],
        ""
    );
    assert_eq!(
        fixture.read(json!({"operation":"second","contains":"ERROR"}))["text"],
        "ERROR literal"
    );
    assert_eq!(
        fixture.read(json!({"operation":"missing"}))["code"],
        "operation_logs_unavailable"
    );
    let closed = observation::offline_status(&fixture.0, "absent", "fixture");
    assert_eq!(closed["execution_state"], "unknown");
    assert_eq!(closed["last_known_operation"]["operation_id"], "second");
    assert_eq!(closed["health"]["checked_now"], false);
}

#[test]
fn rotation_reports_evicted_cursors_and_retains_unicode_and_attribution() {
    let fixture = Fixture::new();
    let mut book = LogBook::new(fixture.0.clone());
    book.segment_limit = 64;
    let running = status("rotating");
    book.begin(&running).unwrap();
    let baseline = book.observe(&json!({"operation":"rotating"}), &running);
    book.append(format!("{}\nROTATION_COMPLETE\n", "ñ✓".repeat(100)));
    let logs = fixture.read(json!({"operation":"rotating","tail":20000}));
    assert_eq!(logs["retention_truncated"], true);
    assert!(logs["text"].as_str().unwrap().contains("ROTATION_COMPLETE"));
    assert!(!logs["text"].as_str().unwrap().contains('�'));
    assert_eq!(
        fixture.read(json!({"operation":"rotating","after":baseline["cursor"]}))["code"],
        "cursor_expired"
    );
    assert!(
        super::fs::read_dir(&fixture.0)
            .unwrap()
            .flatten()
            .filter(|entry| entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "log"))
            .map(|entry| entry.metadata().unwrap().len())
            .sum::<u64>()
            <= 192
    );
}

#[test]
fn incremental_pages_preserve_every_byte_at_utf8_boundaries() {
    let fixture = Fixture::new();
    let mut book = LogBook::new(fixture.0.clone());
    let running = status("pages");
    book.begin(&running).unwrap();
    let baseline = fixture.read(json!({"operation":"pages"}));
    let expected = format!("{}\nEND\n", "ñ✓".repeat(500000));
    book.append(expected.clone());
    let first = fixture.read(json!({"operation":"pages","after":baseline["cursor"]}));
    assert_eq!(first["has_more"], true);
    let second = fixture.read(json!({"operation":"pages","after":first["cursor"]}));
    assert_eq!(second["has_more"], false);
    assert_eq!(
        format!(
            "{}{}",
            first["text"].as_str().unwrap(),
            second["text"].as_str().unwrap()
        ),
        expected
    );
}

#[test]
fn interrupted_and_legacy_history_never_claim_a_live_execution() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("current.log"), b"legacy\n").unwrap();
    assert_eq!(fixture.read(json!({}))["attribution"], "mixed_or_unknown");
    assert_eq!(
        fixture.read(json!({"operation":"legacy"}))["code"],
        "operation_logs_unavailable"
    );
    {
        let mut book = LogBook::new(fixture.0.clone());
        book.begin(&status("interrupted")).unwrap();
        book.append("before-close\n".into());
    }
    let mut reopened = LogBook::new(fixture.0.clone());
    reopened.begin(&status("replacement")).unwrap();
    reopened.append("replacement-only\n".into());
    let interrupted = fixture.read(json!({"operation":"interrupted"}));
    assert_eq!(interrupted["operation"]["state"], "interrupted");
    assert_eq!(interrupted["text"], "before-close");
    fs::write(fixture.0.join("rotation.pending"), b"pending").unwrap();
    assert_eq!(
        fixture.read(json!({"operation":"replacement"}))["code"],
        "operation_logs_unavailable"
    );
    assert_eq!(fixture.read(json!({}))["cursor"], Value::Null);
}
