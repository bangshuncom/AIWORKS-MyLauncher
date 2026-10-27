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
use crate::launch::*;
use crate::models::*;
use crate::scan::*;
use crate::storage::*;


// ═══════════════════════════════════════════
// 路径工具与路径模式转换
// ═══════════════════════════════════════════

// ════════════════════════════════════════════
// 辅助函数
// ════════════════════════════════════════════

pub(crate) fn get_exe_directory() -> Result<PathBuf, String> {
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("无法获取 exe 路径: {}", e))?;
    Ok(exe_path.parent()
        .ok_or("无法获取 exe 所在目录")?
        .to_path_buf())
}

/// 将绝对路径转为相对于 exe 目录的路径（用正斜杠），用于前端 convertFileSrc。
/// 转换失败时原样返回（也替换为正斜杠）。
pub(crate) fn rel_to_exe_dir(path: &Path) -> String {
    let exe_dir = get_exe_directory().unwrap_or_default();
    path.strip_prefix(&exe_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
        .replace('\\', "/")
}

/// 纯词法规范化绝对路径（消除 ..\ 与 .\ 段），不访问文件系统、不加 \\?\ 前缀。
/// SHCreateItemFromParsingName 等 shell API 不接受含 .. 的路径（0x80070057），
/// 而 std canonicalize 会产生 \\?\ 前缀路径同样不被 shell API 接受，故自实现。
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let mut result: Vec<std::ffi::OsString> = Vec::new();
    let mut prefix = String::new();

    for comp in path.components() {
        use std::path::Component;
        match comp {
            Component::Prefix(p) => prefix = p.as_os_str().to_string_lossy().to_string(),
            Component::RootDir => {
                if !prefix.is_empty() && !prefix.ends_with('\\') {
                    prefix.push('\\');
                }
            }
            Component::CurDir => { /* . 段直接丢弃 */ }
            Component::ParentDir => {
                // .. 段弹出一个普通段（前缀/根不弹）
                result.pop();
            }
            Component::Normal(s) => result.push(s.to_os_string()),
        }
    }

    let mut out = PathBuf::from(prefix);
    for seg in result {
        out.push(seg);
    }
    out
}

pub(crate) fn get_data_dir() -> Result<PathBuf, String> {
    let exe_dir = get_exe_directory()?;
    let data_dir = exe_dir.join("data");
    if !data_dir.exists() {
        fs::create_dir_all(&data_dir)
            .map_err(|e| format!("无法创建 data 目录: {}", e))?;
    }
    Ok(data_dir)
}

pub(crate) fn get_assets_dir() -> Result<PathBuf, String> {
    let exe_dir = get_exe_directory()?;
    let assets_dir = exe_dir.join("assets");
    if !assets_dir.exists() {
        fs::create_dir_all(&assets_dir)
            .map_err(|e| format!("无法创建 assets 目录: {}", e))?;
    }
    Ok(assets_dir)
}

pub(crate) fn ensure_dir(path: &Path) -> Result<(), String> {
    if !path.exists() {
        fs::create_dir_all(path)
            .map_err(|e| format!("无法创建目录 {:?}: {}", path, e))?;
    }
    Ok(())
}

pub(crate) fn get_now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// 计算从 from_dir 到 target_path 的相对路径（同盘符时可用，返回如 "..\..\Tools\app.exe"）
/// 不同盘符时返回 None。
/// 输入路径应已规范化（去除 redundant separators，使用统一分隔符）。
pub(crate) fn make_relative_path(from_dir: &Path, target_path: &Path) -> Option<String> {
    // Windows 下需要同盘符
    let from_components: Vec<_> = from_dir.components().collect();
    let target_components: Vec<_> = target_path.components().collect();

    // 检查盘符是否一致（Prefix 组件）
    let from_prefix = from_components.iter().find_map(|c| match c {
        std::path::Component::Prefix(p) => Some(p),
        _ => None,
    });
    let target_prefix = target_components.iter().find_map(|c| match c {
        std::path::Component::Prefix(p) => Some(p),
        _ => None,
    });

    match (&from_prefix, &target_prefix) {
        (Some(f), Some(t)) => {
            if f.kind() != t.kind() {
                return None; // 不同盘符
            }
            // 对于 VerbatimDisk 需要额外检查盘符字母
            if let (std::path::Prefix::VerbatimDisk(fc), std::path::Prefix::VerbatimDisk(tc)) = (f.kind(), t.kind()) {
                if fc != tc { return None; }
            }
            if let (std::path::Prefix::Disk(fc), std::path::Prefix::Disk(tc)) = (f.kind(), t.kind()) {
                if fc != tc { return None; }
            }
        }
        (None, None) => {} // 都没有盘符，继续
        _ => return None, // 一个有盘符一个没有
    }

    // 跳过 Prefix，从 Root 开始比较组件
    let from_parts: Vec<_> = from_components.iter()
        .filter(|c| matches!(c, std::path::Component::RootDir | std::path::Component::Normal(_)))
        .collect();
    let target_parts: Vec<_> = target_components.iter()
        .filter(|c| matches!(c, std::path::Component::RootDir | std::path::Component::Normal(_)))
        .collect();

    // 跳过 RootDir
    let from_dirs: Vec<&str> = from_parts.iter()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    let target_dirs: Vec<&str> = target_parts.iter()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();

    // 找公共前缀
    let mut common = 0;
    while common < from_dirs.len() && common < target_dirs.len() {
        if from_dirs[common].eq_ignore_ascii_case(&target_dirs[common]) {
            common += 1;
        } else {
            break;
        }
    }

    // 构建 ".." 前缀
    let up_count = from_dirs.len() - common;
    let mut parts: Vec<String> = (0..up_count).map(|_| "..".to_string()).collect();
    // 追加剩余目标组件
    for d in &target_dirs[common..] {
        parts.push(d.to_string());
    }

    if parts.is_empty() {
        return Some(".".to_string());
    }

    // Windows 下用反斜杠
    Some(parts.join("\\"))
}

// ─── 路径模式转换 ───

/// 将条目的路径在相对路径和绝对路径之间转换
/// - relative → absolute: 将 relative_path 解析为绝对路径，存入当前环境的 absolute_paths
/// - absolute → relative: 同盘符时用 ..\..\ 形式转为相对路径，不同盘符时失败
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertPathResult {
    pub success: bool,
    pub message: String,
    pub new_path_mode: String,
    pub new_relative_path: String,
    pub new_absolute_path: String,
}

#[command]
pub fn convert_path_mode(entry: Entry, current_env_id: String, target_mode: String) -> Result<ConvertPathResult, String> {
    let exe_dir = get_exe_directory()?;

    // 特殊 URI 条目（shell:/ms-settings:）不参与路径模式转换
    let cur_abs = entry.absolute_paths.get(&current_env_id).cloned().unwrap_or_default();
    if is_special_uri(&entry.relative_path) || is_special_uri(&cur_abs) {
        return Ok(ConvertPathResult {
            success: false,
            message: "系统功能/APPX 条目无需转换路径".to_string(),
            new_path_mode: entry.path_mode.clone(),
            new_relative_path: entry.relative_path.clone(),
            new_absolute_path: cur_abs,
        });
    }

    if target_mode == "relative" {
        // absolute → relative：取当前环境的绝对路径，转为相对路径
        let abs_path = match entry.absolute_paths.get(&current_env_id)
            .filter(|p| !p.is_empty()) {
            Some(p) => p,
            None => return Err("当前环境没有绝对路径，无法转换".to_string()),
        };

        let abs_path_obj = Path::new(abs_path);

        // 检查文件是否存在
        if !abs_path_obj.exists() {
            return Ok(ConvertPathResult {
                success: false,
                message: "绝对路径对应的文件不存在，无法转换".to_string(),
                new_path_mode: entry.path_mode.clone(),
                new_relative_path: entry.relative_path.clone(),
                new_absolute_path: entry.absolute_paths.get(&current_env_id).cloned().unwrap_or_default(),
            });
        }

        // 同盘符时计算相对路径
        if let Some(rel) = make_relative_path(&exe_dir, abs_path_obj) {
            return Ok(ConvertPathResult {
                success: true,
                message: "已转换为相对路径".to_string(),
                new_path_mode: "relative".to_string(),
                new_relative_path: rel,
                new_absolute_path: String::new(),
            });
        } else {
            return Ok(ConvertPathResult {
                success: false,
                message: "文件不在同盘符内，无法转为相对路径".to_string(),
                new_path_mode: entry.path_mode.clone(),
                new_relative_path: entry.relative_path.clone(),
                new_absolute_path: entry.absolute_paths.get(&current_env_id).cloned().unwrap_or_default(),
            });
        }
    } else if target_mode == "absolute" {
        // relative → absolute：将 relative_path 拼接 exe_dir 得到绝对路径
        if entry.relative_path.is_empty() {
            return Ok(ConvertPathResult {
                success: false,
                message: "没有相对路径，无法转换".to_string(),
                new_path_mode: entry.path_mode.clone(),
                new_relative_path: entry.relative_path.clone(),
                new_absolute_path: entry.absolute_paths.get(&current_env_id).cloned().unwrap_or_default(),
            });
        }

        let full = exe_dir.join(&entry.relative_path);
        let exists = full.exists();
        // 词法规范化消除 ..\ 段——路径条目里存的应是规范绝对路径，
        // 否则后续图标提取等 shell API 会拒绝（0x80070057）
        let full = normalize_path(&full);

        return Ok(ConvertPathResult {
            success: true,
            message: if exists {
                "已转换为绝对路径".to_string()
            } else {
                "已转换为绝对路径（文件当前不存在）".to_string()
            },
            new_path_mode: "absolute".to_string(),
            new_relative_path: entry.relative_path.clone(),
            new_absolute_path: full.to_string_lossy().to_string(),
        });
    } else {
        Err(format!("不支持的目标模式: {}", target_mode))
    }
}

