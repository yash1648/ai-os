use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// States — matching docs/07-state-machine.md exactly
// ---------------------------------------------------------------------------

/// Primary (happy-path) states for an objective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum ObjectivePrimaryState {
    Discovered,
    Planned,
    Ready,
    Executing,
    Review,
    Integration,
    Done,
}

/// Failure states — entered when a stage cannot successfully complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum ObjectiveFailureState {
    PlanningFailure,
    PermissionFailure,
    ExecutionFailure,
    ReviewFailure,
    IntegrationFailure,
    HumanRejected,
    Rollback,
}

/// Terminal (absorbing) state — objective will never transition again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum ObjectiveTerminalState {
    Done,
    Abandoned,
}

/// The full state space of any objective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum ObjectiveState {
    Primary(ObjectivePrimaryState),
    Failure(ObjectiveFailureState),
    Terminal(ObjectiveTerminalState),
}

impl ObjectiveState {
    /// True if this is a terminal state — no further transitions are legal.
    pub fn is_terminal(&self) -> bool {
        matches!(self, ObjectiveState::Terminal(_))
    }

    /// True if this is a failure state.
    pub fn is_failure(&self) -> bool {
        matches!(self, ObjectiveState::Failure(_))
    }

    /// Parse from a label string (inverse of `label()`).
    pub fn from_label(s: &str) -> Self {
        match s {
            "DISCOVERED" => Self::Primary(ObjectivePrimaryState::Discovered),
            "PLANNED" => Self::Primary(ObjectivePrimaryState::Planned),
            "READY" => Self::Primary(ObjectivePrimaryState::Ready),
            "EXECUTING" => Self::Primary(ObjectivePrimaryState::Executing),
            "REVIEW" => Self::Primary(ObjectivePrimaryState::Review),
            "INTEGRATION" => Self::Primary(ObjectivePrimaryState::Integration),
            "DONE" => Self::Terminal(ObjectiveTerminalState::Done),
            "PLANNING_FAILURE" => Self::Failure(ObjectiveFailureState::PlanningFailure),
            "PERMISSION_FAILURE" => Self::Failure(ObjectiveFailureState::PermissionFailure),
            "EXECUTION_FAILURE" => Self::Failure(ObjectiveFailureState::ExecutionFailure),
            "REVIEW_FAILURE" => Self::Failure(ObjectiveFailureState::ReviewFailure),
            "INTEGRATION_FAILURE" => Self::Failure(ObjectiveFailureState::IntegrationFailure),
            "HUMAN_REJECTED" => Self::Failure(ObjectiveFailureState::HumanRejected),
            "ROLLBACK" => Self::Failure(ObjectiveFailureState::Rollback),
            "ABANDONED" => Self::Terminal(ObjectiveTerminalState::Abandoned),
            _ => Self::Terminal(ObjectiveTerminalState::Abandoned), // safe fallback
        }
    }

    /// Human-readable label for display / events.
    pub fn label(&self) -> &'static str {
        match self {
            ObjectiveState::Primary(s) => s.label(),
            ObjectiveState::Failure(s) => s.label(),
            ObjectiveState::Terminal(s) => s.label(),
        }
    }
}

impl ObjectivePrimaryState {
    pub fn label(&self) -> &'static str {
        match self {
            ObjectivePrimaryState::Discovered => "DISCOVERED",
            ObjectivePrimaryState::Planned => "PLANNED",
            ObjectivePrimaryState::Ready => "READY",
            ObjectivePrimaryState::Executing => "EXECUTING",
            ObjectivePrimaryState::Review => "REVIEW",
            ObjectivePrimaryState::Integration => "INTEGRATION",
            ObjectivePrimaryState::Done => "DONE",
        }
    }
}

impl ObjectiveFailureState {
    pub fn label(&self) -> &'static str {
        match self {
            ObjectiveFailureState::PlanningFailure => "PLANNING_FAILURE",
            ObjectiveFailureState::PermissionFailure => "PERMISSION_FAILURE",
            ObjectiveFailureState::ExecutionFailure => "EXECUTION_FAILURE",
            ObjectiveFailureState::ReviewFailure => "REVIEW_FAILURE",
            ObjectiveFailureState::IntegrationFailure => "INTEGRATION_FAILURE",
            ObjectiveFailureState::HumanRejected => "HUMAN_REJECTED",
            ObjectiveFailureState::Rollback => "ROLLBACK",
        }
    }
}

impl ObjectiveTerminalState {
    pub fn label(&self) -> &'static str {
        match self {
            ObjectiveTerminalState::Done => "DONE",
            ObjectiveTerminalState::Abandoned => "ABANDONED",
        }
    }
}

// ---------------------------------------------------------------------------
// Transition errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("Transition from {from} to {to} is not allowed")]
    IllegalTransition { from: &'static str, to: &'static str },

    #[error("Cannot transition from a terminal state ({state})")]
    TerminalStateReached { state: &'static str },

    #[error("Retry limit exhausted for objective")]
    RetryLimitExhausted,

    #[error("Transition to {to} requires a reason but none was provided")]
    MissingReason { to: &'static str },
}

pub type TransitionResult = Result<ObjectiveState, TransitionError>;

// ---------------------------------------------------------------------------
// Transition rules
// ---------------------------------------------------------------------------

/// Configuration for retry behaviour.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of times a failed objective may be retried.
    pub max_retries: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_retries: 3 }
    }
}

/// Apply a state transition given the current state and the target.
///
/// Enforces the rules from docs/07-state-machine.md:
///
/// - Transitions are strictly forward, or into a failure state.
/// - From any failure state, the Kernel may re-enter `READY` (retry) if within
///   configured retry limits, or transition to `ABANDONED`.
/// - `ROLLBACK` always resolves to `READY` or `ABANDONED`.
/// - No objective may skip a state.
///
pub fn transition(
    current: ObjectiveState,
    target: ObjectiveState,
    retry_policy: &RetryPolicy,
    retry_count: u32,
) -> TransitionResult {
    // Terminal states are frozen — no transitions allowed.
    if current.is_terminal() {
        return Err(TransitionError::TerminalStateReached {
            state: current.label(),
        });
    }

    let allowed = match (current, target) {
        // ═══════════════════════════════════════════════════════════════════
        // Forward primary-state transitions
        // ═══════════════════════════════════════════════════════════════════
        (ObjectiveState::Primary(ObjectivePrimaryState::Discovered),
         ObjectiveState::Primary(ObjectivePrimaryState::Planned)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Planned),
         ObjectiveState::Primary(ObjectivePrimaryState::Ready)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Ready),
         ObjectiveState::Primary(ObjectivePrimaryState::Executing)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Executing),
         ObjectiveState::Primary(ObjectivePrimaryState::Review)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Review),
         ObjectiveState::Primary(ObjectivePrimaryState::Integration)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Integration),
         ObjectiveState::Primary(ObjectivePrimaryState::Done)) => true,

        // ═══════════════════════════════════════════════════════════════════
        // Failure transitions — from each primary state to its corresponding
        // failure state
        // ═══════════════════════════════════════════════════════════════════
        (ObjectiveState::Primary(ObjectivePrimaryState::Discovered),
         ObjectiveState::Failure(ObjectiveFailureState::PlanningFailure)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Ready),
         ObjectiveState::Failure(ObjectiveFailureState::PermissionFailure)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Executing),
         ObjectiveState::Failure(ObjectiveFailureState::ExecutionFailure)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Executing),
         ObjectiveState::Failure(ObjectiveFailureState::PermissionFailure)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Review),
         ObjectiveState::Failure(ObjectiveFailureState::ReviewFailure)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Integration),
         ObjectiveState::Failure(ObjectiveFailureState::IntegrationFailure)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Integration),
         ObjectiveState::Failure(ObjectiveFailureState::HumanRejected)) => true,

        // Rollback from any post-EXECUTING primary state
        (ObjectiveState::Primary(ObjectivePrimaryState::Review),
         ObjectiveState::Failure(ObjectiveFailureState::Rollback)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Integration),
         ObjectiveState::Failure(ObjectiveFailureState::Rollback)) => true,

        (ObjectiveState::Primary(ObjectivePrimaryState::Executing),
         ObjectiveState::Failure(ObjectiveFailureState::Rollback)) => true,

        // ───────────────────────────────────────────────────────────────
        // Recovery from failure states
        // ───────────────────────────────────────────────────────────────

        // Retry: failure → READY (if retries remain)
        (ObjectiveState::Failure(_fail_state),
         ObjectiveState::Primary(ObjectivePrimaryState::Ready)) => {
            if retry_count >= retry_policy.max_retries {
                return Err(TransitionError::RetryLimitExhausted);
            }
            true
        }

        // Give up: failure → ABANDONED
        (ObjectiveState::Failure(_),
         ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned)) => true,

        // ═══════════════════════════════════════════════════════════════════
        // Everything else is illegal
        // ═══════════════════════════════════════════════════════════════════
        _ => false,
    };

    if allowed {
        Ok(target)
    } else {
        Err(TransitionError::IllegalTransition {
            from: current.label(),
            to: target.label(),
        })
    }
}

// ---------------------------------------------------------------------------
// The full transition graph for reference / debugging
// ---------------------------------------------------------------------------

/// Returns all legal transitions from a given state (for validation UIs, etc.).
pub fn legal_transitions(state: ObjectiveState) -> Vec<ObjectiveState> {
    let candidates = match state {
        ObjectiveState::Primary(ObjectivePrimaryState::Discovered) => {
            vec![
                ObjectiveState::Primary(ObjectivePrimaryState::Planned),
                ObjectiveState::Failure(ObjectiveFailureState::PlanningFailure),
            ]
        }
        ObjectiveState::Primary(ObjectivePrimaryState::Planned) => {
            vec![ObjectiveState::Primary(ObjectivePrimaryState::Ready)]
        }
        ObjectiveState::Primary(ObjectivePrimaryState::Ready) => {
            vec![
                ObjectiveState::Primary(ObjectivePrimaryState::Executing),
                ObjectiveState::Failure(ObjectiveFailureState::PermissionFailure),
            ]
        }
        ObjectiveState::Primary(ObjectivePrimaryState::Executing) => {
            vec![
                ObjectiveState::Primary(ObjectivePrimaryState::Review),
                ObjectiveState::Failure(ObjectiveFailureState::ExecutionFailure),
                ObjectiveState::Failure(ObjectiveFailureState::PermissionFailure),
                ObjectiveState::Failure(ObjectiveFailureState::Rollback),
            ]
        }
        ObjectiveState::Primary(ObjectivePrimaryState::Review) => {
            vec![
                ObjectiveState::Primary(ObjectivePrimaryState::Integration),
                ObjectiveState::Failure(ObjectiveFailureState::ReviewFailure),
                ObjectiveState::Failure(ObjectiveFailureState::Rollback),
            ]
        }
        ObjectiveState::Primary(ObjectivePrimaryState::Integration) => {
            vec![
                ObjectiveState::Primary(ObjectivePrimaryState::Done),
                ObjectiveState::Failure(ObjectiveFailureState::IntegrationFailure),
                ObjectiveState::Failure(ObjectiveFailureState::HumanRejected),
                ObjectiveState::Failure(ObjectiveFailureState::Rollback),
            ]
        }
        ObjectiveState::Primary(ObjectivePrimaryState::Done) => vec![],
        ObjectiveState::Failure(f) => match f {
            ObjectiveFailureState::Rollback => {
                vec![
                    ObjectiveState::Primary(ObjectivePrimaryState::Ready),
                    ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned),
                ]
            }
            _ => vec![
                ObjectiveState::Primary(ObjectivePrimaryState::Ready),
                ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned),
            ],
        },
        ObjectiveState::Terminal(_) => vec![],
    };

    // Filter with a generous retry policy so callers see all structural options
    let policy = RetryPolicy { max_retries: u32::MAX };
    candidates
        .into_iter()
        .filter(|t| transition(state, *t, &policy, 0).is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Happy-path ───────────────────────────────────────────────────────

    #[test]
    fn full_happy_path() {
        let policy = RetryPolicy::default();
        let states = [
            ObjectiveState::Primary(ObjectivePrimaryState::Discovered),
            ObjectiveState::Primary(ObjectivePrimaryState::Planned),
            ObjectiveState::Primary(ObjectivePrimaryState::Ready),
            ObjectiveState::Primary(ObjectivePrimaryState::Executing),
            ObjectiveState::Primary(ObjectivePrimaryState::Review),
            ObjectiveState::Primary(ObjectivePrimaryState::Integration),
            ObjectiveState::Primary(ObjectivePrimaryState::Done),
        ];

        let mut current = states[0];
        for (i, &next) in states.iter().enumerate().skip(1) {
            let result = transition(current, next, &policy, 0);
            assert!(result.is_ok(), "Step {i}: {} → {} should be allowed",
                    current.label(), next.label());
            current = result.unwrap();
        }
    }

    // ── Illegal backward transitions ─────────────────────────────────────

    #[test]
    fn cannot_go_backward() {
        let policy = RetryPolicy::default();
        let forward = vec![
            ObjectivePrimaryState::Discovered,
            ObjectivePrimaryState::Planned,
            ObjectivePrimaryState::Ready,
            ObjectivePrimaryState::Executing,
            ObjectivePrimaryState::Review,
            ObjectivePrimaryState::Integration,
            ObjectivePrimaryState::Done,
        ];

        for i in 1..forward.len() {
            let from = ObjectiveState::Primary(forward[i]);
            for j in 0..i {
                let to = ObjectiveState::Primary(forward[j]);
                let result = transition(from, to, &policy, 0);
                assert!(result.is_err(), "Should not be able to go from {} to {}",
                    from.label(), to.label());
            }
        }
    }

    // ── Failure + retry cycle ────────────────────────────────────────────

    #[test]
    fn failure_to_ready_retry() {
        let policy = RetryPolicy { max_retries: 3 };
        let failed = ObjectiveState::Failure(ObjectiveFailureState::ReviewFailure);
        let ready = ObjectiveState::Primary(ObjectivePrimaryState::Ready);
        let result = transition(failed, ready, &policy, 1);
        assert!(result.is_ok(), "Retry (1 of 3) should be allowed");
    }

    #[test]
    fn retry_limit_exhausted() {
        let policy = RetryPolicy { max_retries: 2 };
        let failed = ObjectiveState::Failure(ObjectiveFailureState::ReviewFailure);
        let ready = ObjectiveState::Primary(ObjectivePrimaryState::Ready);
        let result = transition(failed, ready, &policy, 2);
        assert_eq!(result, Err(TransitionError::RetryLimitExhausted));
    }

    #[test]
    fn failure_to_abandoned() {
        let policy = RetryPolicy::default();
        let failed = ObjectiveState::Failure(ObjectiveFailureState::ReviewFailure);
        let abandoned = ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned);
        let result = transition(failed, abandoned, &policy, 0);
        assert!(result.is_ok());
        assert!(result.unwrap().is_terminal());
    }

    // ── Terminal state is frozen ─────────────────────────────────────────

    #[test]
    fn terminal_state_cannot_transition() {
        let policy = RetryPolicy::default();
        let done = ObjectiveState::Terminal(ObjectiveTerminalState::Done);
        let any = ObjectiveState::Primary(ObjectivePrimaryState::Discovered);
        let result = transition(done, any, &policy, 0);
        assert!(result.is_err());
    }

    #[test]
    fn abandoned_is_terminal() {
        let _policy = RetryPolicy::default();
        let abandoned = ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned);
        assert!(abandoned.is_terminal());
    }

    // ── Rollback resolution ──────────────────────────────────────────────

    #[test]
    fn rollback_to_ready() {
        let policy = RetryPolicy { max_retries: 3 };
        let rollback = ObjectiveState::Failure(ObjectiveFailureState::Rollback);
        let ready = ObjectiveState::Primary(ObjectivePrimaryState::Ready);
        let result = transition(rollback, ready, &policy, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn rollback_exhausted_to_abandoned() {
        let policy = RetryPolicy { max_retries: 1 };
        let rollback = ObjectiveState::Failure(ObjectiveFailureState::Rollback);
        let ready = ObjectiveState::Primary(ObjectivePrimaryState::Ready);
        // After max_retries, rollback retries are exhausted
        let result = transition(rollback, ready, &policy, 1);
        assert_eq!(result, Err(TransitionError::RetryLimitExhausted));

        // Can always go to abandoned
        let abandoned = ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned);
        let result2 = transition(rollback, abandoned, &policy, 1);
        assert!(result2.is_ok());
    }

    // ── Edge cases ───────────────────────────────────────────────────────

    #[test]
    fn cannot_skip_states() {
        let policy = RetryPolicy::default();
        // Trying to go from DISCOVERED straight to READY
        let from = ObjectiveState::Primary(ObjectivePrimaryState::Discovered);
        let to = ObjectiveState::Primary(ObjectivePrimaryState::Ready);
        let result = transition(from, to, &policy, 0);
        assert!(result.is_err(), "Should not be able to skip PLANNED");
    }

    #[test]
    fn human_rejected_only_from_integration() {
        let policy = RetryPolicy::default();
        let rejected = ObjectiveState::Failure(ObjectiveFailureState::HumanRejected);

        // Allowed from INTEGRATION
        let int = ObjectiveState::Primary(ObjectivePrimaryState::Integration);
        assert!(transition(int, rejected, &policy, 0).is_ok());

        // Not allowed from REVIEW
        let rev = ObjectiveState::Primary(ObjectivePrimaryState::Review);
        assert!(transition(rev, rejected, &policy, 0).is_err());
    }

    #[test]
    fn legal_transitions_from_each_state() {
        let states = vec![
            ObjectiveState::Primary(ObjectivePrimaryState::Discovered),
            ObjectiveState::Primary(ObjectivePrimaryState::Planned),
            ObjectiveState::Primary(ObjectivePrimaryState::Ready),
            ObjectiveState::Primary(ObjectivePrimaryState::Executing),
            ObjectiveState::Primary(ObjectivePrimaryState::Review),
            ObjectiveState::Primary(ObjectivePrimaryState::Integration),
            ObjectiveState::Primary(ObjectivePrimaryState::Done),
            ObjectiveState::Failure(ObjectiveFailureState::ReviewFailure),
            ObjectiveState::Failure(ObjectiveFailureState::Rollback),
            ObjectiveState::Terminal(ObjectiveTerminalState::Done),
            ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned),
        ];

        for s in states {
            let transitions = legal_transitions(s);
            // Terminal states should have no legal transitions
            if s.is_terminal() {
                assert!(transitions.is_empty(),
                    "Terminal state {} should have no legal transitions, got {:?}",
                    s.label(), transitions);
            }
        }
    }
}
