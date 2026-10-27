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
use crate::launch::*;
use crate::models::*;
use crate::paths::*;
use crate::scan::*;
use crate::storage::*;


// ═══════════════════════════════════════════
// 扫描结果批量导入
// ═══════════════════════════════════════════

// ─── 扫描结果导入（多线程） ───

/// 批量导入扫描结果的整体统计（后端命令返回值）
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ImportScanSummary {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    /// 新导入并已写盘的 Entry（含 id，前端直接同步内存状态）
    pub entries: Vec<Entry>,
}

/// 批量导入扫描结果：多线程提取图标（程序/平台游戏通用）、落盘平台封面，
/// 构造 Entry 后统一追加写盘一次（避免前端逐条串行）。
/// 线程模型与 batch_refresh_icons 一致（std::thread，线程数 = min(条目数, 8)）。
/// 逐条推送 import-progress 事件（payload：done/total/name/status/message）。
#[command]
pub fn import_scan_items(
    app: tauri::AppHandle,
    items: Vec<ExeInfo>,
    category_id: String,
    current_env_id: String,
) -> Result<ImportScanSummary, String> {
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let total = items.len();
    if total == 0 {
        return Ok(ImportScanSummary { total: 0, success: 0, failed: 0, entries: Vec::new() });
    }

    let indexed: Vec<(usize, ExeInfo)> = items.into_iter().enumerate().collect();
    let results: Arc<Mutex<Vec<(usize, Entry)>>> = Arc::new(Mutex::new(Vec::with_capacity(total)));
    let failed_flags: Arc<Mutex<Vec<(usize, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let done_count = Arc::new(AtomicUsize::new(0));

    // 线程数：条目数与 8 取小（COM/Shell 图标提取与 CDN 下载较重，过多线程收益递减）
    let thread_num = total.min(8).max(1);
    let chunk_size = (total + thread_num - 1) / thread_num;

    let mut handles = Vec::new();
    for chunk in indexed.chunks(chunk_size) {
        let results = Arc::clone(&results);
        let failed_flags = Arc::clone(&failed_flags);
        let done_count = Arc::clone(&done_count);
        let category_id = category_id.clone();
        let env_id = current_env_id.clone();
        let app = app.clone();
        let chunk_vec = chunk.to_vec();
        handles.push(std::thread::spawn(move || {
            for (idx, item) in chunk_vec {
                // COM 初始化：extract_icon_image 内部每次调用 com_init()，
                // 各工作线程独立初始化，无需额外处理
                let provider = item.provider.clone();
                let exe_path = item.absolute_path.clone();

                // 图标提取（失败不影响导入，使用默认图标）
                let icon_path = if exe_path.is_empty() {
                    String::new()
                } else {
                    extract_icon_to_png(&exe_path).unwrap_or_default()
                };

                // 平台封面落盘（来源由扫描结果携带：本地复制优先，缺失走下载，失败静默）
                let cover_path = match (&provider, &item.cover) {
                    (Some(p), Some(c)) => crate::plugin_host::obtain_provider_cover(c, &p.id, &p.item_id),
                    _ => String::new(),
                };

                let now = get_now_iso();
                let mut failed_msg = String::new();

                let entry = if let Some(p) = &provider {
                    // 平台游戏：启动链接存 url（steam:// 等，系统协议处理），
                    // exe 路径仅作「直接运行」与定位用
                    if exe_path.is_empty() && p.launch_url.is_empty() {
                        failed_msg = "游戏 exe 路径与启动链接均为空".to_string();
                    }
                    Entry {
                        id: String::new(),
                        name: item.name.clone(),
                        entry_type: "provider".to_string(),
                        category_id: category_id.clone(),
                        relative_path: String::new(),
                        absolute_paths: if exe_path.is_empty() {
                            HashMap::new()
                        } else {
                            let mut m = HashMap::new();
                            m.insert(env_id.clone(), exe_path.clone());
                            m
                        },
                        path_mode: "absolute".to_string(),
                        launch_args: String::new(),
                        working_directory: String::new(),
                        window_style: "normal".to_string(),
                        run_as_admin: false,
                        url: p.launch_url.clone(),
                        provider: Some(ProviderRef {
                            id: p.id.clone(),
                            item_id: p.item_id.clone(),
                            launch_url: String::new(),
                            meta: p.meta.clone(),
                        }),
                        update_url: String::new(),
                        icon: if icon_path.is_empty() {
                            ImageResource { res_type: "default".to_string(), source: String::new() }
                        } else {
                            ImageResource { res_type: "extracted".to_string(), source: icon_path.clone() }
                        },
                        icon_source: String::new(),
                        cover: if cover_path.is_empty() {
                            CoverResource { enabled: false, source: String::new() }
                        } else {
                            CoverResource { enabled: true, source: cover_path }
                        },
                        tags: Vec::new(),
                        notes: String::new(),
                        last_used: String::new(),
                        add_time: now,
                    }
                } else {
                    if item.absolute_path.is_empty() {
                        failed_msg = "程序路径为空".to_string();
                    }
                    Entry {
                        id: String::new(),
                        name: item.name.clone(),
                        entry_type: "program".to_string(),
                        category_id: category_id.clone(),
                        relative_path: item.relative_path.clone(),
                        absolute_paths: if item.suggested_path_mode == "absolute" && !item.absolute_path.is_empty() {
                            let mut m = HashMap::new();
                            m.insert(env_id.clone(), item.absolute_path.clone());
                            m
                        } else {
                            HashMap::new()
                        },
                        path_mode: item.suggested_path_mode.clone(),
                        launch_args: String::new(),
                        working_directory: String::new(),
                        window_style: "normal".to_string(),
                        run_as_admin: false,
                        url: String::new(),
                        provider: None,
                        update_url: String::new(),
                        icon: if icon_path.is_empty() {
                            ImageResource { res_type: "default".to_string(), source: String::new() }
                        } else {
                            ImageResource { res_type: "extracted".to_string(), source: icon_path.clone() }
                        },
                        icon_source: String::new(),
                        cover: CoverResource { enabled: false, source: String::new() },
                        tags: Vec::new(),
                        notes: String::new(),
                        last_used: String::new(),
                        add_time: now,
                    }
                };

                let status = if failed_msg.is_empty() { "success" } else { "failed" };
                let message = if !failed_msg.is_empty() {
                    failed_msg.clone()
                } else if icon_path.is_empty() && !exe_path.is_empty() {
                    "图标提取失败，使用默认图标".to_string()
                } else {
                    String::new()
                };

                if failed_msg.is_empty() {
                    let mut list = results.lock().unwrap();
                    list.push((idx, entry));
                } else {
                    let mut list = failed_flags.lock().unwrap();
                    list.push((idx, failed_msg));
                }

                let done = done_count.fetch_add(1, Ordering::SeqCst) + 1;
                // 逐条推送进度（前端导入进度弹窗实时展示）
                let _ = app.emit("import-progress", serde_json::json!({
                    "done": done,
                    "total": total,
                    "name": item.name,
                    "status": status,
                    "message": message,
                }));
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }

    // 汇总：按输入顺序排列，统一分配 id 后追加写盘一次
    let mut new_entries = Arc::try_unwrap(results)
        .map_err(|_| "结果收集失败".to_string())?
        .into_inner()
        .unwrap();
    new_entries.sort_by_key(|(idx, _)| *idx);
    let mut new_entries: Vec<Entry> = new_entries.into_iter().map(|(_, e)| e).collect();

    let data_dir = get_data_dir()?;
    let entries_path = data_dir.join("entries.json");
    let mut existing: Vec<Entry> = read_json_file(&entries_path).unwrap_or_default();
    let mut next_id = existing.iter()
        .filter_map(|e| e.id.strip_prefix("entry_").and_then(|s| s.parse::<u32>().ok()))
        .max()
        .unwrap_or(0) + 1;
    for e in &mut new_entries {
        e.id = format!("entry_{}", next_id);
        next_id += 1;
    }
    let success = new_entries.len();
    existing.append(&mut new_entries);
    write_json_file(&entries_path, &existing)?;

    let failed = total - success;
    Ok(ImportScanSummary { total, success, failed, entries: existing[existing.len() - success..].to_vec() })
}

