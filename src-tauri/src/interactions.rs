use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

pub enum RequestKind {
    Tool,
    Question,
}

pub struct RequestGuard {
    app: tauri::AppHandle,
    id: String,
    kind: RequestKind,
    settled: bool,
}
impl RequestGuard {
    pub fn new(app: &tauri::AppHandle, id: &str, kind: RequestKind) -> Self {
        Self {
            app: app.clone(),
            id: id.into(),
            kind,
            settled: false,
        }
    }
    pub fn settle(&mut self) {
        self.settled = true;
    }
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let app = self.app.clone();
        let id = self.id.clone();
        let question = matches!(self.kind, RequestKind::Question);
        // The enclosing turn can be cancelled by dropping its future. Clean
        // up and notify windows even when the waiter's normal tail never runs.
        tauri::async_runtime::spawn(async move {
            let state = app.state::<crate::AppState>();
            if question {
                state.pending_questions.lock().await.remove(&id);
                let _ = app.emit(
                    "ai-ask-user-resolved",
                    serde_json::json!({ "id": id, "answered": false }),
                );
            } else {
                state.pending_confirms.lock().await.remove(&id);
                let _ = app.emit(
                    "ai-tool-confirm-resolved",
                    serde_json::json!({ "id": id, "approved": false }),
                );
            }
        });
    }
}

pub struct PendingRequest<T, P> {
    pub sender: oneshot::Sender<T>,
    pub payload: P,
}

#[derive(Debug, PartialEq)]
pub enum WaitResult<T> {
    Answered(T),
    Cancelled,
    TimedOut,
    Closed,
}

pub async fn wait_for_response<T>(
    receiver: oneshot::Receiver<T>,
    cancel: &CancellationToken,
    timeout: Duration,
) -> WaitResult<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => WaitResult::Cancelled,
        result = receiver => match result {
            Ok(answer) => WaitResult::Answered(answer),
            Err(_) => WaitResult::Closed,
        },
        _ = tokio::time::sleep(timeout) => WaitResult::TimedOut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn uncancelled_question_waits_for_the_actual_user_answer() {
        let cancel = CancellationToken::new();
        let (sender, receiver) = oneshot::channel();
        let mut wait = Box::pin(wait_for_response(
            receiver,
            &cancel,
            Duration::from_secs(300),
        ));
        assert!(tokio::time::timeout(Duration::from_millis(25), &mut wait)
            .await
            .is_err());
        sender.send("中文回答").unwrap();
        assert_eq!(wait.await, WaitResult::Answered("中文回答"));
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_live_wait_and_is_not_a_timeout() {
        let cancel = CancellationToken::new();
        let (_sender, receiver) = oneshot::channel::<String>();
        cancel.cancel();
        assert_eq!(
            wait_for_response(receiver, &cancel, Duration::from_secs(300)).await,
            WaitResult::Cancelled
        );
    }

    #[tokio::test]
    async fn timeout_channel_closure_and_explicit_skip_are_distinct() {
        let cancel = CancellationToken::new();
        let (_sender, receiver) = oneshot::channel::<String>();
        assert_eq!(
            wait_for_response(receiver, &cancel, Duration::from_millis(25)).await,
            WaitResult::TimedOut
        );
        let (sender, receiver) = oneshot::channel::<String>();
        drop(sender);
        assert_eq!(
            wait_for_response(receiver, &cancel, Duration::from_secs(300)).await,
            WaitResult::Closed
        );
        let (sender, receiver) = oneshot::channel::<String>();
        sender.send(String::new()).unwrap();
        assert_eq!(
            wait_for_response(receiver, &cancel, Duration::from_secs(300)).await,
            WaitResult::Answered(String::new())
        );
    }

    #[tokio::test]
    async fn cancellation_wins_over_a_queued_approval() {
        let cancel = CancellationToken::new();
        let (sender, receiver) = oneshot::channel();
        sender.send(true).unwrap();
        cancel.cancel();
        assert_eq!(
            wait_for_response(receiver, &cancel, Duration::from_secs(120)).await,
            WaitResult::Cancelled
        );
    }
}
