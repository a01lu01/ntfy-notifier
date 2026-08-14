use std::path::PathBuf;
use winreg::enums::*;
use winreg::RegKey;

pub fn set_auto_start(enabled: bool, exe: &str) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = hkcu
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            KEY_SET_VALUE,
        )
        .map_err(|e| e.to_string())?;
    if enabled {
        run.set_value("ntfy-Notifier", &format!("\"{exe}\""))
            .map_err(|e| e.to_string())?;
    } else {
        match run.delete_value("ntfy-Notifier") {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn appdata() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// 注册 AUMID：开始菜单快捷方式 + 注册表（失败不影响功能）。
pub fn register_aumid(exe: &str) -> Result<(), String> {
    let data_dir = appdata().join("ntfy-notifier");
    let _ = std::fs::create_dir_all(&data_dir);
    // 通知/开始菜单使用应用图标（AppIcon 生成的 ico），
    // 避免 Toast 显示 Tauri 默认的双环图标。
    let icon = data_dir.join("app.ico");
    let _ = std::fs::write(&icon, include_bytes!("../icons/icon.ico"));

    // 注册表 AUMID
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key_path = r"Software\Classes\AppUserModelId\ntfy-Notifier";
    let (key, _) = hkcu
        .create_subkey(key_path)
        .map_err(|e| e.to_string())?;
    let _ = key.set_value("DisplayName", &"ntfy-Notifier");
    let _ = key.set_value("IconUri", &icon.display().to_string());

    // 开始菜单快捷方式 + AppUserModelID 属性
    let start_menu = appdata().join(r"Microsoft\Windows\Start Menu\Programs");
    let _ = std::fs::create_dir_all(&start_menu);
    let lnk = start_menu.join("ntfy-Notifier.lnk");
    let script = r#"
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
public static class Lnk {
  [ComImport, Guid("00021401-0000-0000-C000-000000000046")] public class ShellLink {}
  [ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("000214F9-0000-0000-C000-000000000046")]
  public interface IShellLinkW {
    void GetPath([Out, MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder p, int c, IntPtr f, int fl);
    void GetIDList(out IntPtr p); void SetIDList(IntPtr p);
    void GetDescription([Out, MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder p, int c);
    void SetDescription([MarshalAs(UnmanagedType.LPWStr)] string p);
    void GetWorkingDirectory([Out, MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder p, int c);
    void SetWorkingDirectory([MarshalAs(UnmanagedType.LPWStr)] string p);
    void GetArguments([Out, MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder p, int c);
    void SetArguments([MarshalAs(UnmanagedType.LPWStr)] string p);
    void GetHotkey(out short p); void SetHotkey(short p);
    void GetShowCmd(out int p); void SetShowCmd(int p);
    void GetIconLocation([Out, MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder p, int c, out int i);
    void SetIconLocation([MarshalAs(UnmanagedType.LPWStr)] string p, int i);
    void SetRelativePath([MarshalAs(UnmanagedType.LPWStr)] string p, int r);
    void Resolve(IntPtr h, int f); void SetPath([MarshalAs(UnmanagedType.LPWStr)] string p);
  }
  [ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99")]
  public interface IPropertyStore {
    int GetCount(out uint c); int GetAt(uint i, out Guid k);
    int GetValue(ref Guid k, out IntPtr v); int SetValue(ref Guid k, ref IntPtr v); int Commit();
  }
  [StructLayout(LayoutKind.Sequential)]
  public struct PropVariant { public ushort vt; public ushort r1; public ushort r2; public ushort r3; public IntPtr v1; public IntPtr v2; }
  [DllImport("ole32.dll")] static extern int CoTaskMemFree(IntPtr p);
  public static void Set(string lnk, string exe, string icon, string aumid) {
    var link = (IShellLinkW)new ShellLink();
    link.SetPath(exe);
    link.SetIconLocation(icon, 0);
    var store = (IPropertyStore)link;
    var key = new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3");
    var pv = new PropVariant { vt = 31, v1 = Marshal.StringToCoTaskMemUni(aumid) };
    store.SetValue(ref key, ref pv);
    store.Commit();
    CoTaskMemFree(pv.v1);
    ((IPersistFile)link).Save(lnk, true);
  }
}
'@
[Lnk]::Set('__LNK__', '__EXE__', '__ICON__', 'ntfy-Notifier')
"#
    .replace("__LNK__", &lnk.display().to_string())
    .replace("__EXE__", exe)
    .replace("__ICON__", &icon.display().to_string());
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output();
    Ok(())
}
