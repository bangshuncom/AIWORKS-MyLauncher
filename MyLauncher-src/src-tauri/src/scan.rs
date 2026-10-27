// ═══════════════════════════════════════════
// 目录扫描入口
//
// 宿主不认识任何游戏平台：目录扫描只有两条路径——
//   1. 有插件认领（manifest 的 dir_markers 全部存在且插件已启用）→ 交给插件
//   2. 没有插件认领 → 普通递归扫 exe
// 平台特征（steamapps / appmanifest / 注册表路径等）全部在插件里，
// 主程序一旦出现某个平台的名字，插件化就失去意义。
// ═══════════════════════════════════════════

use tauri::{command, Emitter};
use std::path::Path;

use crate::models::*;
use crate::paths::*;

/// 扫描目录（平台插件认领的目录 / 普通文件夹）。
/// 扫描阶段不提取图标、不下载封面（保证速度），仅收集文件清单；
/// 图标/封面等重操作移到导入命令 import_scan_items 中执行。
/// 异步执行并推送 scan-progress 事件（前端进度弹窗实时展示）：
///   - phase="walking"：遍历文件阶段，payload 含 found/done/total
/// 事件名：scan-progress
#[command]
pub async fn scan_directory(app: tauri::AppHandle, dir_path: String, max_depth: Option<usize>) -> Result<ScanResult, String> {
    let path = Path::new(&dir_path);
    if !path.exists() || !path.is_dir() {
        return Err(format!("目录不存在: {}", dir_path));
    }

    // 1. 插件探测：有已启用插件认领该目录时，整个目录交给插件扫描。
    //    None = 没有任何插件认领 → 落到普通扫描（插件缺失或已停用都是这个结果，
    //    宿主不保留任何"看起来像某平台库"的内置识别）
    if let Some(outcome) = crate::plugin_host::scan_dir_via_plugins(path) {
        let _ = app.emit("scan-progress", serde_json::json!({
            "phase": "walking",
            "found": 0,
            "done": 0,
            "total": 0,
        }));
        let outcome = outcome.map_err(|e| format!("插件扫描失败: {}", e))?;
        let total = outcome.items.len();
        return Ok(ScanResult { results: outcome.items, total, warnings: outcome.warnings });
    }

    // 2. 普通递归扫描
    let depth = max_depth.unwrap_or(3);
    let exe_dir = get_exe_directory()?;
    let mut results = Vec::new();

    // 阶段一：遍历目录收集 exe 文件（全程推送 found 计数）
    let mut walk_paths = Vec::new();
    for entry in walkdir::WalkDir::new(&dir_path)
        .max_depth(depth)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        if ext != "exe" {
            continue;
        }
        let abs_path = path.to_string_lossy().to_string();
        walk_paths.push(abs_path);
        // 每发现一个 exe 推送一次（前端进度文字「正在扫描目录… 已发现 N 个程序」）
        let _ = app.emit("scan-progress", serde_json::json!({
            "phase": "walking",
            "found": walk_paths.len(),
            "done": 0,
            "total": 0,
        }));
    }

    let total = walk_paths.len();
    let mut done = 0usize;

    // 阶段二：逐条解析名称与相对路径（图标提取移到导入阶段，保证扫描速度）
    for abs_path in &walk_paths {
        let path = Path::new(abs_path);
        let name = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("未知")
            .to_string();

        let (suggested_mode, rel_path) = if let Some(rel) = make_relative_path(&exe_dir, path) {
            ("relative", rel)
        } else {
            ("absolute", String::new())
        };

        results.push(ExeInfo {
            name: name.clone(),
            icon_path: String::new(),
            suggested_path_mode: suggested_mode.to_string(),
            relative_path: rel_path,
            absolute_path: abs_path.clone(),
            file_exists: true,
            provider: None,
            cover: None,
            multi_exe: false,
            is_primary: true,
        });

        done += 1;
        let _ = app.emit("scan-progress", serde_json::json!({
            "phase": "walking",
            "found": total,
            "done": done,
            "total": total,
            "current": name,
        }));
    }

    Ok(ScanResult { results, total, warnings: Vec::new() })
}
