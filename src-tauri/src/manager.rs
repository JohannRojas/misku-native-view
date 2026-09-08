use crate::{
    installation,
    model::{self, AppProfile, CreateRequest, UpdateRequest},
    platform::InstanceGuard,
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[derive(Clone)]
struct ManagerState {
    registry: PathBuf,
    runtime: Arc<Mutex<Option<PathBuf>>>,
}

impl ManagerState {
    fn runtime(&self) -> Result<PathBuf, String> {
        let mut cached = self.runtime.lock().map_err(|e| e.to_string())?;
        if let Some(path) = &*cached {
            return Ok(path.clone());
        }
        let path = installation::ensure_runtime()?;
        *cached = Some(path.clone());
        Ok(path)
    }
}

#[derive(Serialize)]
struct Snapshot {
    apps: Vec<AppProfile>,
    version: &'static str,
}

#[derive(Serialize)]
struct SavedApp {
    profile: AppProfile,
    warning: Option<String>,
}

fn complete_save(state: &ManagerState, profile: AppProfile, open_after: bool) -> SavedApp {
    // Persisting the profile is the transaction. Report optional integration failures
    // separately so retrying a failed shortcut never creates a second app.
    let result = (|| {
        let runtime = state.runtime()?;
        let installed = installation::materialize(&state.registry, &profile, &runtime)?;
        if open_after {
            installation::open(&installed.config, &profile, &runtime)?;
        }
        Ok::<_, String>(())
    })();
    SavedApp {
        profile,
        warning: result
            .err()
            .map(|e| format!("La app se guardó. Pulsa Abrir para reintentar su instalación: {e}")),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppInput {
    url: String,
    name: String,
    #[serde(default)]
    allowed_origins: Vec<String>,
    #[serde(default)]
    suspend_when_minimized: bool,
    #[serde(default)]
    allow_insecure_http: bool,
}

fn authorize(window: &WebviewWindow, state: &ManagerState) -> Result<ManagerState, String> {
    let url = window.url().map_err(|e| e.to_string())?;
    if window.label() != "manager" || !is_manager_url(&url) {
        return Err("Esta operación solo está disponible en el gestor de Misku".into());
    }
    Ok(state.clone())
}

fn is_manager_url(url: &url::Url) -> bool {
    (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https")
            && url.host_str() == Some("tauri.localhost")
            && url.port().is_none())
}

#[tauri::command]
async fn manager_snapshot(
    window: WebviewWindow,
    state: tauri::State<'_, ManagerState>,
) -> Result<Snapshot, String> {
    let state = authorize(&window, &state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let config = model::load_config_for_write(&state.registry)?;
        Ok(Snapshot {
            apps: config.apps,
            version: env!("CARGO_PKG_VERSION"),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn manager_create(
    window: WebviewWindow,
    state: tauri::State<'_, ManagerState>,
    input: AppInput,
    open_after: bool,
) -> Result<SavedApp, String> {
    let state = authorize(&window, &state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let profile = {
            let _lock = model::acquire_config_lock(&state.registry)?;
            let mut config = model::load_config_for_write(&state.registry)?;
            let created = model::create_profiles(
                &mut config,
                &state.registry,
                CreateRequest {
                    urls: vec![input.url],
                    id: None,
                    name: if input.name.trim().is_empty() {
                        None
                    } else {
                        Some(input.name.trim().into())
                    },
                    icon: None,
                    allowed_origins: input.allowed_origins,
                    allow_insecure_http: input.allow_insecure_http,
                },
            )?;
            let mut profile = created[0].clone();
            profile.suspend_when_minimized = input.suspend_when_minimized;
            if let Some(last) = config.apps.last_mut() {
                *last = profile.clone();
            }
            model::save_config_atomic(&state.registry, &config)?;
            profile
        };
        Ok(complete_save(&state, profile, open_after))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn manager_update(
    window: WebviewWindow,
    state: tauri::State<'_, ManagerState>,
    id: String,
    input: AppInput,
) -> Result<SavedApp, String> {
    let state = authorize(&window, &state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let profile = {
            let _lock = model::acquire_config_lock(&state.registry)?;
            let mut config = model::load_config(&state.registry)?;
            let profile = model::update_profile(
                &mut config,
                &state.registry,
                UpdateRequest {
                    selector: id,
                    name: Some(input.name.trim().into()),
                    url: Some(input.url),
                    allowed_origins: Some(input.allowed_origins),
                    allow_insecure_http: Some(input.allow_insecure_http),
                    suspend_when_minimized: Some(input.suspend_when_minimized),
                    ..Default::default()
                },
            )?;
            model::save_config_atomic(&state.registry, &config)?;
            profile
        };
        Ok(complete_save(&state, profile, false))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn manager_open(
    window: WebviewWindow,
    state: tauri::State<'_, ManagerState>,
    id: String,
) -> Result<(), String> {
    let state = authorize(&window, &state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let profile = model::load_profile_for_open(&state.registry, Some(&id))?;
        let runtime = state.runtime()?;
        let installed = installation::ensure_installed(&state.registry, &profile, &runtime)?;
        installation::open(&installed.config, &profile, &runtime)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn manager_remove(
    window: WebviewWindow,
    state: tauri::State<'_, ManagerState>,
    id: String,
) -> Result<(), String> {
    let state = authorize(&window, &state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = model::acquire_config_lock(&state.registry)?;
        let mut config = model::load_config(&state.registry)?;
        let removed = model::remove_profile(&mut config, &id)?;
        installation::remove_shortcut(&removed)?;
        model::save_config_atomic(&state.registry, &config)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) fn launch(explicit: Option<PathBuf>) -> Result<(), String> {
    let registry = match explicit.or_else(|| std::env::var_os("MISKU_NV_CONFIG").map(PathBuf::from))
    {
        Some(path) => crate::absolute_path(&path)?,
        None => installation::default_registry()?,
    };
    let identity = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        registry.to_string_lossy().as_bytes(),
    );
    let Some(mut instance) = InstanceGuard::acquire(&format!("manager-{identity}"))? else {
        return Ok(());
    };
    let active = Arc::new(Mutex::new(None::<WebviewWindow>));
    let activation = active.clone();
    instance.listen(move || {
        if let Ok(current) = activation.lock()
            && let Some(window) = &*current
        {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    });
    tauri::Builder::default()
        .manage(ManagerState {
            registry,
            runtime: Default::default(),
        })
        .invoke_handler(tauri::generate_handler![
            manager_snapshot,
            manager_create,
            manager_update,
            manager_open,
            manager_remove
        ])
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_filename(".manager-window.json")
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .setup(move |app| {
            let window =
                WebviewWindowBuilder::new(app, "manager", WebviewUrl::App("index.html".into()))
                    .title("Misku Native Views")
                    .data_directory(installation::managed_home()?.join("manager-webview"))
                    .inner_size(920., 690.)
                    .min_inner_size(440., 480.)
                    .center()
                    .prevent_overflow()
                    .on_navigation(is_manager_url)
                    .build()?;
            *active.lock().map_err(|e| e.to_string())? = Some(window);
            Ok(())
        })
        .run(tauri::generate_context!())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manager_never_navigates_to_remote_or_lookalike_origins() {
        assert!(is_manager_url(
            &url::Url::parse("http://tauri.localhost/index.html").unwrap()
        ));
        assert!(!is_manager_url(
            &url::Url::parse("https://example.com").unwrap()
        ));
        assert!(!is_manager_url(
            &url::Url::parse("https://tauri.localhost.example.com").unwrap()
        ));
    }
}
