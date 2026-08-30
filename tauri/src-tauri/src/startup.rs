use std::ffi::{OsStr, OsString};
use std::mem::ManuallyDrop;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use windows::core::{Interface, GUID, PCWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
#[cfg(test)]
use windows::Win32::System::Com::STGM_READ;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
#[cfg(test)]
use windows::Win32::UI::Shell::SLGP_RAWPATH;
use windows::Win32::UI::Shell::{
    FOLDERID_Programs, FOLDERID_RoamingAppData, IShellLinkW, SHGetKnownFolderPath, SHStrDupW,
    ShellLink, KF_FLAG_DEFAULT,
};
use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
use winreg::RegKey;

const APP_NAME: &str = "ntfy-Notifier";
const APP_USER_MODEL_ID: &str = "ntfy-Notifier";
const SHORTCUT_FILE_NAME: &str = "ntfy-Notifier.lnk";
const RUN_VALUE_NAME: &str = "ntfy-Notifier";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APP_USER_MODEL_REGISTRY_KEY: &str = r"Software\Classes\AppUserModelId\ntfy-Notifier";

#[cfg(test)]
const MAX_SHELL_PATH_CHARS: usize = 32_768;

const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};

static SHORTCUT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, String> {
        let result =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        result
            .ok()
            .map_err(|error| format!("initializing a shortcut COM apartment failed: {error}"))?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct ShortcutSpec<'a> {
    target: &'a Path,
    arguments: &'a OsStr,
    working_directory: &'a Path,
    icon: &'a Path,
    description: &'a OsStr,
    app_user_model_id: &'a str,
}

pub fn set_auto_start(enabled: bool, exe: &Path) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if enabled {
        validate_executable(exe)?;
        let run = hkcu
            .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
            .map_err(|error| format!("opening the Windows Run registry key failed: {error}"))?;
        let command = quoted_executable_command(exe)?;
        let command_value = command.as_os_str();
        run.set_value(RUN_VALUE_NAME, &command_value)
            .map_err(|error| format!("writing the Windows Run registry value failed: {error}"))
    } else {
        let run = match hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
            Ok(run) => run,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "opening the Windows Run registry key failed: {error}"
                ))
            }
        };
        match run.delete_value(RUN_VALUE_NAME) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "removing the Windows Run registry value failed: {error}"
            )),
        }
    }
}

/// Register the AppUserModelID and Start Menu shortcut used by Windows Toasts.
pub fn register_aumid(exe: &Path) -> Result<(), String> {
    validate_executable(exe)?;

    let data_dir = known_folder(&FOLDERID_RoamingAppData)?.join("ntfy-notifier");
    std::fs::create_dir_all(&data_dir).map_err(|error| {
        format!("creating application data directory {data_dir:?} failed: {error}")
    })?;

    // Keep the generated AppIcon as the Toast/Start Menu icon rather than the Tauri default.
    let icon = data_dir.join("app.ico");
    std::fs::write(&icon, include_bytes!("../icons/icon.ico"))
        .map_err(|error| format!("writing application icon {icon:?} failed: {error}"))?;

    register_app_user_model_id(&icon)?;

    let start_menu = known_folder(&FOLDERID_Programs)?;
    std::fs::create_dir_all(&start_menu)
        .map_err(|error| format!("creating Start Menu directory {start_menu:?} failed: {error}"))?;
    let shortcut = start_menu.join(SHORTCUT_FILE_NAME);
    create_shortcut(
        &shortcut,
        &ShortcutSpec {
            target: exe,
            arguments: OsStr::new(""),
            working_directory: executable_directory(exe)?,
            icon: &icon,
            description: OsStr::new(APP_NAME),
            app_user_model_id: APP_USER_MODEL_ID,
        },
    )
}

fn register_app_user_model_id(icon: &Path) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(APP_USER_MODEL_REGISTRY_KEY)
        .map_err(|error| format!("creating AppUserModelID registry key failed: {error}"))?;
    key.set_value("DisplayName", &APP_NAME)
        .map_err(|error| format!("writing AppUserModelID DisplayName failed: {error}"))?;
    let icon_path = icon.as_os_str();
    key.set_value("IconUri", &icon_path)
        .map_err(|error| format!("writing AppUserModelID IconUri failed: {error}"))
}

fn quoted_executable_command(exe: &Path) -> Result<OsString, String> {
    encode_wide_nul("auto-start executable", exe.as_os_str())?;
    let mut command = OsString::from("\"");
    command.push(exe.as_os_str());
    command.push("\"");
    Ok(command)
}

fn validate_executable(exe: &Path) -> Result<(), String> {
    if !exe.is_absolute() {
        return Err("the executable path must be absolute; PATH lookup is not allowed".to_string());
    }
    if !exe.is_file() {
        return Err(format!("the executable path is not a file: {exe:?}"));
    }
    Ok(())
}

fn executable_directory(exe: &Path) -> Result<&Path, String> {
    exe.parent()
        .filter(|parent| parent.is_absolute())
        .ok_or_else(|| format!("the executable has no absolute working directory: {exe:?}"))
}

fn known_folder(folder_id: &GUID) -> Result<PathBuf, String> {
    let raw = unsafe { SHGetKnownFolderPath(folder_id, KF_FLAG_DEFAULT, None) }
        .map_err(|error| format!("resolving a Windows known folder failed: {error}"))?;
    let path = unsafe { PathBuf::from(OsString::from_wide(raw.as_wide())) };
    unsafe { CoTaskMemFree(Some(raw.as_ptr().cast())) };
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!(
            "Windows returned a non-absolute known folder path: {path:?}"
        ))
    }
}

fn create_shortcut(path: &Path, spec: &ShortcutSpec<'_>) -> Result<(), String> {
    validate_shortcut_spec(path, spec)?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("shortcut path has no parent: {path:?}"))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("creating shortcut directory {parent:?} failed: {error}"))?;

    let temporary_path = reserve_temporary_shortcut(path)?;
    let result = run_in_sta(|| create_shortcut_in_sta(&temporary_path, spec))
        .and_then(|()| replace_shortcut(&temporary_path, path));
    if result.is_err() {
        return cleanup_temporary_shortcut(&temporary_path, result);
    }
    Ok(())
}

fn create_shortcut_in_sta(path: &Path, spec: &ShortcutSpec<'_>) -> Result<(), String> {
    let target = encode_wide_nul("shortcut target", spec.target.as_os_str())?;
    let arguments = encode_wide_nul("shortcut arguments", spec.arguments)?;
    let working_directory = encode_wide_nul(
        "shortcut working directory",
        spec.working_directory.as_os_str(),
    )?;
    let icon = encode_wide_nul("shortcut icon", spec.icon.as_os_str())?;
    let description = encode_wide_nul("shortcut description", spec.description)?;
    let shortcut_path = encode_wide_nul("shortcut path", path.as_os_str())?;
    if spec.app_user_model_id.encode_utf16().any(|unit| unit == 0) {
        return Err("the AppUserModelID contains an embedded NUL".to_string());
    }

    let link: IShellLinkW = unsafe {
        CoCreateInstance(
            &ShellLink,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER,
        )
    }
    .map_err(|error| format!("creating IShellLinkW failed: {error}"))?;

    unsafe {
        link.SetPath(PCWSTR::from_raw(target.as_ptr()))
            .map_err(|error| format!("setting shortcut target failed: {error}"))?;
        link.SetArguments(PCWSTR::from_raw(arguments.as_ptr()))
            .map_err(|error| format!("setting shortcut arguments failed: {error}"))?;
        link.SetWorkingDirectory(PCWSTR::from_raw(working_directory.as_ptr()))
            .map_err(|error| format!("setting shortcut working directory failed: {error}"))?;
        link.SetIconLocation(PCWSTR::from_raw(icon.as_ptr()), 0)
            .map_err(|error| format!("setting shortcut icon failed: {error}"))?;
        link.SetDescription(PCWSTR::from_raw(description.as_ptr()))
            .map_err(|error| format!("setting shortcut description failed: {error}"))?;
    }

    let property_store: IPropertyStore = link
        .cast()
        .map_err(|error| format!("opening shortcut IPropertyStore failed: {error}"))?;
    let app_id = string_propvariant(spec.app_user_model_id)?;
    unsafe {
        property_store
            .SetValue(&PKEY_APP_USER_MODEL_ID, &app_id)
            .map_err(|error| format!("setting shortcut AppUserModelID failed: {error}"))?;
        property_store
            .Commit()
            .map_err(|error| format!("committing shortcut properties failed: {error}"))?;
    }

    let persist: IPersistFile = link
        .cast()
        .map_err(|error| format!("opening shortcut IPersistFile failed: {error}"))?;
    unsafe {
        persist
            .Save(PCWSTR::from_raw(shortcut_path.as_ptr()), true)
            .map_err(|error| format!("saving shortcut {path:?} failed: {error}"))
    }
}

fn run_in_sta<T, F>(operation: F) -> Result<T, String>
where
    T: Send,
    F: FnOnce() -> Result<T, String> + Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let _apartment = ComApartment::initialize()?;
                operation()
            })
            .join()
            .map_err(|_| "the Windows shortcut COM worker panicked".to_string())?
    })
}

fn reserve_temporary_shortcut(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("shortcut path has no parent: {path:?}"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("shortcut path has no file name: {path:?}"))?;

    for _ in 0..64 {
        let sequence = SHORTCUT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(
            ".{}.{}.temporary.lnk",
            std::process::id(),
            sequence
        ));
        let candidate = parent.join(temporary_name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                drop(file);
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "reserving temporary shortcut {candidate:?} failed: {error}"
                ));
            }
        }
    }
    Err(format!(
        "could not reserve a unique temporary shortcut next to {path:?}"
    ))
}

fn replace_shortcut(source: &Path, destination: &Path) -> Result<(), String> {
    let source_wide = encode_wide_nul("temporary shortcut path", source.as_os_str())?;
    let destination_wide = encode_wide_nul("shortcut path", destination.as_os_str())?;
    unsafe {
        MoveFileExW(
            PCWSTR::from_raw(source_wide.as_ptr()),
            PCWSTR::from_raw(destination_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| {
        format!("atomically replacing shortcut {destination:?} from {source:?} failed: {error}")
    })
}

fn cleanup_temporary_shortcut(
    path: &Path,
    original_result: Result<(), String>,
) -> Result<(), String> {
    let original_error = original_result.expect_err("cleanup is only used for failed saves");
    match std::fs::remove_file(path) {
        Ok(()) => Err(original_error),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(original_error),
        Err(cleanup_error) => Err(format!(
            "{original_error}; removing temporary shortcut {path:?} also failed: {cleanup_error}"
        )),
    }
}

fn validate_shortcut_spec(path: &Path, spec: &ShortcutSpec<'_>) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("the shortcut path must be absolute".to_string());
    }
    validate_executable(spec.target)?;
    if !spec.working_directory.is_absolute() || !spec.working_directory.is_dir() {
        return Err(format!(
            "the shortcut working directory is invalid: {:?}",
            spec.working_directory
        ));
    }
    if !spec.icon.is_absolute() || !spec.icon.is_file() {
        return Err(format!(
            "the shortcut icon path is invalid: {:?}",
            spec.icon
        ));
    }
    Ok(())
}

fn encode_wide_nul(label: &str, value: &OsStr) -> Result<Vec<u16>, String> {
    let mut encoded: Vec<u16> = value.encode_wide().collect();
    if encoded.contains(&0) {
        return Err(format!("{label} contains an embedded NUL"));
    }
    encoded.push(0);
    Ok(encoded)
}

fn string_propvariant(value: &str) -> Result<PROPVARIANT, String> {
    let encoded = encode_wide_nul("property value", OsStr::new(value))?;
    let duplicated = unsafe { SHStrDupW(PCWSTR::from_raw(encoded.as_ptr())) }
        .map_err(|error| format!("allocating shortcut property value failed: {error}"))?;
    Ok(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_LPWSTR,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    pwszVal: duplicated,
                },
            }),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::Builder;

    struct ShortcutProperties {
        target: PathBuf,
        arguments: OsString,
        working_directory: PathBuf,
        icon: PathBuf,
        description: OsString,
        app_user_model_id: String,
    }

    #[test]
    fn shortcut_round_trips_unicode_spaces_and_quotes() {
        let temp = Builder::new()
            .prefix("ntfy shortcut 中文 '")
            .tempdir()
            .expect("create temp directory");
        let executable_directory = temp.path().join("应用 folder's files");
        std::fs::create_dir_all(&executable_directory).expect("create executable directory");
        let executable = executable_directory.join("ntfy notifier's 程序.exe");
        std::fs::write(&executable, b"test executable").expect("write dummy executable");
        let icon = executable_directory.join("notification icon's 图标.ico");
        std::fs::write(&icon, b"test icon").expect("write dummy icon");
        let shortcut = temp.path().join("Start Menu 启动's link.lnk");
        let arguments = OsStr::new("--profile \"中文 with spaces\" --apostrophe 'value'");
        let description = OsStr::new("ntfy 通知's shortcut");
        let app_id = "ntfy-Notifier.Test.Unicode";

        create_shortcut(
            &shortcut,
            &ShortcutSpec {
                target: &executable,
                arguments,
                working_directory: &executable_directory,
                icon: &icon,
                description,
                app_user_model_id: app_id,
            },
        )
        .expect("create COM shortcut");

        let actual = read_shortcut_properties(&shortcut).expect("read COM shortcut");
        assert_eq!(actual.target, executable);
        assert_eq!(actual.arguments, OsString::from(arguments));
        assert_eq!(actual.working_directory, executable_directory);
        assert_eq!(actual.icon, icon);
        assert_eq!(actual.description, OsString::from(description));
        assert_eq!(actual.app_user_model_id, app_id);
    }

    #[test]
    fn shortcut_save_replaces_every_property_without_shelling_out() {
        let temp = Builder::new()
            .prefix("ntfy overwrite 中文 '")
            .tempdir()
            .expect("create temp directory");
        let first_directory = temp.path().join("first target's folder");
        let second_directory = temp.path().join("second 目标 folder");
        std::fs::create_dir_all(&first_directory).expect("create first directory");
        std::fs::create_dir_all(&second_directory).expect("create second directory");
        let first = first_directory.join("first.exe");
        let second = second_directory.join("second app's 中文.exe");
        std::fs::write(&first, b"first").expect("write first executable");
        std::fs::write(&second, b"second").expect("write second executable");
        let shortcut = temp.path().join("replace me's 快捷方式.lnk");

        create_shortcut(
            &shortcut,
            &ShortcutSpec {
                target: &first,
                arguments: OsStr::new("--first"),
                working_directory: &first_directory,
                icon: &first,
                description: OsStr::new("first"),
                app_user_model_id: "ntfy-Notifier.First",
            },
        )
        .expect("create first shortcut");
        create_shortcut(
            &shortcut,
            &ShortcutSpec {
                target: &second,
                arguments: OsStr::new("--second '中文 value'"),
                working_directory: &second_directory,
                icon: &second,
                description: OsStr::new("second 中文"),
                app_user_model_id: "ntfy-Notifier.Second",
            },
        )
        .expect("replace shortcut");

        let actual = read_shortcut_properties(&shortcut).expect("read replaced shortcut");
        assert_eq!(actual.target, second);
        assert_eq!(actual.arguments, OsString::from("--second '中文 value'"));
        assert_eq!(actual.working_directory, second_directory);
        assert_eq!(actual.icon, second);
        assert_eq!(actual.description, OsString::from("second 中文"));
        assert_eq!(actual.app_user_model_id, "ntfy-Notifier.Second");
    }

    #[test]
    fn run_registry_command_preserves_unicode_spaces_and_apostrophes() {
        let executable = Path::new(r"C:\Program Files\用户's app\ntfy notifier.exe");
        let command = quoted_executable_command(executable).expect("quote executable");
        assert_eq!(
            command,
            OsString::from(r#""C:\Program Files\用户's app\ntfy notifier.exe""#)
        );
    }

    #[test]
    fn relative_executables_are_rejected_instead_of_searched_on_path() {
        let error = validate_executable(Path::new("ntfy-notifier.exe"))
            .expect_err("relative executable must fail");
        assert!(error.contains("absolute"));
        assert!(error.contains("PATH"));
    }

    #[test]
    fn embedded_nul_is_rejected_before_any_windows_api_call() {
        let invalid = OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            b'a' as u16,
            0,
            b'b' as u16,
        ]);
        let error =
            quoted_executable_command(Path::new(&invalid)).expect_err("embedded NUL must fail");
        assert!(error.contains("embedded NUL"));
    }

    fn read_shortcut_properties(path: &Path) -> Result<ShortcutProperties, String> {
        run_in_sta(|| read_shortcut_properties_in_sta(path))
    }

    fn read_shortcut_properties_in_sta(path: &Path) -> Result<ShortcutProperties, String> {
        let shortcut_path = encode_wide_nul("shortcut path", path.as_os_str())?;
        let link: IShellLinkW = unsafe {
            CoCreateInstance(
                &ShellLink,
                None::<&windows::core::IUnknown>,
                CLSCTX_INPROC_SERVER,
            )
        }
        .map_err(|error| format!("creating IShellLinkW for test failed: {error}"))?;
        let persist: IPersistFile = link
            .cast()
            .map_err(|error| format!("opening test IPersistFile failed: {error}"))?;
        unsafe {
            persist
                .Load(PCWSTR::from_raw(shortcut_path.as_ptr()), STGM_READ)
                .map_err(|error| format!("loading test shortcut failed: {error}"))?;
        }

        let mut target = vec![0_u16; MAX_SHELL_PATH_CHARS];
        let mut arguments = vec![0_u16; MAX_SHELL_PATH_CHARS];
        let mut working_directory = vec![0_u16; MAX_SHELL_PATH_CHARS];
        let mut icon = vec![0_u16; MAX_SHELL_PATH_CHARS];
        let mut description = vec![0_u16; MAX_SHELL_PATH_CHARS];
        let mut icon_index = -1;
        unsafe {
            link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
                .map_err(|error| format!("reading test shortcut target failed: {error}"))?;
            link.GetArguments(&mut arguments)
                .map_err(|error| format!("reading test shortcut arguments failed: {error}"))?;
            link.GetWorkingDirectory(&mut working_directory)
                .map_err(|error| {
                    format!("reading test shortcut working directory failed: {error}")
                })?;
            link.GetIconLocation(&mut icon, &mut icon_index)
                .map_err(|error| format!("reading test shortcut icon failed: {error}"))?;
            link.GetDescription(&mut description)
                .map_err(|error| format!("reading test shortcut description failed: {error}"))?;
        }
        if icon_index != 0 {
            return Err(format!("unexpected shortcut icon index: {icon_index}"));
        }

        let property_store: IPropertyStore = link
            .cast()
            .map_err(|error| format!("opening test IPropertyStore failed: {error}"))?;
        let app_id = unsafe {
            property_store
                .GetValue(&PKEY_APP_USER_MODEL_ID)
                .map_err(|error| format!("reading test AppUserModelID failed: {error}"))?
        };
        if app_id.vt() != VT_LPWSTR {
            return Err(format!(
                "unexpected AppUserModelID property type: {:?}",
                app_id.vt()
            ));
        }

        Ok(ShortcutProperties {
            target: PathBuf::from(os_string_from_nul_buffer(&target)),
            arguments: os_string_from_nul_buffer(&arguments),
            working_directory: PathBuf::from(os_string_from_nul_buffer(&working_directory)),
            icon: PathBuf::from(os_string_from_nul_buffer(&icon)),
            description: os_string_from_nul_buffer(&description),
            app_user_model_id: app_id.to_string(),
        })
    }

    fn os_string_from_nul_buffer(buffer: &[u16]) -> OsString {
        let length = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        OsString::from_wide(&buffer[..length])
    }
}
