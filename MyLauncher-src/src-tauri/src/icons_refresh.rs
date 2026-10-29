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
use crate::icons_system::*;
use crate::import::*;
use crate::launch::*;
use crate::models::*;
use crate::paths::*;
use crate::scan::*;
use crate::storage::*;


// ═══════════════════════════════════════════
// 图标重新提取（单条与批量）
// ═══════════════════════════════════════════

// ─── 图标重新提取 ───

/// 图标重提取结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IconRefreshResult {
    pub success: bool,
    /// 新图标相对路径（失败时为空）
    pub icon_path: String,
    /// 失败原因（成功时为空）
    pub message: String,
}

impl IconRefreshResult {
    fn ok(path: String) -> Self {
        IconRefreshResult { success: true, icon_path: path, message: String::new() }
    }
    fn err(msg: String) -> Self {
        IconRefreshResult { success: false, icon_path: String::new(), message: msg }
    }
}

/// 按条目类型分派图标重提取，返回新的图标相对路径（assets/ 相对路径）。
/// 不修改 JSON 数据（由前端保存），仅生成新图标文件。
///
/// force 语义：
/// - true  = 单条重置：url 强制绕过 domain 缓存重新下载（用户明确要刷新）
/// - false = 批量重置：url 命中 domain 缓存文件直接复用、不联网，
///           只为缺失/损坏的图标真正下载（批量几百条时绝大多数秒回）
pub(crate) fn refresh_icon_inner(entry: &Entry, current_env_id: &str, force: bool) -> Result<String, String> {
    match entry.entry_type.as_str() {
        "provider" => {
            // 平台游戏：与 program 一致，从解析出的主程序 exe 提取图标
            // （resolve_path_inner 对 provider 条目优先解析 absolute_paths 中的 exe 路径）
            let info = resolve_path_inner(entry, current_env_id)?;
            if !info.exists {
                return Err(format!("游戏主程序不存在: {}", info.resolved));
            }
            extract_icon_to_png(&info.resolved)
        }
        "program" | "folder" | "file" => {
            // 复用 resolve_path 的解析逻辑（相对 → 当前环境绝对 → fallback）
            let info = resolve_path_inner(entry, current_env_id)?;
            if !info.exists {
                return Err(format!("目标不存在: {}", info.resolved));
            }
            if entry.entry_type == "program" {
                extract_icon_to_png(&info.resolved)
            } else {
                // 文件夹/文件：沿用 get_file_info 的缓存逻辑（文件夹共用 folder.png，文件按扩展名缓存）
                let path = Path::new(&info.resolved);
                let cache_key = if path.is_dir() {
                    "folder".to_string()
                } else {
                    let ext = path.extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("none")
                        .to_lowercase();
                    format!("ext_{}", ext)
                };
                let cache_dir = get_assets_dir()?.join("icons").join("file_cache");
                ensure_dir(&cache_dir)?;
                let icon_file = cache_dir.join(format!("{}.png", cache_key));
                extract_icon_to_path(&info.resolved, &icon_file)?;
                Ok(rel_to_exe_dir(&icon_file))
            }
        }
        "url" => {
            // 网址类型：多源 favicon 链（各 2 秒超时）。
            // force=true 强制重新下载（单条重置语义）；
            // force=false 时 domain 缓存文件存在即复用（批量语义：已有图标不重复下载）。
            // 全部失败返回 Err，由调用方跳过该条目（保持原图标不变）
            if entry.url.trim().is_empty() {
                return Err("网址为空".to_string());
            }
            fetch_favicon_inner(&entry.url, force)
        }
        "appx" => {
            // 目标存于 relative_path（shell:AppsFolder\{family}!{appid}），app_key 与枚举时推导一致
            let shell_uri = entry.relative_path.trim();
            if shell_uri.is_empty() {
                return Err("APPX 启动目标为空".to_string());
            }
            let appid = shell_uri
                .strip_prefix("shell:AppsFolder\\")
                .unwrap_or(shell_uri);
            let app_key: String = appid
                .chars()
                .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
                .collect();
            // 重置语义 = 强制重新提取：先删旧缓存文件，inner 检测不到缓存即走真实提取路径
            let old_cache = get_assets_dir()?
                .join("icons")
                .join("extracted")
                .join(format!("appx_{}.png", app_key));
            let _ = fs::remove_file(&old_cache);
            extract_appx_icon_inner(&app_key, shell_uri)
        }
        // system 类型：有 icon_source 的按来源重新提取（重置语义 = force）；旧 emoji 条目跳过
        "system" => {
            let spec = entry.icon_source.trim();
            if spec.is_empty() {
                return Err("系统功能使用 Emoji 图标，无需重新提取".to_string());
            }
            extract_system_icon_cached(spec, true)
        }
        other => Err(format!("未知类型: {}", other)),
    }
}

/// extract_appx_icon 的内部封装（供命令与重提取共用，绕过缓存时先删旧缓存文件）
pub(crate) fn extract_appx_icon_inner(app_key: &str, shell_uri: &str) -> Result<String, String> {
    let assets_dir = get_assets_dir()?;
    let icon_dir = assets_dir.join("icons").join("extracted");
    ensure_dir(&icon_dir)?;
    let icon_file = icon_dir.join(format!("appx_{}.png", app_key));

    // 已有缓存直接返回（重提取场景下旧图标仍有效，避免重复提取开销）
    if icon_file.exists() {
        return Ok(rel_to_exe_dir(&icon_file));
    }

    extract_icon_to_path(shell_uri, &icon_file)?;
    Ok(rel_to_exe_dir(&icon_file))
}

/// 重新提取条目图标（单条）。
/// async + spawn_blocking：单条重置最坏要等 3 源 × 2 秒网络超时，
/// 不能跑在主线程（Tauri 同步命令默认主线程执行，会卡住整个 UI）。
#[command]
pub async fn refresh_entry_icon(entry: Entry, current_env_id: String, force: Option<bool>) -> Result<IconRefreshResult, String> {
    // 单条重置默认 force=true（绕过缓存）；批量路径可显式传 false 走 domain 缓存
    let force = force.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || match refresh_icon_inner(&entry, &current_env_id, force) {
        Ok(path) => Ok(IconRefreshResult::ok(path)),
        Err(e) => Ok(IconRefreshResult::err(e)),
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
}

// ─── 批量重新提取图标（多线程 + 进度事件） ───

/// 批量重置图标的单条结果
#[derive(Debug, Clone, Serialize)]
pub(crate) struct BatchIconItemResult {
    entry_id: String,
    entry_name: String,
    entry_type: String,
    /// success / skipped / failed
    status: String,
    /// 跳过或失败原因（成功时为空）
    message: String,
    /// 成功时的新图标相对路径（assets/ 相对路径）
    icon_path: String,
    /// 在输入列表中的原始下标（用于结果排序，不序列化）
    #[serde(skip_serializing)]
    entry_index: usize,
}

/// 批量重置图标的整体统计
#[derive(Debug, Clone, Serialize)]
pub(crate) struct BatchIconSummary {
    total: usize,
    success: usize,
    skipped: usize,
    failed: usize,
    results: Vec<BatchIconItemResult>,
}

/// 批量重新提取图标（后端 std::thread 多线程，逐条通过事件推送进度）。
/// 条目全部处理完毕后返回整体统计。
/// 规则与单条 refresh_entry_icon 一致：system 类型直接跳过；
/// url 类型 favicon 链全失败记为 skipped（保持原图标不变）；其余类型失败记 failed。
/// 事件：batch-icon-progress（单条完成，payload 含 done/total 与单条结果）。
///
/// ⚠️ 必须保持 async：Tauri 同步命令跑在主线程，本函数内部 join 全部工作线程，
/// 500 条 × 最坏 6 秒/条 会把主线程完全堵死（窗口假死、进度事件送不进前端）。
/// 用 spawn_blocking 把整个批量过程丢进阻塞线程池，主线程保持响应。
#[command]
pub async fn batch_refresh_icons(
    app: tauri::AppHandle,
    entries: Vec<Entry>,
    current_env_id: String,
    force: Option<bool>,
) -> Result<BatchIconSummary, String> {
    // 默认 force=false（命中本地缓存/已有图标直接复用）；
    // 前端勾选「强制」时 force=true，忽略缓存全部重新下载/提取
    let force = force.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || batch_refresh_icons_blocking(app, entries, current_env_id, force))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

/// 批量重置的实际执行体（阻塞，运行于阻塞线程池）。
fn batch_refresh_icons_blocking(
    app: tauri::AppHandle,
    entries: Vec<Entry>,
    current_env_id: String,
    force: bool,
) -> Result<BatchIconSummary, String> {
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let total = entries.len();
    if total == 0 {
        return Ok(BatchIconSummary { total: 0, success: 0, skipped: 0, failed: 0, results: Vec::new() });
    }

    let indexed: Vec<(usize, Entry)> = entries.into_iter().enumerate().collect();
    let results: Arc<Mutex<Vec<BatchIconItemResult>>> = Arc::new(Mutex::new(Vec::with_capacity(total)));
    let done_count = Arc::new(AtomicUsize::new(0));

    // 线程数：条目数与 8 取小（COM/Shell API 图标提取较重，过多线程收益递减）
    let thread_num = total.min(8).max(1);
    let chunk_size = (total + thread_num - 1) / thread_num;

    let mut handles = Vec::new();
    for chunk in indexed.chunks(chunk_size) {
        let results = Arc::clone(&results);
        let done_count = Arc::clone(&done_count);
        let env_id = current_env_id.clone();
        let app = app.clone();
        let chunk_vec = chunk.to_vec();
        handles.push(std::thread::spawn(move || {
            for (idx, entry) in chunk_vec {
                // COM 初始化：extract_icon_image 内部每次调用 com_init()，
                // 各工作线程独立初始化，无需额外处理
                let (status, message, icon_path) =
                    if entry.entry_type == "system" && entry.icon_source.trim().is_empty() {
                        // 旧 emoji 系统功能条目：无图标来源，跳过
                        ("skipped".to_string(), "系统功能使用 Emoji 图标，无需提取".to_string(), String::new())
                    } else {
                    // force 语义：false=命中 domain 缓存直接复用不重复下载；
                    // true=强制忽略缓存重新下载/提取（前端「强制」勾选框）
                    match refresh_icon_inner(&entry, &env_id, force) {
                        Ok(path) => ("success".to_string(), String::new(), path),
                        Err(e) if entry.entry_type == "url" =>
                            // url 类型 favicon 链全失败：跳过，保持原图标不变
                            ("skipped".to_string(), e, String::new()),
                        Err(e) => ("failed".to_string(), e, String::new()),
                    }
                };
                let item = BatchIconItemResult {
                    entry_id: entry.id.clone(),
                    entry_name: entry.name.clone(),
                    entry_type: entry.entry_type.clone(),
                    status: status.clone(),
                    message: message.clone(),
                    icon_path: icon_path.clone(),
                    entry_index: idx,
                };
                {
                    let mut list = results.lock().unwrap();
                    list.push(item);
                }
                let done = done_count.fetch_add(1, Ordering::SeqCst) + 1;
                // 逐条推送进度（前端弹窗列表实时展示）
                let _ = app.emit("batch-icon-progress", serde_json::json!({
                    "done": done,
                    "total": total,
                    "entry_id": entry.id,
                    "entry_name": entry.name,
                    "entry_type": entry.entry_type,
                    "status": status,
                    "message": message,
                    "icon_path": icon_path,
                }));
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }

    let mut sorted = Arc::try_unwrap(results)
        .map_err(|_| "结果收集失败".to_string())?
        .into_inner()
        .unwrap();
    // 恢复与输入一致的顺序
    sorted.sort_by_key(|r| r.entry_index);

    let success = sorted.iter().filter(|r| r.status == "success").count();
    let skipped = sorted.iter().filter(|r| r.status == "skipped").count();
    let failed = sorted.iter().filter(|r| r.status == "failed").count();

    Ok(BatchIconSummary { total, success, skipped, failed, results: sorted })
}

/// 为 APPX 应用提取图标（从 shell:AppsFolder URI），返回图标相对路径
#[command]
pub fn extract_appx_icon(app_key: String, shell_uri: String) -> Result<String, String> {
    let assets_dir = get_assets_dir()?;
    let icon_dir = assets_dir.join("icons").join("extracted");
    ensure_dir(&icon_dir)?;

    let icon_file = icon_dir.join(format!("appx_{}.png", app_key));

    // 已有缓存直接返回
    if icon_file.exists() {
        return Ok(rel_to_exe_dir(&icon_file));
    }

    extract_icon_to_path(&shell_uri, &icon_file)?;
    Ok(rel_to_exe_dir(&icon_file))
}

