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
// Favicon 获取
// ═══════════════════════════════════════════

// ─── Favicon 获取 ───

/// domain → 安全文件名（非法字符替换为 _，用于 assets/icons/custom/ 下的缓存命名）
pub(crate) fn safe_domain_filename(domain: &str) -> String {
    domain.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

/// favicon 多源获取（内部实现，供命令与图标重提取共用）。
/// 图标按 domain 命名缓存于 assets/icons/custom/{domain}.png：
/// 同 domain 的不同链接共用同一文件，缓存命中时不再重复下载。
/// force = true 时绕过缓存强制重新下载（重置图标语义：确保拿到最新图）。
/// 缓存未命中时按顺序尝试获取源（可在设置中自定义，模板支持占位符
/// {url} 完整链接 | {domain} 域名 | {scheme} 协议），任一步成功即返回
/// 保存后的相对路径；读取配置失败或列表为空时回落内置默认三源。
/// 每步 2 秒超时；全部失败返回 Err。
pub(crate) fn fetch_favicon_inner(url: &str, force: bool) -> Result<String, String> {
    let parsed = url::Url::parse(url)
        .map_err(|e| format!("URL 解析失败: {}", e))?;
    let domain = parsed.host_str()
        .ok_or("无法获取域名")?
        .to_string();

    // 同 domain 缓存命中：直接复用，不重复下载（重置场景 force 绕过）
    let cached_path = get_assets_dir()?
        .join("icons")
        .join("custom")
        .join(format!("{}.png", safe_domain_filename(&domain)));
    if !force && cached_path.exists() {
        return Ok(rel_to_exe_dir(&cached_path));
    }

    // 2 秒超时的客户端（总超时：连接 + 响应 + 读取响应体）
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    // 获取源模板：优先读用户设置（每条调用读一次小文件，开销可接受）；
    // 读失败或过滤空行后为空 → 回落内置默认
    let mut templates = get_data_dir()
        .ok()
        .and_then(|dir| read_json_file::<AppConfig>(&dir.join("config.json")).ok())
        .map(|cfg| cfg.favicon_api_sources)
        .unwrap_or_default();
    templates.retain(|t| !t.trim().is_empty());
    if templates.is_empty() {
        templates = default_favicon_api_sources();
    }

    // 模板替换占位符 → 实际请求地址，按序尝试
    let scheme = parsed.scheme();
    let candidates: Vec<String> = templates
        .iter()
        .map(|t| t.replace("{url}", url).replace("{domain}", &domain).replace("{scheme}", scheme))
        .collect();

    let mut last_err = String::new();
    for src in &candidates {
        match client.get(src).send() {
            Ok(resp) if resp.status().is_success() => match resp.bytes() {
                Ok(bytes) => {
                    if bytes.is_empty() {
                        last_err = format!("{} 返回空内容", src);
                        continue;
                    }
                    // 校验像不像图片（魔数）：png/jpg/gif/ico/bmp/webp
                    let b = &bytes;
                    let looks_like_image = b.starts_with(&[0x89, b'P', b'N', b'G'])
                        || b.starts_with(&[0xFF, 0xD8, 0xFF])
                        || b.starts_with(b"GIF8")
                        || (b.len() >= 4 && b[0] == 0 && b[1] == 0 && b[2] == 1 && b[3] == 0)
                        || (b.len() >= 2 && b[0] == 0x42 && b[1] == 0x4D)
                        || (b.len() >= 4 && &b[0..4] == b"RIFF" && b.len() >= 12 && &b[8..12] == b"WEBP");
                    if !looks_like_image {
                        last_err = format!("{} 返回的不是图片", src);
                        continue;
                    }
                    return save_favicon_bytes(&bytes, &domain);
                }
                Err(e) => last_err = format!("读取响应失败: {}", e),
            },
            Ok(resp) => last_err = format!("{} 状态码 {}", src, resp.status()),
            Err(e) => last_err = format!("请求失败: {}", e),
        }
    }
    Err(format!("无法获取 favicon（{}）", last_err))
}

/// 把 favicon 字节保存到 assets/icons/custom/{domain}.png，返回相对 exe 目录的路径。
/// 按 domain 命名：同 domain 不同链接共用同一文件（多线程并发写同一目标时容忍最后覆盖，
/// 内容同为该 domain 的合法 favicon，最终状态一致）。
pub(crate) fn save_favicon_bytes(bytes: &[u8], domain: &str) -> Result<String, String> {
    let assets_dir = get_assets_dir()?;
    let icon_dir = assets_dir.join("icons").join("custom");
    ensure_dir(&icon_dir)?;

    let file_path = icon_dir.join(format!("{}.png", safe_domain_filename(domain)));

    // 尝试用 image crate 解码并转为 PNG（统一格式）
    match image::load_from_memory(bytes) {
        Ok(img) => {
            img.to_rgba8().save(&file_path)
                .map_err(|e| format!("保存图标失败: {}", e))?;
        }
        Err(_) => {
            // 解码失败，直接保存原始字节
            fs::write(&file_path, bytes)
                .map_err(|e| format!("保存图标失败: {}", e))?;
        }
    }

    Ok(rel_to_exe_dir(&file_path))
}

#[command]
pub fn fetch_favicon(url: String, force: Option<bool>) -> Result<String, String> {
    fetch_favicon_inner(&url, force.unwrap_or(false))
}

