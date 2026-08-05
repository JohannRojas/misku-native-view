use crate::model::{AppProfile, allowed_origin_set, resolve_icon_path};
use std::{collections::HashSet, error::Error, fs, io, path::Path};
use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    image::Image,
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
            open_profile_window(app, &profile_for_setup, &config_for_setup)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .map_err(|error| error.to_string())
}

fn open_profile_window(
    app: &mut tauri::App,
    profile: &AppProfile,
    config_path: &Path,
) -> Result<(), Box<dyn Error>> {
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
        let directory = root.join("profiles").join(profile.profile_key());
        fs::create_dir_all(&directory)?;
        Some(directory)
    } else {
        None
    };

    let label = format!("app-{}", profile.instance_id());
    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title(&profile.name)
        .inner_size(profile.width, profile.height)
        .min_inner_size(profile.min_width, profile.min_height)
        .resizable(profile.resizable)
        .zoom_hotkeys_enabled(profile.zoom_hotkeys_enabled)
        .devtools(profile.devtools)
        .center()
        .prevent_overflow()
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
        let icon_path = resolve_icon_path(config_path, icon)
            .map_err(|error| io::Error::other(error.to_string()))?;
        builder = builder.icon(Image::from_path(icon_path)?)?;
    }
    if let Some(user_agent) = &profile.user_agent {
        builder = builder.user_agent(user_agent);
    }

    builder.build()?;
    Ok(())
}

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
