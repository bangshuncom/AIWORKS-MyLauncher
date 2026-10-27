#![allow(unused_imports)]

use tauri::{command, Emitter};
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[cfg(windows)]
use windows::{
    core::{GUID, PCWSTR, Interface, HSTRING},
    Win32::{
        Foundation::{BOOL, HWND, SIZE},
        Graphics::Gdi::*,
        Storage::FileSystem::WIN32_FIND_DATAW,
        System::Com::{CoInitializeEx, CoUninitialize, CoCreateInstance, COINIT_APARTMENTTHREADED, CLSCTX_INPROC_SERVER, IPersistFile, STGM},
        UI::{
            Shell::{ShellExecuteW, IShellItemImageFactory, IShellLinkW, SHCreateItemFromParsingName, SIIGBF_ICONONLY, SHFILEOPSTRUCTW, SHFileOperationW},
            WindowsAndMessaging::*,
        },
    },
};

use crate::assets::*;
use crate::bookmarks::*;
use crate::defaults::*;
use crate::favicon::*;
use crate::icons::*;
use crate::icons_refresh::*;
use crate::icons_system::*;
use crate::import::*;
use crate::launch::*;
use crate::models::*;
use crate::paths::*;
use crate::scan::*;
use crate::storage::*;


// ═══════════════════════════════════════════
// 开机自启（注册表 HKCU Run 键）
// ═══════════════════════════════════════════

// ════════════════════════════════════════════
// 开机自启（注册表 HKCU Run 键）
// ════════════════════════════════════════════

/// 自启命令串中的静默启动参数（开机自启且勾选"不显示主界面"时附加）
pub(crate) const AUTORUN_HIDDEN_ARG: &str = "--hidden";

/// 注册表 HKCU Run 键操作封装（仅 Windows）
#[cfg(windows)]
mod autorun_registry {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::*;

    pub const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    pub const VALUE_NAME: &str = "MyLauncher";

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    /// 打开（或创建）HKCU Run 键
    /// 注：不用 RegCreateKeyExW（在 windows 0.58 中需 Win32_Security feature），RegCreateKeyW 足够
    fn open_run_key() -> Result<HKEY, String> {
        let sub_key_w = to_wide(RUN_KEY_PATH);
        unsafe {
            let mut key = HKEY::default();
            let result = RegCreateKeyW(
                HKEY_CURRENT_USER,
                PCWSTR(sub_key_w.as_ptr()),
                &mut key,
            );
            if result.is_err() {
                return Err(format!("打开注册表 Run 键失败: {:?}", result));
            }
            Ok(key)
        }
    }

    /// 写入 Run 值（REG_SZ，存在则覆盖）
    pub fn set_run_value(data: &str) -> Result<(), String> {
        let value_name = to_wide(VALUE_NAME);
        let value_data = to_wide(data);
        let bytes: Vec<u8> = value_data
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        let key = open_run_key()?;
        let result = unsafe {
            RegSetValueExW(
                key,
                PCWSTR(value_name.as_ptr()),
                0,
                REG_SZ,
                Some(&bytes),
            )
        };
        unsafe { let _ = RegCloseKey(key); }
        if result.is_err() {
            return Err(format!("写入注册表值失败: {:?}", result));
        }
        Ok(())
    }

    /// 读取 Run 值（值不存在或类型非 REG_SZ 返回 None）
    pub fn get_run_value() -> Option<String> {
        let value_name = to_wide(VALUE_NAME);
        let key = open_run_key().ok()?;
        let mut buf_len: u32 = 0;
        let query = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(value_name.as_ptr()),
                None,
                None,
                None,
                Some(&mut buf_len),
            )
        };
        if query.is_err() || buf_len == 0 {
            unsafe { let _ = RegCloseKey(key); }
            return None;
        }
        let mut buf = vec![0u8; buf_len as usize];
        let result = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(value_name.as_ptr()),
                None,
                None,
                Some(buf.as_mut_ptr()),
                Some(&mut buf_len),
            )
        };
        unsafe { let _ = RegCloseKey(key); }
        if result.is_err() {
            return None;
        }
        // 字节 → UTF-16 字符串（去掉结尾 NUL）
        let words: Vec<u16> = buf
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let s = String::from_utf16_lossy(&words);
        let s = s.trim_end_matches('\0').to_string();
        if s.is_empty() { None } else { Some(s) }
    }

    /// 删除 Run 值（值不存在视为成功）
    pub fn delete_run_value() -> Result<(), String> {
        let value_name = to_wide(VALUE_NAME);
        let key = open_run_key()?;
        let result = unsafe { RegDeleteValueW(key, PCWSTR(value_name.as_ptr())) };
        unsafe { let _ = RegCloseKey(key); }
        // ERROR_FILE_NOT_FOUND(2) = 值本来就不存在，视为成功
        if result.is_err() && result.0 != 2 {
            return Err(format!("删除注册表值失败: {:?}", result));
        }
        Ok(())
    }
}

/// 当前 exe 绝对路径（用于自启注册表命令串）
pub(crate) fn current_exe_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("获取 exe 路径失败: {}", e))
}

/// 按开关与静默选项写/删自启注册表值；命令串格式: "exe路径" [--hidden]
pub(crate) fn apply_autorun_registry(enabled: bool, silent: bool) -> Result<(), String> {
    if !enabled {
        #[cfg(windows)]
        autorun_registry::delete_run_value()?;
        return Ok(());
    }
    let exe = current_exe_path()?;
    let data = if silent {
        format!("\"{}\" {}", exe, AUTORUN_HIDDEN_ARG)
    } else {
        format!("\"{}\"", exe)
    };
    #[cfg(windows)]
    autorun_registry::set_run_value(&data)?;
    Ok(())
}

/// 启动时校正自启注册表值：程序被移动后旧路径失效，用当前实际路径重写（保留 --hidden 标记）
pub(crate) fn fix_autorun_registry() {
    #[cfg(windows)]
    {
        if let Some(data) = autorun_registry::get_run_value() {
            if let Ok(exe) = current_exe_path() {
                let quoted = format!("\"{}\"", exe);
                if !data.starts_with(&quoted) {
                    let silent = data.contains(AUTORUN_HIDDEN_ARG);
                    let new_data = if silent {
                        format!("\"{}\" {}", exe, AUTORUN_HIDDEN_ARG)
                    } else {
                        quoted
                    };
                    let _ = autorun_registry::set_run_value(&new_data);
                }
            }
        }
    }
}

/// 读取自启注册表值当前状态（是否存在 + 是否静默启动）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutorunStatus {
    pub enabled: bool,
    pub silent: bool,
}

pub(crate) fn get_autorun_status() -> AutorunStatus {
    #[cfg(windows)]
    {
        if let Some(data) = autorun_registry::get_run_value() {
            return AutorunStatus {
                enabled: true,
                silent: data.contains(AUTORUN_HIDDEN_ARG),
            };
        }
    }
    AutorunStatus { enabled: false, silent: false }
}

/// 设置开机自启（enabled=false 时删除注册表值；silent 控制自启时是否隐藏主窗口）
#[command]
pub fn set_autorun(enabled: bool, silent: bool) -> Result<(), String> {
    apply_autorun_registry(enabled, silent)
}

/// 查询当前自启状态（读注册表实际值，非前端内存）
#[command]
pub fn get_autorun() -> AutorunStatus {
    get_autorun_status()
}

