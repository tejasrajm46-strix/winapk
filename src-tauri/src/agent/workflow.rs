//! Multi-step automation that reports dispatch and verification separately.
//!
//! One step is one action: it goes through `ControlService::dispatch`, which is
//! the same path a single MCP tool call takes — no second action layer. What a
//! step adds is an observation afterwards and an optional expectation, so a
//! caller can tell "the tap was sent" from "the tap did what I wanted".
//!
//! ```text
//! dispatch ──▶ observed ──▶ verified?
//!   │             │             │
//!   │             │             └─ did the observation meet the expectation?
//!   │             └─ app running, window open, display id, screenshot hash
//!   └─ did the tool accept the action at all?
//! ```
//!
//! A workflow stops at the first step that cannot be dispatched (nothing after it
//! could be trusted), but a failed expectation does not stop it: the caller asked
//! for those steps, and the result says which ones did not verify.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::control::{Cancel, ControlService};
use super::ui::Selector;
use crate::domain::{now_ms, DwError, DwResult, ErrorCode};

/// One action inside a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    /// Any tool name `ControlService::dispatch` accepts.
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    /// What should be true once the step has run. Omit it to only observe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<Expect>,
}

impl Step {
    pub fn new(tool: impl Into<String>, arguments: Value) -> Self {
        Self {
            tool: tool.into(),
            arguments,
            verify: None,
        }
    }

    pub fn expecting(mut self, expect: Expect) -> Self {
        self.verify = Some(expect);
        self
    }

    fn validate(&self) -> DwResult<()> {
        let name = self.tool.trim();
        if name.is_empty() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "A workflow step needs a tool name.",
            ));
        }
        if !self.arguments.is_null() && !self.arguments.is_object() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                format!("The arguments for {} must be an object.", self.tool),
            ));
        }
        Ok(())
    }

    fn arguments(&self) -> Value {
        if self.arguments.is_null() {
            json!({})
        } else {
            self.arguments.clone()
        }
    }
}

/// An expectation over what DroidWin can actually observe. There is no
/// "screen looks right" check here: a screenshot is compared by hash, which says
/// the picture changed, not that it changed correctly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expect {
    /// The package has a process.
    AppRunning { package_id: String },
    /// DroidWin has a window open for the package.
    WindowOpen { package_id: String },
    /// The package owns a display, so input can be addressed to it.
    DisplayPresent { package_id: String },
    /// An accessibility element matching the selector is on the display.
    ElementPresent {
        package_id: String,
        selector: Selector,
    },
    /// No element matches the selector.
    ElementAbsent {
        package_id: String,
        selector: Selector,
    },
    /// The screenshot is not the one taken before this step.
    ScreenChanged,
    /// Delegate to the `verify_action` tool's own expectations.
    State { expect: Value },
}

impl Expect {
    /// The package this expectation is about, when it names one.
    pub fn package_id(&self) -> Option<&str> {
        match self {
            Expect::AppRunning { package_id }
            | Expect::WindowOpen { package_id }
            | Expect::DisplayPresent { package_id }
            | Expect::ElementPresent { package_id, .. }
            | Expect::ElementAbsent { package_id, .. } => Some(package_id),
            Expect::ScreenChanged | Expect::State { .. } => None,
        }
    }
}

/// What a workflow was asked to do, kept together so a trigger and a direct call
/// store and run exactly the same shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The app this workflow is about, used when a step does not name one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    pub steps: Vec<Step>,
}

impl Workflow {
    pub fn validate(&self) -> DwResult<()> {
        if self.steps.is_empty() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "A workflow needs at least one step.",
            ));
        }
        if self.steps.len() > 40 {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "A workflow can hold at most 40 steps.",
            ));
        }
        for step in &self.steps {
            step.validate()?;
        }
        Ok(())
    }
}

/// Run every step, in order, and report what happened to each.
pub async fn run(
    control: &ControlService,
    workflow: &Workflow,
    cancel: &Cancel,
) -> DwResult<Value> {
    workflow.validate()?;
    let started = std::time::Instant::now();
    let mut results: Vec<Value> = Vec::new();
    let mut stopped: Option<&'static str> = None;

    for (index, step) in workflow.steps.iter().enumerate() {
        if cancel.tripped() {
            stopped = Some("cancelled");
            break;
        }
        let arguments = with_default_package(step.arguments(), workflow.package_id.as_deref());
        let package = package_of(&arguments).or_else(|| workflow.package_id.clone());
        let display = argument_display(&arguments);

        // The "before" picture only when the step's expectation is a comparison.
        let before = if matches!(step.verify, Some(Expect::ScreenChanged)) {
            control
                .screen_hash(package.as_deref(), display)
                .await
                .ok()
                .flatten()
        } else {
            None
        };

        let dispatched = control.dispatch(&step.tool, &arguments, cancel).await;
        let observation = control
            .observe(package.as_deref(), display, step.tool.as_str())
            .await
            .unwrap_or(Value::Null);

        match dispatched {
            Ok(outcome) => {
                let mut entry = json!({
                    "index": index,
                    "tool": step.tool,
                    "dispatched": true,
                    "dispatch": {
                        "summary": outcome.summary,
                        "display_id": outcome.display_id,
                        "package_id": outcome.package_id,
                    },
                    "observation": observation,
                });
                let (verified, detail) = match &step.verify {
                    None => (
                        Value::Null,
                        "no expectation was given, so nothing was checked".to_string(),
                    ),
                    Some(expect) => match verify(
                        control,
                        expect,
                        &observation,
                        before.as_deref(),
                        package.as_deref(),
                        display,
                    )
                    .await
                    {
                        Ok((ok, detail)) => (json!(ok), detail),
                        Err(err) => (json!(false), err.summary),
                    },
                };
                entry["verified"] = verified;
                entry["verify_detail"] = json!(detail);
                results.push(entry);
            }
            Err(err) if err.code == ErrorCode::Cancelled => {
                results.push(json!({
                    "index": index,
                    "tool": step.tool,
                    "dispatched": false,
                    "error": { "code": err.code.as_str(), "summary": err.summary },
                    "verified": false,
                    "verify_detail": "the step was cancelled before it was dispatched",
                }));
                stopped = Some("cancelled");
                break;
            }
            Err(err) => {
                results.push(json!({
                    "index": index,
                    "tool": step.tool,
                    "dispatched": false,
                    "error": { "code": err.code.as_str(), "summary": err.summary, "detail": err.detail },
                    "observation": observation,
                    "verified": false,
                    "verify_detail": "the action was not dispatched, so nothing could verify it",
                }));
                stopped = Some("dispatch_failed");
                break;
            }
        }
    }

    let verified = results.iter().all(|step| {
        step.get("dispatched").and_then(Value::as_bool) == Some(true)
            && step
                .get("verified")
                .map(|value| value.is_null() || value.as_bool() == Some(true))
                .unwrap_or(false)
    });
    let dispatched_count = results
        .iter()
        .filter(|step| step.get("dispatched").and_then(Value::as_bool) == Some(true))
        .count();
    Ok(json!({
        "name": workflow.name,
        "package_id": workflow.package_id,
        "steps": results,
        "dispatched_count": dispatched_count,
        "verified": verified,
        "stopped": stopped,
        "cancelled": stopped == Some("cancelled"),
        "finished_at": now_ms(),
        "duration_ms": started.elapsed().as_millis() as u64,
    }))
}

/// A step that names no package inherits the workflow's, so a tap does not have
/// to repeat the app it is tapping.
fn with_default_package(mut arguments: Value, package_id: Option<&str>) -> Value {
    if let (Some(object), Some(package_id)) = (arguments.as_object_mut(), package_id) {
        if !object.contains_key("package_id") {
            object.insert("package_id".into(), json!(package_id));
        }
    }
    arguments
}

fn package_of(arguments: &Value) -> Option<String> {
    arguments
        .get("package_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn argument_display(arguments: &Value) -> Option<u32> {
    arguments
        .get("display_id")
        .and_then(Value::as_u64)
        .map(|value| value as u32)
}

async fn verify(
    control: &ControlService,
    expect: &Expect,
    observation: &Value,
    before: Option<&str>,
    package: Option<&str>,
    display: Option<u32>,
) -> DwResult<(bool, String)> {
    match expect {
        Expect::AppRunning { package_id } => {
            let running = control.app_running(package_id).await?;
            Ok((
                running,
                format!(
                    "{package_id} is {}",
                    if running { "running" } else { "not running" }
                ),
            ))
        }
        Expect::WindowOpen { package_id } => {
            let open = observation
                .get("window_open")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok((
                open,
                format!(
                    "the window for {package_id} is {}",
                    if open { "open" } else { "closed" }
                ),
            ))
        }
        Expect::DisplayPresent { package_id } => {
            let open = observation
                .get("display_id")
                .map(|value| !value.is_null())
                .unwrap_or(false);
            Ok((
                open,
                format!(
                    "{package_id} {} a display input can target",
                    if open { "has" } else { "has no" }
                ),
            ))
        }
        Expect::ElementPresent {
            package_id,
            selector,
        } => {
            let present = control.has_element(package_id, display, selector).await?;
            Ok((
                present,
                format!(
                    "the element is {}present on {package_id}",
                    if present { "" } else { "not " }
                ),
            ))
        }
        Expect::ElementAbsent {
            package_id,
            selector,
        } => {
            let present = control.has_element(package_id, display, selector).await?;
            Ok((
                !present,
                format!(
                    "the element is {}present on {package_id}",
                    if present { "still " } else { "not " }
                ),
            ))
        }
        Expect::ScreenChanged => {
            let after = control.screen_hash(package, display).await?;
            let changed = match (&before, &after) {
                (Some(before), Some(after)) => before != after,
                _ => false,
            };
            Ok((
                changed,
                if changed {
                    "the screenshot differs from the one taken before the step".to_string()
                } else {
                    "the screenshot is unchanged, so nothing visible happened".to_string()
                },
            ))
        }
        Expect::State { expect } => {
            let outcome = control
                .dispatch(
                    "verify_action",
                    &json!({ "expect": expect, "display_id": display, "package_id": package }),
                    &Cancel::new(),
                )
                .await?;
            let verified = outcome
                .payload
                .get("verified")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok((verified, outcome.summary))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quickstep(tool: &str) -> Step {
        Step::new(tool, json!({}))
    }

    #[test]
    fn a_workflow_needs_steps_and_an_object_of_arguments() {
        let empty = Workflow {
            name: None,
            package_id: None,
            steps: Vec::new(),
        };
        assert_eq!(
            empty.validate().unwrap_err().code,
            ErrorCode::InvalidRequest
        );

        let bad = Workflow {
            name: None,
            package_id: None,
            steps: vec![Step {
                tool: "tap".into(),
                arguments: json!([1, 2]),
                verify: None,
            }],
        };
        assert!(bad.validate().is_err());

        let good = Workflow {
            name: Some("start the game".into()),
            package_id: Some("org.asafonov.blockbuster".into()),
            steps: vec![quickstep("open_app"), quickstep("tap")],
        };
        assert!(good.validate().is_ok());
    }

    #[test]
    fn a_step_without_arguments_dispatches_an_empty_object() {
        let step = Step::new("get_runtime_status", Value::Null);
        assert_eq!(step.arguments(), json!({}));
    }

    #[test]
    fn a_step_inherits_the_workflow_package_but_keeps_its_own_display() {
        let inherited = with_default_package(json!({ "x": 1 }), Some("org.example"));
        assert_eq!(inherited["package_id"], "org.example");
        assert_eq!(inherited["x"], 1);

        let explicit = with_default_package(
            json!({ "package_id": "org.other", "display_id": 7 }),
            Some("org.example"),
        );
        assert_eq!(explicit["package_id"], "org.other");
        assert_eq!(explicit["display_id"], 7);
    }

    #[test]
    fn an_expectation_names_the_package_it_is_about() {
        let expect = Expect::WindowOpen {
            package_id: "org.example".into(),
        };
        assert_eq!(expect.package_id(), Some("org.example"));
        assert_eq!(Expect::ScreenChanged.package_id(), None);
    }
}
