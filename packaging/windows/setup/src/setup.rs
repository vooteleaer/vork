use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows::core::{w, Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.zst"));
/// The version the build script names (`0.1.0`, `0.1.0-dev.88`).
const VERSION: &str = match option_env!("VORK_SETUP_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const UNINSTALL_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\vork");
const ENV_KEY: PCWSTR = w!("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment");
const FIREWALL_RULE: &str = "vØrk";
const SERVICE: &str = "vork";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn install_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into())).join("vork")
}

fn config_path() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into())).join("vork").join("vork.toml")
}

fn shortcut_path() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into())).join("Microsoft\\Windows\\Start Menu\\Programs\\vØrk.lnk")
}

fn ask(text: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    unsafe { MessageBoxW(None, &HSTRING::from(text), w!("vØrk setup"), flags) }
}

pub fn run() {
    // The node is a service and the interface needs an administrator: ask
    // Windows to start this again elevated (the UAC prompt).
    if !unsafe { IsUserAnAdmin() }.as_bool() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let Ok(me) = std::env::current_exe() {
            let r = unsafe { ShellExecuteW(None, w!("runas"), &HSTRING::from(me.as_os_str()), &HSTRING::from(args.join(" ")), None, SW_SHOWNORMAL) };
            if r.0 as isize <= 32 {
                ask("vØrk needs an administrator to install (it runs as a Windows service and makes a network interface).", MB_OK | MB_ICONERROR);
            }
        }
        return;
    }
    if std::env::args().any(|a| a == "--uninstall") {
        uninstall();
    } else {
        install();
    }
}

fn install() {
    let dir = install_dir();
    let quiet = std::env::args().any(|a| a == "--quiet");
    let upgrade = dir.join("vork.exe").exists();
    if !quiet
        && ask(
            &format!(
                "Install vØrk {VERSION}?\n\nThe vØrk node runs as a Windows service and starts with the computer; the vØrk window waits in the notification area. They go to {}.\n\nvØrk is a prototype: its network format still changes between versions.",
                dir.display()
            ),
            MB_OKCANCEL | MB_ICONINFORMATION,
        ) != IDOK
    {
        return;
    }
    match install_all(&dir) {
        Ok(()) => {
            // Started as the signed-in user, not as the administrator this runs as.
            let _ = Command::new("explorer.exe").arg(dir.join("vork-net.exe")).spawn();
            if !quiet {
                let text = if upgrade {
                    format!("vØrk is updated to {VERSION} and running again.")
                } else {
                    "vØrk is installed and running. Its window is open, and its icon stays in the notification area.\n\nNext: link your other devices (Devices → Show a code), and `vork` works in a new terminal.".to_string()
                };
                ask(&text, MB_OK | MB_ICONINFORMATION);
            }
        }
        Err(e) => {
            ask(&format!("vØrk couldn't be installed:\n\n{e}"), MB_OK | MB_ICONERROR);
        }
    }
}

fn install_all(dir: &Path) -> Result<(), String> {
    stop_programs();
    install_files(dir)?;
    configure(dir)?;
    service(dir)?;
    firewall(dir);
    register(dir)
}

trait Hidden {
    fn hidden(&mut self) -> &mut Self;
}

impl Hidden for Command {
    fn hidden(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

/// Whether a program of this name runs.
fn running(exe: &str) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {exe}"), "/NH"])
        .hidden()
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains(&exe.to_ascii_lowercase()))
        .unwrap_or(false)
}

fn service_exists() -> bool {
    Command::new("sc.exe").args(["query", SERVICE]).hidden().output().is_ok_and(|o| o.status.success())
}

/// The window first (every user's), then the service, which saves its state
/// as it stops; whatever is left of vork.exe after 15 s is ended.
fn stop_programs() {
    let _ = Command::new("taskkill").args(["/IM", "vork-net.exe", "/F"]).hidden().output();
    if service_exists() {
        let _ = Command::new("sc.exe").args(["stop", SERVICE]).hidden().output();
    }
    for _ in 0..30 {
        if !running("vork.exe") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let _ = Command::new("taskkill").args(["/IM", "vork.exe", "/F"]).hidden().output();
    std::thread::sleep(std::time::Duration::from_millis(500));
}

/// Writes a program file, waiting for a stopped program to let go of it
/// (`taskkill /F` returns before the process is gone).
fn write_when_free(target: &Path, bytes: &[u8]) -> Result<(), String> {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    let mut tries = 0;
    loop {
        match std::fs::write(target, bytes) {
            Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) && tries < 20 => {
                tries += 1;
                stop_programs();
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            r => return r.map_err(|e| format!("{}: {e}", target.display())),
        }
    }
}

fn install_files(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut packed = Vec::new();
    zstd::Decoder::new(PAYLOAD).and_then(|mut d| d.read_to_end(&mut packed)).map_err(|e| format!("the installer is damaged: {e}"))?;
    let mut at = 0;
    while at < packed.len() {
        let n = u16::from_le_bytes([packed[at], packed[at + 1]]) as usize;
        let name = String::from_utf8_lossy(&packed[at + 2..at + 2 + n]).into_owned();
        at += 2 + n;
        let len = u64::from_le_bytes(packed[at..at + 8].try_into().unwrap()) as usize;
        at += 8;
        write_when_free(&dir.join(&name), &packed[at..at + len])?;
        at += len;
    }
    // A self-update's leftovers (ADR-0028): the binary it replaced.
    let _ = std::fs::remove_file(dir.join("vork.exe.previous"));
    // The installer itself, to uninstall with.
    if let Ok(me) = std::env::current_exe() {
        if me.parent() != Some(dir) {
            let _ = std::fs::copy(me, dir.join("vork-setup.exe"));
        }
    }
    Ok(())
}

/// A new node: its configuration and identity in %ProgramData%\vork, with the
/// interface on. An existing configuration is left as it is.
fn configure(dir: &Path) -> Result<(), String> {
    let config = config_path();
    if config.exists() {
        return Ok(());
    }
    let o = Command::new(dir.join("vork.exe")).arg("init").hidden().output().map_err(|e| format!("vork init: {e}"))?;
    if !o.status.success() {
        return Err(format!("vork init: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    let text = std::fs::read_to_string(&config).map_err(|e| format!("{}: {e}", config.display()))?;
    // Programs reach vØrk addresses and .vork names through the interface.
    let text = text.replacen("# tun = \"vork0\"", "tun = \"vork\"", 1);
    std::fs::write(&config, text).map_err(|e| format!("{}: {e}", config.display()))
}

fn service(dir: &Path) -> Result<(), String> {
    if service_exists() {
        let _ = Command::new("sc.exe").args(["start", SERVICE]).hidden().output();
        return Ok(());
    }
    let o = Command::new(dir.join("vork.exe"))
        .args(["service", "install", "--config"])
        .arg(config_path())
        .hidden()
        .output()
        .map_err(|e| format!("vork service install: {e}"))?;
    if !o.status.success() {
        return Err(format!("vork service install: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(())
}

/// Lets the node's UDP in (any port: it may be told to listen elsewhere).
fn firewall(dir: &Path) {
    let _ = Command::new("netsh").args(["advfirewall", "firewall", "delete", "rule", &format!("name={FIREWALL_RULE}")]).hidden().output();
    let _ = Command::new("netsh")
        .args(["advfirewall", "firewall", "add", "rule", &format!("name={FIREWALL_RULE}"), "dir=in", "action=allow", "protocol=UDP", "enable=yes"])
        .arg(format!("program={}", dir.join("vork.exe").display()))
        .hidden()
        .output();
}

unsafe fn set_string(key: HKEY, name: PCWSTR, value: &str, kind: REG_VALUE_TYPE) -> Result<(), String> {
    let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
    RegSetValueExW(key, name, 0, kind, Some(bytes)).ok().map_err(|e| e.to_string())
}

unsafe fn open_key(path: PCWSTR) -> Result<HKEY, String> {
    let mut key = HKEY::default();
    RegCreateKeyExW(HKEY_LOCAL_MACHINE, path, 0, None, REG_OPTION_NON_VOLATILE, KEY_ALL_ACCESS, None, &mut key, None).ok().map_err(|e| e.to_string())?;
    Ok(key)
}

/// The machine's PATH (unexpanded, as stored).
unsafe fn system_path() -> String {
    let Ok(key) = open_key(ENV_KEY) else { return String::new() };
    let mut size = 0u32;
    let mut kind = REG_VALUE_TYPE::default();
    let _ = RegQueryValueExW(key, w!("Path"), None, Some(&mut kind), None, Some(&mut size));
    let mut buf = vec![0u16; size as usize / 2 + 1];
    let r = RegQueryValueExW(key, w!("Path"), None, Some(&mut kind), Some(buf.as_mut_ptr() as *mut u8), Some(&mut size));
    let _ = RegCloseKey(key);
    if r.is_err() {
        return String::new();
    }
    String::from_utf16_lossy(&buf).trim_end_matches('\0').to_string()
}

unsafe fn set_system_path(value: &str) -> Result<(), String> {
    let key = open_key(ENV_KEY)?;
    let r = set_string(key, w!("Path"), value, REG_EXPAND_SZ);
    let _ = RegCloseKey(key);
    // Running programs (Explorer) learn the new PATH.
    let mut result = 0usize;
    let _ = SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, WPARAM(0), LPARAM(w!("Environment").as_ptr() as isize), SMTO_ABORTIFHUNG, 2000, Some(&mut result));
    r
}

fn register(dir: &Path) -> Result<(), String> {
    let d = dir.display().to_string();
    unsafe {
        // The window, in the tray, at every user's login.
        let run = open_key(RUN_KEY)?;
        set_string(run, w!("vork"), &format!("\"{d}\\vork-net.exe\" --hidden"), REG_SZ)?;
        let _ = RegCloseKey(run);

        let path = system_path();
        if !path.split(';').any(|p| p.eq_ignore_ascii_case(&d)) {
            let new = if path.trim().is_empty() { d.clone() } else { format!("{};{d}", path.trim_end_matches(';')) };
            set_system_path(&new)?;
        }

        let un = open_key(UNINSTALL_KEY)?;
        set_string(un, w!("DisplayName"), "vØrk", REG_SZ)?;
        set_string(un, w!("DisplayVersion"), VERSION, REG_SZ)?;
        set_string(un, w!("Publisher"), "vØrk", REG_SZ)?;
        set_string(un, w!("InstallLocation"), &d, REG_SZ)?;
        set_string(un, w!("DisplayIcon"), &format!("{d}\\vork-net.exe"), REG_SZ)?;
        set_string(un, w!("UninstallString"), &format!("\"{d}\\vork-setup.exe\" --uninstall"), REG_SZ)?;
        set_string(un, w!("URLInfoAbout"), "https://github.com/vooteleaer/vork", REG_SZ)?;
        let one = 1u32.to_le_bytes();
        let _ = RegSetValueExW(un, w!("NoModify"), 0, REG_DWORD, Some(&one));
        let _ = RegSetValueExW(un, w!("NoRepair"), 0, REG_DWORD, Some(&one));
        let _ = RegCloseKey(un);

        shortcut(&dir.join("vork-net.exe"), &shortcut_path(), "vØrk: your devices, the network, the firewall")?;
    }
    Ok(())
}

/// A Start menu shortcut (a .lnk, through the shell's own IShellLink).
unsafe fn shortcut(target: &Path, at: &Path, about: &str) -> Result<(), String> {
    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
    link.SetPath(&HSTRING::from(target.as_os_str())).map_err(|e| e.to_string())?;
    link.SetDescription(&HSTRING::from(about)).map_err(|e| e.to_string())?;
    if let Some(parent) = target.parent() {
        let _ = link.SetWorkingDirectory(&HSTRING::from(parent.as_os_str()));
    }
    let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
    if let Some(parent) = at.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    file.Save(&HSTRING::from(at.as_os_str()), true).map_err(|e| e.to_string())
}

fn uninstall() {
    let dir = install_dir();
    if ask(
        "Uninstall vØrk?\n\nThe service and the programs are removed. This node's identity and settings stay in %ProgramData%\\vork (delete the folder yourself to forget them).",
        MB_OKCANCEL | MB_ICONQUESTION,
    ) != IDOK
    {
        return;
    }
    let _ = Command::new("taskkill").args(["/IM", "vork-net.exe", "/F"]).hidden().output();
    if service_exists() {
        let _ = Command::new(dir.join("vork.exe")).args(["service", "uninstall"]).hidden().output();
    }
    stop_programs();
    let _ = Command::new("netsh").args(["advfirewall", "firewall", "delete", "rule", &format!("name={FIREWALL_RULE}")]).hidden().output();
    let d = dir.display().to_string();
    unsafe {
        if let Ok(run) = open_key(RUN_KEY) {
            let _ = RegDeleteValueW(run, w!("vork"));
            let _ = RegCloseKey(run);
        }
        let path = system_path();
        let kept: Vec<&str> = path.split(';').filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case(&d)).collect();
        if kept.len() != path.split(';').filter(|p| !p.is_empty()).count() {
            let _ = set_system_path(&kept.join(";"));
        }
        let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, UNINSTALL_KEY);
    }
    let _ = std::fs::remove_file(shortcut_path());
    // Every file but the running uninstaller, which a hidden shell removes
    // with the folder once this has exited.
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if e.file_name() != "vork-setup.exe" {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let _ = Command::new("cmd").args(["/C", &format!("ping -n 3 127.0.0.1 >nul & rmdir /S /Q \"{d}\"")]).hidden().spawn();
    ask("vØrk is uninstalled. This node's identity is still in %ProgramData%\\vork.", MB_OK | MB_ICONINFORMATION);
}
