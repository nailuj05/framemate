//! Toasts in the headset: `/api/notify` hands requests to the CDP session (cdp.rs)
//! calls `window.__framemate.notify` inside Steam (shim.js) reports back what Steam did

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

/// How long a request waits for Steam
pub const DEADLINE: Duration = Duration::from_secs(4);
/// Token bucket: a burst of this many toasts, then one every `REFILL`.
const BURST: f64 = 5.0;
const REFILL: Duration = Duration::from_secs(2);
const QUEUE: usize = 16;
/// Lengths the shim also enforces
pub const MAX_TITLE: usize = 120;
pub const MAX_BODY: usize = 500;
/// A 96 px app icon as a base64 PNG is ~15 KB
pub const MAX_ICON: usize = 64 * 1024;

#[derive(Deserialize)]
pub struct Toast {
    /// Name of the app the notification came from; the title when the notification has none.
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// `data:image/png;base64,...`; anything else is dropped by the shim.
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case", tag = "result")]
pub enum Outcome {
    Shown,
    // No notif shown
    HeadsetAsleep,
}

pub struct Request {
    /// JSON argument for `__framemate.notify`.
    pub args: serde_json::Value,
    pub deadline: Instant,
    pub reply: oneshot::Sender<Result<Outcome, String>>,
}

pub enum Error {
    Invalid(&'static str),
    RateLimited,
    /// Steam (or the toast hook) isn't available
    Unavailable(String),
}

pub struct Notifier {
    tx: mpsc::Sender<Request>,
    bucket: Mutex<(f64, Instant)>,
}

impl Notifier {
    /// The receiver goes to the CDP task -> owns the Steam connection
    pub fn new() -> (Self, mpsc::Receiver<Request>) {
        let (tx, rx) = mpsc::channel(QUEUE);
        (Self { tx, bucket: Mutex::new((BURST, Instant::now())) }, rx)
    }

    pub async fn send(&self, toast: Toast) -> Result<Outcome, Error> {
        let args = arguments(toast)?;
        if !self.take_token() {
            return Err(Error::RateLimited);
        }
        let (reply, answer) = oneshot::channel();
        let deadline = Instant::now() + DEADLINE;
        self.tx
            .try_send(Request { args, deadline, reply })
            .map_err(|_| Error::Unavailable("too many toasts waiting for Steam".into()))?;
        match tokio::time::timeout_at(deadline.into(), answer).await {
            Ok(Ok(result)) => result.map_err(Error::Unavailable),
            Ok(Err(_)) => Err(Error::Unavailable("the connection to Steam dropped".into())),
            Err(_) => Err(Error::Unavailable("Steam didn't answer in time".into())),
        }
    }

    fn take_token(&self) -> bool {
        let mut guard = self.bucket.lock().unwrap_or_else(|e| e.into_inner());
        let (tokens, last) = &mut *guard;
        let now = Instant::now();
        *tokens = (*tokens + now.duration_since(*last).as_secs_f64() / REFILL.as_secs_f64()).min(BURST);
        *last = now;
        if *tokens < 1.0 {
            return false;
        }
        *tokens -= 1.0;
        true
    }
}

fn arguments(toast: Toast) -> Result<serde_json::Value, Error> {
    let cut = |s: &str, max: usize| s.trim().chars().take(max).collect::<String>();
    let (app, title, body) = (cut(&toast.app, MAX_TITLE), cut(&toast.title, MAX_TITLE), cut(&toast.body, MAX_BODY));
    let title = if title.is_empty() { app } else { title };
    if title.is_empty() && body.is_empty() {
        return Err(Error::Invalid("a toast needs a title or a body"));
    }
    let icon = toast.icon.filter(|icon| icon.len() <= MAX_ICON && icon.starts_with("data:image/"));
    Ok(serde_json::json!({ "title": title, "body": body, "icon": icon }))
}

/// What the shim returned: `{id, suppressed}` or `{unavailable: reason}`.
pub fn outcome(value: &serde_json::Value) -> Result<Outcome, String> {
    if let Some(reason) = value["unavailable"].as_str() {
        return Err(reason.to_owned());
    }
    match value["suppressed"].as_bool() {
        Some(true) => Ok(Outcome::HeadsetAsleep),
        Some(false) => Ok(Outcome::Shown),
        None => Err("unexpected answer from the toast hook".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toast(app: &str, title: &str, body: &str) -> Toast {
        Toast { app: app.into(), title: title.into(), body: body.into(), icon: None }
    }

    #[test]
    fn falls_back_to_the_app_name_and_cuts_long_fields() {
        let args = arguments(toast("Signal", "", "hi")).ok().unwrap();
        assert_eq!(args["title"], "Signal");
        let args = arguments(toast("Signal", "Alex", &"x".repeat(2000))).ok().unwrap();
        assert_eq!(args["title"], "Alex");
        assert_eq!(args["body"].as_str().unwrap().chars().count(), MAX_BODY);
        assert!(matches!(arguments(toast(" ", "", "  ")), Err(Error::Invalid(_))));
    }

    #[test]
    fn drops_icons_that_are_not_inline_images() {
        let mut t = toast("", "t", "");
        t.icon = Some("https://example.com/x.png".into());
        assert!(arguments(t).ok().unwrap()["icon"].is_null());
        let mut t = toast("", "t", "");
        t.icon = Some(format!("data:image/png;base64,{}", "A".repeat(MAX_ICON)));
        assert!(arguments(t).ok().unwrap()["icon"].is_null());
    }

    #[test]
    fn reads_the_shims_answer() {
        assert_eq!(outcome(&serde_json::json!({"id": 1, "suppressed": false})), Ok(Outcome::Shown));
        assert_eq!(outcome(&serde_json::json!({"id": 1, "suppressed": true})), Ok(Outcome::HeadsetAsleep));
        assert!(outcome(&serde_json::json!({"unavailable": "no hook"})).is_err());
    }

    #[test]
    fn rate_limits_after_a_burst() {
        let (notifier, _rx) = Notifier::new();
        let allowed = (0..10).filter(|_| notifier.take_token()).count();
        assert_eq!(allowed, BURST as usize);
    }
}
