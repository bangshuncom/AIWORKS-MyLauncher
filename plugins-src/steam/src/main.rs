// ════════════════════════════════════════════
// Steam 扫描插件（MyLauncher 平台插件协议 api_version 1）
//
// 用法：
//   steam-scanner scan-dir --dir <Steam 安装目录>
//   steam-scanner scan-id  --id <appid>
//
// stdout 只输出 JSON（协议见 docs/plugin-architecture.md），日志走 stderr。
// 不读不写主程序任何数据，纯函数：输入路径，输出清单。
// ════════════════════════════════════════════

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::exit;

/// 扫描条目（协议 api_version 1）
#[derive(Serialize)]
struct ScanItem {
    name: String,
    provider_item_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    exe_path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    candidates: Vec<ExeCandidate>,
    #[serde(skip_serializing_if = "String::is_empty")]
    launch_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cover: Option<Cover>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    meta: BTreeMap<String, String>,
}

#[derive(Serialize, Clone)]
struct ExeCandidate {
    exe_path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    label: String,
}

#[derive(Serialize)]
struct Cover {
    #[serde(skip_serializing_if = "String::is_empty")]
    local_path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    url: String,
}

#[derive(Serialize)]
struct ScanOutput {
    api_version: u32,
    items: Vec<ScanItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [cmd, flag, dir] if cmd == "scan-dir" && flag == "--dir" => scan_dir(Path::new(dir)),
        [cmd, flag, id] if cmd == "scan-id" && flag == "--id" => scan_id(id.trim()),
        _ => Err("用法: steam-scanner scan-dir --dir <路径> | scan-id --id <appid>".to_string()),
    };

    match result {
        Ok(output) => {
            // stdout 只放 JSON；任何日志在成功路径上不输出
            println!("{}", serde_json::to_string(&output).unwrap_or_else(|_| "{}".to_string()));
        }
        Err(msg) => {
            eprintln!("{}", msg);
            exit(1);
        }
    }
}

// ─── VDF（acf / libraryfolders）行级 key-value 解析 ───

/// 从 "key" "value" 形式的行中提取 value
fn acf_value(line: &str, key: &str) -> Option<String> {
    let rest = line.trim().strip_prefix(&format!("\"{}\"", key))?;
    let rest = rest.trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// 解析 libraryfolders.vdf，返回所有库的 steamapps 目录（含主库，去重保序）
fn library_dirs(steam_root: &Path) -> Vec<PathBuf> {
    let main_steamapps = steam_root.join("steamapps");
    let mut libs = vec![main_steamapps.clone()];
    let content = match std::fs::read_to_string(main_steamapps.join("libraryfolders.vdf")) {
        Ok(c) => c,
        Err(_) => return libs,
    };
    for line in content.lines() {
        if let Some(p) = acf_value(line, "path") {
            if p.is_empty() { continue; }
            let dir = PathBuf::from(p.replace("\\\\", "\\")).join("steamapps");
            if !libs.contains(&dir) {
                libs.push(dir);
            }
        }
    }
    libs
}

// ─── exe 候选启发式 ───

/// 在游戏安装目录下递归查找候选 exe（跳过卸载器 / redist / win* 开头）。
/// 返回全部候选，第一个视为主程序。
fn find_game_exes(install_dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in walkdir::WalkDir::new(install_dir)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() { continue; }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        if ext != "exe" { continue; }
        let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        if fname.starts_with("unins") { continue; }
        if fname.starts_with("win") { continue; }
        let rel = path.strip_prefix(install_dir).unwrap_or(path).to_string_lossy().to_lowercase();
        if rel.contains("redist") || rel.contains("vcredist") || rel.contains("directx") {
            continue;
        }
        found.push(path.to_path_buf());
    }
    found
}

// ─── 封面来源 ───

/// 封面：本地缓存 appcache/librarycache/{appid}_header.jpg（位于 Steam 主目录），
/// 缺失时宿主按 url 下载。只给来源，不复制不下载——那是宿主的职责。
fn cover_source(steam_root: &Path, appid: &str) -> Cover {
    Cover {
        local_path: steam_root.join("appcache").join("librarycache")
            .join(format!("{}_header.jpg", appid)).to_string_lossy().to_string(),
        url: format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{}/header.jpg", appid),
    }
}

// ─── acf 清单解析 ───

/// 读取 appmanifest_{appid}.acf，返回 (name, installdir)
fn parse_manifest(path: &Path) -> Option<(String, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut name = String::new();
    let mut installdir = String::new();
    for line in content.lines() {
        if let Some(v) = acf_value(line, "name") { name = v; }
        else if let Some(v) = acf_value(line, "installdir") { installdir = v; }
    }
    if name.is_empty() || installdir.is_empty() { None } else { Some((name, installdir)) }
}

/// 把一个已安装游戏构造成扫描条目（多候选 exe 展开说明交给 candidates，
/// 名称区分由宿主负责）
fn build_item(steam_root: &Path, appid: &str, name: &str, installdir: &str, lib_steamapps: &Path) -> ScanItem {
    let install_dir = lib_steamapps.join("common").join(installdir);
    let exes = find_game_exes(&install_dir);
    let candidates: Vec<ExeCandidate> = exes.iter()
        .map(|p| {
            let label = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            ExeCandidate { exe_path: p.to_string_lossy().to_string(), label }
        })
        .collect();
    let mut meta = BTreeMap::new();
    meta.insert("install_dir".to_string(), installdir.to_string());
    ScanItem {
        name: name.to_string(),
        provider_item_id: appid.to_string(),
        exe_path: candidates.first().map(|c| c.exe_path.clone()).unwrap_or_default(),
        candidates,
        launch_url: format!("steam://rungameid/{}", appid),
        cover: Some(cover_source(steam_root, appid)),
        meta,
    }
}

// ─── scan-dir ───

/// 扫描 Steam 安装目录：遍历所有库的 appmanifest_*.acf 生成游戏清单。
/// --dir 必须是含 steamapps 子目录的 Steam 目录（宿主已按 probe 规则认领）。
fn scan_dir(dir: &Path) -> Result<ScanOutput, String> {
    let steamapps = dir.join("steamapps");
    if !steamapps.is_dir() {
        return Err(format!("不是 Steam 目录（缺少 steamapps）: {}", dir.display()));
    }

    let mut items = Vec::new();
    let mut warnings = Vec::new();

    for lib in library_dirs(dir) {
        let entries = match std::fs::read_dir(&lib) {
            Ok(e) => e,
            Err(_) => {
                warnings.push(format!("库目录无法访问: {}", lib.display()));
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if !fname.starts_with("appmanifest_") || !fname.ends_with(".acf") { continue; }
            let appid = fname
                .strip_prefix("appmanifest_").unwrap_or("")
                .strip_suffix(".acf").unwrap_or("")
                .to_string();
            if appid.is_empty() { continue; }
            let Some((name, installdir)) = parse_manifest(&path) else { continue };
            items.push(build_item(dir, &appid, &name, &installdir, &lib));
        }
    }

    Ok(ScanOutput { api_version: 1, items, warnings })
}

// ─── scan-id ───

/// 从注册表 HKCU\Software\Valve\Steam\SteamPath 读取 Steam 安装目录
#[cfg(windows)]
fn steam_root_from_registry() -> Result<PathBuf, String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey(r"Software\Valve\Steam") {
        if let Ok(path) = key.get_value::<String, _>("SteamPath") {
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }
    // 部分 Steam 安装只写了 HKLM（DllFallback），兜底读一次
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(key) = hklm.open_subkey(r"SOFTWARE\Valve\Steam") {
        if let Ok(path) = key.get_value::<String, _>("SteamPath") {
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }
    Err("未检测到 Steam 客户端（注册表 Software\\Valve\\Steam）".to_string())
}

#[cfg(not(windows))]
fn steam_root_from_registry() -> Result<PathBuf, String> {
    Err("此插件仅支持 Windows".to_string())
}

/// 按appid 查询单个已安装游戏：注册表定位 Steam → 遍历所有库找 appmanifest
fn scan_id(appid: &str) -> Result<ScanOutput, String> {
    if appid.is_empty() {
        return Err("appid 为空".to_string());
    }
    let steam_root = steam_root_from_registry()?;
    let manifest_file = format!("appmanifest_{}.acf", appid);

    for lib in library_dirs(&steam_root) {
        let manifest_path = lib.join(&manifest_file);
        if !manifest_path.is_file() { continue; }
        let Some((name, installdir)) = parse_manifest(&manifest_path) else { continue };
        let install_dir = lib.join("common").join(&installdir);
        if !install_dir.is_dir() {
            return Err(format!("游戏目录不存在: {}", install_dir.display()));
        }
        return Ok(ScanOutput {
            api_version: 1,
            items: vec![build_item(&steam_root, appid, &name, &installdir, &lib)],
            warnings: Vec::new(),
        });
    }

    Err(format!("appid {} 不存在或未安装", appid))
}
