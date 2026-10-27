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
use crate::paths::*;
use crate::scan::*;
use crate::storage::*;


// ═══════════════════════════════════════════
// 资源文件操作与过期资源清理
// ═══════════════════════════════════════════

// ─── 文件操作 ───

#[command]
pub fn copy_file_to_assets(source_path: String, target_subdir: String) -> Result<String, String> {
    let src = Path::new(&source_path);
    if !src.exists() {
        return Err(format!("源文件不存在: {}", source_path));
    }

    let assets_dir = get_assets_dir()?;
    let target_dir = assets_dir.join(&target_subdir);
    ensure_dir(&target_dir)?;

    // 生成目标文件名
    let ext = src.extension().and_then(|s| s.to_str()).unwrap_or("png");
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let safe_stem: String = stem.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    let timestamp = chrono::Local::now().format("%H%M%S%3f").to_string();
    let target_name = format!("{}_{}.{}", safe_stem, timestamp, ext);
    let target_path = target_dir.join(&target_name);

    fs::copy(&source_path, &target_path)
        .map_err(|e| format!("复制文件失败: {}", e))?;

    Ok(rel_to_exe_dir(&target_path))
}


// ─── 过期资源清理 ───

/// 清理扫描/清理结果
#[derive(Debug, Clone, Serialize)]
pub struct AssetCleanResult {
    /// 过期文件数
    pub orphan_count: usize,
    /// 过期文件总字节数
    pub orphan_bytes: u64,
    /// 实际删除的文件数（仅 clean_assets 有值）
    pub deleted_count: usize,
    /// 实际释放的字节数（仅 clean_assets 有值）
    pub deleted_bytes: u64,
}

/// 收集所有 JSON 数据中对 assets 资源的引用（相对路径集合）。
/// 覆盖四处引用：条目图标、条目封面、分类图标、LOGO 图标/图片。
/// 注意：封面即使 enabled=false，只要 source 非空也算被引用（编辑器只有点"清除"才清空 source）。
///
/// 健壮性设计（防止误删）：
/// 1. 任何 JSON 读取/解析失败立即报错中止——引用收集不全时清理会误删在用文件；
/// 2. 兜底文件名匹配：除完整路径匹配外，同时记录引用文件名，扫描时"路径或文件名任一命中即算被引用"，
///    兼容旧格式/异常格式路径（如反斜杠、缺少 assets/ 前缀）。
pub(crate) fn collect_asset_refs() -> Result<(std::collections::HashSet<String>, std::collections::HashSet<String>), String> {
    let data_dir = get_data_dir()?;
    let mut refs = std::collections::HashSet::new();
    let mut ref_names = std::collections::HashSet::new();

    let push = |source: &str, refs: &mut std::collections::HashSet<String>, ref_names: &mut std::collections::HashSet<String>| {
        let s = source.trim();
        if s.is_empty() {
            return;
        }
        // 统一转正斜杠后记录完整路径
        let norm = s.replace('\\', "/");
        refs.insert(norm);
        // 兜底：同时记录纯文件名（取最后一段）
        if let Some(name) = s.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next() {
            if !name.is_empty() {
                ref_names.insert(name.to_string());
            }
        }
    };

    let entries: Vec<Entry> = read_json_file(&data_dir.join("entries.json"))
        .map_err(|e| format!("引用收集中止（防止误删）：{}", e))?;
    for e in &entries {
        push(&e.icon.source, &mut refs, &mut ref_names);
        push(&e.cover.source, &mut refs, &mut ref_names);
    }

    let categories: Vec<Category> = read_json_file(&data_dir.join("categories.json"))
        .map_err(|e| format!("引用收集中止（防止误删）：{}", e))?;
    for c in &categories {
        push(&c.icon.source, &mut refs, &mut ref_names);
    }

    let config: AppConfig = read_json_file(&data_dir.join("config.json"))
        .map_err(|e| format!("引用收集中止（防止误删）：{}", e))?;
    if config.logo_icon_type == "custom" {
        push(&config.logo_icon, &mut refs, &mut ref_names);
    }
    push(&config.logo_image, &mut refs, &mut ref_names);

    Ok((refs, ref_names))
}

/// 扫描清理范围内未被引用的过期文件（白名单 6 个子目录，不碰 assets 根部的前端资源）。
/// 返回 (过期文件列表, 过期总字节数)。
///
/// 安全防线：
/// 1. 完整路径匹配（规范化正斜杠）；
/// 2. 文件名兜底匹配——任一 JSON 中出现过同名文件即视为被引用；
/// 3. 防呆拒清：引用集合为空但磁盘上有文件时，视为数据异常，拒绝清理并报错。
pub(crate) fn find_orphan_assets() -> Result<(Vec<std::path::PathBuf>, u64), String> {
    let assets_dir = get_assets_dir()?;
    let (refs, ref_names) = collect_asset_refs()?;

    // 清理范围白名单（相对 assets 的子目录）
    // 封面子目录名由插件 id 决定（covers/custom 与 covers/<provider_id>），动态发现，
    // 不在这里硬编码任何平台名——宿主不认识平台的规则同样适用于资源清理
    let mut scopes: Vec<String> = vec![
        "icons/extracted".to_string(),
        "icons/custom".to_string(),
        "icons/bookmarks".to_string(),
        "icons/file_cache".to_string(),
        "logo".to_string(),
    ];
    if let Ok(entries) = std::fs::read_dir(assets_dir.join("covers")) {
        for item in entries.flatten() {
            if !item.path().is_dir() {
                continue;
            }
            if let Some(name) = item.file_name().to_str() {
                scopes.push(format!("covers/{}", name));
            }
        }
    }

    // 先数一遍磁盘文件总数，用于防呆判断
    let mut disk_file_count = 0usize;
    for sub in &scopes {
        let dir = assets_dir.join(sub);
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            disk_file_count += entries.flatten().filter(|i| i.path().is_file()).count();
        }
    }

    // 防呆拒清：引用为空但磁盘有文件，说明 JSON 数据异常（解析失败已提前报错，
    // 这里兜底空文件等极端情况），宁可拒绝清理也不能全删
    if refs.is_empty() && ref_names.is_empty() && disk_file_count > 0 {
        return Err(format!(
            "引用数据为空但磁盘上存在 {} 个资源文件，疑似数据异常，已拒绝清理以防误删",
            disk_file_count
        ));
    }

    let mut orphans = Vec::new();
    let mut total_bytes = 0u64;

    for sub in &scopes {
        let dir = assets_dir.join(sub);
        if !dir.is_dir() {
            continue;
        }
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("读取目录失败 {}: {}", sub, e))?;
        for item in entries.flatten() {
            let path = item.path();
            if !path.is_file() {
                continue;
            }
            // 转成 assets/xxx/yyy.png 形式与引用集合比对
            let rel = path.strip_prefix(&assets_dir).unwrap_or(&path);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            // 双重匹配：完整路径命中 或 文件名兜底命中，均视为被引用
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let is_referenced = refs.contains(&rel_str) || (!file_name.is_empty() && ref_names.contains(file_name));
            if !is_referenced {
                if let Ok(meta) = item.metadata() {
                    total_bytes += meta.len();
                }
                orphans.push(path);
            }
        }
    }

    Ok((orphans, total_bytes))
}

/// 扫描过期资源（无副作用，仅统计）
#[command]
pub fn scan_orphan_assets() -> Result<AssetCleanResult, String> {
    let (orphans, bytes) = find_orphan_assets()?;
    Ok(AssetCleanResult {
        orphan_count: orphans.len(),
        orphan_bytes: bytes,
        deleted_count: 0,
        deleted_bytes: 0,
    })
}

/// 将文件移入回收站（SHFileOperationW + FOF_ALLOWUNDO）。
/// paths 为绝对路径列表，返回失败文件描述列表（空表示全部成功）。
/// 注意：pFrom 必须是双 \0 结尾的宽字符串，多个路径用 \0 分隔。
#[cfg(windows)]
pub(crate) fn recycle_files(paths: &[std::path::PathBuf]) -> Vec<String> {
    use std::os::windows::ffi::OsStrExt;

    if paths.is_empty() {
        return Vec::new();
    }

    // 构造双 \0 结尾的多路径宽字符串（\0 分隔）
    let mut from: Vec<u16> = Vec::new();
    for p in paths {
        for c in p.as_os_str().encode_wide() {
            from.push(c);
        }
        from.push(0);
    }
    from.push(0);

    let mut op = SHFILEOPSTRUCTW {
        hwnd: HWND::default(),
        wFunc: 3, // FO_DELETE
        pFrom: PCWSTR(from.as_ptr()),
        pTo: PCWSTR::null(),
        // FOF_ALLOWUNDO(64) 进回收站 | FOF_NOCONFIRMATION(16) 不弹确认 | FOF_SILENT(4) 不显示进度
        fFlags: (64 | 16 | 4) as u16,
        fAnyOperationsAborted: BOOL::default(),
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: PCWSTR::null(),
    };

    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 {
        // 整体失败：列出所有路径供用户排查
        return paths.iter()
            .map(|p| format!("{}: SHFileOperation 错误码 {}", p.display(), code))
            .collect();
    }
    // 部分被用户中止也算失败
    if op.fAnyOperationsAborted.as_bool() {
        return vec!["操作被中止".to_string()];
    }
    Vec::new()
}


/// 清理过期资源（重新扫描后移入回收站，返回实际清理结果）
#[command]
pub fn clean_orphan_assets() -> Result<AssetCleanResult, String> {
    let (orphans, bytes) = find_orphan_assets()?;

    // 统计总大小（回收站删除前统计）
    let mut deleted_bytes = 0u64;
    for path in &orphans {
        if let Ok(meta) = std::fs::metadata(path) {
            deleted_bytes += meta.len();
        }
    }

    let failures = recycle_files(&orphans);
    Ok(AssetCleanResult {
        orphan_count: orphans.len(),
        orphan_bytes: bytes,
        deleted_count: orphans.len() - failures.len(),
        deleted_bytes: if failures.is_empty() { deleted_bytes } else { 0 },
    })
}

/// 资源删除白名单：固定资源目录 + `covers/<子目录>/文件`。
/// 封面子目录名来自插件 id，不枚举具体平台；只允许一层目录，防 `../` 越界。
fn is_allowed_asset_path(src: &str) -> bool {
    const FIXED: [&str; 6] = [
        "assets/icons/extracted/",
        "assets/icons/custom/",
        "assets/icons/bookmarks/",
        "assets/icons/file_cache/",
        "assets/covers/custom/",
        "assets/logo/",
    ];
    if FIXED.iter().any(|p| src.starts_with(p)) {
        return true;
    }
    if let Some(rest) = src.strip_prefix("assets/covers/") {
        let mut parts = rest.split('/');
        let dir = parts.next().unwrap_or("");
        let file = parts.next().unwrap_or("");
        let extra = parts.next();
        return !dir.is_empty()
            && !file.is_empty()
            && extra.is_none()
            && !dir.contains("..")
            && !dir.contains('\\');
    }
    false
}

/// 删除条目后的资源联动清理：
/// 前端删除条目并保存 JSON 后调用，传入被删条目的图标/封面相对路径列表。
/// 复用 collect_asset_refs 重建「删除后」的全量引用集合，候选文件只有
/// 「完整路径或文件名均不再被引用」才移入回收站（复用 recycle_files，可还原）。
/// domain 缓存图标（{domain}.png）被同 domain 其他链接引用时按文件名命中，不会误删。
/// 返回实际移入回收站的文件数（失败/路径不在 assets 内的不计入）。
#[command]
pub fn cleanup_deleted_assets(sources: Vec<String>) -> Result<usize, String> {
    // 过滤空路径
    let candidates: Vec<String> = sources
        .into_iter()
        .map(|s| s.trim().replace('\\', "/"))
        .filter(|s| !s.is_empty())
        .collect();
    if candidates.is_empty() {
        return Ok(0);
    }

    // 白名单前缀：只允许删 assets 下资源目录内的文件，防误传任意路径

    let assets_dir = get_assets_dir()?;

    // 收集「删除后」的最新引用集合（条目/分类/LOGO 全量 JSON）
    let (refs, ref_names) = collect_asset_refs()?;

    let mut to_recycle: Vec<std::path::PathBuf> = Vec::new();
    for src in &candidates {
        // 仅处理 assets 白名单前缀内的路径
        if !is_allowed_asset_path(src) {
            continue;
        }
        // 拼绝对路径（词法规范化，防 .. 段）
        let abs = assets_dir.join(
            src.strip_prefix("assets/")
                .unwrap_or(src),
        );
        let abs = normalize_path(&abs);
        // 确认文件仍存在
        if !abs.is_file() {
            continue;
        }
        // 引用判断：完整路径或文件名任一命中即视为仍被引用，跳过
        let file_name = abs.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if refs.contains(src) || (!file_name.is_empty() && ref_names.contains(file_name)) {
            continue;
        }
        to_recycle.push(abs);
    }

    if to_recycle.is_empty() {
        return Ok(0);
    }

    let failures = recycle_files(&to_recycle);
    Ok(to_recycle.len() - failures.len())
}

