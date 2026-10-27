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
// 初始化默认数据
// ═══════════════════════════════════════════

// ════════════════════════════════════════════
// 初始化默认数据
// ════════════════════════════════════════════

pub(crate) fn default_environments() -> Vec<Environment> {
    vec![Environment {
        id: "env_001".to_string(),
        name: "本机".to_string(),
        description: "当前电脑".to_string(),
        drive_mapping: HashMap::new(),
        is_current: true,
    }]
}

pub(crate) fn default_categories() -> Vec<Category> {
    let make_icon = |t: &str, s: &str| ImageResource {
        res_type: t.to_string(),
        source: s.to_string(),
    };

    vec![
        Category {
            id: "cat_001".to_string(),
            name: "开发工具".to_string(),
            parent_id: None,
            icon: make_icon("builtin", "dev"),
            view_mode: "icon_grid".to_string(),
            sort_order: 1,
            is_top_level: true,
            hidden: false,
        },
        Category {
            id: "cat_002".to_string(),
            name: "游戏".to_string(),
            parent_id: None,
            icon: make_icon("builtin", "game"),
            view_mode: "vertical_card".to_string(),
            sort_order: 2,
            is_top_level: true,
            hidden: false,
        },
        Category {
            id: "cat_003".to_string(),
            name: "影音".to_string(),
            parent_id: None,
            icon: make_icon("builtin", "media"),
            view_mode: "horizontal_card".to_string(),
            sort_order: 3,
            is_top_level: true,
            hidden: false,
        },
        Category {
            id: "cat_004".to_string(),
            name: "系统工具".to_string(),
            parent_id: None,
            icon: make_icon("builtin", "system"),
            view_mode: "icon_grid".to_string(),
            sort_order: 4,
            is_top_level: true,
            hidden: false,
        },
        Category {
            id: "cat_005".to_string(),
            name: "网址收藏".to_string(),
            parent_id: None,
            icon: make_icon("builtin", "url"),
            view_mode: "icon_grid".to_string(),
            sort_order: 5,
            is_top_level: true,
            hidden: false,
        },
    ]
}

pub(crate) fn default_config() -> AppConfig {
    AppConfig {
        version: "1.0.0".to_string(),
        logo_icon: default_logo_icon(),
        logo_icon_type: default_logo_icon_type(),
        logo_text: default_logo_text(),
        logo_image: String::new(),
        logo_image_enabled: false,
        close_action: default_close_action(),
        custom_emojis: vec![],
        favicon_api_sources: default_favicon_api_sources(),
        theme: default_theme(),
        sidebar_collapsed: false,
        card_cover_ratio: default_card_cover_ratio(),
        masonry_max_height: default_masonry_max_height(),
        language: default_language(),
        disabled_plugins: Vec::new(),
    }
}

