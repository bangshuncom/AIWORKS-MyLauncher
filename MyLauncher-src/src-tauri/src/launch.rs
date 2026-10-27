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
use crate::autorun::*;
use crate::bookmarks::*;
use crate::defaults::*;
use crate::favicon::*;
use crate::icons::*;
use crate::icons_refresh::*;
use crate::icons_system::*;
use crate::import::*;
use crate::models::*;
use crate::paths::*;
use crate::scan::*;
use crate::storage::*;


// ═══════════════════════════════════════════
// 路径解析与程序启动
// ═══════════════════════════════════════════

// ─── 路径解析 ───

/// 特殊启动目标前缀（shell: URI / 协议 URI 等），不是文件路径，恒视为有效
pub(crate) fn is_special_uri(path: &str) -> bool {
    let p = path.trim();
    p.starts_with("shell:")
        || p.starts_with("ms-settings:")
        || p.starts_with("ms-get-started:")
        || p.starts_with("ms-contact-support:")
}

/// 裸命令名（如 taskmgr.exe）：无盘符/分隔符，依赖系统 PATH 搜索，不应拼接工作目录
pub(crate) fn is_bare_command(path: &str) -> bool {
    let p = path.trim();
    !p.is_empty() && !p.contains('\\') && !p.contains('/') && !p.contains(':')
}

/// 经 ShellExecuteW 启动目标：open/runas 两种动作，参数原样透传（不自动加引号），
/// 适合 explorer /select,xxx 这类对引号敏感的调用
#[cfg(windows)]
pub(crate) fn shell_execute_open(target: &str, args: Option<&str>, run_as_admin: bool, show_cmd: i32) -> Result<bool, String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let to_wide = |s: &str| -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    };
    let target_w = to_wide(target);
    let op_w = to_wide(if run_as_admin { "runas" } else { "open" });
    let params_w = args.map(to_wide);
    unsafe {
        let result = ShellExecuteW(
            None,
            PCWSTR(op_w.as_ptr()),
            PCWSTR(target_w.as_ptr()),
            match &params_w {
                Some(p) => PCWSTR(p.as_ptr()),
                None => PCWSTR::null(),
            },
            PCWSTR::null(),
            SHOW_WINDOW_CMD(show_cmd),
        );
        let h = result.0 as usize;
        if h <= 32 {
            return Err(format!("启动失败（错误码 {}）", h));
        }
    }
    Ok(true)
}


/// 路径解析核心逻辑（相对 → 当前环境绝对 → 特殊 URI/裸命令 → fallback），供命令与图标重提取共用
pub(crate) fn resolve_path_inner(entry: &Entry, current_env_id: &str) -> Result<PathInfo, String> {
    let exe_dir = get_exe_directory()?;

    // 如果是网址，不需要解析路径
    if entry.entry_type == "url" {
        return Ok(PathInfo {
            resolved: entry.url.clone(),
            exists: !entry.url.is_empty(),
            path_mode: "url".to_string(),
        });
    }

    // 平台游戏：优先解析 exe 路径（「直接运行程序」/「目录」定位/图标提取都依赖它），
    // 存在性即有效性；启动链接（steam:// 等平台协议）存在即可视有效，exe 缺失不阻断启动
    if entry.entry_type == "provider" {
        // 优先解析 exe 路径（「直接运行程序」/「目录」定位/图标提取都依赖它）
        if let Some(abs_path) = entry.absolute_paths.get(current_env_id) {
            if !abs_path.is_empty() {
                let exists = Path::new(abs_path).exists();
                let resolved = if exists {
                    normalize_path(Path::new(abs_path)).to_string_lossy().to_string()
                } else {
                    abs_path.clone()
                };
                return Ok(PathInfo {
                    resolved,
                    exists,
                    path_mode: "absolute".to_string(),
                });
            }
        }
        // 无 exe 路径：回落到启动链接（平台协议恒有效）
        return Ok(PathInfo {
            resolved: entry.url.clone(),
            exists: !entry.url.is_empty(),
            path_mode: "url".to_string(),
        });
    }

    // 特殊 URI（shell:/ms-settings: 等）：不是文件路径，恒视为存在
    // 检查相对路径和当前环境绝对路径两处
    if !entry.relative_path.is_empty() && is_special_uri(&entry.relative_path) {
        return Ok(PathInfo {
            resolved: entry.relative_path.clone(),
            exists: true,
            path_mode: "relative".to_string(),
        });
    }
    if let Some(abs_path) = entry.absolute_paths.get(current_env_id) {
        if !abs_path.is_empty() && is_special_uri(abs_path) {
            return Ok(PathInfo {
                resolved: abs_path.clone(),
                exists: true,
                path_mode: "absolute".to_string(),
            });
        }
    }

    // 裸命令名（如 taskmgr.exe / devmgmt.msc）：由系统按 PATH 搜索，
    // 不拼 exe 目录检查存在性，恒视为有效，列表不标失效
    if !entry.relative_path.is_empty() && is_bare_command(&entry.relative_path) {
        return Ok(PathInfo {
            resolved: entry.relative_path.clone(),
            exists: true,
            path_mode: "relative".to_string(),
        });
    }
    if let Some(abs_path) = entry.absolute_paths.get(current_env_id) {
        if !abs_path.is_empty() && is_bare_command(abs_path) {
            return Ok(PathInfo {
                resolved: abs_path.clone(),
                exists: true,
                path_mode: "absolute".to_string(),
            });
        }
    }

    // 1. 尝试相对路径
    if entry.path_mode == "relative" && !entry.relative_path.is_empty() {
        let full = exe_dir.join(&entry.relative_path);
        if full.exists() {
            // 规范化路径（词法消除 ..\ 等）—— SHCreateItemFromParsingName 等 shell API
            // 不接受含 .. 的路径（返回 0x80070057 参数错误）
            return Ok(PathInfo {
                resolved: normalize_path(&full).to_string_lossy().to_string(),
                exists: true,
                path_mode: "relative".to_string(),
            });
        }
    }

    // 2. 尝试当前环境的绝对路径
    if let Some(abs_path) = entry.absolute_paths.get(current_env_id) {
        if !abs_path.is_empty() && Path::new(abs_path).exists() {
            // 同上：规范化，防旧数据含 .. 形式的绝对路径
            let canonical = normalize_path(Path::new(abs_path));
            return Ok(PathInfo {
                resolved: canonical.to_string_lossy().to_string(),
                exists: true,
                path_mode: "absolute".to_string(),
            });
        }
    }

    // 3. 都不存在
    let fallback = if entry.path_mode == "relative" {
        exe_dir.join(&entry.relative_path).to_string_lossy().to_string()
    } else {
        entry.absolute_paths.get(current_env_id)
            .cloned()
            .unwrap_or_default()
    };

    Ok(PathInfo {
        resolved: fallback,
        exists: false,
        path_mode: entry.path_mode.clone(),
    })
}

#[command]
pub fn resolve_path(entry: Entry, current_env_id: String) -> Result<PathInfo, String> {
    resolve_path_inner(&entry, &current_env_id)
}

#[command]
pub fn check_paths_batch(entries: Vec<Entry>, current_env_id: String) -> Result<Vec<bool>, String> {
    let mut results = Vec::with_capacity(entries.len());
    for entry in entries {
        let info = resolve_path(entry, current_env_id.clone())?;
        results.push(info.exists);
    }
    Ok(results)
}

// ─── 启动程序 ───

/// 用系统默认浏览器打开外部网址（条目的更新地址等）。
/// 无协议前缀时自动补 https://；仅允许 http/https，防止误触其他协议
#[command]
pub fn open_external_url(url: String) -> Result<bool, String> {
    let u = url.trim();
    if u.is_empty() {
        return Err("网址为空".to_string());
    }
    let lower = u.to_lowercase();
    let normalized = if lower.starts_with("http://") || lower.starts_with("https://") {
        u.to_string()
    } else if lower.contains("://") {
        return Err(format!("不支持的链接协议: {}", u));
    } else {
        format!("https://{}", u)
    };
    shell_execute_open(&normalized, None, false, SW_SHOWNORMAL.0)
}

#[command]
pub fn launch_program(entry: Entry, current_env_id: String) -> Result<bool, String> {
    // 网址：用默认浏览器打开
    if entry.entry_type == "url" {
        if entry.url.is_empty() {
            return Err("网址为空".to_string());
        }
        return shell_execute_open(entry.url.trim(), None, false, SW_SHOWNORMAL.0);
    }

    // 平台游戏：url 非空走平台协议（steam:// 等由系统唤起对应客户端），
    // 「直接运行程序」由前端构造临时 program 条目走下方的程序分支
    if entry.entry_type == "provider" && !entry.url.trim().is_empty() {
        return shell_execute_open(entry.url.trim(), None, false, SW_SHOWNORMAL.0);
    }

    // 系统功能 / APPX：目标存于 relative_path（URI 或裸命令名），直接启动
    if entry.entry_type == "system" || entry.entry_type == "appx" {
        let target = entry.relative_path.trim();
        if target.is_empty() {
            return Err("启动目标为空".to_string());
        }
        // 运行方式仅对窗口类程序有意义，这里统一按设置传参
        let show_cmd = match entry.window_style.as_str() {
            "maximized" => SW_MAXIMIZE.0,
            "minimized" => SW_MINIMIZE.0,
            _ => SW_SHOWNORMAL.0,
        };
        let args = if entry.launch_args.is_empty() { None } else { Some(entry.launch_args.as_str()) };
        return shell_execute_open(target, args, entry.run_as_admin, show_cmd);
    }

    // 程序/文件/文件夹：解析路径
    let path_info = resolve_path(entry.clone(), current_env_id)?;
    if !path_info.exists {
        return Err(format!("路径失效: {}", path_info.resolved));
    }

    let exe_path = path_info.resolved.clone();

    // 裸命令名（如 taskmgr.exe / devmgmt.msc）：直接交给 ShellExecuteW 搜索 PATH 启动
    // （.msc 由关联的 mmc.exe 打开，ShellExecuteW 原生处理）
    if is_bare_command(&exe_path) {
        let show_cmd = match entry.window_style.as_str() {
            "maximized" => SW_MAXIMIZE.0,
            "minimized" => SW_MINIMIZE.0,
            _ => SW_SHOWNORMAL.0,
        };
        let args = if entry.launch_args.is_empty() { None } else { Some(entry.launch_args.as_str()) };
        return shell_execute_open(&exe_path, args, entry.run_as_admin, show_cmd)
            .map_err(|e| format!("{}，请检查命令名是否有效", e));
    }

    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        let to_wide = |s: &str| -> Vec<u16> {
            OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
        };

        let operation = if entry.run_as_admin { "runas" } else { "open" };
        let params = if entry.launch_args.is_empty() {
            None
        } else {
            Some(entry.launch_args.as_str())
        };

        // 普通程序：默认工作目录为 exe 所在目录
        let working_dir = if entry.working_directory.is_empty() {
            Path::new(&exe_path).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            entry.working_directory.clone()
        };

        let show_cmd = match entry.window_style.as_str() {
            "maximized" => SW_MAXIMIZE.0,
            "minimized" => SW_MINIMIZE.0,
            _ => SW_SHOWNORMAL.0,
        };

        let file_w = to_wide(&exe_path);
        let op_w = to_wide(operation);
        let params_w = params.map(|p| to_wide(p));
        let dir_w = to_wide(&working_dir);

        // 使用 ShellExecuteW
        unsafe {
            let result = ShellExecuteW(
                None,
                PCWSTR(op_w.as_ptr()),
                PCWSTR(file_w.as_ptr()),
            match &params_w {
                Some(p) => PCWSTR(p.as_ptr()),
                None => PCWSTR::null(),
            },
            PCWSTR(dir_w.as_ptr()),
            SHOW_WINDOW_CMD(show_cmd),
        );

            let h = result.0 as usize;
            if h <= 32 {
                // UAC 拒绝或其他错误，静默返回
                return Ok(false);
            }
        }
    }


    Ok(true)
}

/// 打开条目所在目录（资源管理器定位并选中目标）
#[command]
pub fn open_in_explorer(entry: Entry, current_env_id: String) -> Result<bool, String> {
    let path_info = resolve_path(entry, current_env_id)?;
    let path = path_info.resolved.trim().to_string();

    // 特殊 URI / 裸命令名没有文件路径，无法定位
    if is_special_uri(&path) || is_bare_command(&path) || !Path::new(&path).exists() {
        return Err(format!("路径失效: {}", path));
    }

    #[cfg(windows)]
    {
        // explorer /select,"路径"：打开父目录并选中目标。
        // 必须经 ShellExecuteW 透传参数：Rust Command 传参会给含空格的参数
        // 自动整体加引号，把 /select 开关也包进去，explorer 认不出开关
        // 就退化为无参数启动（打开默认目录"文档"）
        shell_execute_open(
            "explorer.exe",
            Some(&format!("/select,\"{}\"", path)),
            false,
            SW_SHOWNORMAL.0,
        )?;
        return Ok(true);
    }

}

