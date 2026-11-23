#[cfg(target_os = "windows")]
use std::env;
#[cfg(target_os = "windows")]
use std::ffi::OsStr;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
#[cfg(target_os = "windows")]
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};

#[cfg(target_os = "windows")]
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
#[cfg(target_os = "windows")]
const APP_NAME: &str = "LinkWatch";

#[cfg(target_os = "windows")]
fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// Checks if LinkWatch is set to start automatically on Windows boot.
pub fn is_autostart_enabled() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        let key_wide = to_wide_null(RUN_KEY);
        let app_wide = to_wide_null(APP_NAME);
        let mut hkey: HKEY = std::ptr::null_mut();

        unsafe {
            let status = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                key_wide.as_ptr(),
                0,
                KEY_READ,
                &mut hkey,
            );
            if status != ERROR_SUCCESS {
                return Ok(false);
            }

            let mut data_type: u32 = 0;
            let mut data_len: u32 = 0;

            let query_status = RegQueryValueExW(
                hkey,
                app_wide.as_ptr(),
                std::ptr::null_mut(),
                &mut data_type,
                std::ptr::null_mut(),
                &mut data_len,
            );

            RegCloseKey(hkey);

            if query_status == ERROR_SUCCESS {
                Ok(true)
            } else if query_status == ERROR_FILE_NOT_FOUND {
                Ok(false)
            } else {
                Ok(false)
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(false)
    }
}

/// Enables or disables autostart in the Windows Current User Run registry key.
/// If `start_minimized` is true, `--minimized` is appended to the executable command.
pub fn set_autostart_enabled(enabled: bool, start_minimized: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let key_wide = to_wide_null(RUN_KEY);
        let app_wide = to_wide_null(APP_NAME);
        let mut hkey: HKEY = std::ptr::null_mut();

        unsafe {
            if enabled {
                let current_exe = env::current_exe().map_err(|e| e.to_string())?;
                let exe_str = current_exe.to_str().ok_or("Invalid executable path")?;

                let cmd = if start_minimized {
                    format!("\"{}\" --minimized", exe_str)
                } else {
                    format!("\"{}\"", exe_str)
                };

                let cmd_wide = to_wide_null(&cmd);

                let status = RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    key_wide.as_ptr(),
                    0,
                    KEY_WRITE,
                    &mut hkey,
                );
                if status != ERROR_SUCCESS {
                    return Err(format!("Failed to open registry key (error code {})", status));
                }

                let bytes_len = (cmd_wide.len() * std::mem::size_of::<u16>()) as u32;
                let set_status = RegSetValueExW(
                    hkey,
                    app_wide.as_ptr(),
                    0,
                    REG_SZ,
                    cmd_wide.as_ptr() as *const u8,
                    bytes_len,
                );

                RegCloseKey(hkey);

                if set_status != ERROR_SUCCESS {
                    return Err(format!("Failed to set registry value (error code {})", set_status));
                }
                Ok(())
            } else {
                let status = RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    key_wide.as_ptr(),
                    0,
                    KEY_WRITE,
                    &mut hkey,
                );
                if status != ERROR_SUCCESS {
                    return Ok(()); // Key cannot be opened or does not exist
                }

                let del_status = RegDeleteValueW(hkey, app_wide.as_ptr());
                RegCloseKey(hkey);

                if del_status == ERROR_SUCCESS || del_status == ERROR_FILE_NOT_FOUND {
                    Ok(())
                } else {
                    Err(format!("Failed to delete registry value (error code {})", del_status))
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (enabled, start_minimized);
        Ok(())
    }
}
