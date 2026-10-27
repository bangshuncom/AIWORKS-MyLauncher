#![allow(unused_imports)]

use tauri::command;
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::io::Read;
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

use crate::models::*;
use crate::paths::*;


// ═══════════════════════════════════════════
// 插件宿主：发现 / 探测 / 调度 / 输出归一化
//
// 契约见 docs/plugin-architecture.md：
//   plugins/<id>/plugin.json 声明能力，插件 exe 按
//   `scan-dir --dir <路径>` / `scan-id --id <ID>` 被调用，
//   stdout 只吐 JSON（api_version 1），日志走 stderr。
// 宿主不认识任何具体平台，平台特征全部来自插件 manifest。
// ═══════════════════════════════════════════

/// 宿主支持的协议版本（只支持 1，不匹配拒绝加载）
const SUPPORTED_API_VERSION: i64 = 1;

/// 输出限制（防失控，超限按非法处理）
const MAX_STDOUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 4 * 1024;
const MAX_ITEMS: usize = 5000;
const MAX_NAME_LEN: usize = 200;
const MAX_PATH_LEN: usize = 4096;
const MAX_META_VALUE_LEN: usize = 1024;
const MAX_WARNINGS_BYTES: usize = 64 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 120_000;

// ─── manifest 数据结构（未知字段一律忽略，协议只增不改） ───

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PluginManifest {
    pub api_version: i64,
    pub id: String,
    pub name: String,
    pub version: String,
    pub entry: String,
    pub priority: i32,
    pub timeout_ms: u64,
    pub probe: PluginProbe,
    pub installed_check: Option<PluginInstalledCheck>,
    pub capabilities: Vec<String>,
}

impl Default for PluginManifest {
    fn default() -> Self {
        PluginManifest {
            api_version: 0,
            id: String::new(),
            name: String::new(),
            version: String::new(),
            entry: String::new(),
            priority: 0,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            probe: PluginProbe::default(),
            installed_check: None,
            capabilities: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct PluginProbe {
    /// 这些子项全部存在时认领扫描目录
    pub dir_markers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PluginInstalledCheck {
    /// "HKCU" | "HKLM"
    pub hive: String,
    /// 注册表键路径（如 Software\Valve\Steam）
    pub key: String,
    /// 值名
    pub value: String,
}

impl Default for PluginInstalledCheck {
    fn default() -> Self {
        PluginInstalledCheck { hive: "HKCU".to_string(), key: String::new(), value: String::new() }
    }
}

/// 校验插件 id：小写字母开头，只允许小写字母数字连字符，1-32 位。
/// 校验通过才允许拼目录路径（防路径穿越）。
fn is_valid_plugin_id(id: &str) -> bool {
    let b = id.as_bytes();
    if b.is_empty() || b.len() > 32 { return false; }
    if !b[0].is_ascii_lowercase() { return false; }
    b[1..].iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

// ─── 插件发现 ───

/// 一个已通过校验、可调度的插件
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    /// 插件目录（exe 目录下 plugins/<id>/）
    pub dir: PathBuf,
    /// 入口可执行文件完整路径
    pub exe_path: PathBuf,
}

impl LoadedPlugin {
    pub fn effective_timeout(&self) -> Duration {
        Duration::from_millis(self.manifest.timeout_ms.clamp(1_000, MAX_TIMEOUT_MS))
    }
}

/// 插件面板条目（list_plugins 命令返回）
#[derive(Debug, Clone, Serialize)]
pub struct PluginStatus {
    pub id: String,
    /// 显示名（manifest 缺失时回落目录名）
    pub name: String,
    pub version: String,
    /// 插件目录绝对路径
    pub dir: String,
    /// ok | error
    pub status: String,
    /// 状态说明（error 时为人能看懂的原因）
    pub message: String,
    /// 能力列表（scan_dir / scan_id）
    pub capabilities: Vec<String>,
    /// 平台是否检测到安装（manifest 提供 installed_check 时才有值）
    pub installed: Option<bool>,
    /// 是否启用（用户在设置页开关控制；停用后不参与扫描与平台下拉）
    pub enabled: bool,
    /// 入口文件修改时间（面板展示，便于确认插件新旧）
    pub entry_mtime: String,
}

/// 扫描 plugins/ 目录，返回所有插件的加载结果（含加载失败项，面板展示用）。
/// 无 plugins/ 目录返回空列表——没有插件就是"没装平台扫描器"，一切按普通目录处理。
pub(crate) fn discover_plugins() -> Vec<Result<LoadedPlugin, PluginStatus>> {    let mut out = Vec::new();
    let plugins_root = match get_exe_directory() {
        Ok(d) => d.join("plugins"),
        Err(_) => return out,
    };
    let entries = match fs::read_dir(&plugins_root) {
        Ok(e) => e,
        Err(_) => return out, // 目录不存在 = 没装任何插件
    };

    for item in entries.flatten() {
        let dir = item.path();
        if !dir.is_dir() { continue; }
        let dir_name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let manifest_path = dir.join("plugin.json");

        let manifest: PluginManifest = match read_json_file_generic(&manifest_path) {
            Ok(m) => m,
            Err(e) => {
                out.push(Err(status_error(&dir_name, &dir, &format!("plugin.json 解析失败: {}", e))));
                continue;
            }
        };

        // 逐项校验，任何一项不过都拒绝加载（面板标红，不静默跳过）
        if !is_valid_plugin_id(&manifest.id) {
            out.push(Err(status_error(&dir_name, &dir, "插件 id 非法（需匹配 ^[a-z][a-z0-9-]{0,31}$）")));
            continue;
        }
        if manifest.api_version != SUPPORTED_API_VERSION {
            let msg = if manifest.api_version > SUPPORTED_API_VERSION {
                "插件协议版本高于主程序，请更新主程序"
            } else {
                "插件协议版本过低，请更新插件"
            };
            out.push(Err(status_error(&dir_name, &dir, msg)));
            continue;
        }
        if manifest.entry.is_empty() || manifest.entry.contains('\\') || manifest.entry.contains('/') {
            out.push(Err(status_error(&dir_name, &dir, "entry 缺失或含路径分隔符")));
            continue;
        }
        let exe_path = dir.join(&manifest.entry);
        if !exe_path.is_file() {
            out.push(Err(status_error(&dir_name, &dir, format!("入口文件不存在: {}", manifest.entry))));
            continue;
        }
        if manifest.capabilities.is_empty() {
            out.push(Err(status_error(&dir_name, &dir, "capabilities 为空")));
            continue;
        }

        out.push(Ok(LoadedPlugin { manifest, dir, exe_path }));
    }
    out
}

fn status_error(dir_name: &str, dir: &Path, message: impl Into<String>) -> PluginStatus {
    PluginStatus {
        id: dir_name.to_string(),
        name: dir_name.to_string(),
        version: String::new(),
        dir: dir.to_string_lossy().to_string(),
        status: "error".to_string(),
        message: message.into(),
        capabilities: Vec::new(),
        installed: None,
        enabled: true,
        entry_mtime: String::new(),
    }
}

// ─── 插件启停 ───

/// 读取配置里的停用插件 id 列表。读取失败（老配置无该字段、文件缺失）一律视为"全部启用"。
/// 配置由前端保存，宿主只读，不改写——避免与设置页的表单保存互相覆盖。
pub(crate) fn load_disabled_plugin_ids() -> Vec<String> {
    let Ok(data_dir) = get_data_dir() else { return Vec::new() };
    let cfg: AppConfig = match crate::storage::read_json_file(&data_dir.join("config.json")) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    cfg.disabled_plugins
}

/// 仅返回"已加载且未被用户停用"的插件（扫描 / 按 ID 查询实际使用的集合）
pub(crate) fn discover_enabled_plugins() -> Vec<LoadedPlugin> {
    let disabled = load_disabled_plugin_ids();
    discover_plugins()
        .into_iter()
        .flatten()
        .filter(|p| !disabled.iter().any(|d| d == &p.manifest.id))
        .collect()
}

fn read_json_file_generic<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("读取失败: {}", e))?;
    serde_json::from_str(&content).map_err(|e| format!("{}", e))
}

// ─── 目录探测 ───

/// 找出认领指定扫描目录的插件：dir_markers 全部存在 → 候选；
/// 多个候选按 priority 降序、同值按 id 字典序，取第一个。
pub(crate) fn find_plugin_for_dir<'a>(plugins: &'a [LoadedPlugin], dir: &Path) -> Option<&'a LoadedPlugin> {
    let mut candidates: Vec<&LoadedPlugin> = plugins.iter()
        .filter(|p| {
            p.manifest.capabilities.iter().any(|c| c == "scan_dir")
                && p.manifest.probe.dir_markers.iter().all(|m| dir.join(m).exists())
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.manifest.priority.cmp(&a.manifest.priority)
            .then_with(|| a.manifest.id.cmp(&b.manifest.id))
    });
    candidates.into_iter().next()
}

// ─── 插件进程调度 ───

/// 插件输出 JSON（api_version 1）
#[derive(Debug, Deserialize)]
pub(crate) struct PluginOutput {
    #[serde(default)]
    api_version: i64,
    #[serde(default)]
    items: Vec<PluginItem>,
    #[serde(default)]
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PluginItem {
    name: String,
    provider_item_id: String,
    #[serde(default)]
    exe_path: String,
    #[serde(default)]
    candidates: Vec<PluginExeCandidate>,
    #[serde(default)]
    launch_url: String,
    #[serde(default)]
    cover: Option<PluginCover>,
    #[serde(default)]
    meta: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PluginExeCandidate {
    exe_path: String,
    #[serde(default)]
    label: String,
}

#[derive(Debug, Deserialize)]
struct PluginCover {
    #[serde(default)]
    local_path: String,
    #[serde(default)]
    url: String,
}

/// 调度结果：归一化条目 + 警告
pub(crate) struct PluginScanOutcome {
    pub items: Vec<ExeInfo>,
    pub warnings: Vec<String>,
}

/// 调用插件子命令并解析输出。stdout 只放 JSON，日志走 stderr；
/// 超时 / 非 0 退出码 / 输出超限 / JSON 非法 都按失败处理，不采信部分结果。
fn run_plugin(plugin: &LoadedPlugin, args: &[&str]) -> Result<PluginOutput, String> {
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(&plugin.exe_path);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        // 插件是控制台程序，直接 spawn 会闪出一个 cmd 黑框。
        // CREATE_NO_WINDOW 让子进程继承「无控制台」状态，管道读写不受影响。
        // 这里统一在宿主侧加上，第三方插件作者无需自己处理。
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("插件进程启动失败: {}", e))?;

    // stdout/stderr 各用独立线程读取，避免管道写满导致子进程阻塞
    let mut stdout_pipe = child.stdout.take().ok_or("无法读取插件 stdout")?;
    let mut stderr_pipe = child.stderr.take().ok_or("无法读取插件 stderr")?;
    let stdout_reader = std::thread::spawn(move || read_capped(&mut stdout_pipe, MAX_STDOUT_BYTES));
    let stderr_reader = std::thread::spawn(move || read_capped(&mut stderr_pipe, MAX_STDERR_BYTES));

    // 轮询等待退出，超时 kill
    let deadline = Instant::now() + plugin.effective_timeout();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(format!("等待插件进程失败: {}", e)),
        }
    };

    let stdout_data = stdout_reader.join().unwrap_or_default();
    let stderr_data = stderr_reader.join().unwrap_or_default();
    let stderr_text = String::from_utf8_lossy(&stderr_data);

    let Some(status) = status else {
        return Err(format!("插件超时（{} 秒），已终止", plugin.effective_timeout().as_secs()));
    };
    if !status.success() {
        // 退出码非 0 视为失败，即使 stdout 有内容也不采信；stderr 首行作为错误详情
        let reason = stderr_text.lines().next().unwrap_or("").trim();
        return Err(if reason.is_empty() {
            format!("插件退出码非 0（{:?}）", status.code())
        } else {
            format!("插件失败: {}", truncate_chars(reason, 300))
        });
    }

    // 解析与契约校验（超限按"输出非法"处理，不做部分解析）
    if stdout_data.len() > MAX_STDOUT_BYTES {
        return Err("插件输出超过 8MB 上限".to_string());
    }
    let output: PluginOutput = serde_json::from_slice(&stdout_data)
        .map_err(|e| format!("插件输出格式错误: {}", e))?;
    if output.api_version != SUPPORTED_API_VERSION {
        return Err(format!("插件输出协议版本不支持（{}）", output.api_version));
    }
    if output.items.len() > MAX_ITEMS {
        return Err(format!("插件输出条目数超过 {} 上限", MAX_ITEMS));
    }
    Ok(output)
}

/// 读取流至多 cap 字节（超出部分丢弃但记录溢出，由调用方按上限判断处理）
fn read_capped(stream: &mut impl Read, cap: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let remain = cap.saturating_sub(buf.len());
                if remain > 0 {
                    let take = n.min(remain);
                    buf.extend_from_slice(&chunk[..take]);
                }
                // 继续读完以便子进程正常退出，但不再累积
            }
            Err(_) => break,
        }
    }
    buf
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max).collect::<String>()) }
}

/// 校验单条插件输出的字段长度（契约上限），返回原因
fn validate_item(item: &PluginItem) -> Result<(), String> {
    if item.name.is_empty() { return Err("条目 name 为空".to_string()); }
    if item.name.chars().count() > MAX_NAME_LEN { return Err("条目 name 超长".to_string()); }
    if item.provider_item_id.is_empty() { return Err("条目 provider_item_id 为空".to_string()); }
    if item.exe_path.chars().count() > MAX_PATH_LEN { return Err("条目 exe_path 超长".to_string()); }
    for c in &item.candidates {
        if c.exe_path.chars().count() > MAX_PATH_LEN { return Err("候选 exe_path 超长".to_string()); }
    }
    if item.launch_url.chars().count() > MAX_PATH_LEN { return Err("launch_url 超长".to_string()); }
    if let Some(cover) = &item.cover {
        if cover.local_path.chars().count() > MAX_PATH_LEN || cover.url.chars().count() > MAX_PATH_LEN {
            return Err("cover 路径超长".to_string());
        }
    }
    for (k, v) in &item.meta {
        if v.chars().count() > MAX_META_VALUE_LEN {
            return Err(format!("meta[{}] 值超长", truncate_chars(k, 32)));
        }
    }
    Ok(())
}

/// 插件输出 → ExeInfo 归一化：
/// 路径模式/存在性由宿主按现有规则计算；候选 exe 展开为多条（第一个为主程序）；
/// 图标不在扫描阶段提取（沿用导入时批量提取），封面只记录来源不下载。
pub(crate) fn normalize_plugin_output(plugin_id: &str, output: PluginOutput) -> Result<PluginScanOutcome, String> {
    let exe_dir = get_exe_directory()?;
    let mut items = Vec::new();
    let mut warnings = output.warnings;

    let warnings_bytes: usize = warnings.iter().map(|w| w.len() + 1).sum();
    if warnings_bytes > MAX_WARNINGS_BYTES {
        warnings.truncate(8);
        warnings.push("插件警告过多，已截断".to_string());
    }

    for item in output.items {
        validate_item(&item)?;

        // 候选 exe：未提供 candidates 时按单 exe 处理
        let mut candidates = item.candidates.clone();
        if candidates.is_empty() {
            candidates.push(PluginExeCandidate { exe_path: item.exe_path.clone(), label: String::new() });
        }
        let multi_exe = candidates.len() > 1;

        for (idx, cand) in candidates.iter().enumerate() {
            let abs = cand.exe_path.trim().to_string();
            if abs.chars().count() > MAX_PATH_LEN {
                return Err("exe 路径超长".to_string());
            }
            let abs_norm = if abs.is_empty() { String::new() } else { normalize_path(Path::new(&abs)).to_string_lossy().to_string() };
            let file_exists = !abs_norm.is_empty() && Path::new(&abs_norm).is_file();

            let (suggested_mode, rel_path) = if !abs_norm.is_empty() {
                match make_relative_path(&exe_dir, Path::new(&abs_norm)) {
                    Some(rel) => ("relative".to_string(), rel),
                    None => ("absolute".to_string(), String::new()),
                }
            } else {
                ("absolute".to_string(), String::new())
            };

            // 多候选时名称附 label（无 label 用 exe 文件名）区分
            let display_name = if multi_exe {
                let label = if cand.label.is_empty() {
                    Path::new(&abs_norm).file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
                } else {
                    cand.label.clone()
                };
                if label.is_empty() { item.name.clone() } else { format!("{} [{}]", item.name, label) }
            } else {
                item.name.clone()
            };

            items.push(ExeInfo {
                name: display_name,
                icon_path: String::new(),
                suggested_path_mode: suggested_mode,
                relative_path: rel_path,
                absolute_path: abs_norm,
                file_exists,
                provider: Some(ProviderRef {
                    id: plugin_id.to_string(),
                    item_id: item.provider_item_id.clone(),
                    launch_url: item.launch_url.clone(),
                    meta: item.meta.clone(),
                }),
                cover: item.cover.as_ref().map(|c| CoverSource {
                    local_path: c.local_path.clone(),
                    url: c.url.clone(),
                }),
                multi_exe,
                is_primary: idx == 0,
            });
        }
    }

    Ok(PluginScanOutcome { items, warnings })
}

/// 用插件扫描目录（宿主处理流程的插件分支）。
/// 返回 None 表示没有任何插件认领该目录（调用方走普通递归扫描）。
pub(crate) fn scan_dir_via_plugins(dir: &Path) -> Option<Result<PluginScanOutcome, String>> {
    let loaded = discover_enabled_plugins();
    let plugin = find_plugin_for_dir(&loaded, dir)?;
    Some(run_plugin(plugin, &["scan-dir", "--dir", &dir.to_string_lossy()])
        .and_then(|output| normalize_plugin_output(&plugin.manifest.id, output)))
}

// ─── 命令：插件列表 / 按 ID 查询 / 打开插件目录 ───

/// 各插件加载结果与运行状态（设置页插件面板）
#[command]
pub fn list_plugins() -> Result<Vec<PluginStatus>, String> {
    let mut out = Vec::new();
    // 停用列表一次性读出，避免逐插件读盘
    let disabled = load_disabled_plugin_ids();
    for result in discover_plugins() {
        match result {
            Ok(p) => {
                let entry_mtime = fs::metadata(&p.exe_path).ok()
                    .and_then(|m| m.modified().ok())
                    .map(|t| {
                        let dt: chrono::DateTime<chrono::Local> = t.into();
                        dt.format("%Y-%m-%d %H:%M").to_string()
                    })
                    .unwrap_or_default();
                out.push(PluginStatus {
                    id: p.manifest.id.clone(),
                    name: if p.manifest.name.is_empty() { p.manifest.id.clone() } else { p.manifest.name.clone() },
                    version: p.manifest.version.clone(),
                    dir: p.dir.to_string_lossy().to_string(),
                    status: "ok".to_string(),
                    message: String::new(),
                    capabilities: p.manifest.capabilities.clone(),
                    installed: p.manifest.installed_check.as_ref().and_then(|c| check_registry_value_exists(c)),
                    enabled: !disabled.iter().any(|d| d == &p.manifest.id),
                    entry_mtime,
                });
            }
            Err(st) => out.push(st),
        }
    }
    // 按插件 id 排序，面板顺序稳定
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// 按 ID 查询单个平台条目的结果（EntryEditor「平台游戏」用）
#[derive(Debug, Clone, Serialize)]
pub struct ProviderItemResult {
    pub name: String,
    /// 主程序 exe 绝对路径（可能为空，仅靠 launch_url 启动）
    pub exe_path: String,
    /// 启动链接（如 steam://rungameid/730）
    pub launch_url: String,
    /// 封面相对路径（已复制/下载到 assets/covers/<provider_id>/，空表示未获取到）
    pub cover_path: String,
    /// 图标相对路径（从游戏 exe 提取，可能为空）
    pub icon_path: String,
}

/// 平台条目 ID 合法性：非空、长度合理、不含控制字符（作为进程参数数组传递，无注入风险，
/// 校验只为防误用）
fn is_valid_item_id(id: &str) -> bool {
    !id.is_empty() && id.chars().count() <= 256 && id.chars().all(|c| !c.is_control())
}

/// 按平台 ID 查询单个条目（插件 scan-id 能力）。
/// 查询成功即完成封面落盘与图标提取（单条流程量小，即时处理体验更好）。
#[command]
pub fn scan_provider_item(provider_id: String, item_id: String) -> Result<ProviderItemResult, String> {
    let provider_id = provider_id.trim().to_string();
    let item_id = item_id.trim().to_string();
    if !is_valid_plugin_id(&provider_id) {
        return Err(format!("非法的平台标识: {}", provider_id));
    }
    if !is_valid_item_id(&item_id) {
        return Err("条目 ID 为空或含非法字符".to_string());
    }

    let plugin = discover_enabled_plugins().into_iter()
        .find(|p| p.manifest.id == provider_id)
        .ok_or_else(|| format!("未安装或已停用平台扫描插件: {}", provider_id))?;
    if !plugin.manifest.capabilities.iter().any(|c| c == "scan_id") {
        return Err(format!("插件 {} 不支持按 ID 查询", plugin.manifest.name));
    }

    let output = run_plugin(&plugin, &["scan-id", "--id", &item_id])?;
    let item = output.items.into_iter().next()
        .ok_or_else(|| format!("平台未返回条目（ID {} 不存在或未安装）", item_id))?;
    validate_item(&item)?;

    let exe_path = if item.exe_path.is_empty() {
        item.candidates.first().map(|c| c.exe_path.clone()).unwrap_or_default()
    } else {
        item.exe_path.clone()
    };
    let exe_path = if exe_path.is_empty() { String::new() } else { normalize_path(Path::new(&exe_path)).to_string_lossy().to_string() };

    // 封面：本地存在则复制，缺失走下载（2 秒超时，失败静默）
    let cover_path = match &item.cover {
        Some(c) => obtain_provider_cover(&CoverSource { local_path: c.local_path.clone(), url: c.url.clone() }, &provider_id, &item_id),
        None => String::new(),
    };
    // 图标：从主程序 exe 提取（失败静默，条目用默认图标）
    let icon_path = if exe_path.is_empty() {
        String::new()
    } else {
        crate::icons::extract_icon_to_png(&exe_path).unwrap_or_default()
    };

    Ok(ProviderItemResult {
        name: item.name,
        exe_path,
        launch_url: item.launch_url,
        cover_path,
        icon_path,
    })
}

/// 打开插件目录（资源管理器）
#[command]
pub fn open_plugins_dir() -> Result<(), String> {
    let plugins_root = get_exe_directory()?.join("plugins");
    let _ = ensure_dir(&plugins_root);
    crate::launch::shell_execute_open("explorer.exe", Some(&format!("\"{}\"", plugins_root.display())), false, 1)
        .map(|_| ())
}

// ─── 封面落盘 ───

/// 把插件给的封面来源落到 assets/covers/<provider_id>/<item_id>.<ext>：
/// 本地文件优先复制；缺失时下载 cover.url（2 秒超时，JPEG/PNG 校验，失败静默返回空）。
pub(crate) fn obtain_provider_cover(cover: &CoverSource, provider_id: &str, item_id: &str) -> String {
    // 目录名由插件 id 决定，id 已在发现阶段校验过，这里再防一次
    if !is_valid_plugin_id(provider_id) || item_id.is_empty() { return String::new(); }
    // item_id 只用作文件名，过滤路径分隔符与控制字符
    let safe_item: String = item_id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .collect();
    if safe_item.is_empty() { return String::new(); }

    let ext = cover.local_path.rsplit('.').next()
        .filter(|e| !e.is_empty() && e.len() <= 5 && !e.contains('\\') && !e.contains('/'))
        .unwrap_or("jpg");
    let file_name = format!("{}.{}", safe_item, ext);

    let assets_dir = match get_assets_dir() {
        Ok(d) => d,
        Err(_) => return String::new(),
    };
    let target_dir = assets_dir.join("covers").join(provider_id);
    if ensure_dir(&target_dir).is_err() {
        return String::new();
    }
    let target = target_dir.join(&file_name);

    // 目标已存在直接复用（同一游戏多 exe 并发导入时避免并发写同一文件失败）
    if target.is_file() {
        return rel_to_exe_dir(&target);
    }

    // 1. 本地文件复制
    if !cover.local_path.is_empty() {
        let src = Path::new(&cover.local_path);
        if src.is_file() && fs::copy(src, &target).is_ok() {
            return rel_to_exe_dir(&target);
        }
    }

    // 2. 下载（与 favicon 相同的 2 秒超时客户端）
    if !cover.url.is_empty() {
        if let Some(bytes) = download_image(&cover.url) {
            if fs::write(&target, &bytes).is_ok() {
                return rel_to_exe_dir(&target);
            }
        }
    }
    String::new()
}

/// 下载图片（2 秒超时），校验 JPEG/PNG 魔数，失败返回 None
fn download_image(url: &str) -> Option<Vec<u8>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let bytes = client.get(url).send().and_then(|r| r.bytes()).ok()?;
    let v = bytes.to_vec();
    let is_jpeg = v.starts_with(&[0xFF, 0xD8, 0xFF]);
    let is_png = v.starts_with(&[0x89, 0x50, 0x4E, 0x47]);
    if v.is_empty() || (!is_jpeg && !is_png) { return None; }
    Some(v)
}

// ─── 注册表探测（installed_check） ───

/// 检查注册表值是否存在（仅用于插件面板显示"已检测到/未安装"，不参与扫描）
#[cfg(windows)]
fn check_registry_value_exists(check: &PluginInstalledCheck) -> Option<bool> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    let hive = match check.hive.to_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        _ => return None,
    };
    if check.key.is_empty() || check.value.is_empty() { return None; }

    unsafe {
        let sub_key = to_wide(&check.key);
        let value_name = to_wide(&check.value);
        let mut key = HKEY::default();
        if RegOpenKeyW(hive, PCWSTR(sub_key.as_ptr()), &mut key).is_err() {
            return Some(false);
        }
        let mut buf_len: u32 = 0;
        let result = RegQueryValueExW(
            key,
            PCWSTR(value_name.as_ptr()),
            None,
            None,
            None,
            Some(&mut buf_len),
        );
        let _ = RegCloseKey(key);
        Some(result.is_ok())
    }
}

#[cfg(not(windows))]
fn check_registry_value_exists(_check: &PluginInstalledCheck) -> Option<bool> {
    None
}
