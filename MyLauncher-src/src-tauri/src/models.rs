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
use crate::paths::*;
use crate::scan::*;
use crate::storage::*;


// ═══════════════════════════════════════════
// 数据模型
// ═══════════════════════════════════════════

// ════════════════════════════════════════════
// 数据模型
// ════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageResource {
    #[serde(rename = "type")]
    pub res_type: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverResource {
    pub enabled: bool,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub category_id: String,
    pub relative_path: String,
    pub absolute_paths: HashMap<String, String>,
    pub path_mode: String,
    pub launch_args: String,
    pub working_directory: String,
    pub window_style: String,
    pub run_as_admin: bool,
    pub url: String,
    /// 平台来源（type="provider" 的条目专用）：插件 id + 平台条目 ID。
    /// 启动链接存于 url 字段，此处不重复
    #[serde(default)]
    pub provider: Option<ProviderRef>,
    /// 更新地址（可选）：开源软件的 GitHub 仓库/发布页等，编辑页可一键访问查看新版本。
    /// 仅 program/file 类型使用，旧数据无此字段
    #[serde(default)]
    pub update_url: String,
    pub icon: ImageResource,
    /// 图标来源描述（可选）：系统功能条目专用，
    /// 格式 "sys:<文件>|<组索引>"（如 "sys:%windir%\\system32\\devmgr.dll|5"）
    /// 或 "glyph:<字形码点十六进制>"（如 "glyph:E713"，Segoe 图标字体）。
    /// 重提取图标时按此定位来源；emoji 图标的旧条目为空
    #[serde(default)]
    pub icon_source: String,
    pub cover: CoverResource,
    pub tags: Vec<String>,
    pub notes: String,
    pub last_used: String,
    pub add_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub icon: ImageResource,
    pub view_mode: String,
    pub sort_order: i32,
    pub is_top_level: bool,
    /// 前台隐藏（仅非顶级分类；LOGO 三击弹关于时临时解锁显示，重启恢复隐藏）
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub description: String,
    pub drive_mapping: HashMap<String, String>,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: String,
    /// LOGO 图标（emoji 字符或自定义图片相对路径，按 logo_icon_type 区分），默认 🪷
    #[serde(default = "default_logo_icon")]
    pub logo_icon: String,
    /// LOGO 图标类型: "emoji"（字符） | "custom"（上传的图片相对路径）
    #[serde(default = "default_logo_icon_type")]
    pub logo_icon_type: String,
    /// LOGO 文字，默认 "MyLauncher"
    #[serde(default = "default_logo_text")]
    pub logo_text: String,
    /// LOGO 图片相对路径（如 "assets/logo/xxx.png"），空表示未设置
    #[serde(default)]
    pub logo_image: String,
    /// 是否启用 LOGO 图片（启用后 LOGO 区域整体显示为图片）
    #[serde(default)]
    pub logo_image_enabled: bool,
    /// 关闭按钮行为: "tray"（隐藏到托盘） | "close"（直接退出）
    #[serde(default = "default_close_action")]
    pub close_action: String,
    /// 用户自定义 emoji 列表（在设置中粘贴添加）
    #[serde(default)]
    pub custom_emojis: Vec<String>,
    /// Favicon 获取源 API 模板列表（一行一个），支持占位符：
    /// {url} 完整链接 | {domain} 域名 | {scheme} 协议（http/https）
    /// 获取时按顺序逐个尝试，任一成功即用；列表为空时回落内置默认
    #[serde(default = "default_favicon_api_sources")]
    pub favicon_api_sources: Vec<String>,
    /// 界面主题: "light"（明亮） | "dark"（暗黑） | "system"（跟随系统）
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 侧导航是否折叠隐藏
    #[serde(default)]
    pub sidebar_collapsed: bool,
    /// 竖向卡片封面宽高比例，如 "16:9" | "4:3" | "1:1" | "3:4" | "9:16"
    #[serde(default = "default_card_cover_ratio")]
    pub card_cover_ratio: String,
    /// 瀑布流封面图最大高度（px），默认 480
    #[serde(default = "default_masonry_max_height")]
    pub masonry_max_height: u32,
    /// 界面语言代码（lang/ 目录下的文件名，如 zh-CN / zh-TW / en-US），默认 zh-CN。
    /// 空字符串表示未设置（老配置升级场景），前端按系统语言判定后回写。
    #[serde(default = "default_language")]
    pub language: String,
    /// 已停用的插件 id 列表（设置页开关控制）。
    /// 停用后插件不参与目录探测、按 ID 查询与平台下拉，但插件文件保留、
    /// 已导入条目的启动不受影响（启动只依赖条目的 url/exe 路径）
    #[serde(default)]
    pub disabled_plugins: Vec<String>,
}

pub(crate) fn default_logo_icon() -> String { "🪷".to_string() }
pub(crate) fn default_logo_icon_type() -> String { "emoji".to_string() }
pub(crate) fn default_logo_text() -> String { "MyLauncher".to_string() }
pub(crate) fn default_close_action() -> String { "tray".to_string() }
pub(crate) fn default_theme() -> String { "system".to_string() }
pub(crate) fn default_card_cover_ratio() -> String { "16:9".to_string() }
pub(crate) fn default_masonry_max_height() -> u32 { 480 }
pub(crate) fn default_language() -> String { "zh-CN".to_string() }

/// 默认 favicon 获取源（三个 API 模板，按序尝试）
pub(crate) fn default_favicon_api_sources() -> Vec<String> {
    vec![
        "https://a.favicon.im/{url}?larger=true".to_string(),
        "https://faviconsnap.com/api/favicon?url={url}".to_string(),
        "{scheme}://{domain}/favicon.ico".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllData {
    pub config: AppConfig,
    pub environments: Vec<Environment>,
    pub categories: Vec<Category>,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitResult {
    pub data_dir: String,
}

/// 平台来源引用：条目由哪个扫描插件提供。
/// 宿主不认识具体平台，id 对应 plugins/<id>/ 目录；launch_url 为平台协议启动链接
/// （存进 Entry.url），meta 透传插件自定义字段仅供显示。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRef {
    /// 插件 id（如 "steam"）
    pub id: String,
    /// 平台条目 ID（如 Steam 的 appid）
    pub item_id: String,
    /// 启动链接（如 steam://rungameid/730），可与 Entry.url 相互对应
    #[serde(default)]
    pub launch_url: String,
    /// 插件自定义元数据（原样透传）
    #[serde(default)]
    pub meta: HashMap<String, String>,
}

/// 封面来源（扫描插件给出，宿主负责复制/下载到 assets/covers/<provider_id>/）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverSource {
    /// 本地图片路径（存在时优先复制）
    #[serde(default)]
    pub local_path: String,
    /// 网络图片地址（本地缺失时下载，失败静默）
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExeInfo {
    pub name: String,
    pub icon_path: String,
    pub suggested_path_mode: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub file_exists: bool,
    /// 平台来源（插件扫描结果；普通程序为 None）
    #[serde(default)]
    pub provider: Option<ProviderRef>,
    /// 封面来源（插件/内置扫描给出，导入时宿主复制或下载）
    #[serde(default)]
    pub cover: Option<CoverSource>,
    /// 发现多个候选 exe（后端已展开为多条，前端红色提示用户勾选）
    #[serde(default)]
    pub multi_exe: bool,
    /// 是否主程序 exe（多候选时的第一个，默认选中；单 exe 时为 true）
    #[serde(default)]
    pub is_primary: bool,
}

/// 快捷方式解析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LnkInfo {
    /// 快捷方式文件名（不含扩展名），作为条目名称
    pub name: String,
    /// 目标程序绝对路径
    pub target_path: String,
    /// 目标是否存在
    pub target_exists: bool,
    /// 工作目录（起始位置）
    pub working_directory: String,
    /// 启动参数
    pub arguments: String,
    /// 运行方式: normal | maximized | minimized
    pub window_style: String,
    /// 备注（快捷方式 comment 属性）
    pub description: String,
    /// 图标路径（提取后）
    pub icon_path: String,
    /// 建议的路径模式
    pub suggested_path_mode: String,
    /// 相对路径（如果目标在软件目录内）
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub results: Vec<ExeInfo>,
    pub total: usize,
    /// 扫描警告（插件超时/输出非法/目录不可访问等，前端提示区展示）
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathInfo {
    pub resolved: String,
    pub exists: bool,
    pub path_mode: String,
}

/// 书签链接（浏览器书签文件中的一条 <A> 记录）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkLink {
    pub title: String,
    pub url: String,
    /// 保存后的图标相对路径（如 "assets/icons/bookmarks/xxx.png"），无图标时为空
    pub icon_path: String,
}

/// 书签文件夹（可嵌套）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkFolder {
    pub title: String,
    /// 该文件夹下直属的链接（按文件内顺序）
    pub links: Vec<BookmarkLink>,
    /// 子文件夹
    pub children: Vec<BookmarkFolder>,
}

/// 书签解析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkParseResult {
    /// 解析出的所有文件夹（顶层列表，通常含一个"收藏栏"）
    pub folders: Vec<BookmarkFolder>,
    /// 直接位于根层级（不属任何文件夹）的链接
    pub root_links: Vec<BookmarkLink>,
    pub total_links: usize,
    pub total_folders: usize,
}

