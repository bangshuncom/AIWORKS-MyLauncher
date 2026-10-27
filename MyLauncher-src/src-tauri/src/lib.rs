// ════════════════════════════════════════════
// MyLauncher - Rust 后端实现
// 绿色便携程序管理器
//
// 本文件只做三件事：声明模块、持有应用入口 run()、装配系统托盘。
// 具体业务按职责分散到各模块，见下方 mod 声明。
// ════════════════════════════════════════════

#![allow(unused_imports)]

mod models;
mod defaults;
mod paths;
mod storage;
mod launch;
mod icons;
mod icons_refresh;
mod icons_system;
mod scan;
mod plugin_host;
mod import;
mod favicon;
mod bookmarks;
mod assets;
mod autorun;
mod lang;
mod window;

use tauri::{command, Emitter, Manager};
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

use crate::models::*;
use crate::defaults::*;
use crate::paths::*;
use crate::storage::*;
use crate::launch::*;
use crate::icons::*;
use crate::icons_refresh::*;
use crate::icons_system::*;
use crate::scan::*;
use crate::plugin_host::*;
use crate::import::*;
use crate::favicon::*;
use crate::bookmarks::*;
use crate::assets::*;
use crate::autorun::*;
use crate::lang::*;
use crate::window::*;

// ════════════════════════════════════════════
// 应用入口
// ════════════════════════════════════════════

/// 托盘菜单项 ID
const TRAY_MENU_SHOW: &str = "show";
const TRAY_MENU_QUIT: &str = "quit";

/// 托盘"显示/隐藏"菜单项文案随窗口状态切换：窗口可用→"最小化到托盘"，否则→"恢复窗口"
const TRAY_TEXT_MINIMIZE: &str = "最小化到托盘";
const TRAY_TEXT_RESTORE: &str = "恢复窗口";

/// 托盘共享状态：持有"显示/隐藏"菜单项，供窗口事件等闭包跨作用域更新文案
struct TrayState {
    show_item: tauri::menu::MenuItem<tauri::Wry>,
}

/// 主窗口是否处于"可用"状态：可见且未最小化
fn main_window_usable(app: &tauri::AppHandle) -> bool {
    app.get_webview_window("main")
        .map(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false))
        .unwrap_or(false)
}

/// 同步托盘"显示/隐藏"菜单项文案，使其与当前窗口状态一致
fn sync_tray_show_text(app: &tauri::AppHandle, item: &tauri::menu::MenuItem<tauri::Wry>) {
    let text = if main_window_usable(app) { TRAY_TEXT_MINIMIZE } else { TRAY_TEXT_RESTORE };
    let _ = item.set_text(text);
}

/// 托盘双击 / 菜单点击的统一切换逻辑：窗口可用 → 最小化到托盘（隐藏）；否则 → 恢复显示
fn toggle_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let visible = win.is_visible().unwrap_or(false);
        let minimized = win.is_minimized().unwrap_or(false);
        if visible && !minimized {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.unminimize();
            let _ = win.set_focus();
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            use tauri::{
                menu::{Menu, MenuItem},
                tray::{TrayIconBuilder, TrayIconEvent},
                Manager,
            };

            // ─── 系统托盘：图标 + 右键菜单 ───
            // "显示/隐藏"菜单项文案随窗口状态切换：可用→"最小化到托盘"，否则→"恢复窗口"
            let show_item = MenuItem::with_id(app, TRAY_MENU_SHOW, TRAY_TEXT_RESTORE, true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, TRAY_MENU_QUIT, "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            // 各闭包 / 定时器分别持有一份菜单项引用（MenuItem 可 Clone 且 Send + Sync）
            let show_item_menu = show_item.clone();
            let show_item_tray = show_item.clone();
            let show_item_timer = show_item.clone();

            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| {
                    match event.id().as_ref() {
                        // 显示/隐藏切换：与双击一致——窗口可用则最小化到托盘，否则恢复窗口
                        TRAY_MENU_SHOW => {
                            toggle_main_window(app);
                            sync_tray_show_text(app, &show_item_menu);
                        }
                        // 退出：清理托盘图标后真正退出
                        TRAY_MENU_QUIT => {
                            if let Some(tray) = app.tray_by_id("main-tray") {
                                let _ = tray.set_visible(false);
                            }
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(move |_tray, event| {
                    let app = _tray.app_handle();
                    // 手势统一为「左键双击」：窗口可用 ↔ 最小化到托盘 之间切换。
                    // 为什么单击不响应：Windows 托盘双击会依次产生
                    // Click → DoubleClick → Click 三个事件——若单击也绑定行为
                    // （如恢复窗口），双击末尾的 Click 会把切换效果撤销，
                    // 且「单击切换」方案下双击会触发两次切换互相抵消并伴随窗口闪烁。
                    // 因此单击故意不绑定任何操作，只认双击。
                    if let TrayIconEvent::DoubleClick { button: tauri::tray::MouseButton::Left, .. } = event {
                        toggle_main_window(app);
                        sync_tray_show_text(app, &show_item_tray);
                    }
                })
                .build(app)?;

            // ─── 轮询窗口状态，同步托盘菜单文案 ───
            // 说明：tauri 的 WindowEvent 不转发"最小化/还原"事件，双击切换、菜单点击、
            // 关闭隐藏都会显式同步文案；但用户直接点任务栏最小化时只有轮询能感知到。
            // 800ms 一次状态查询开销可忽略，且仅在状态变化时才写菜单。
            // （项目未引 tokio，用独立线程阻塞 sleep，比拉 async 依赖轻）
            let timer_app = app.handle().clone();
            std::thread::spawn(move || {
                let mut last_usable: Option<bool> = None;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    let usable = main_window_usable(&timer_app);
                    if last_usable != Some(usable) {
                        last_usable = Some(usable);
                        sync_tray_show_text(&timer_app, &show_item_timer);
                    }
                }
            });

            // ─── 启动显示策略 ───
            // 窗口在 tauri.conf.json 中默认不可见；手动双击启动（无 --hidden 参数）显示主窗口，
            // 开机自启静默模式（带 --hidden 参数）保持隐藏只留托盘
            if let Some(win) = app.get_webview_window("main") {
                let silent_start = std::env::args().any(|a| a == AUTORUN_HIDDEN_ARG);
                if !silent_start {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
            // 初始化托盘菜单文案（静默启动为"恢复窗口"，正常启动为"最小化到托盘"）
            sync_tray_show_text(&app.handle(), &show_item);

            // ─── 自启路径校正：程序移动位置后用当前路径重写注册表（保留静默标记）───
            fix_autorun_registry();

            // 托盘菜单项存入 app state，供 setup 外的窗口事件闭包更新文案
            app.manage(TrayState { show_item });

            Ok(())
        })
        .on_window_event(|window, event| {
            use tauri::Manager;
            // 主窗口关闭请求 → 按设置分流
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    // 读取关闭行为设置（每次关闭时读取，开销可接受）
                    let close_action = get_data_dir()
                        .ok()
                        .and_then(|dir| read_json_file::<AppConfig>(&dir.join("config.json")).ok())
                        .map(|cfg| cfg.close_action)
                        .unwrap_or_else(default_close_action);

                    if close_action == "close" {
                        // 直接退出：清理托盘图标后关闭窗口（触发默认退出流程）
                        if let Some(tray) = window.app_handle().tray_by_id("main-tray") {
                            let _ = tray.set_visible(false);
                        }
                        // 不调用 prevent_close，让窗口正常关闭退出
                    } else {
                        // 默认：隐藏到托盘
                        api.prevent_close();
                        let _ = window.hide();
                        // 同步托盘菜单文案为"恢复窗口"
                        if let Some(tray_state) = window.try_state::<TrayState>() {
                            sync_tray_show_text(window.app_handle(), &tray_state.show_item);
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            init_data_dir,
            load_all_data,
            save_entries,
            save_categories,
            save_environments,
            save_config,
            get_exe_dir,
            get_now,
            resolve_path,
            check_paths_batch,
            launch_program,
            open_external_url,
            get_exe_info,
            get_file_info,
            resolve_lnk,
            scan_directory,
            list_plugins,
            scan_provider_item,
            open_plugins_dir,
            import_scan_items,
            convert_path_mode,
            copy_file_to_assets,
            fetch_favicon,
            parse_bookmarks,
            list_appx_apps,
            extract_appx_icon,
            extract_system_icon,
            open_in_explorer,
            scan_orphan_assets,
            clean_orphan_assets,
            cleanup_deleted_assets,
            refresh_entry_icon,
            batch_refresh_icons,
            set_autorun,
            get_autorun,
            init_lang_dir,
            list_languages,
            read_language,
            open_lang_dir,
            set_window_title
        ])
        .run(tauri::generate_context!())
        .expect("MyLauncher 启动失败");
}
