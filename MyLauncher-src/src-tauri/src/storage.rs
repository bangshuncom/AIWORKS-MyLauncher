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
use crate::lang::*;
use crate::launch::*;
use crate::models::*;
use crate::paths::*;
use crate::scan::*;


// ═══════════════════════════════════════════
// 数据持久化（JSON 读写与数据目录）
// ═══════════════════════════════════════════

pub(crate) fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    if !path.exists() {
        return Err(format!("文件不存在: {:?}", path));
    }
    let content = fs::read_to_string(path)
        .map_err(|e| format!("读取文件失败 {:?}: {}", path, e))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("解析 JSON 失败 {:?}: {}", path, e))
}

pub(crate) fn write_json_file<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    let content = serde_json::to_string_pretty(data)
        .map_err(|e| format!("序列化 JSON 失败: {}", e))?;
    fs::write(path, content)
        .map_err(|e| format!("写入文件失败 {:?}: {}", path, e))?;
    Ok(())
}


#[command]
pub fn init_data_dir() -> Result<InitResult, String> {
    let data_dir = get_data_dir()?;
    let assets_dir = get_assets_dir()?;

    // 创建子目录（封面目录 covers/<provider_id> 由封面落盘时按需创建）
    for sub in &["icons/custom", "icons/extracted", "covers/custom", "logo"] {
        let _ = ensure_dir(&assets_dir.join(sub));
    }

    // 初始化默认数据文件
    let env_path = data_dir.join("environments.json");
    let cat_path = data_dir.join("categories.json");
    let entry_path = data_dir.join("entries.json");
    let config_path = data_dir.join("config.json");

    if !env_path.exists() {
        let envs = default_environments();
        write_json_file(&env_path, &envs)?;
    }
    if !cat_path.exists() {
        let cats = default_categories();
        write_json_file(&cat_path, &cats)?;
    }
    if !entry_path.exists() {
        let entries: Vec<Entry> = vec![];
        write_json_file(&entry_path, &entries)?;
    }
    if !config_path.exists() {
        let config = default_config();
        write_json_file(&config_path, &config)?;
    }

    // 释放内置默认语言文件（幂等：缺哪个补哪个，永不覆盖）
    init_lang_dir()?;

    Ok(InitResult {
        data_dir: data_dir.to_string_lossy().to_string(),
    })
}

/// 旧数据迁移：type="steam" → type="provider" + provider 引用。
/// 启动链接不变（仍是 steam://rungameid/{appid}），item_id 从链接中提取；
/// 内存中转换，下次保存条目时自然落盘。
/// （本函数是唯一保留 Steam 字面量的地方——它服务于历史遗留数据，随旧数据消亡而删）
fn migrate_legacy_entries(entries: &mut [Entry]) {
    for e in entries.iter_mut() {
        if e.entry_type != "steam" { continue; }
        let item_id = e.url.trim()
            .strip_prefix("steam://rungameid/")
            .unwrap_or("")
            .trim()
            .to_string();
        e.entry_type = "provider".to_string();
        e.provider = Some(crate::models::ProviderRef {
            id: "steam".to_string(),
            item_id,
            launch_url: String::new(),
            meta: Default::default(),
        });
    }
}

#[command]
pub fn load_all_data() -> Result<AllData, String> {
    let data_dir = get_data_dir()?;

    let config: AppConfig = read_json_file(&data_dir.join("config.json")).unwrap_or_else(|_| default_config());
    let environments: Vec<Environment> = read_json_file(&data_dir.join("environments.json")).unwrap_or_default();
    let categories: Vec<Category> = read_json_file(&data_dir.join("categories.json")).unwrap_or_default();
    let mut entries: Vec<Entry> = read_json_file(&data_dir.join("entries.json")).unwrap_or_default();
    migrate_legacy_entries(&mut entries);

    Ok(AllData {
        config,
        environments,
        categories,
        entries,
    })
}

#[command]
pub fn save_entries(entries: Vec<Entry>) -> Result<(), String> {
    let data_dir = get_data_dir()?;
    write_json_file(&data_dir.join("entries.json"), &entries)
}

#[command]
pub fn save_categories(categories: Vec<Category>) -> Result<(), String> {
    let data_dir = get_data_dir()?;
    write_json_file(&data_dir.join("categories.json"), &categories)
}

#[command]
pub fn save_environments(environments: Vec<Environment>) -> Result<(), String> {
    let data_dir = get_data_dir()?;
    write_json_file(&data_dir.join("environments.json"), &environments)
}

#[command]
pub fn save_config(config: AppConfig) -> Result<(), String> {
    let data_dir = get_data_dir()?;
    write_json_file(&data_dir.join("config.json"), &config)
}

#[command]
pub fn get_exe_dir() -> Result<String, String> {
    Ok(get_exe_directory()?.to_string_lossy().to_string())
}

#[command]
pub fn get_now() -> String {
    get_now_iso()
}

