//! Phone notifications → toast in the headset (`POST /api/notify` on the agent).
//!
//! Called by the Kotlin `NotificationListenerService` through JNI. Android can start that service
//! without the app's window, so Tauri, the WebView and the loopback proxy may not exist: this
//! reads the pairing straight from disk and talks pinned TLS itself, reusing proxy.rs.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use crate::pairing::{self, Pairing};
use crate::proxy::Target;

/// The last target, kept so the address that worked (mDNS name or IP) is tried first next time.
static TARGET: Mutex<Option<(Pairing, std::sync::Arc<Target>)>> = Mutex::new(None);

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Sends one toast. `config_dir` holds `pairing.json`, returns a short status
pub fn send(config_dir: &Path, json: &str) -> String {
    let Some(pairing) = pairing::load(&config_dir.join("pairing.json")) else {
        return "not_paired".into();
    };
    let target = {
        let mut cached = TARGET.lock().unwrap_or_else(|e| e.into_inner());
        match cached.as_ref() {
            Some((p, target)) if *p == pairing => target.clone(),
            _ => match Target::new(pairing.clone()) {
                Ok(target) => {
                    let target = std::sync::Arc::new(target);
                    *cached = Some((pairing, target.clone()));
                    target
                }
                Err(e) => return format!("error: {e}"),
            },
        }
    };
    match runtime().block_on(target.post_json("/api/notify", json)) {
        Ok((200, body)) => serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["result"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "error: unexpected answer from the Frame".into()),
        Ok((401, _)) => "error: the Frame rejected the token, pair again".into(),
        Ok((429, _)) => "rate_limited".into(),
        // Agents before v0.3.0 have no /api/notify.
        Ok((404, _)) => "error: the FrameMate agent on the Frame is too old for notifications, update it".into(),
        Ok((status, body)) => format!("error: {status} {body}"),
        Err(e) => format!("error: {e}"),
    }
}

#[cfg(target_os = "android")]
mod jni_glue {
    use jni::JNIEnv;
    use jni::objects::{JClass, JString};
    use jni::sys::jstring;

    /// `dev.framemate.app.NotificationBridge.send(dataDir, json)`; blocking
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_framemate_app_NotificationBridge_send<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
        data_dir: JString<'local>,
        json: JString<'local>,
    ) -> jstring {
        let result = match (env.get_string(&data_dir), env.get_string(&json)) {
            (Ok(dir), Ok(json)) => {
                let (dir, json): (String, String) = (dir.into(), json.into());
                super::send(std::path::Path::new(&dir), &json)
            }
            _ => "error: bad arguments".into(),
        };
        env.new_string(result).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_a_missing_pairing() {
        let dir = std::env::temp_dir().join("framemate-forward-test-empty");
        let _ = std::fs::create_dir_all(&dir);
        assert_eq!(super::send(&dir, "{}"), "not_paired");
    }
}
