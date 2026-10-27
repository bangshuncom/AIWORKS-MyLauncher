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
// 浏览器书签解析与导入
// ═══════════════════════════════════════════

// ─── 浏览器书签解析与导入 ───

/// 从 HTML 属性列表字符串中提取指定属性值。
/// 属性值可能被双引号包裹，也可能裸露（Netscape 格式导出的 HREF 总是带引号）。
/// 返回解码了常见 HTML 实体（&amp; &lt; &gt; &quot; &#39;）后的字符串。
pub(crate) fn extract_attr(attrs: &str, key: &str) -> Option<String> {
    // 找到属性名（前面必须是空白或行首，后面跟 =）
    let bytes = attrs.as_bytes();
    let pattern = format!("{}=", key);
    let mut search_from = 0usize;
    while let Some(rel) = attrs[search_from..].find(&pattern) {
        let start = search_from + rel;
        // 检查前一个字符是否为空白或属性行首
        let prev_ok = start == 0 || bytes[start - 1].is_ascii_whitespace() || bytes[start - 1] == b'"';
        if !prev_ok {
            search_from = start + pattern.len();
            continue;
        }
        let mut pos = start + pattern.len();
        // 跳过空白
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
        if pos >= bytes.len() {
            return None;
        }
        if bytes[pos] == b'"' {
            // 带引号的值
            let value_start = pos + 1;
            let value_end = attrs[value_start..].find('"').map(|i| value_start + i)?;
            let raw = &attrs[value_start..value_end];
            return Some(decode_html_entities(raw));
        } else if bytes[pos] == b'\'' {
            let value_start = pos + 1;
            let value_end = attrs[value_start..].find('\'').map(|i| value_start + i)?;
            let raw = &attrs[value_start..value_end];
            return Some(decode_html_entities(raw));
        } else {
            // 裸值：读到下一个空白
            let value_start = pos;
            let value_end = attrs[value_start..]
                .find(|c: char| c.is_whitespace())
                .map(|i| value_start + i)
                .unwrap_or(attrs.len());
            let raw = &attrs[value_start..value_end];
            return Some(decode_html_entities(raw));
        }
    }
    None
}

/// 解码常见 HTML 实体
pub(crate) fn decode_html_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// 将 base64 data URI（如 "data:image/png;base64,iVBOR..."）解码保存为图标文件。
/// 返回相对 exe 目录的路径（如 "assets/icons/bookmarks/icon_1400000000_0.png"）。
pub(crate) fn save_bookmark_icon(data_uri: &str, seq: usize) -> Result<String, String> {
    let base64_part = match data_uri.find("base64,") {
        Some(idx) => &data_uri[idx + 7..],
        None => return Err("不是 base64 data URI".to_string()),
    };
    if base64_part.is_empty() {
        return Err("data URI 内容为空".to_string());
    }

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_part.trim())
        .map_err(|e| format!("base64 解码失败: {}", e))?;

    if bytes.is_empty() || bytes.len() < 8 {
        return Err("图标数据过小".to_string());
    }

    // 通过魔数判断扩展名
    let ext = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.len() >= 4 && bytes[0] == 0 && bytes[1] == 0 && bytes[2] == 1 && bytes[3] == 0 {
        "ico" // ICO 魔数 00 00 01 00
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else {
        // 未知格式，尝试用 image crate 解码为 PNG 统一处理
        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let assets_dir = get_assets_dir()?;
                let icon_dir = assets_dir.join("icons").join("bookmarks");
                ensure_dir(&icon_dir)?;
                let ts = chrono::Local::now().format("%H%M%S%3f").to_string();
                let file_path = icon_dir.join(format!("icon_{}_{}.png", ts, seq));
                img.to_rgba8().save(&file_path)
                    .map_err(|e| format!("保存图标失败: {}", e))?;
                return Ok(rel_to_exe_dir(&file_path));
            }
            Err(_) => return Err("无法识别的图标格式".to_string()),
        }
    };

    // ico 格式转 PNG 保存（image crate 支持 ico 解码）
    if ext == "ico" {
        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let assets_dir = get_assets_dir()?;
                let icon_dir = assets_dir.join("icons").join("bookmarks");
                ensure_dir(&icon_dir)?;
                let ts = chrono::Local::now().format("%H%M%S%3f").to_string();
                let file_path = icon_dir.join(format!("icon_{}_{}.png", ts, seq));
                img.to_rgba8().save(&file_path)
                    .map_err(|e| format!("保存图标失败: {}", e))?;
                return Ok(rel_to_exe_dir(&file_path));
            }
            Err(_) => return Err("ico 解码失败".to_string()),
        }
    }

    // png/jpg/gif 直接保存
    let assets_dir = get_assets_dir()?;
    let icon_dir = assets_dir.join("icons").join("bookmarks");
    ensure_dir(&icon_dir)?;
    let ts = chrono::Local::now().format("%H%M%S%3f").to_string();
    let file_path = icon_dir.join(format!("icon_{}_{}.{}", ts, seq, ext));
    fs::write(&file_path, &bytes)
        .map_err(|e| format!("写入图标文件失败: {}", e))?;

    Ok(rel_to_exe_dir(&file_path))
}

/// 解析 <DL> 内部（不含首尾标签）的一段内容，收集其中直接出现的链接与子文件夹。
/// links 与 folders 均为该层级的直接成员。
pub(crate) fn parse_bookmark_dl(content: &str, links: &mut Vec<BookmarkLink>, folders: &mut Vec<BookmarkFolder>, counters: &mut (usize, usize)) {
    let mut i = 0usize;
    let len = content.len();

    while i < len {
        // 寻找下一个 <DT>
        let Some(dt_rel) = content[i..].find("<DT>") else { break };
        let item_start = i + dt_rel + 4;

        // 判断是 <H3（文件夹）、<A（链接）还是 <DL（匿名内容块）
        let rest = &content[item_start..];
        let trimmed = rest.trim_start();
        let trim_off = rest.len() - trimmed.len();

        if trimmed.starts_with("<H3") {
            // ── 文件夹：解析标题与内容块，直属链接/子文件夹存入自身 ──
            let Some(tag_end_rel) = trimmed.find('>') else { break };
            let after_tag_off = trim_off + tag_end_rel + 1;
            let after_tag = &content[item_start + after_tag_off..];
            let Some(title_end_rel) = after_tag.find("</H3>") else { break };
            let title = decode_html_entities(after_tag[..title_end_rel].trim());

            let mut sub_folders = Vec::new();
            let mut sub_links = Vec::new();
            let consumed = if let Some(dl_rel) = after_tag[title_end_rel + 5..].find("<DL>") {
                // 有内容块：跳过开标签，然后找配对的 </DL>（处理嵌套）
                let inner = &after_tag[title_end_rel + 5 + dl_rel + 4..];
                match find_dl_end(inner) {
                    Some(end) => {
                        parse_bookmark_dl(&inner[..end], &mut sub_links, &mut sub_folders, counters);
                        // 内容块全部长度 = 5(</H3>) + dl_rel + 4(<DL>) + end + 5(</DL>)
                        title_end_rel + 5 + dl_rel + 4 + end + 5
                    }
                    None => {
                        // 未找到配对 </DL>：整段作为该文件夹内容，防解析重复
                        parse_bookmark_dl(inner, &mut sub_links, &mut sub_folders, counters);
                        usize::MAX
                    }
                }
            } else {
                // 没有内容块，空文件夹
                0
            };
            if consumed == usize::MAX {
                break;
            }
            folders.push(BookmarkFolder {
                title,
                links: sub_links,
                children: sub_folders,
            });
            counters.1 += 1;
            i = item_start + after_tag_off + consumed;
        } else if trimmed.starts_with("<A") {
            // ── 链接 ──
            let Some(tag_end_rel) = trimmed.find('>') else { break };
            let after_tag_off = trim_off + tag_end_rel + 1;
            let after_tag = &content[item_start + after_tag_off..];
            let Some(title_end_rel) = after_tag.find("</A>") else { break };
            let title = decode_html_entities(after_tag[..title_end_rel].trim());

            let a_attrs = &trimmed[..tag_end_rel];
            let href = extract_attr(a_attrs, "HREF").unwrap_or_default();
            let icon_uri = extract_attr(a_attrs, "ICON").unwrap_or_default();

            if !href.is_empty() && !href.starts_with("javascript:") && !href.starts_with("place:") {
                let icon_path = if icon_uri.starts_with("data:") {
                    match save_bookmark_icon(&icon_uri, counters.0) {
                        Ok(p) => p,
                        Err(_) => String::new(),
                    }
                } else {
                    String::new()
                };
                links.push(BookmarkLink {
                    title: if title.is_empty() { href.clone() } else { title },
                    url: href,
                    icon_path,
                });
                counters.0 += 1;
            }
            i = item_start + after_tag_off + title_end_rel + 4;
        } else if trimmed.starts_with("<DL>") {
            // ── 匿名 <DL>：内容并入当前层级 ──
            match find_dl_end(&trimmed[4..]) {
                Some(end) => {
                    parse_bookmark_dl(&trimmed[4..4 + end], links, folders, counters);
                    i = item_start + trim_off + 4 + end + 5;
                }
                None => {
                    parse_bookmark_dl(&trimmed[4..], links, folders, counters);
                    i = len;
                }
            }
        } else {
            // 未知标签（<p> 等）：跳过到下一个 <DT>
            let next_dt = rest[trim_off..].find("<DT>").map(|r| r + trim_off).unwrap_or(usize::MAX);
            let next_dl = rest[trim_off..].find("<DL>").map(|r| r + trim_off).unwrap_or(usize::MAX);
            let skip = next_dt.min(next_dl);
            if skip == usize::MAX {
                i = len;
            } else if skip == 0 {
                i += 1; // 防止死循环
            } else {
                i = item_start + skip;
            }
        }
    }
}

/// 在 inner 内查找配对的 </DL> 结束位置（inner 为 <DL> 之后的内容）。
/// 返回该 DL 块内部内容的长度（不含 </DL> 本身），找不到则返回 None。
pub(crate) fn find_dl_end(inner: &str) -> Option<usize> {
    let mut depth = 1usize;
    let mut scan = 0usize;
    while scan < inner.len() {
        // 取当前位置后的第一个开标签与闭标签，按位置先后处理
        let open = inner[scan..].find("<DL>").map(|r| scan + r);
        let close = inner[scan..].find("</DL>").map(|r| scan + r);
        match (open, close) {
            (Some(o), Some(c)) if c < o => {
                depth -= 1;
                scan = c + 5;
                if depth == 0 {
                    return Some(c);
                }
            }
            (Some(o), _) => {
                depth += 1;
                scan = o + 4;
            }
            (None, Some(c)) => {
                depth -= 1;
                scan = c + 5;
                if depth == 0 {
                    return Some(c);
                }
            }
            (None, None) => break,
        }
    }
    None
}

/// 解析 Netscape 书签文件（Chrome/Edge/360 等浏览器导出的 HTML）
#[command]
pub fn parse_bookmarks(file_path: String) -> Result<BookmarkParseResult, String> {
    let content = fs::read_to_string(&file_path)
        .map_err(|e| format!("读取书签文件失败: {}", e))?;

    // 校验格式
    let upper = content.to_uppercase();
    if !upper.contains("NETSCAPE-Bookmark-file") && !upper.contains("<DL>") {
        return Err("不是有效的浏览器书签文件（Netscape Bookmark 格式）".to_string());
    }

    // 提取最外层 <DL>...</DL>（从第一个 <DL> 到最后一个 </DL>）
    let Some(first_dl) = content.find("<DL>") else {
        return Err("书签文件中没有 <DL> 结构".to_string());
    };
    let Some(last_dl_end) = content.rfind("</DL>") else {
        return Err("书签文件中没有 </DL> 结束标签".to_string());
    };
    let outer = &content[first_dl + 4..last_dl_end];

    let mut root_links: Vec<BookmarkLink> = Vec::new();
    let mut folders: Vec<BookmarkFolder> = Vec::new();
    let mut counters = (0usize, 0usize); // (链接计数, 文件夹计数)

    parse_bookmark_dl(outer, &mut root_links, &mut folders, &mut counters);

    if folders.is_empty() && root_links.is_empty() {
        return Err("书签文件中没有解析到任何书签内容".to_string());
    }

    // 统计总数（递归）
    fn count_folder(f: &BookmarkFolder) -> (usize, usize) {
        let mut links = f.links.len();
        let mut folders = 1;
        for c in &f.children {
            let (l, fo) = count_folder(c);
            links += l;
            folders += fo;
        }
        (links, folders)
    }
    let mut total_links = root_links.len();
    let mut total_folders = 0;
    for f in &folders {
        let (l, fo) = count_folder(f);
        total_links += l;
        total_folders += fo;
    }

    Ok(BookmarkParseResult {
        folders,
        root_links,
        total_links,
        total_folders,
    })
}

