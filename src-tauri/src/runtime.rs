use crate::model::{AppProfile, allowed_origin_set, resolve_icon_path};
use std::{
    collections::HashSet,
    error::Error,
    fs, io,
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};
use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    image::Image,
    menu::{Menu, MenuItem, Submenu},
    webview::{NewWindowFeatures, NewWindowResponse},
};
use tauri_plugin_window_state::StateFlags;
use url::{Host, Url};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NavigationDecision {
    Allow,
    OpenExternal,
    Deny,
}

pub(crate) fn launch(profile: AppProfile, config_path: &Path) -> Result<(), String> {
    let started = Instant::now();
    let Some(mut instance) =
        crate::platform::InstanceGuard::acquire(&profile.instance_id().to_string())?
    else {
        return Ok(());
    };
    let current = Arc::new(Mutex::new(None::<tauri::WebviewWindow>));
    let activation = current.clone();
    instance.listen(move || {
        if let Ok(current) = activation.lock()
            && let Some(window) = &*current
        {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    });
    set_process_app_id(profile.instance_id())?;

    let state_file = format!(".window-state-{}.json", profile.instance_id());
    let profile_for_setup = profile.clone();
    let config_for_setup = config_path.to_path_buf();

    tauri::Builder::default()
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_filename(state_file)
                .with_state_flags(
                    StateFlags::SIZE
                        | StateFlags::POSITION
                        | StateFlags::MAXIMIZED
                        | StateFlags::FULLSCREEN,
                )
                .build(),
        )
        .setup(move |app| {
            let window = open_profile_window(app, &profile_for_setup, &config_for_setup)?;
            *current.lock().map_err(|e| e.to_string())? = Some(window);
            trace_timing("window-created", started);
            Ok(())
        })
        .run(tauri::generate_context!())
        .map_err(|error| error.to_string())
}

fn open_profile_window(
    app: &mut tauri::App,
    profile: &AppProfile,
    config_path: &Path,
) -> Result<tauri::WebviewWindow, Box<dyn Error>> {
    let url = Url::parse(&profile.url)?;
    let allowed_origins =
        allowed_origin_set(profile).map_err(|error| io::Error::other(error.to_string()))?;
    let allowed_for_navigation = allowed_origins.clone();
    let allow_insecure_http = profile.allow_insecure_http;
    let allow_http_for_new_windows = profile.allow_insecure_http;

    let profile_dir = if profile.isolated_profile {
        let root = if profile.uses_legacy_data_root() {
            app.path().app_data_dir()?
        } else {
            app.path().app_local_data_dir()?
        };
        let root = std::env::var_os("MISKU_NV_PROFILE_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("profiles"));
        let directory = root.join(profile.profile_key());
        fs::create_dir_all(&directory)?;
        Some(directory)
    } else {
        None
    };

    let label = format!("app-{}", profile.instance_id());
    let menu = app_menu(app)?;
    let home = url.clone();
    let menu_label = label.clone();
    let page_started = Instant::now();
    let title = profile.name.clone();
    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title(&profile.name)
        .inner_size(profile.width, profile.height)
        .min_inner_size(profile.min_width, profile.min_height)
        .resizable(profile.resizable)
        .zoom_hotkeys_enabled(profile.zoom_hotkeys_enabled)
        .devtools(profile.devtools)
        .center()
        .prevent_overflow()
        .menu(menu)
        .on_menu_event(move |window, event| {
            let Some(view) = window.app_handle().get_webview_window(&menu_label) else {
                return;
            };
            let result = match event.id().as_ref() {
                "back" => view.eval("history.back()"),
                "forward" => view.eval("history.forward()"),
                "reload" => view.reload(),
                "home" => view.navigate(home.clone()),
                "browser" => {
                    if let Ok(url) = view.url()
                        && can_open_externally(&url, allow_insecure_http)
                    {
                        let _ = open_external_url(&url);
                    }
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                crate::platform::show_error("No se pudo completar la acción", &error.to_string());
            }
        })
        .on_page_load(move |window, payload| {
            let loading = payload.event() == tauri::webview::PageLoadEvent::Started;
            let _ = window.set_title(&if loading {
                format!("Cargando… · {title}")
            } else {
                title.clone()
            });
            if !loading {
                trace_timing("navigation-finished", page_started);
            }
        })
        .on_navigation(move |target| {
            match navigation_decision(target, &allowed_for_navigation, allow_insecure_http) {
                NavigationDecision::Allow => true,
                NavigationDecision::OpenExternal => {
                    if let Err(error) = open_external_url(target) {
                        eprintln!("misku-native-views: no se pudo abrir {target}: {error}");
                    }
                    false
                }
                NavigationDecision::Deny => false,
            }
        })
        .on_new_window(move |target: Url, _features: NewWindowFeatures| {
            if can_open_externally(&target, allow_http_for_new_windows)
                && let Err(error) = open_external_url(&target)
            {
                eprintln!("misku-native-views: no se pudo abrir {target}: {error}");
            }
            NewWindowResponse::Deny
        });

    if let Some(profile_dir) = profile_dir {
        builder = builder.data_directory(profile_dir);
    }
    if let Some(icon) = profile.icon.as_deref() {
        match resolve_icon_path(config_path, icon)
            .ok()
            .and_then(|p| Image::from_path(p).ok())
        {
            Some(image) => {
                builder = builder.icon(image)?;
            }
            None => eprintln!("Misku: icono no disponible, se usará el predeterminado"),
        }
    }
    if let Some(user_agent) = &profile.user_agent {
        builder = builder.user_agent(user_agent);
    }

    let window = builder.build()?;
    attach_native_events(&window, profile, config_path);
    if profile.suspend_when_minimized {
        let view = window.clone();
        let desired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        window.on_window_event(move |event| {
            if matches!(
                event,
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Focused(_)
            ) {
                let minimized = view.is_minimized().unwrap_or(false);
                if desired.swap(minimized, std::sync::atomic::Ordering::AcqRel) != minimized {
                    set_suspended(&view, minimized, desired.clone());
                }
            }
        });
    }
    Ok(window)
}

fn trace_timing(stage: &str, started: Instant) {
    if std::env::var_os("MISKU_NV_TRACE").is_some() {
        eprintln!(
            "misku-timing stage={stage} elapsed_ms={}",
            started.elapsed().as_millis()
        );
    }
}

fn app_menu(app: &tauri::App) -> tauri::Result<Menu<tauri::Wry>> {
    let back = MenuItem::with_id(app, "back", "Volver", true, Some("Alt+Left"))?;
    let forward = MenuItem::with_id(app, "forward", "Adelante", true, Some("Alt+Right"))?;
    let reload = MenuItem::with_id(app, "reload", "Recargar", true, Some("CmdOrCtrl+R"))?;
    let home = MenuItem::with_id(app, "home", "Ir al inicio", true, Some("Alt+Home"))?;
    let browser = MenuItem::with_id(app, "browser", "Abrir en el navegador", true, None::<&str>)?;
    let navigation = Submenu::with_items(
        app,
        "Navegación",
        true,
        &[&back, &forward, &reload, &home, &browser],
    )?;
    Menu::with_items(app, &[&navigation])
}

#[cfg(windows)]
fn set_suspended(
    window: &tauri::WebviewWindow,
    suspended: bool,
    desired: Arc<std::sync::atomic::AtomicBool>,
) {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::ICoreWebView2_3, TrySuspendCompletedHandler,
    };
    use windows::core::Interface;
    let _ = window.with_webview(move |platform| unsafe {
        let controller = platform.controller();
        let Ok(core) = controller.CoreWebView2() else {
            return;
        };
        let Ok(core) = core.cast::<ICoreWebView2_3>() else {
            return;
        };
        if suspended {
            let _ = controller.SetIsVisible(false);
            let resume = core.clone();
            let _ = core.TrySuspend(&TrySuspendCompletedHandler::create(Box::new(
                move |result, _| {
                    // Restore can race with completion of the asynchronous suspension.
                    if !desired.load(std::sync::atomic::Ordering::Acquire) {
                        let _ = resume.Resume();
                    }
                    result
                },
            )));
        } else {
            let _ = core.Resume();
            let _ = controller.SetIsVisible(true);
        }
    });
}

#[cfg(not(windows))]
fn set_suspended(_: &tauri::WebviewWindow, _: bool, _: Arc<std::sync::atomic::AtomicBool>) {}

#[cfg(windows)]
fn attach_native_events(window: &tauri::WebviewWindow, profile: &AppProfile, config_path: &Path) {
    use webview2_com::{
        FaviconChangedEventHandler, GetFaviconCompletedHandler,
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_FAVICON_IMAGE_FORMAT_PNG,
            COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED, ICoreWebView2_15,
        },
        NavigationCompletedEventHandler,
    };
    use windows::core::{BOOL, Interface};
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    let name = profile.name.clone();
    let original = profile.clone();
    let config = config_path.to_owned();
    let _ = window.with_webview(move |platform| unsafe {
        let Ok(core) = platform.controller().CoreWebView2() else { return; };
        let mut token = 0;
        let _ = core.add_NavigationCompleted(&NavigationCompletedEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                let mut success = BOOL::default(); args.IsSuccess(&mut success)?;
                let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED; args.WebErrorStatus(&mut status)?;
                if !success.as_bool() && status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                    let app = app.clone(); let label = label.clone(); let name = name.clone();
                    std::thread::spawn(move || {
                        if crate::platform::retry_error(&format!("{name} no pudo cargar la página. Comprueba la conexión y vuelve a intentarlo.\n\nDetalles: error de WebView2 {}", status.0))
                            && let Some(view) = app.get_webview_window(&label) { let _ = view.reload(); }
                    });
                }
            }
            Ok(())
        })), &mut token);
        if original.icon.is_some() { return; }
        let Ok(favicon_core) = core.cast::<ICoreWebView2_15>() else { return; };
        let attempted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _ = favicon_core.add_FaviconChanged(&FaviconChangedEventHandler::create(Box::new(move |sender, _| {
            if attempted.swap(true, std::sync::atomic::Ordering::AcqRel) { return Ok(()); }
            let Some(sender) = sender else { return Ok(()); };
            let core: ICoreWebView2_15 = sender.cast()?;
            let original = original.clone(); let config = config.clone();
            core.GetFavicon(COREWEBVIEW2_FAVICON_IMAGE_FORMAT_PNG, &GetFaviconCompletedHandler::create(Box::new(move |result, stream| {
                result?;
                let Some(stream) = stream else { return Ok(()); };
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0u8; 8192]; let mut read = 0;
                    stream.Read(chunk.as_mut_ptr().cast(), chunk.len() as u32, Some(&mut read)).ok()?;
                    if read == 0 { break; }
                    if bytes.len() + read as usize > 2 * 1024 * 1024 { return Ok(()); }
                    bytes.extend_from_slice(&chunk[..read as usize]);
                }
                let config = config.clone(); let original = original.clone();
                std::thread::spawn(move || { if let Err(e) = crate::installation::save_discovered_icon(&config, &original, &bytes) { eprintln!("Misku: no se pudo completar el icono: {e}"); } });
                Ok(())
            })))?;
            Ok(())
        })), &mut token);
    });
}

#[cfg(not(windows))]
fn attach_native_events(_: &tauri::WebviewWindow, _: &AppProfile, _: &Path) {}

fn navigation_decision(
    target: &Url,
    allowed_origins: &HashSet<String>,
    allow_insecure_http: bool,
) -> NavigationDecision {
    let origin = target.origin().ascii_serialization();
    if allowed_origins.contains(&origin)
        && (target.scheme() == "https"
            || (target.scheme() == "http" && allow_insecure_http && is_loopback(target)))
    {
        return NavigationDecision::Allow;
    }
    if can_open_externally(target, allow_insecure_http) {
        NavigationDecision::OpenExternal
    } else {
        NavigationDecision::Deny
    }
}

fn can_open_externally(target: &Url, allow_insecure_http: bool) -> bool {
    target.scheme() == "https"
        || (target.scheme() == "http" && allow_insecure_http && is_loopback(target))
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

#[cfg(windows)]
fn set_process_app_id(instance_id: uuid::Uuid) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID, core::PCWSTR};

    let app_id = format!("dev.misku.nativeviews.{}", instance_id.simple());
    let wide: Vec<u16> = std::ffi::OsStr::new(&app_id)
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe { SetCurrentProcessExplicitAppUserModelID(PCWSTR(wide.as_ptr())) }
        .map_err(|error| format!("no se pudo configurar AppUserModelID: {error}"))
}

#[cfg(not(windows))]
fn set_process_app_id(_instance_id: uuid::Uuid) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn open_external_url(url: &Url) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Foundation::HWND,
            UI::{
                Shell::ShellExecuteW,
                WindowsAndMessaging::{SHOW_WINDOW_CMD, SW_SHOWNORMAL},
            },
        },
        core::{PCWSTR, w},
    };

    let wide: Vec<u16> = std::ffi::OsStr::new(url.as_str())
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        ShellExecuteW(
            Some(HWND::default()),
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SHOW_WINDOW_CMD(SW_SHOWNORMAL.0),
        )
    };
    if result.0 as isize <= 32 {
        return Err(format!("ShellExecuteW devolvio {}", result.0 as isize));
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_external_url(url: &Url) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(url.as_str())
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn open_external_url(url: &Url) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(url.as_str())
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_only_configured_origin_in_the_webview() {
        let allowed = HashSet::from(["https://example.com".to_string()]);
        assert_eq!(
            navigation_decision(
                &Url::parse("https://example.com/A?Token=X").unwrap(),
                &allowed,
                false,
            ),
            NavigationDecision::Allow
        );
        assert_eq!(
            navigation_decision(
                &Url::parse("https://accounts.example.com/login").unwrap(),
                &allowed,
                false,
            ),
            NavigationDecision::OpenExternal
        );
        assert_eq!(
            navigation_decision(&Url::parse("http://example.com").unwrap(), &allowed, false,),
            NavigationDecision::Deny
        );
    }

    #[test]
    fn loopback_http_requires_explicit_opt_in() {
        let allowed = HashSet::from(["http://localhost:3000".to_string()]);
        let target = Url::parse("http://localhost:3000/app").unwrap();
        assert_eq!(
            navigation_decision(&target, &allowed, false),
            NavigationDecision::Deny
        );
        assert_eq!(
            navigation_decision(&target, &allowed, true),
            NavigationDecision::Allow
        );
    }

    #[test]
    fn dangerous_schemes_are_denied() {
        let allowed = HashSet::new();
        assert_eq!(
            navigation_decision(
                &Url::parse("file:///C:/Windows/System32").unwrap(),
                &allowed,
                false,
            ),
            NavigationDecision::Deny
        );
    }
}
