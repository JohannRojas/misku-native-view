//! Small Windows integration layer. App identity is per UUID, not per binary.
use std::path::Path;

#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize, IPersistFile,
            },
            Threading::{CreateEventW, CreateMutexW, INFINITE, SetEvent, WaitForSingleObject},
        },
        UI::{
            Shell::{IShellLinkW, ShellLink},
            WindowsAndMessaging::{IDRETRY, MB_ICONERROR, MB_OK, MB_RETRYCANCEL, MessageBoxW},
        },
    },
    core::{Interface, PCWSTR},
};

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

pub(crate) fn show_error(title: &str, details: &str) {
    #[cfg(windows)]
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(wide(details).as_ptr()),
            PCWSTR(wide(title).as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
    #[cfg(not(windows))]
    eprintln!("{title}: {details}");
}

pub(crate) fn retry_error(details: &str) -> bool {
    #[cfg(windows)]
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide(details).as_ptr()),
            PCWSTR(wide("No se pudo cargar la página").as_ptr()),
            MB_RETRYCANCEL | MB_ICONERROR,
        ) == IDRETRY
    }
    #[cfg(not(windows))]
    {
        eprintln!("{details}");
        false
    }
}

#[cfg(windows)]
struct Handle(HANDLE);
#[cfg(windows)]
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
pub(crate) struct InstanceGuard {
    _mutex: Handle,
    event: Handle,
    name: String,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    listener: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl InstanceGuard {
    pub(crate) fn acquire(id: &str) -> Result<Option<Self>, String> {
        let name = format!("Local\\Misku.NativeViews.{id}");
        unsafe {
            let event = Handle(
                CreateEventW(
                    None,
                    false,
                    false,
                    PCWSTR(wide(&format!("{name}.activate")).as_ptr()),
                )
                .map_err(|e| e.to_string())?,
            );
            let mutex = Handle(
                CreateMutexW(None, false, PCWSTR(wide(&name).as_ptr()))
                    .map_err(|e| e.to_string())?,
            );
            if GetLastError() == ERROR_ALREADY_EXISTS {
                SetEvent(event.0).map_err(|e| e.to_string())?;
                return Ok(None);
            }
            Ok(Some(Self {
                _mutex: mutex,
                event,
                name,
                stop: Default::default(),
                listener: None,
            }))
        }
    }

    pub(crate) fn listen(&mut self, activate: impl Fn() + Send + 'static) {
        let name = self.name.clone();
        let stop = self.stop.clone();
        self.listener = Some(std::thread::spawn(move || unsafe {
            let Ok(raw) = CreateEventW(
                None,
                false,
                false,
                PCWSTR(wide(&format!("{name}.activate")).as_ptr()),
            ) else {
                return;
            };
            let event = Handle(raw);
            loop {
                if WaitForSingleObject(event.0, INFINITE) != WAIT_OBJECT_0 {
                    break;
                }
                if stop.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                activate();
            }
        }));
    }
}

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        unsafe {
            let _ = SetEvent(self.event.0);
        }
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
    }
}

#[cfg(not(windows))]
pub(crate) struct InstanceGuard;
#[cfg(not(windows))]
impl InstanceGuard {
    pub(crate) fn acquire(_: &str) -> Result<Option<Self>, String> {
        Ok(Some(Self))
    }
    pub(crate) fn listen(&mut self, _: impl Fn() + Send + 'static) {}
}

pub(crate) fn create_shortcut(
    shortcut: &Path,
    executable: &Path,
    config: &Path,
    id: &str,
    icon: &Path,
) -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        struct ComGuard;
        impl Drop for ComGuard {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _com = ComGuard;
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        link.SetPath(PCWSTR(wide(&executable.to_string_lossy()).as_ptr()))
            .map_err(|e| e.to_string())?;
        let arguments = format!(
            "--config {} --app {}",
            quote_argument(&config.to_string_lossy()),
            quote_argument(id)
        );
        link.SetArguments(PCWSTR(wide(&arguments).as_ptr()))
            .map_err(|e| e.to_string())?;
        link.SetIconLocation(PCWSTR(wide(&icon.to_string_lossy()).as_ptr()), 0)
            .map_err(|e| e.to_string())?;
        let persist: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        persist
            .Save(PCWSTR(wide(&shortcut.to_string_lossy()).as_ptr()), true)
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        let _ = (shortcut, executable, config, id, icon);
    }
    Ok(())
}

pub(crate) fn quote_argument(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        if ch == '"' {
            quoted.push_str(&"\\".repeat(slashes * 2 + 1));
        } else {
            quoted.push_str(&"\\".repeat(slashes));
        }
        quoted.push(ch);
        slashes = 0;
    }
    quoted.push_str(&"\\".repeat(slashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quotes_windows_paths_and_embedded_quotes() {
        assert_eq!(quote_argument("C:\\Some App\\"), "\"C:\\Some App\\\\\"");
        assert_eq!(quote_argument("a\"b"), "\"a\\\"b\"");
    }
    #[cfg(windows)]
    #[test]
    fn duplicate_instance_signals_original_but_other_uuid_is_independent() {
        let id = uuid::Uuid::new_v4().to_string();
        let mut first = InstanceGuard::acquire(&id).unwrap().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        first.listen(move || {
            let _ = tx.send(());
        });
        assert!(InstanceGuard::acquire(&id).unwrap().is_none());
        rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        assert!(
            InstanceGuard::acquire(&uuid::Uuid::new_v4().to_string())
                .unwrap()
                .is_some()
        );
    }
}
