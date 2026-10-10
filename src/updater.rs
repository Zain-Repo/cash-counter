//! Verified GitHub release downloads and an installer handoff for installed Windows apps.
pub mod preferences;
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const REPO: &str = "https://github.com/Zain-Repo/cash-counter";
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
const MARKER: &str = "cash-ledger-installation.ini";
const APP_ID: &str = "707A222C-758B-4E20-A86E-3A577E428B95";
#[cfg(any(windows, test))]
const APP_MUTEX: &str = "Local\\CashLedger-707A222C-758B-4E20-A86E-3A577E428B95";

#[derive(Clone, Debug)]
pub struct AvailableUpdate {
    pub version: String,
    url: String,
    sha256: String,
    size: u64,
}
#[derive(Clone, Debug)]
pub struct PreparedUpdate {
    installer: PathBuf,
    sha256: String,
    size: u64,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
    size: u64,
}

fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent("CashLedger-Updater")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            let allowed = matches!(
                url.host_str(),
                Some(
                    "github.com"
                        | "release-assets.githubusercontent.com"
                        | "objects.githubusercontent.com"
                        | "api.github.com"
                )
            );
            if attempt.previous().len() >= 5 || url.scheme() != "https" || !allowed {
                attempt.error("Untrusted update redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

/// Validate metadata before any asset is downloaded. Only the exact official stable asset is accepted.
pub fn validate_release(json: &[u8], current: &str) -> Result<Option<AvailableUpdate>, String> {
    let release: Release = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    let version = semver::Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .map_err(|e| e.to_string())?;
    let current = semver::Version::parse(current).map_err(|e| e.to_string())?;
    if release.draft || release.prerelease || !version.pre.is_empty() || version <= current {
        return Ok(None);
    }
    let name = format!("CashLedger-{version}-windows-x64-setup.exe");
    let url = format!("{REPO}/releases/download/{}/{name}", release.tag_name);
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == name)
        .ok_or("Release has no compatible Windows installer")?;
    if asset.browser_download_url != url || asset.size == 0 || asset.size > MAX_DOWNLOAD {
        return Err("Invalid installer URL or size".into());
    }
    let hash = asset
        .digest
        .and_then(|digest| digest.strip_prefix("sha256:").map(str::to_owned))
        .ok_or("Release is missing its GitHub SHA-256 digest")?;
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid SHA-256 digest".into());
    }
    Ok(Some(AvailableUpdate {
        version: version.to_string(),
        url,
        sha256: hash.to_ascii_lowercase(),
        size: asset.size,
    }))
}

pub fn check() -> Result<Option<AvailableUpdate>, String> {
    let response = client()?
        .get("https://api.github.com/repos/Zain-Repo/cash-counter/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .map_err(|e| e.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response.error_for_status().map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > 2 * 1024 * 1024 {
        return Err("Release metadata exceeds the size limit".into());
    }
    validate_release(&data, env!("CARGO_PKG_VERSION"))
}

pub fn download(update: &AvailableUpdate) -> Result<PreparedUpdate, String> {
    let directory =
        std::env::temp_dir().join(format!("cash-ledger-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let part = directory.join("installer.part");
    let installer = directory.join("installer.exe");
    let result = (|| {
        let response = client()?
            .get(&update.url)
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|e| e.to_string())?;
        let mut source = response.take(update.size + 1);
        let mut destination = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .map_err(|e| e.to_string())?;
        std::io::copy(&mut source, &mut destination).map_err(|e| e.to_string())?;
        destination
            .flush()
            .and_then(|()| destination.sync_all())
            .map_err(|e| e.to_string())?;
        verify_file(&part, &update.sha256, update.size)?;
        fs::rename(&part, &installer).map_err(|e| e.to_string())?;
        Ok(PreparedUpdate {
            installer,
            sha256: update.sha256.clone(),
            size: update.size,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
        let _ = fs::remove_dir(&directory);
    }
    result
}

pub fn verify_file(path: &Path, expected: &str, size: u64) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() != size || size == 0 || size > MAX_DOWNLOAD
    {
        return Err("Installer size does not match the release".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if format!("{:x}", hash.finalize()) != expected {
        return Err("Installer SHA-256 verification failed".into());
    }
    Ok(())
}

fn installed_executable() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    validate_installed_executable(&exe)
}

/// Verify the installed desktop target before a local integration can update it.
pub fn validate_installed_executable(path: &Path) -> Result<PathBuf, String> {
    if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return Err("Updates require an installed Windows x64 app".into());
    }
    let exe = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if exe
        .file_name()
        .is_none_or(|name| name != "cash-counter.exe")
    {
        return Err("Unsupported installed executable".into());
    }
    let directory = exe.parent().ok_or("Missing installation directory")?;
    let marker = fs::read_to_string(directory.join(MARKER))
        .map_err(|_| "This copy is not an installed app")?;
    if !marker
        .lines()
        .any(|line| line.trim() == format!("AppId={APP_ID}"))
        || !directory.join("unins000.exe").is_file()
    {
        return Err("Installation identity could not be verified".into());
    }
    Ok(exe)
}
pub fn is_installed() -> bool {
    installed_executable().is_ok()
}

/// Start the helper before the UI exits. The helper waits for this exact process handle.
pub fn launch_update(update: &PreparedUpdate) -> Result<(), String> {
    let exe = installed_executable()?;
    launch_update_for_executable(update, &exe)
}

/// Hand off to the desktop helper; the caller must flush its response and exit.
/// The installer refuses to replace a desktop application that is still open.
pub fn launch_update_for_executable(
    update: &PreparedUpdate,
    executable_path: &Path,
) -> Result<(), String> {
    let exe = validate_installed_executable(executable_path)?;
    verify_file(&update.installer, &update.sha256, update.size)?;
    // A separate helper copy releases the installed executable before Setup replaces it.
    let helper = update
        .installer
        .parent()
        .ok_or("Missing update directory")?
        .join("update-helper.exe");
    copy_new(&exe, &helper)?;
    let runtime = exe
        .parent()
        .ok_or("Missing installation directory")?
        .join("vcruntime140.dll");
    if runtime.is_file() {
        copy_new(
            &runtime,
            &helper
                .parent()
                .ok_or("Missing helper directory")?
                .join("vcruntime140.dll"),
        )?;
    }
    let ready = update
        .installer
        .parent()
        .ok_or("Missing update directory")?
        .join("ready");
    Command::new(helper)
        .arg("--apply-update")
        .arg(&update.installer)
        .arg(&exe)
        .arg(std::process::id().to_string())
        .arg(&update.sha256)
        .arg(update.size.to_string())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !ready.is_file() {
        if std::time::Instant::now() >= deadline {
            return Err("Update helper did not become ready; application remains open".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

pub fn handle_update_args() -> Result<bool, String> {
    let result = apply_update_args();
    if let Err(error) = &result {
        record_helper_failure(error);
    }
    result
}

fn failure_file() -> Option<PathBuf> {
    directories::ProjectDirs::from("ca", "Local", "Cash Register")
        .map(|dirs| dirs.data_local_dir().join("last-update-error.txt"))
}

/// Preserve helper failures because a silent installer has no terminal for diagnostics.
pub fn record_helper_failure(message: &str) {
    if let Some(path) = failure_file() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let limited: String = message.chars().take(2048).collect();
        let _ = fs::write(path, limited);
    }
}

pub fn read_last_failure() -> Option<String> {
    let mut data = String::new();
    fs::File::open(failure_file()?)
        .ok()?
        .take(8192)
        .read_to_string(&mut data)
        .ok()?;
    Some(data)
}

fn apply_update_args() -> Result<bool, String> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_none_or(|arg| arg != "--apply-update") {
        return Ok(false);
    }
    if args.len() != 7 {
        return Err("Invalid update helper arguments".into());
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let installer = fs::canonicalize(PathBuf::from(&args[2])).map_err(|e| e.to_string())?;
        let exe = fs::canonicalize(PathBuf::from(&args[3])).map_err(|e| e.to_string())?;
        if exe
            .file_name()
            .is_none_or(|name| name != "cash-counter.exe")
        {
            return Err("Unsupported update target".into());
        }
        let directory = exe.parent().ok_or("Invalid target")?;
        let marker = fs::read_to_string(directory.join(MARKER)).map_err(|e| e.to_string())?;
        if !marker
            .lines()
            .any(|line| line.trim() == format!("AppId={APP_ID}"))
            || !directory.join("unins000.exe").is_file()
        {
            return Err("Invalid update target".into());
        }
        let helper = std::env::current_exe()
            .and_then(fs::canonicalize)
            .map_err(|e| e.to_string())?;
        if installer
            .file_name()
            .is_none_or(|name| name != "installer.exe")
            || installer.parent() != helper.parent()
            || !installer
                .starts_with(fs::canonicalize(std::env::temp_dir()).map_err(|e| e.to_string())?)
        {
            return Err("Invalid staged installer path".into());
        }
        let pid = args[4]
            .to_str()
            .ok_or("Invalid process")?
            .parse::<u32>()
            .map_err(|e| e.to_string())?;
        let hash = args[5].to_str().ok_or("Invalid digest")?;
        let size = args[6]
            .to_str()
            .ok_or("Invalid size")?
            .parse::<u64>()
            .map_err(|e| e.to_string())?;
        verify_file(&installer, hash, size)?;
        let log = installer
            .parent()
            .ok_or("Missing staging directory")?
            .join("setup.log");
        let directory_argument = installer_path_argument("/DIR=", directory)?;
        let log_argument = installer_path_argument("/LOG=", &log)?;
        // The helper never terminates the app. A busy or stalled parent cancels the handoff.
        unsafe {
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if process.is_null() {
                return Err("Cannot acquire application process handle".into());
            }
            if let Err(error) = fs::write(
                installer
                    .parent()
                    .ok_or("Missing staging directory")?
                    .join("ready"),
                b"ready",
            ) {
                CloseHandle(process);
                return Err(error.to_string());
            }
            {
                let result = WaitForSingleObject(process, 120_000);
                CloseHandle(process);
                if result != WAIT_OBJECT_0 {
                    return Err("App did not close; update cancelled".into());
                }
            }
        }
        if let Err(error) = verify_file(&installer, hash, size) {
            record_helper_failure(&error);
            let _ = Command::new(&exe).spawn();
            return Err(error);
        }
        // Setup can relaunch the GUI before its process returns. Clear the previous
        // diagnostic now so the successfully updated app does not inherit it.
        if let Some(path) = failure_file() {
            let _ = fs::remove_file(path);
        }
        let status = match Command::new(&installer)
            .args([
                "/VERYSILENT",
                "/SUPPRESSMSGBOXES",
                "/SP-",
                "/NORESTART",
                "/NOCLOSEAPPLICATIONS",
                "/RESTARTEXITCODE=3010",
                "/UPDATEFROMAPP",
            ])
            .arg(directory_argument)
            .arg(log_argument)
            .status()
        {
            Ok(status) => status,
            Err(error) => {
                let message = error.to_string();
                record_helper_failure(&message);
                let _ = Command::new(&exe).spawn();
                return Err(message);
            }
        };
        if !status.success() {
            let message = format!(
                "Installer failed with status {status}. Close other Cash Ledger windows and retry. Setup log: {}",
                log.display()
            );
            record_helper_failure(&message);
            // Parent has exited. Restore the app on an ordinary installer failure,
            // but never launch a partially replaced installation awaiting reboot.
            if status.code() != Some(3010) {
                let _ = Command::new(&exe).spawn();
            }
            return Err(message);
        }
        let _ = fs::remove_file(installer);
        if let Some(path) = failure_file() {
            let _ = fs::remove_file(path);
        }
        Ok(true)
    }
    #[cfg(not(windows))]
    Err("Update installation requires Windows".into())
}

/// Inno Setup 6 expects ordinary DOS/UNC paths, not Rust's canonical verbatim paths.
#[cfg(windows)]
fn installer_path_argument(option: &str, path: &Path) -> Result<std::ffi::OsString, String> {
    use std::path::{Component, Prefix};
    let mut components = path.components();
    let mut normalized = PathBuf::new();
    match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(drive) => normalized.push(format!("{}:", char::from(drive))),
            Prefix::VerbatimUNC(server, share) => {
                normalized.push(r"\\");
                normalized.push(server);
                normalized.push(share);
            }
            Prefix::Disk(_) | Prefix::UNC(_, _) => normalized.push(prefix.as_os_str()),
            _ => return Err("Unsupported installer path".into()),
        },
        _ => return Err("Installer paths must be absolute".into()),
    }
    for component in components {
        normalized.push(component.as_os_str());
    }
    if !normalized.is_absolute() {
        return Err("Installer paths must be absolute".into());
    }
    let mut argument = std::ffi::OsString::from(option);
    argument.push(normalized);
    Ok(argument)
}

fn copy_new(source: &Path, destination: &Path) -> Result<(), String> {
    let mut source = fs::File::open(source).map_err(|e| e.to_string())?;
    let mut destination = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|e| e.to_string())?;
    std::io::copy(&mut source, &mut destination).map_err(|e| e.to_string())?;
    destination.sync_all().map_err(|e| e.to_string())
}

/// Keep a named application-presence handle alive for Inno Setup's AppMutex check.
pub struct AppPresence {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}
impl Drop for AppPresence {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}
pub fn hold_app_mutex() -> Result<AppPresence, String> {
    #[cfg(windows)]
    {
        let name: Vec<u16> = APP_MUTEX.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe {
            windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(), 0, name.as_ptr())
        };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(AppPresence { handle })
    }
    #[cfg(not(windows))]
    Ok(AppPresence {})
}

#[cfg(test)]
mod tests {
    use super::{APP_ID, APP_MUTEX, MARKER};

    #[test]
    fn external_update_target_requires_the_desktop_installation_identity() {
        let directory = tempfile::tempdir().unwrap();
        let desktop = directory.path().join("cash-counter.exe");
        let server = directory.path().join("cash-ledger-mcp.exe");
        std::fs::write(&desktop, b"test target; never executed").unwrap();
        std::fs::write(&server, b"test target; never executed").unwrap();
        assert!(super::validate_installed_executable(&desktop).is_err());
        std::fs::write(directory.path().join(MARKER), format!("AppId={APP_ID}\n")).unwrap();
        assert!(super::validate_installed_executable(&desktop).is_err());
        std::fs::write(directory.path().join("unins000.exe"), b"test uninstaller").unwrap();
        assert!(super::validate_installed_executable(&server).is_err());
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        assert_eq!(
            super::validate_installed_executable(&desktop).unwrap(),
            desktop.canonicalize().unwrap()
        );
        std::fs::write(directory.path().join(MARKER), "AppId=another-app\n").unwrap();
        assert!(super::validate_installed_executable(&desktop).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn installer_arguments_use_normal_windows_paths() {
        use super::installer_path_argument;
        use std::{ffi::OsString, path::Path};
        for (input, expected) in [
            (
                r"\\?\C:\Users\Test User\Till",
                r"/DIR=C:\Users\Test User\Till",
            ),
            (r"\\?\UNC\server\share\Till", r"/DIR=\\server\share\Till"),
            (r"C:\Users\Élodie\Till", r"/DIR=C:\Users\Élodie\Till"),
        ] {
            assert_eq!(
                installer_path_argument("/DIR=", Path::new(input)).unwrap(),
                OsString::from(expected)
            );
        }
        assert!(installer_path_argument("/DIR=", Path::new("Till")).is_err());
        assert!(installer_path_argument("/DIR=", Path::new(r"\\?\Volume{unknown}\Till")).is_err());
    }

    #[test]
    fn installer_and_updater_share_upgrade_identity() {
        let installer = include_str!("../packaging/windows/till.iss");
        assert!(
            installer
                .lines()
                .any(|line| line == format!("AppMutex={APP_MUTEX}"))
        );
        assert!(installer.contains(&format!("AppId={{{{{APP_ID}}}")));
        assert!(installer.contains(&format!("Filename: \"{{app}}\\{MARKER}\"")));
        assert!(
            installer.contains("OutputBaseFilename=CashLedger-{#AppVersion}-windows-x64-setup")
        );
    }
}
