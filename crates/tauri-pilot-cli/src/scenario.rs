use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::Client;
use crate::{target_params, with_window};

// ── TOML schema ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Scenario {
    pub connect: Option<Connect>,
    #[serde(default)]
    pub scenario: ScenarioMeta,
    #[serde(default)]
    pub step: Vec<Step>,
}

#[derive(Debug, Deserialize, Default)]
pub struct Connect {
    pub socket: Option<PathBuf>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ScenarioMeta {
    pub name: Option<String>,
    #[serde(default = "default_true")]
    pub fail_fast: bool,
    pub global_timeout_ms: Option<u64>,
}

impl Default for ScenarioMeta {
    fn default() -> Self {
        Self {
            name: None,
            fail_fast: true,
            global_timeout_ms: None,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct Step {
    pub name: Option<String>,
    pub action: String,
    pub timeout_ms: Option<u64>,
    pub target: Option<String>,
    pub value: Option<String>,
    pub text: Option<String>,
    pub key: Option<String>,
    pub url: Option<String>,
    pub script: Option<String>,
    pub expected: Option<String>,
    pub selector: Option<String>,
    pub direction: Option<String>,
    pub amount: Option<i32>,
    #[serde(rename = "ref")]
    pub step_ref: Option<String>,
    pub gone: Option<bool>,
    pub stable: Option<u64>,
    pub require_mutation: Option<bool>,
    pub path: Option<PathBuf>,
}

impl Step {
    fn display_name(&self, idx: usize) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("step-{}", idx + 1))
    }
}

// ── Execution result types ────────────────────────────────────────────────────

#[derive(Debug)]
pub enum StepOutcome {
    Passed { duration: Duration },
    Failed { duration: Duration, message: String },
    Skipped,
}

#[derive(Debug)]
pub struct StepResult {
    pub name: String,
    pub outcome: StepOutcome,
}

#[derive(Debug)]
pub struct ScenarioReport {
    pub name: String,
    pub results: Vec<StepResult>,
    pub total_duration: Duration,
}

impl ScenarioReport {
    pub fn passed(&self) -> usize {
        self.results
            .iter()
            .filter(|r| matches!(r.outcome, StepOutcome::Passed { .. }))
            .count()
    }

    pub fn failed(&self) -> usize {
        self.results
            .iter()
            .filter(|r| matches!(r.outcome, StepOutcome::Failed { .. }))
            .count()
    }

    pub fn skipped(&self) -> usize {
        self.results
            .iter()
            .filter(|r| matches!(r.outcome, StepOutcome::Skipped))
            .count()
    }

    pub fn all_passed(&self) -> bool {
        self.failed() == 0
    }
}

// ── Main runner ───────────────────────────────────────────────────────────────

pub fn load_scenario(path: &Path) -> Result<Scenario> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read scenario file: {}", path.display()))?;
    toml::from_str(&content).with_context(|| {
        format!("Failed to parse scenario TOML: {}", path.display())
    })
}

pub async fn run_scenario(
    client: &mut Client,
    scenario: &Scenario,
    window: Option<&str>,
    fail_fast_override: Option<bool>,
) -> Result<ScenarioReport> {
    let meta = &scenario.scenario;
    let name = meta
        .name
        .clone()
        .unwrap_or_else(|| "unnamed scenario".to_string());
    let fail_fast = fail_fast_override.unwrap_or(meta.fail_fast);

    let total_start = Instant::now();
    let mut results = Vec::with_capacity(scenario.step.len());
    let mut failed = false;

    for (idx, step) in scenario.step.iter().enumerate() {
        let step_name = step.display_name(idx);

        if failed && fail_fast {
            print_step_line(idx, scenario.step.len(), &step_name, "SKIP");
            results.push(StepResult {
                name: step_name,
                outcome: StepOutcome::Skipped,
            });
            continue;
        }

        let step_start = Instant::now();

        let outcome = match run_step(client, step, window).await {
            Ok(_) => {
                let dur = step_start.elapsed();
                print_step_line(idx, scenario.step.len(), &step_name, "ok");
                StepOutcome::Passed { duration: dur }
            }
            Err(e) => {
                let dur = step_start.elapsed();
                let msg = format!("{e:#}");
                print_step_fail(idx, scenario.step.len(), &step_name, &msg);
                // Capture screenshot on failure
                let _ = take_failure_screenshot(client, &step_name, window).await;
                failed = true;
                StepOutcome::Failed {
                    duration: dur,
                    message: msg,
                }
            }
        };

        results.push(StepResult {
            name: step_name,
            outcome,
        });
    }

    Ok(ScenarioReport {
        name,
        results,
        total_duration: total_start.elapsed(),
    })
}

async fn run_step(client: &mut Client, step: &Step, window: Option<&str>) -> Result<Value> {
    let timeout_ms = step.timeout_ms;
    match step.action.as_str() {
        "click" => {
            let t = require_target(step)?;
            client
                .call("click", with_window(Some(target_params(t)), window))
                .await
        }
        "fill" => {
            let t = require_target(step)?;
            let value = step.value.as_deref().unwrap_or("");
            let mut p = target_params(t);
            p["value"] = json!(value);
            client.call("fill", with_window(Some(p), window)).await
        }
        "type" => {
            let t = require_target(step)?;
            let text = step.text.as_deref().unwrap_or("");
            let mut p = target_params(t);
            p["text"] = json!(text);
            client.call("type", with_window(Some(p), window)).await
        }
        "press" => {
            let key = step
                .key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("press step requires 'key'"))?;
            client
                .call("press", with_window(Some(json!({"key": key})), window))
                .await
        }
        "select" => {
            let t = require_target(step)?;
            let value = step.value.as_deref().unwrap_or("");
            let mut p = target_params(t);
            p["value"] = json!(value);
            client.call("select", with_window(Some(p), window)).await
        }
        "check" => {
            let t = require_target(step)?;
            client
                .call("check", with_window(Some(target_params(t)), window))
                .await
        }
        "scroll" => {
            let direction = step.direction.as_deref().unwrap_or("down");
            client
                .call(
                    "scroll",
                    with_window(
                        Some(json!({
                            "direction": direction,
                            "amount": step.amount,
                            "ref": step.step_ref,
                        })),
                        window,
                    ),
                )
                .await
        }
        "navigate" => {
            let url = step
                .url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("navigate step requires 'url'"))?;
            client
                .call("navigate", with_window(Some(json!({"url": url})), window))
                .await
        }
        "wait" => {
            let timeout = timeout_ms.unwrap_or(10_000);
            client
                .call(
                    "wait",
                    with_window(
                        Some(json!({
                            "target": step.target,
                            "selector": step.selector,
                            "gone": step.gone.unwrap_or(false),
                            "timeout": timeout,
                        })),
                        window,
                    ),
                )
                .await
        }
        "watch" => {
            let timeout = timeout_ms.unwrap_or(10_000);
            let stable = step.stable.unwrap_or(300);
            let require_mutation = step.require_mutation.unwrap_or(false);
            let mut params = serde_json::Map::new();
            params.insert("timeout".into(), json!(timeout));
            params.insert("stable".into(), json!(stable));
            if require_mutation {
                params.insert("requireMutation".into(), json!(true));
            }
            if let Some(sel) = &step.selector {
                params.insert("selector".into(), json!(sel));
            }
            client
                .call(
                    "watch",
                    with_window(Some(Value::Object(params)), window),
                )
                .await
        }
        "eval" => {
            let script = step
                .script
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("eval step requires 'script'"))?;
            client
                .call(
                    "eval",
                    with_window(Some(json!({"script": script})), window),
                )
                .await
        }
        "screenshot" => {
            let result = client
                .call(
                    "screenshot",
                    with_window(
                        Some(json!({"path": step.path, "selector": step.selector})),
                        window,
                    ),
                )
                .await?;
            if let Some(path) = &step.path {
                save_screenshot_result(&result, path)?;
            }
            Ok(result)
        }
        "assert-text" => {
            let t = require_target(step)?;
            let expected = step
                .expected
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("assert-text requires 'expected'"))?;
            let result = client
                .call("text", with_window(Some(target_params(t)), window))
                .await?;
            let actual = result.as_str().unwrap_or_default();
            anyhow::ensure!(
                actual == expected,
                "expected text {:?}, got {:?}",
                expected,
                actual
            );
            Ok(json!({"ok": true}))
        }
        "assert-exists" => {
            let t = require_target(step)?;
            // Call visible; if it returns any valid response, element exists
            client
                .call("visible", with_window(Some(target_params(t)), window))
                .await?;
            Ok(json!({"ok": true}))
        }
        "assert-visible" => {
            let t = require_target(step)?;
            let result = client
                .call("visible", with_window(Some(target_params(t)), window))
                .await?;
            let visible = result
                .get("visible")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            anyhow::ensure!(visible, "element is not visible");
            Ok(json!({"ok": true}))
        }
        "assert-hidden" => {
            let t = require_target(step)?;
            let result = client
                .call("visible", with_window(Some(target_params(t)), window))
                .await?;
            let visible = result
                .get("visible")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            anyhow::ensure!(!visible, "element is visible");
            Ok(json!({"ok": true}))
        }
        "assert-value" => {
            let t = require_target(step)?;
            let expected = step
                .expected
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("assert-value requires 'expected'"))?;
            let result = client
                .call("value", with_window(Some(target_params(t)), window))
                .await?;
            let actual = result.as_str().unwrap_or_default();
            anyhow::ensure!(
                actual == expected,
                "expected value {:?}, got {:?}",
                expected,
                actual
            );
            Ok(json!({"ok": true}))
        }
        "assert-url" => {
            let expected = step
                .expected
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("assert-url requires 'expected'"))?;
            let result = client.call("url", with_window(None, window)).await?;
            let actual = result.as_str().unwrap_or_default();
            anyhow::ensure!(
                actual.contains(expected),
                "URL does not contain {:?}, got {:?}",
                expected,
                actual
            );
            Ok(json!({"ok": true}))
        }
        other => anyhow::bail!("unknown step action: {:?}", other),
    }
}

fn require_target(step: &Step) -> Result<&str> {
    step.target
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("step '{}' requires 'target'", step.action))
}

// ── Screenshot on failure ─────────────────────────────────────────────────────

async fn take_failure_screenshot(
    client: &mut Client,
    step_name: &str,
    window: Option<&str>,
) -> Result<()> {
    let dir = Path::new("tauri-pilot-failures");
    std::fs::create_dir_all(dir)?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    // Sanitize step name for use in a filename
    let safe_name: String = step_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let filename = format!("{safe_name}-{ts}.png");
    let path = dir.join(&filename);

    let result = client
        .call("screenshot", with_window(Some(json!({})), window))
        .await?;
    save_screenshot_result(&result, &path)?;
    eprintln!(
        "  {} {}",
        crate::style::dim("failure screenshot →"),
        path.display()
    );
    Ok(())
}

fn save_screenshot_result(result: &Value, path: &Path) -> Result<()> {
    let data_url = result
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("screenshot result is not a string"))?;
    let base64_data = data_url
        .strip_prefix("data:image/png;base64,")
        .unwrap_or(data_url);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| anyhow::anyhow!("Failed to decode base64 screenshot: {e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

// ── Terminal output helpers ───────────────────────────────────────────────────

fn print_step_line(idx: usize, total: usize, name: &str, status: &str) {
    let colored = match status {
        "ok" => crate::style::success(status),
        "SKIP" => crate::style::dim(status),
        _ => crate::style::failure(status),
    };
    eprintln!("  [{}/{}] {} {}", idx + 1, total, name, colored);
}

fn print_step_fail(idx: usize, total: usize, name: &str, msg: &str) {
    eprintln!(
        "  [{}/{}] {} {}\n    {}",
        idx + 1,
        total,
        name,
        crate::style::failure("FAIL"),
        crate::style::failure(msg)
    );
}

pub fn print_report(report: &ScenarioReport) {
    let total = report.results.len();
    let passed = report.passed();
    let failed = report.failed();
    let skipped = report.skipped();
    let secs = report.total_duration.as_secs_f64();

    eprintln!();
    eprintln!(
        "Scenario: {}",
        crate::style::bold(&report.name)
    );
    eprintln!(
        "  {} passed · {} failed · {} skipped  ({:.3}s)",
        passed, failed, skipped, secs
    );
    eprintln!();
}

// ── JUnit XML output ──────────────────────────────────────────────────────────

pub fn write_junit_xml(report: &ScenarioReport, path: &Path) -> Result<()> {
    use quick_xml::Writer;
    use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};

    let total = report.results.len();
    let failures = report.failed();
    let skipped = report.skipped();
    let elapsed = report.total_duration.as_secs_f64();

    let mut buf = Vec::new();
    let mut writer = Writer::new(&mut buf);

    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
    writer.write_event(Event::Text(BytesText::new("\n")))?;

    let mut testsuites = BytesStart::new("testsuites");
    writer.write_event(Event::Start(testsuites))?;
    writer.write_event(Event::Text(BytesText::new("\n  ")))?;

    let mut suite = BytesStart::new("testsuite");
    suite.push_attribute(("name", report.name.as_str()));
    suite.push_attribute(("tests", total.to_string().as_str()));
    suite.push_attribute(("failures", failures.to_string().as_str()));
    suite.push_attribute(("errors", "0"));
    suite.push_attribute(("skipped", skipped.to_string().as_str()));
    suite.push_attribute(("time", format!("{elapsed:.3}").as_str()));
    writer.write_event(Event::Start(suite))?;

    for result in &report.results {
        writer.write_event(Event::Text(BytesText::new("\n    ")))?;
        let dur = match &result.outcome {
            StepOutcome::Passed { duration } | StepOutcome::Failed { duration, .. } => {
                duration.as_secs_f64()
            }
            StepOutcome::Skipped => 0.0,
        };

        let mut tc = BytesStart::new("testcase");
        tc.push_attribute(("name", result.name.as_str()));
        tc.push_attribute(("time", format!("{dur:.3}").as_str()));
        writer.write_event(Event::Start(tc))?;

        match &result.outcome {
            StepOutcome::Passed { .. } => {}
            StepOutcome::Skipped => {
                writer.write_event(Event::Empty(BytesStart::new("skipped")))?;
            }
            StepOutcome::Failed { message, .. } => {
                let mut failure = BytesStart::new("failure");
                // Escape message for XML attribute
                let escaped = xml_attr_escape(message);
                failure.push_attribute(("message", escaped.as_str()));
                writer.write_event(Event::Empty(failure))?;
            }
        }

        writer.write_event(Event::End(BytesEnd::new("testcase")))?;
    }

    writer.write_event(Event::Text(BytesText::new("\n  ")))?;
    writer.write_event(Event::End(BytesEnd::new("testsuite")))?;
    writer.write_event(Event::Text(BytesText::new("\n")))?;
    writer.write_event(Event::End(BytesEnd::new("testsuites")))?;
    writer.write_event(Event::Text(BytesText::new("\n")))?;

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, &buf)
        .with_context(|| format!("Failed to write JUnit XML to {}", path.display()))?;

    Ok(())
}

fn xml_attr_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_report(results: Vec<(&str, StepOutcome)>) -> ScenarioReport {
        ScenarioReport {
            name: "test-scenario".to_string(),
            results: results
                .into_iter()
                .map(|(name, outcome)| StepResult {
                    name: name.to_string(),
                    outcome,
                })
                .collect(),
            total_duration: Duration::from_millis(1234),
        }
    }

    #[test]
    fn test_scenario_report_counts() {
        let report = make_report(vec![
            ("step-1", StepOutcome::Passed { duration: Duration::from_millis(100) }),
            ("step-2", StepOutcome::Failed { duration: Duration::from_millis(50), message: "oops".into() }),
            ("step-3", StepOutcome::Skipped),
        ]);
        assert_eq!(report.passed(), 1);
        assert_eq!(report.failed(), 1);
        assert_eq!(report.skipped(), 1);
        assert!(!report.all_passed());
    }

    #[test]
    fn test_scenario_report_all_passed() {
        let report = make_report(vec![
            ("step-1", StepOutcome::Passed { duration: Duration::from_millis(10) }),
            ("step-2", StepOutcome::Passed { duration: Duration::from_millis(20) }),
        ]);
        assert!(report.all_passed());
    }

    #[test]
    fn test_toml_parse_minimal() {
        let toml = r#"
[[step]]
action = "click"
target = "#btn"
"#;
        let scenario: Scenario = toml::from_str(toml).unwrap();
        assert_eq!(scenario.step.len(), 1);
        assert_eq!(scenario.step[0].action, "click");
        assert_eq!(scenario.step[0].target.as_deref(), Some("#btn"));
        assert!(scenario.scenario.fail_fast); // default true
    }

    #[test]
    fn test_toml_parse_full_meta() {
        let toml = r#"
[connect]
socket = "/tmp/test.sock"
timeout_ms = 5000

[scenario]
name = "login flow"
fail_fast = false
global_timeout_ms = 60000

[[step]]
name = "navigate"
action = "navigate"
url = "http://localhost:5173"
timeout_ms = 3000

[[step]]
name = "assert title"
action = "assert-text"
target = "h1"
expected = "Login"
"#;
        let scenario: Scenario = toml::from_str(toml).unwrap();
        assert_eq!(scenario.scenario.name.as_deref(), Some("login flow"));
        assert!(!scenario.scenario.fail_fast);
        assert_eq!(scenario.scenario.global_timeout_ms, Some(60000));
        assert_eq!(scenario.step.len(), 2);

        let connect = scenario.connect.as_ref().unwrap();
        assert_eq!(connect.socket.as_deref(), Some(Path::new("/tmp/test.sock")));
        assert_eq!(connect.timeout_ms, Some(5000));

        let step = &scenario.step[1];
        assert_eq!(step.name.as_deref(), Some("assert title"));
        assert_eq!(step.action, "assert-text");
        assert_eq!(step.target.as_deref(), Some("h1"));
        assert_eq!(step.expected.as_deref(), Some("Login"));
    }

    #[test]
    fn test_toml_default_fail_fast() {
        let toml = r#"
[[step]]
action = "ping"
"#;
        let scenario: Scenario = toml::from_str(toml).unwrap();
        assert!(scenario.scenario.fail_fast);
    }

    #[test]
    fn test_toml_step_display_name_uses_name_field() {
        let step = Step {
            name: Some("my step".to_string()),
            action: "click".to_string(),
            timeout_ms: None,
            target: None,
            value: None,
            text: None,
            key: None,
            url: None,
            script: None,
            expected: None,
            selector: None,
            direction: None,
            amount: None,
            step_ref: None,
            gone: None,
            stable: None,
            require_mutation: None,
            path: None,
        };
        assert_eq!(step.display_name(0), "my step");
    }

    #[test]
    fn test_toml_step_display_name_fallback() {
        let step = Step {
            name: None,
            action: "click".to_string(),
            timeout_ms: None,
            target: None,
            value: None,
            text: None,
            key: None,
            url: None,
            script: None,
            expected: None,
            selector: None,
            direction: None,
            amount: None,
            step_ref: None,
            gone: None,
            stable: None,
            require_mutation: None,
            path: None,
        };
        assert_eq!(step.display_name(2), "step-3");
    }

    #[test]
    fn test_junit_xml_all_passed() {
        let report = make_report(vec![
            ("click button", StepOutcome::Passed { duration: Duration::from_millis(123) }),
            ("fill form", StepOutcome::Passed { duration: Duration::from_millis(45) }),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.xml");
        write_junit_xml(&report, &path).unwrap();
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains(r#"name="test-scenario""#));
        assert!(xml.contains(r#"failures="0""#));
        assert!(xml.contains(r#"name="click button""#));
        assert!(xml.contains(r#"name="fill form""#));
        assert!(!xml.contains("<failure"));
        assert!(!xml.contains("<skipped"));
    }

    #[test]
    fn test_junit_xml_with_failures_and_skips() {
        let report = make_report(vec![
            ("step-1", StepOutcome::Passed { duration: Duration::from_millis(10) }),
            ("step-2", StepOutcome::Failed { duration: Duration::from_millis(20), message: "oops & done".into() }),
            ("step-3", StepOutcome::Skipped),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.xml");
        write_junit_xml(&report, &path).unwrap();
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains(r#"failures="1""#));
        assert!(xml.contains(r#"skipped="1""#));
        assert!(xml.contains(r#"message="oops &amp; done""#));
        assert!(xml.contains("<skipped"));
    }

    #[test]
    fn test_xml_attr_escape() {
        assert_eq!(xml_attr_escape("a & b"), "a &amp; b");
        assert_eq!(xml_attr_escape(r#"say "hi""#), "say &quot;hi&quot;");
        assert_eq!(xml_attr_escape("<tag>"), "&lt;tag&gt;");
    }
}
