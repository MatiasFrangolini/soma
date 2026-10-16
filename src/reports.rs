use crate::model::Result;
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufReader, Read},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Evidence {
    pub status: String,
    pub reports: u64,
    pub tests: u64,
    pub executed: u64,
    pub failures: u64,
    pub errors: u64,
    pub skipped: u64,
    pub test_reported_ms: f64,
    pub details: Vec<Detail>,
    pub diagnostics: Vec<String>,
    pub details_truncated: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Detail {
    pub class: String,
    pub test: String,
    pub outcome: String,
    pub message: String,
}
fn bounded(value: &str, max: usize) -> String {
    let mut end = value.len().min(max);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].into()
}
fn attrs(event: &BytesStart<'_>) -> Result<BTreeMap<String, String>> {
    event
        .attributes()
        .map(|attribute| {
            let a = attribute.map_err(|_| "Atributo XML inválido")?;
            let key = a.key.as_ref().to_owned();
            let value = a
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|_| "Valor XML inválido")?;
            Ok((key, value.into_owned()))
        })
        .collect()
}
fn number(values: &BTreeMap<String, String>, key: &str) -> Result<u64> {
    values
        .get(key)
        .map(String::as_str)
        .unwrap_or("0")
        .parse()
        .map_err(|_| format!("Conteo XML inválido: {key}"))
}
fn file(path: &Path, job: &str) -> Result<Evidence> {
    let f = fs::File::open(path).map_err(|e| e.to_string())?;
    if f.metadata().map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
        return Err("Reporte mayor que el límite de lectura de 8 MiB".into());
    }
    let mut reader = Reader::from_reader(BufReader::new(f.take(8 * 1024 * 1024 + 1)));
    let mut buffer = Vec::new();
    let mut result = Evidence::default();
    let mut cases = 0;
    let mut case = Detail::default();
    let mut capturing = false;
    let mut depth = 0;
    let mut root_started = false;
    let mut root_closed = false;
    let (mut failures, mut errors, mut skipped) = (0, 0, 0);
    let suffix = format!("-{job}");
    let class_suffix = format!("({job})");
    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|_| "Reporte XML incompleto o inválido")?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                if root_closed || depth > 64 {
                    return Err("Estructura XML inválida".into());
                }
                if depth == 0 {
                    if root_started || element.name().as_ref() != "testsuite" {
                        return Err("Raíz XML inválida".into());
                    }
                    root_started = true;
                }
                match element.name().as_ref() {
                    "testsuite" => {
                        if depth != 0 {
                            return Err("Suite anidada no admitida".into());
                        }
                        let values = attrs(element)?;
                        result.tests = number(&values, "tests")?;
                        result.failures = number(&values, "failures")?;
                        result.errors = number(&values, "errors")?;
                        result.skipped = number(&values, "skipped")?;
                        let seconds: f64 = values
                            .get("time")
                            .map(String::as_str)
                            .unwrap_or("0")
                            .parse()
                            .map_err(|_| "Duración XML inválida")?;
                        if !seconds.is_finite() || seconds < 0.0 {
                            return Err("Duración XML inválida".into());
                        }
                        result.test_reported_ms = seconds * 1000.0;
                    }
                    "testcase" => {
                        if depth != 1 {
                            return Err("Test fuera de la suite".into());
                        }
                        let values = attrs(element)?;
                        cases += 1;
                        let class = values.get("classname").cloned().unwrap_or_default();
                        case = Detail {
                            class: bounded(
                                class
                                    .strip_suffix(&class_suffix)
                                    .or_else(|| class.strip_suffix(&suffix))
                                    .unwrap_or(&class),
                                512,
                            ),
                            test: bounded(
                                values.get("name").map(String::as_str).unwrap_or(""),
                                512,
                            ),
                            ..Default::default()
                        };
                    }
                    "failure" | "error" | "skipped" => {
                        if depth != 2 || !case.outcome.is_empty() {
                            return Err("Resultado de test inválido".into());
                        }
                        let values = attrs(element)?;
                        case.outcome = element.name().as_ref().into();
                        match case.outcome.as_str() {
                            "failure" => failures += 1,
                            "error" => errors += 1,
                            _ => skipped += 1,
                        }
                        case.message = bounded(
                            values.get("message").map(String::as_str).unwrap_or(""),
                            2048,
                        );
                        capturing = !empty;
                    }
                    _ => {}
                }
                if empty {
                    if depth == 0 {
                        root_closed = true;
                    }
                    if element.name().as_ref() == "testcase" {
                        case = Detail::default();
                    }
                } else {
                    depth += 1;
                }
            }
            Event::Text(text) if capturing && case.message.len() < 2048 => {
                if let Ok(decoded) = quick_xml::escape::unescape(text.as_ref()) {
                    let room = 2048 - case.message.len();
                    case.message.push_str(&bounded(&decoded, room));
                }
            }
            Event::CData(text) if capturing && case.message.len() < 2048 => {
                let room = 2048 - case.message.len();
                case.message.push_str(&bounded(text.as_ref(), room));
            }
            Event::End(element) => {
                if depth == 0 {
                    return Err("Cierre XML inválido".into());
                }
                depth -= 1;
                if depth == 0 {
                    root_closed = true;
                }
                match element.name().as_ref() {
                    "failure" | "error" | "skipped" => {
                        capturing = false;
                    }
                    "testcase" => {
                        if !case.outcome.is_empty() {
                            if result.details.len() < 20 {
                                result.details.push(std::mem::take(&mut case));
                            } else {
                                result.details_truncated = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(text) if depth == 0 && !text.as_ref().trim().is_empty() => {
                return Err("Texto fuera de la suite".into())
            }
            Event::DocType(_) => return Err("El reporte no admite DTD".into()),
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !root_closed
        || depth != 0
        || cases != result.tests
        || result.skipped > result.tests
        || (failures, errors, skipped) != (result.failures, result.errors, result.skipped)
        || result
            .failures
            .saturating_add(result.errors)
            .saturating_add(result.skipped)
            > result.tests
    {
        return Err("El reporte no contiene evidencia completa de sus tests declarados".into());
    }
    result.reports = 1;
    result.executed = result.tests - result.skipped;
    Ok(result)
}
pub fn collect(modules: &[PathBuf], job: &str, requested: bool) -> Evidence {
    if !requested {
        return Evidence {
            status: "not_requested".into(),
            ..Default::default()
        };
    }
    let mut output = Evidence::default();
    let directories = modules.iter().flat_map(|module| {
        [
            module.join("target/surefire-reports"),
            module.join("target/failsafe-reports"),
        ]
    });
    let suffix = format!("-{job}.xml");
    let mut visited = 0;
    for dir in directories {
        if visited > 100_000 {
            output
                .diagnostics
                .push("Se alcanzó el límite de exploración de reportes".into());
            break;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                if output.diagnostics.len() < 20 {
                    output
                        .diagnostics
                        .push("No se pudo explorar un directorio de reportes".into());
                }
                continue;
            }
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > 100_000 {
                output
                    .diagnostics
                    .push("Se alcanzó el límite de exploración de reportes".into());
                break;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("TEST-") || !name.ends_with(&suffix) {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_attributes() & 0x400 != 0 {
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            match file(&path, job) {
                Ok(e) => {
                    output.reports += e.reports;
                    output.tests += e.tests;
                    output.executed += e.executed;
                    output.failures += e.failures;
                    output.errors += e.errors;
                    output.skipped += e.skipped;
                    output.test_reported_ms += e.test_reported_ms;
                    for detail in e.details {
                        if output.details.len() < 20 {
                            output.details.push(detail);
                        } else {
                            output.details_truncated = true;
                        }
                    }
                    output.details_truncated |= e.details_truncated;
                }
                Err(error) => {
                    if output.diagnostics.len() < 20 {
                        output.diagnostics.push(error);
                    }
                }
            }
        }
    }
    output.status = if !output.diagnostics.is_empty() {
        "incomplete"
    } else if output.executed == 0 {
        "no_tests"
    } else if output.failures + output.errors > 0 {
        "failed"
    } else if output.skipped > 0 {
        "passed_with_skips"
    } else {
        "passed"
    }
    .into();
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    // Contract owner: report evidence must never bless truncated or inconsistent XML.
    #[test]
    fn evidence_rejects_incomplete_reports_and_ignores_other_runs() {
        let root = std::env::temp_dir().join(format!("soma-evidence-{}", std::process::id()));
        let dir = root.join("target/surefire-reports");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("TEST-Probe-job-current.xml");
        fs::write(dir.join("TEST-stale.xml"), "<testsuite tests=\"999\"/>").unwrap();
        let unrelated = root.join("unrelated/target/surefire-reports");
        fs::create_dir_all(&unrelated).unwrap();
        let unrelated_report = unrelated.join("TEST-unrelated-job-current.xml");
        fs::write(&unrelated_report, "<truncated-unrelated-checkout").unwrap();
        let suite =
            "<testsuite tests=\"1\" failures=\"0\" errors=\"0\" skipped=\"0\" time=\"0.25\">";
        for tail in [
            "<testcase name=\"x\"/>",
            "<testcase name=\"x\"><failure/></testcase></testsuite>",
            "<testcase name=\"x\"/></testsuite><other>",
            "<testcase name=\"x\"><other></testcase></testsuite>",
        ] {
            fs::write(&path, format!("{suite}{tail}")).unwrap();
            assert_eq!(
                collect(std::slice::from_ref(&root), "job-current", true).status,
                "incomplete"
            );
        }
        fs::write(&path, format!("{suite}<testcase name=\"x\"/></testsuite>")).unwrap();
        let evidence = collect(std::slice::from_ref(&root), "job-current", true);
        assert_eq!(
            (evidence.status.as_str(), evidence.tests, evidence.executed),
            ("passed", 1, 1)
        );
        assert_eq!(evidence.test_reported_ms, 250.0);
        fs::remove_file(&path).unwrap();
        assert_eq!(
            collect(std::slice::from_ref(&root), "job-current", true).status,
            "no_tests"
        );
        let _ = fs::remove_file(dir.join("TEST-stale.xml"));
        let _ = fs::remove_dir(&dir);
        let _ = fs::remove_dir(root.join("target"));
        let _ = fs::remove_file(unrelated_report);
        let _ = fs::remove_dir(unrelated);
        let _ = fs::remove_dir(root.join("unrelated/target"));
        let _ = fs::remove_dir(root.join("unrelated"));
        let _ = fs::remove_dir(root);
    }
}
