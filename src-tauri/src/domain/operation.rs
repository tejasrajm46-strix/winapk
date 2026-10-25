//! Operation stages. Progress is a real stage sequence with real predecessors,
//! never a synthetic percentage.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Received,
    Validating,
    Inspecting,
    CheckingRuntime,
    PreparingRuntime,
    Installing,
    Verifying,
    Launching,
    Completed,
    Failed,
    Cancelled,
    NeedsUserAction,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Received => "Received",
            Stage::Validating => "Validating file",
            Stage::Inspecting => "Inspecting package",
            Stage::CheckingRuntime => "Checking runtime",
            Stage::PreparingRuntime => "Preparing runtime",
            Stage::Installing => "Installing package",
            Stage::Verifying => "Verifying installation",
            Stage::Launching => "Launching app",
            Stage::Completed => "Completed",
            Stage::Failed => "Failed",
            Stage::Cancelled => "Cancelled",
            Stage::NeedsUserAction => "Waiting for you",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Stage::Completed | Stage::Failed | Stage::Cancelled)
    }

    /// Ordered stages a successful install+launch passes through.
    pub fn sequence() -> [Stage; 8] {
        [
            Stage::Received,
            Stage::Validating,
            Stage::Inspecting,
            Stage::CheckingRuntime,
            Stage::PreparingRuntime,
            Stage::Installing,
            Stage::Verifying,
            Stage::Launching,
        ]
    }

    /// Stages that may legally follow `previous`. A skip that bypasses real work
    /// (install, verify) is not a state, it is a bug — so this is a table, not a
    /// "any forward move" rule.
    fn allowed_after(previous: Stage) -> &'static [Stage] {
        match previous {
            Stage::Received => &[Stage::Validating, Stage::Failed, Stage::Cancelled],
            Stage::Validating => &[Stage::Inspecting, Stage::Failed, Stage::Cancelled],
            Stage::Inspecting => &[
                Stage::CheckingRuntime,
                Stage::NeedsUserAction,
                Stage::Failed,
                Stage::Cancelled,
            ],
            Stage::CheckingRuntime => &[
                // PreparingRuntime is skipped when the runtime is already up.
                Stage::PreparingRuntime,
                Stage::Installing,
                Stage::NeedsUserAction,
                Stage::Failed,
                Stage::Cancelled,
            ],
            Stage::PreparingRuntime => &[
                Stage::Installing,
                Stage::NeedsUserAction,
                Stage::Failed,
                Stage::Cancelled,
            ],
            Stage::Installing => &[Stage::Verifying, Stage::Failed, Stage::Cancelled],
            Stage::Verifying => &[
                Stage::Launching,
                Stage::Completed,
                Stage::Failed,
                Stage::Cancelled,
            ],
            Stage::Launching => &[Stage::Completed, Stage::Failed, Stage::Cancelled],
            Stage::NeedsUserAction => &[
                Stage::CheckingRuntime,
                Stage::PreparingRuntime,
                Stage::Installing,
                Stage::Failed,
                Stage::Cancelled,
            ],
            Stage::Completed | Stage::Failed | Stage::Cancelled => &[],
        }
    }

    /// A transition is legal only if it is in the table for its predecessor.
    pub fn can_follow(self, previous: Option<Stage>) -> bool {
        match previous {
            None => self == Stage::Received,
            Some(previous) => Self::allowed_after(previous).contains(&self),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageEvent {
    pub op: String,
    pub package_id: Option<String>,
    pub stage: Stage,
    pub message: String,
    pub at_ms: i64,
}

/// Somewhere to publish stage changes. The desktop app forwards them to the
/// webview; the proof harness prints them.
pub trait StageSink: Send + Sync {
    fn stage(&self, event: &StageEvent);
}

pub struct NoopSink;

impl StageSink for NoopSink {
    fn stage(&self, _event: &StageEvent) {}
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_is_legal() {
        let mut previous = None;
        for stage in Stage::sequence() {
            assert!(stage.can_follow(previous), "{stage:?} after {previous:?}");
            previous = Some(stage);
        }
        assert!(Stage::Completed.can_follow(previous));
    }

    #[test]
    fn backwards_is_illegal() {
        assert!(!Stage::Inspecting.can_follow(Some(Stage::Installing)));
        assert!(!Stage::Received.can_follow(Some(Stage::Validating)));
        assert!(!Stage::Verifying.can_follow(Some(Stage::Inspecting)));
    }

    #[test]
    fn terminal_states_accept_nothing() {
        assert!(!Stage::Installing.can_follow(Some(Stage::Completed)));
        assert!(!Stage::Failed.can_follow(Some(Stage::Failed)));
        for stage in [Stage::Received, Stage::Installing, Stage::Completed] {
            assert!(!stage.can_follow(Some(Stage::Cancelled)));
        }
    }

    #[test]
    fn ready_runtime_may_skip_preparation_only() {
        assert!(Stage::Installing.can_follow(Some(Stage::CheckingRuntime)));
        assert!(!Stage::Verifying.can_follow(Some(Stage::CheckingRuntime)));
        assert!(!Stage::PreparingRuntime.can_follow(Some(Stage::Inspecting)));
    }

    #[test]
    fn failure_and_cancel_end_any_live_operation() {
        for live in Stage::sequence() {
            assert!(Stage::Failed.can_follow(Some(live)));
            assert!(Stage::Cancelled.can_follow(Some(live)));
        }
    }

    #[test]
    fn skipped_stages_are_illegal() {
        // Installing must not follow Inspecting: the runtime check is required.
        assert!(!Stage::Installing.can_follow(Some(Stage::Inspecting)));
        assert!(!Stage::Launching.can_follow(Some(Stage::Installing)));
    }
}
