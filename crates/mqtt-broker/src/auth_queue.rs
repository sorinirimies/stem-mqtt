//! Pull-style authentication, for runtimes that can't implement
//! [`MqttAuthProvider`](crate::config::MqttAuthProvider) (Dart, Haskell):
//! foreign callbacks can't be invoked from Rust's own threads there, and the
//! provider's synchronous `bool` return can't be modelled by a posted event.
//!
//! Instead each CONNECT becomes an [`AuthRequest`] the application *pulls*
//! with [`MqttBroker::next_auth_request`](crate::MqttBroker::next_auth_request)
//! and answers asynchronously with
//! [`resolve_auth_request`](crate::MqttBroker::resolve_auth_request). The
//! connection task just awaits a oneshot — no thread is blocked — and the
//! whole exchange **fails closed**: no answer in [`AUTH_QUEUE_TIMEOUT`], a
//! full queue that drops the request, or disabling the queue all deny.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use mqtt_client::support::{EventQueue, LockExt};
use tokio::sync::oneshot;

/// How long a connection waits for the application to answer.
pub const AUTH_QUEUE_TIMEOUT: Duration = Duration::from_secs(10);

/// One CONNECT awaiting an allow/deny decision.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct AuthRequest {
    /// Pass this to `resolve_auth_request`.
    pub id: u64,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<Vec<u8>>,
}

impl std::fmt::Debug for AuthRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthRequest")
            .field("id", &self.id)
            .field("client_id", &self.client_id)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Queue of pending [`AuthRequest`]s and the channels that answer them.
#[derive(Default)]
pub struct AuthQueue {
    requests: EventQueue<AuthRequest>,
    pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    next_id: AtomicU64,
}

impl AuthQueue {
    /// Start queueing (at most `capacity` unanswered requests are kept; the
    /// oldest is dropped — and so times out, i.e. is denied — beyond that).
    /// `0` disables the queue and denies everything still waiting.
    pub fn enable(&self, capacity: usize) {
        self.requests.enable(capacity);
        if capacity == 0 {
            self.pending.lock_safe().clear(); // dropped senders => denied
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.requests.is_enabled()
    }

    /// Ask the application whether to admit this client. `false` on denial,
    /// timeout, or a disabled queue.
    pub async fn ask(
        &self,
        client_id: String,
        username: Option<String>,
        password: Option<Vec<u8>>,
        timeout: Duration,
    ) -> bool {
        if !self.is_enabled() {
            return false;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock_safe().insert(id, tx);
        self.requests.push(AuthRequest {
            id,
            client_id,
            username,
            password,
        });
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(allow)) => allow,
            _ => {
                self.pending.lock_safe().remove(&id);
                false
            }
        }
    }

    /// The next unanswered request, waiting up to `timeout`.
    pub async fn next(&self, timeout: Duration) -> Option<AuthRequest> {
        self.requests.next(timeout).await
    }

    /// Answer request `id`. `false` if it is unknown or already expired.
    pub fn resolve(&self, id: u64, allow: bool) -> bool {
        let tx = self.pending.lock_safe().remove(&id);
        tx.is_some_and(|tx| tx.send(allow).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn queue() -> Arc<AuthQueue> {
        let q = Arc::new(AuthQueue::default());
        q.enable(8);
        q
    }

    async fn ask(q: &Arc<AuthQueue>, timeout: Duration) -> bool {
        q.ask("c".into(), Some("u".into()), Some(b"p".to_vec()), timeout)
            .await
    }

    #[tokio::test]
    async fn answer_allows_or_denies() {
        for verdict in [true, false] {
            let q = queue();
            let asker = tokio::spawn({
                let q = q.clone();
                async move { ask(&q, Duration::from_secs(5)).await }
            });
            let req = q.next(Duration::from_secs(5)).await.expect("request");
            assert_eq!(req.client_id, "c");
            assert_eq!(req.username.as_deref(), Some("u"));
            assert_eq!(req.password.as_deref(), Some(&b"p"[..]));
            assert!(q.resolve(req.id, verdict));
            assert_eq!(asker.await.unwrap(), verdict);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn unanswered_request_times_out_denied_and_is_forgotten() {
        let q = queue();
        assert!(!ask(&q, AUTH_QUEUE_TIMEOUT).await);
        let req = q
            .next(Duration::from_millis(1))
            .await
            .expect("it was queued");
        assert!(
            !q.resolve(req.id, true),
            "a late answer must not resurrect it"
        );
    }

    #[tokio::test]
    async fn disabled_queue_denies_immediately() {
        let q = Arc::new(AuthQueue::default());
        assert!(!ask(&q, Duration::from_secs(5)).await);
    }

    #[tokio::test]
    async fn disabling_denies_everything_waiting() {
        let q = queue();
        let asker = tokio::spawn({
            let q = q.clone();
            async move { ask(&q, Duration::from_secs(60)).await }
        });
        q.next(Duration::from_secs(5)).await.expect("request");
        q.enable(0);
        assert!(!asker.await.unwrap());
    }

    #[test]
    fn unknown_id_does_not_resolve() {
        assert!(!queue().resolve(42, true));
    }

    #[test]
    fn debug_never_prints_the_password() {
        let req = AuthRequest {
            id: 1,
            client_id: "c".into(),
            username: Some("u".into()),
            password: Some(b"hunter2".to_vec()),
        };
        let text = format!("{req:?}");
        assert!(!text.contains("hunter2") && !text.contains("104"), "{text}");
    }
}
