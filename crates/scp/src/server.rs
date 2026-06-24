use tokio::sync::{mpsc, oneshot};

pub struct CommandRequest {
    pub source: String,
    pub reply: oneshot::Sender<CommandRequest>
}

/// Spawn tokio runtime + axum server on a dedicated thread.
/// Returns the receiver the game thread drains, and the join handle
pub fn spawn()
