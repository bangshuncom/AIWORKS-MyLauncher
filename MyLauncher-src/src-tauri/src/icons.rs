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
use crate::icons_refresh::*;
use crate::icons_system::*;
use crate::import::*;
use crate::launch::*;
use crate::models::*;
use crate::paths::*;
use crate::scan::*;
use crate::storage::*;


// ═══════════════════════════════════════════
// 图标提取（程序 / 文件 / 快捷方式 / APPX）
// ═══════════════════════════════════════════

// ─── 图标提取 ───

/// COM 初始化守卫：初始化 COM 并在作用域结束时调用 CoUninitialize。
/// 使用方式：`let _com = com_init()?;`
#[cfg(windows)]
pub(crate) struct ComGuard;
#[cfg(windows)]
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

/// RPC_E_CHANGED_MODE 常量值
#[cfg(windows)]
pub(crate) const RPC_E_CHANGED_MODE: windows::core::HRESULT = windows::core::HRESULT(-2147417850i32);

/// 初始化 COM（允许重复调用，容忍 RPC_E_CHANGED_MODE）。
/// 返回 ComGuard，drop 时自动调用 CoUninitialize。
#[cfg(windows)]
pub(crate) fn com_init() -> Result<ComGuard, String> {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.0 < 0 && hr != RPC_E_CHANGED_MODE {
            return Err(format!("CoInitializeEx 失败: 0x{:08X}", hr.0 as u32));
        }
    }
    Ok(ComGuard)
}

/// 从 IShellItemImageFactory 获取指定尺寸的图标位图（BGRA → RGBA）
#[cfg(windows)]
pub(crate) fn get_shell_item_image(
    factory: &IShellItemImageFactory,
    size: i32,
) -> Option<image::RgbaImage> {
    use windows::Win32::Graphics::Gdi::GetObjectW;

    let h_bitmap = unsafe {
        factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY).ok()?
    };

    let mut bitmap = BITMAP::default();
    unsafe {
        GetObjectW(
            h_bitmap,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bitmap as *mut _ as *mut _),
        );
    }

    let width = bitmap.bmWidth as u32;
    let height = bitmap.bmHeight as u32;
    if width == 0 || height == 0 || bitmap.bmBits.is_null() {
        return None;
    }

    let pixel_data: &[u8] = unsafe {
        std::slice::from_raw_parts(
            bitmap.bmBits as *const u8,
            (width * height * 4) as usize,
        )
    };

    let mut pixels = pixel_data.to_vec();
    for chunk in pixels.chunks_mut(4) {
        chunk.swap(0, 2); // BGRA → RGBA
    }

    image::RgbaImage::from_raw(width, height, pixels)
}

/// 核心：提取 exe/文件的图标为 RgbaImage（COM 初始化 + 256px 图标 + 透明度回退 + 垂直翻转）。
/// 调用方负责保存 PNG。
#[cfg(windows)]
pub(crate) fn extract_icon_image(exe_path: &str) -> Result<image::RgbaImage, String> {
    let _com = com_init()?;

    let path_h = HSTRING::from(exe_path);
    let factory: IShellItemImageFactory = unsafe {
        SHCreateItemFromParsingName(PCWSTR(path_h.as_ptr()), None)
            .map_err(|e| format!("创建 IShellItemImageFactory 失败: {}", e))?
    };

    let mut image_buffer = get_shell_item_image(&factory, 256)
        .ok_or("无法获取图标（文件可能没有图标）")?;

    // 透明像素占比 >= 70% 时回退 48px 获取真实小图标，避免模糊
    let mut transparency: f64 = 0.0;
    let mut non_transparency: f64 = 0.0;
    for pixel in image_buffer.pixels() {
        if pixel[3] == 0 {
            transparency += 1.0;
        } else {
            non_transparency += 1.0;
        }
    }
    let total = transparency + non_transparency;
    let transparent_ratio = if total > 0.0 {
        (transparency / total * 100.0).round() as u32
    } else {
        0
    };
    if transparent_ratio >= 70 {
        if let Some(small) = get_shell_item_image(&factory, 48) {
            image_buffer = small;
        }
    }

    // BITMAP 数据是 bottom-up，需垂直翻转
    Ok(image::imageops::flip_vertical(&image_buffer))
}

/// 提取图标并保存到 assets/icons/extracted/，返回相对路径
#[cfg(windows)]
pub(crate) fn extract_icon_to_png(exe_path: &str) -> Result<String, String> {
    let image_buffer = extract_icon_image(exe_path)?;

    let assets_dir = get_assets_dir()?;
    let icon_dir = assets_dir.join("icons").join("extracted");
    ensure_dir(&icon_dir)?;

    let safe_name = Path::new(exe_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("icon")
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect::<String>();

    // 毫秒级时间戳：同秒内多次提取（如先单条重置又批量）不会生成同名文件互相覆盖
    let timestamp = chrono::Local::now().format("%H%M%S%3f").to_string();
    let icon_file = icon_dir.join(format!("{}_{}.png", safe_name, timestamp));

    image_buffer
        .save(&icon_file)
        .map_err(|e| format!("保存图标失败: {}", e))?;

    Ok(rel_to_exe_dir(&icon_file))
}

/// 提取图标并保存到指定路径
#[cfg(windows)]
pub(crate) fn extract_icon_to_path(exe_path: &str, icon_file: &Path) -> Result<(), String> {
    let image_buffer = extract_icon_image(exe_path)?;
    image_buffer
        .save(icon_file)
        .map_err(|e| format!("保存图标失败: {}", e))?;
    Ok(())
}



#[command]
pub fn get_exe_info(exe_path: String) -> Result<ExeInfo, String> {
    let path = Path::new(&exe_path);
    if !path.exists() {
        return Ok(ExeInfo {
            name: String::new(),
            icon_path: String::new(),
            suggested_path_mode: "absolute".to_string(),
            relative_path: String::new(),
            absolute_path: exe_path.clone(),
            file_exists: false,
            provider: None,
            cover: None,
            multi_exe: false,
            is_primary: true,
        });
    }

    let name = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("未知")
        .to_string();

    let exe_dir = get_exe_directory()?;

    // 同盘符时建议相对路径（..\..\ 形式），不同盘符时用绝对路径
    let (suggested_mode, rel_path) = if let Some(rel) = make_relative_path(&exe_dir, path) {
        ("relative", rel)
    } else {
        ("absolute", String::new())
    };

    // 尝试提取图标
    let icon_path = extract_icon_to_png(&exe_path).unwrap_or_default();

    Ok(ExeInfo {
        name,
        icon_path,
        suggested_path_mode: suggested_mode.to_string(),
        relative_path: rel_path,
        absolute_path: exe_path.clone(),
        file_exists: true,
        provider: None,
        cover: None,
        multi_exe: false,
        is_primary: true,
    })
}

// ─── 文件/文件夹信息 ───

/// 文件/文件夹信息（用于添加文件夹和文件条目）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    pub entry_type: String,   // "folder" | "file"
    pub icon_path: String,    // 提取的图标相对路径
    pub suggested_path_mode: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub file_exists: bool,
}

/// 获取文件/文件夹信息，图标按类型/扩展名缓存去重
#[command]
pub fn get_file_info(file_path: String) -> Result<FileInfo, String> {
    let path = Path::new(&file_path);
    if !path.exists() {
        return Ok(FileInfo {
            name: String::new(),
            entry_type: "file".to_string(),
            icon_path: String::new(),
            suggested_path_mode: "absolute".to_string(),
            relative_path: String::new(),
            absolute_path: file_path.clone(),
            file_exists: false,
        });
    }

    let is_folder = path.is_dir();
    let entry_type = if is_folder { "folder" } else { "file" };

    let name = path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(if is_folder { "文件夹" } else { "文件" })
        .to_string();

    let exe_dir = get_exe_directory()?;
    let (suggested_mode, rel_path) = if let Some(rel) = make_relative_path(&exe_dir, path) {
        ("relative", rel)
    } else {
        ("absolute", String::new())
    };

    // 图标缓存：文件夹共用一个图标，文件按扩展名共用
    let assets_dir = get_assets_dir()?;
    let cache_dir = assets_dir.join("icons").join("file_cache");
    ensure_dir(&cache_dir)?;

    let cache_key = if is_folder {
        "folder".to_string()
    } else {
        let ext = path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("none")
            .to_lowercase();
        format!("ext_{}", ext)
    };
    let icon_file = cache_dir.join(format!("{}.png", cache_key));

    // 已有缓存则直接用，没有则提取
    let icon_path = if icon_file.exists() {
        rel_to_exe_dir(&icon_file)
    } else {
        match extract_icon_to_path(&file_path, &icon_file) {
            Ok(()) => rel_to_exe_dir(&icon_file),
            Err(_) => String::new(),
        }
    };

    Ok(FileInfo {
        name,
        entry_type: entry_type.to_string(),
        icon_path,
        suggested_path_mode: suggested_mode.to_string(),
        relative_path: rel_path,
        absolute_path: file_path.clone(),
        file_exists: true,
    })
}

// ─── 快捷方式解析 ───

/// Windows 快捷方式解析核心：用 IShellLinkW + IPersistFile 读取 .lnk 属性
#[cfg(windows)]
pub(crate) fn resolve_lnk_file(lnk_path: &str) -> Result<LnkInfo, String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let to_wide = |s: &str| -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    };

    // 1. 初始化 COM
    let _com = com_init()?;

    // 2. 创建 IShellLinkW 实例
    // ShellLink CLSID: {00021401-0000-0000-C000-000000000046}
    const SHELL_LINK_CLSID: GUID = GUID::from_u128(0x00021401_0000_0000_C000_000000000046);
    let shell_link: IShellLinkW = unsafe {
        CoCreateInstance(
            &SHELL_LINK_CLSID,
            None,
            CLSCTX_INPROC_SERVER,
        )
        .map_err(|e| format!("创建 IShellLinkW 失败: {}", e))?
    };

    // 3. 用 IPersistFile 加载 .lnk 文件
    let persist_file: IPersistFile = shell_link
        .cast()
        .map_err(|e| format!("获取 IPersistFile 失败: {}", e))?;

    let lnk_w = to_wide(lnk_path);
    unsafe {
        persist_file.Load(PCWSTR(lnk_w.as_ptr()), STGM(0))
            .map_err(|e| format!("加载 .lnk 失败: {}", e))?;
    }

    // 4. 读取各属性
    let exe_dir = get_exe_directory()?;

    // 目标路径
    let mut target_buf = [0u16; 260];
    let mut find_data = WIN32_FIND_DATAW::default();
    unsafe {
        shell_link.GetPath(&mut target_buf, &mut find_data, 0)
            .map_err(|e| format!("GetPath 失败: {}", e))?;
    }
    let target_path = String::from_utf16_lossy(
        &target_buf[..target_buf.iter().position(|&c| c == 0).unwrap_or(0)]
    );

    // 工作目录
    let mut work_dir_buf = [0u16; 260];
    unsafe {
        shell_link.GetWorkingDirectory(&mut work_dir_buf)
            .ok();
    }
    let working_directory = String::from_utf16_lossy(
        &work_dir_buf[..work_dir_buf.iter().position(|&c| c == 0).unwrap_or(0)]
    );

    // 启动参数
    let mut args_buf = [0u16; 512];
    unsafe {
        shell_link.GetArguments(&mut args_buf)
            .ok();
    }
    let arguments = String::from_utf16_lossy(
        &args_buf[..args_buf.iter().position(|&c| c == 0).unwrap_or(0)]
    );

    // 运行方式
    let show_cmd = unsafe { shell_link.GetShowCmd() }.unwrap_or(SHOW_WINDOW_CMD(1));
    let window_style = match show_cmd.0 {
        3 => "maximized",   // SW_MAXIMIZE
        2 => "minimized",   // SW_MINIMIZE
        _ => "normal",
    };

    // 备注（description）
    let mut desc_buf = [0u16; 1024];
    unsafe {
        shell_link.GetDescription(&mut desc_buf)
            .ok();
    }
    let description = String::from_utf16_lossy(
        &desc_buf[..desc_buf.iter().position(|&c| c == 0).unwrap_or(0)]
    );

    // 快捷方式文件名（不含扩展名）
    let lnk_name = Path::new(lnk_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("快捷方式")
        .to_string();

    // 目标是否存在
    let target_exists = Path::new(&target_path).exists();

    // 同盘符时建议相对路径
    let target_path_obj = Path::new(&target_path);
    let (suggested_mode, rel_path) = if !target_path.is_empty() {
        if let Some(rel) = make_relative_path(&exe_dir, target_path_obj) {
            ("relative", rel)
        } else {
            ("absolute", String::new())
        }
    } else {
        ("absolute", String::new())
    };

    // 提取目标程序图标
    let icon_path = if target_exists {
        extract_icon_to_png(&target_path).unwrap_or_default()
    } else {
        String::new()
    };

    Ok(LnkInfo {
        name: lnk_name,
        target_path,
        target_exists,
        working_directory,
        arguments,
        window_style: window_style.to_string(),
        description,
        icon_path,
        suggested_path_mode: suggested_mode.to_string(),
        relative_path: rel_path,
    })
}


#[command]
pub fn resolve_lnk(lnk_path: String) -> Result<LnkInfo, String> {
    let path = Path::new(&lnk_path);
    if !path.exists() {
        return Err(format!("文件不存在: {}", lnk_path));
    }
    let ext = path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext != "lnk" {
        return Err(format!("仅支持 .lnk 快捷方式文件，当前: .{}", ext));
    }
    resolve_lnk_file(&lnk_path)
}

// ─── APPX / UWP 应用枚举 ───

/// APPX 应用信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppxAppInfo {
    /// 显示名称
    pub name: String,
    /// shell:AppsFolder\{family_name}!{app_id}
    pub shell_uri: String,
    /// 应用唯一标识（PackageFullName!AppId），用于图标缓存命名
    pub app_key: String,
    /// 提取的图标相对路径（assets/icons/extracted/ 下）
    pub icon_path: String,
}

/// 枚举开始菜单应用（含 APPX/UWP + 传统快捷方式），返回 AUMID（含 !）的 UWP 应用。
/// 纯 Win32 Shell API 实现（IShellItem 枚举 shell:AppsFolder），不再拉起 powershell：
/// 原 Get-StartApps 方案是"无签名 exe 调用 powershell"，容易触发杀软行为告警，
/// 且 powershell 冷启动需 1-2 秒；原生枚举同进程完成，数据与 Get-StartApps 一致。
#[cfg(windows)]
pub(crate) fn enumerate_appx_apps() -> Result<Vec<AppxAppInfo>, String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        BHID_EnumItems, IEnumShellItems, IShellItem, SIGDN_DESKTOPABSOLUTEPARSING,
        SIGDN_NORMALDISPLAY,
    };

    let _com = com_init()?;

    let apps_uri = HSTRING::from("shell:AppsFolder");
    let folder: IShellItem = unsafe {
        SHCreateItemFromParsingName(PCWSTR(apps_uri.as_ptr()), None)
            .map_err(|e| format!("打开 shell:AppsFolder 失败: {}", e))?
    };
    let enum_items: IEnumShellItems = unsafe {
        folder
            .BindToHandler(None, &BHID_EnumItems)
            .map_err(|e| format!("枚举 AppsFolder 失败: {}", e))?
    };

    // PWSTR（CoTaskMem 分配的 UTF-16 串）→ String，读后必须 CoTaskMemFree 释放
    let pwstr_to_string = |p: windows::core::PWSTR| -> String {
        unsafe {
            if p.0.is_null() {
                return String::new();
            }
            let mut len = 0usize;
            while *p.0.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p.0, len));
            CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
            s
        }
    };

    let mut apps = Vec::new();
    loop {
        let mut slot: [Option<IShellItem>; 1] = std::array::from_fn(|_| None);
        let mut fetched: u32 = 0;
        let hr = unsafe { enum_items.Next(&mut slot, Some(&mut fetched)) };
        if hr.is_err() || fetched == 0 {
            break;
        }
        let Some(item) = slot[0].take() else { break };

        // 显示名走 NORMALDISPLAY；解析名走 DESKTOPABSOLUTEPARSING
        let name = unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY) }
            .map(pwstr_to_string)
            .unwrap_or_default()
            .trim()
            .to_string();
        let parsing = unsafe { item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) }
            .map(pwstr_to_string)
            .unwrap_or_default();

        // 解析名形如 "shell:AppsFolder\<AUMID>" 或 "::{CLSID_AppsFolder}\<AUMID>"，
        // AUMID 本身不含反斜杠，取最后一段即为 AUMID；
        // .lnk 条目的解析名是文件路径（无 !），与 Get-StartApps 的 AppID 规则一致，在此排除
        let appid = parsing
            .rsplit('\\')
            .next()
            .unwrap_or(&parsing)
            .trim()
            .to_string();
        if name.is_empty() || appid.is_empty() || !appid.contains('!') {
            continue;
        }

        let shell_uri = format!("shell:AppsFolder\\{}", appid);
        let app_key: String = appid
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();

        apps.push(AppxAppInfo {
            name,
            shell_uri,
            app_key,
            icon_path: String::new(),
        });
    }

    // 按名称排序
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(apps)
}


/// 枚举 APPX 应用（不提取图标，前端按需批量提取）
#[command]
pub fn list_appx_apps() -> Result<Vec<AppxAppInfo>, String> {
    enumerate_appx_apps()
}
