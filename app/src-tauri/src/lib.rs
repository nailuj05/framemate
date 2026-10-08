pub mod pairing;
pub mod proxy;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Manager, State};

use pairing::Pairing;

/// connection from frontend -> proxy -> agent, secret is for proxy so no other android app can connect
#[derive(Serialize)]
struct Connection {
    port: u16,
    secret: String,
    token: String,
    paired: bool,
}

struct App {
    current: proxy::Current,
    status: Arc<proxy::Status>,
    proxy: Mutex<Option<proxy::Proxy>>,
    store: PathBuf,
}

/// Takes a lock, ignoring poisoning: a panicked relay task must not leave the app permanently
/// unable to report its own connection details. Everything guarded here is plain data.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl App {
    fn connection(&self) -> Option<Connection> {
        let proxy = lock(&self.proxy);
        let proxy = proxy.as_ref()?;
        let current = lock(&self.current);
        Some(Connection {
            port: proxy.port,
            secret: proxy.secret.clone(),
            token: current.as_ref().map(|t| t.token()).unwrap_or_default(),
            paired: current.is_some(),
        })
    }
}

#[tauri::command]
fn connection(app: State<'_, App>) -> Option<Connection> {
    app.connection()
}

/// Accept scanned pairing payload, or one typed in by hand.
#[tauri::command]
fn pair(payload: String, app: State<'_, App>) -> Result<Connection, String> {
    let pairing = Pairing::parse(payload.trim())?;
    // Built before it is stored, so a pairing that can't be used never reaches the disk.
    let target = proxy::Target::new(pairing.clone())?;
    pairing::store(&app.store, &pairing)?;
    *lock(&app.current) = Some(target);
    app.connection().ok_or_else(|| "the local proxy isn't running".to_owned())
}

/// A failure the user has to act on
#[tauri::command]
fn last_error(app: State<'_, App>) -> Option<String> {
    app.status.take()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());
    builder
        .invoke_handler(tauri::generate_handler![connection, pair, last_error])
        .setup(|app| {
            let store = app.path().app_config_dir()?.join("pairing.json");
            let current: proxy::Current = Arc::new(Mutex::new(
                pairing::load(&store).and_then(|p| proxy::Target::new(p).ok()),
            ));
            let status = Arc::new(proxy::Status::default());
            let state = App {
                current: current.clone(),
                status: status.clone(),
                proxy: Mutex::new(None),
                store,
            };
            app.manage(state);

            // Bound before the WebView can ask for the port. Only the bind is awaited; the accept loop runs on its own task.
            match tauri::async_runtime::block_on(proxy::spawn(current, status)) {
                Ok(proxy) => *lock(&app.state::<App>().proxy) = Some(proxy),
                Err(e) => eprintln!("framemate: local proxy failed to start: {e}"),
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
