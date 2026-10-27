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
use crate::models::*;
use crate::paths::*;
use crate::scan::*;
use crate::storage::*;


// ═══════════════════════════════════════════
// 多语言（i18n）支持
//
// 目录约定：<exe>/lang/*.json，文件名即语言代码（BCP 47，如 zh-CN / en-US）。
// 内置三种默认语言随二进制编译（include_str!），首次启动由 init_data_dir
// 调用的 init_lang_dir 幂等释放——缺哪个补哪个，永不覆盖已存在文件。
// 用户复制模板改名即可新增语言，无需改代码。
//
// key 规范：`域.组件.语义` 三段扁平结构（如 dialog.entryEditor.saveBtn）。
// 权威 key 源 = 内置 zh-CN；其他语言缺失的 key 由前端回退链兜底，
// 因此不强制各语言 key 齐全（完成度仅作提示）。
// ═══════════════════════════════════════════

/// 默认回落语言（权威 key 源）
pub(crate) const DEFAULT_LANG: &str = "zh-CN";

/// 内置默认语言模板（编译进二进制，与 src-tauri/lang/*.json 同源）
const BUILTIN_LANGS: [(&str, &str); 3] = [
    ("zh-CN", include_str!("../lang/zh-CN.json")),
    ("zh-TW", include_str!("../lang/zh-TW.json")),
    ("en-US", include_str!("../lang/en-US.json")),
];

/// 语言文件 meta 信息（文件头，与消息表分离）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LangMeta {
    /// 显示名（下拉列表中展示，如 "简体中文" / "English"）
    #[serde(default)]
    pub name: String,
    /// 语言自称（母语者看到的名称）
    #[serde(default)]
    pub native_name: String,
    /// 英文名
    #[serde(default)]
    pub english_name: String,
    /// 作者
    #[serde(default)]
    pub author: String,
    /// 模板版本（与内置模板比对，提示用户补充新增 key）
    #[serde(default)]
    pub template_version: String,
}

/// 语言列表条目（list_languages 返回，供设置页下拉展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangInfo {
    /// 语言代码（文件名去扩展名）
    pub code: String,
    /// 显示名（meta.name 为空时回落 code）
    pub name: String,
    /// 语言自称
    pub native_name: String,
    /// 英文名
    pub english_name: String,
    /// 是否内置默认语言（三种模板之一）
    pub is_default: bool,
    /// 文件是否可解析（损坏时置灰不可选）
    pub valid: bool,
    /// 相对权威 key 源的完成度（0-100）
    pub completeness: u32,
    /// 该语言已翻译的 key 数
    pub key_count: usize,
    /// 相对权威 key 源缺失的 key 数
    pub missing_count: usize,
}

/// 单个语言文件的完整内容（read_language 返回）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangFile {
    pub code: String,
    /// 显示名（meta.name，为空时回落 code）
    pub name: String,
    /// 扁平消息表（点分层 key → 译文）
    pub messages: HashMap<String, String>,
}

/// 语言文件 JSON 的反序列化结构（meta + messages 两段）
#[derive(Debug, Clone, Deserialize)]
struct LangFileRaw {
    #[serde(default)]
    meta: LangMeta,
    #[serde(default)]
    messages: HashMap<String, String>,
}

/// 语言目录：<exe>/lang
pub(crate) fn get_lang_dir() -> Result<PathBuf, String> {
    let exe_dir = get_exe_directory()?;
    let lang_dir = exe_dir.join("lang");
    if !lang_dir.exists() {
        fs::create_dir_all(&lang_dir)
            .map_err(|e| format!("无法创建 lang 目录: {}", e))?;
    }
    Ok(lang_dir)
}

/// 校验语言代码安全性（防路径穿越）：只允许字母、数字、-、_、.
fn is_valid_lang_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 32
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// 释放内置默认语言（幂等：缺哪个补哪个，永不覆盖已存在文件）。
/// 由 init_data_dir 在启动时调用；也作为独立命令暴露（前端可主动触发）。
#[command]
pub fn init_lang_dir() -> Result<(), String> {
    let lang_dir = get_lang_dir()?;
    for (code, content) in BUILTIN_LANGS {
        let path = lang_dir.join(format!("{}.json", code));
        if !path.exists() {
            fs::write(&path, content)
                .map_err(|e| format!("释放默认语言文件失败 {:?}: {}", path, e))?;
        }
    }
    Ok(())
}

/// 解析单个语言文件；损坏时返回 None（调用方标记 valid=false）
fn parse_lang_file(path: &Path) -> Option<LangFileRaw> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<LangFileRaw>(&content).ok()
}

/// 内置权威 key 集合（zh-CN），用于完成度计算
fn builtin_reference_keys() -> std::collections::HashSet<String> {
    let zh_cn = BUILTIN_LANGS
        .iter()
        .find(|(code, _)| *code == DEFAULT_LANG)
        .map(|(_, content)| *content)
        .unwrap_or("{}");
    parse_lang_content(zh_cn)
        .map(|raw| raw.messages.keys().cloned().collect())
        .unwrap_or_default()
}

/// 从 JSON 字符串解析语言文件内容
fn parse_lang_content(content: &str) -> Option<LangFileRaw> {
    serde_json::from_str::<LangFileRaw>(content).ok()
}

/// 扫描 lang 目录，返回全部可用语言（含损坏标记与完成度）。
/// 每次调用实时读盘——用户手动放入的新文件无需重启即可出现在列表。
#[command]
pub fn list_languages() -> Result<Vec<LangInfo>, String> {
    let lang_dir = get_lang_dir()?;
    let reference = builtin_reference_keys();
    let ref_total = reference.len().max(1);

    let mut result: Vec<LangInfo> = Vec::new();
    let entries = fs::read_dir(&lang_dir)
        .map_err(|e| format!("读取 lang 目录失败: {}", e))?;

    for item in entries.flatten() {
        let path = item.path();
        if !path.is_file() {
            continue;
        }
        // 只认 .json 文件
        if path.extension().and_then(|s| s.to_str()).map(|e| !e.eq_ignore_ascii_case("json")).unwrap_or(true) {
            continue;
        }
        let code = match path.file_stem().and_then(|s| s.to_str()) {
            Some(c) if is_valid_lang_code(c) => c.to_string(),
            _ => continue, // 非法文件名直接忽略
        };

        let is_default = BUILTIN_LANGS.iter().any(|(c, _)| *c == code);
        match parse_lang_file(&path) {
            Some(raw) => {
                let missing = reference
                    .iter()
                    .filter(|k| !raw.messages.contains_key(*k))
                    .count();
                let completeness = ((ref_total - missing) as u32 * 100 / ref_total as u32).min(100);
                let name = if raw.meta.name.trim().is_empty() {
                    code.clone()
                } else {
                    raw.meta.name.clone()
                };
                result.push(LangInfo {
                    code,
                    name,
                    native_name: raw.meta.native_name,
                    english_name: raw.meta.english_name,
                    is_default,
                    valid: true,
                    completeness,
                    key_count: raw.messages.len(),
                    missing_count: missing,
                });
            }
            None => {
                // 损坏文件：保留条目供用户看到问题，但不可选
                result.push(LangInfo {
                    code: code.clone(),
                    name: code,
                    native_name: String::new(),
                    english_name: String::new(),
                    is_default,
                    valid: false,
                    completeness: 0,
                    key_count: 0,
                    missing_count: ref_total,
                });
            }
        }
    }

    // 排序：默认语言优先，其余按代码字母序，保证列表稳定
    result.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| a.code.cmp(&b.code))
    });
    Ok(result)
}

/// 读取指定语言文件的完整内容（meta + messages）。
/// code 仅接受安全字符（防路径穿越）；文件不存在或损坏返回错误。
#[command]
pub fn read_language(code: String) -> Result<LangFile, String> {
    if !is_valid_lang_code(&code) {
        return Err(format!("非法语言代码: {}", code));
    }
    let lang_dir = get_lang_dir()?;
    let path = lang_dir.join(format!("{}.json", code));
    if !path.exists() {
        return Err(format!("语言文件不存在: {}", code));
    }
    let raw = parse_lang_file(&path)
        .ok_or_else(|| format!("语言文件解析失败: {}", code))?;
    let name = if raw.meta.name.trim().is_empty() {
        code.clone()
    } else {
        raw.meta.name
    };
    Ok(LangFile {
        code,
        name,
        messages: raw.messages,
    })
}

/// 用资源管理器打开 lang 目录（复用 ShellExecute，与 open_in_explorer 同机制）
#[command]
pub fn open_lang_dir() -> Result<(), String> {
    let lang_dir = get_lang_dir()?;
    let dir_str = lang_dir.to_string_lossy().to_string();

    #[cfg(windows)]
    {
        // explorer <目录>：直接打开文件夹（不加 /select，让用户看到全部语言文件）
        shell_execute_open(
            "explorer.exe",
            Some(&dir_str),
            false,
            SW_SHOWNORMAL.0,
        )?;
    }
    Ok(())
}

// ═══════════════════════════════════════════
// 测试（在测试二进制的 exe 同级的 lang/ 中验证，结束清理）
// ═══════════════════════════════════════════

#[cfg(test)]
mod lang_tests {
    use super::*;

    /// 首次启动链路：释放 → 扫描 → 读取 → 幂等 → 非法 code 防护
    #[test]
    fn test_lang_release_scan_read_flow() {
        // 清理可能残留的旧数据，保证测试从「首次启动」状态开始
        let lang_dir = get_lang_dir().expect("获取 lang 目录失败");
        if lang_dir.exists() {
            let _ = fs::remove_dir_all(&lang_dir);
        }

        // 1) 释放三个默认语言
        init_lang_dir().expect("init_lang_dir 失败");
        for code in ["zh-CN", "zh-TW", "en-US"] {
            assert!(
                lang_dir.join(format!("{}.json", code)).exists(),
                "默认语言未释放: {}",
                code
            );
        }

        // 2) 扫描：三种默认语言均应 valid、is_default、完成度 100%
        let list = list_languages().expect("list_languages 失败");
        assert!(list.len() >= 3, "语言列表至少应有 3 项");
        for code in ["zh-CN", "zh-TW", "en-US"] {
            let info = list
                .iter()
                .find(|l| l.code == code)
                .unwrap_or_else(|| panic!("语言列表缺少 {}", code));
            assert!(info.valid, "{} 应可解析", code);
            assert!(info.is_default, "{} 应标记为默认语言", code);
            assert_eq!(info.completeness, 100, "{} 完成度应为 100%", code);
            assert!(!info.name.is_empty(), "{} 显示名不应为空", code);
        }

        // 3) 读取：消息表包含关键 key
        let file = read_language("en-US".to_string()).expect("read_language 失败");
        assert_eq!(file.code, "en-US");
        assert_eq!(file.messages.get("common.save").map(|s| s.as_str()), Some("Save"));

        // 4) 幂等：重复释放不报错、不改变内容
        let before = fs::read_to_string(lang_dir.join("zh-CN.json")).unwrap();
        init_lang_dir().expect("重复 init_lang_dir 失败");
        let after = fs::read_to_string(lang_dir.join("zh-CN.json")).unwrap();
        assert_eq!(before, after, "重复释放不应覆盖已存在文件");

        // 5) 防护：非法 code（路径穿越）必须拒绝
        assert!(read_language("../secrets".to_string()).is_err());
        assert!(read_language("..%2F..%2Fetc".to_string()).is_err());
        assert!(read_language(String::new()).is_err());

        // 清理测试产物
        let _ = fs::remove_dir_all(&lang_dir);
    }
}
