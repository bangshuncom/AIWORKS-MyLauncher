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
use crate::import::*;
use crate::launch::*;
use crate::models::*;
use crate::paths::*;
use crate::scan::*;
use crate::storage::*;


// ═══════════════════════════════════════════
// 系统功能图标提取（sys: / glyph:）与回归测试
// ═══════════════════════════════════════════

// ─── 系统功能图标提取（sys: 系统文件/别名 | glyph: Segoe 字形） ───
//
// 背景：系统管理工具的专属图标不在 .msc 文件里（.msc 无图标资源），
// 而在独立的资源 DLL 中（如设备管理器在 devmgr.dll 组索引 5）；
// ms-settings: 类条目没有文件，用 Segoe 图标字体字形渲染。
// 本组图标源已在本机逐项实测（见 .temp/sysmap_probe.py）。

/// 系统图标缓存：assets/icons/extracted/system_{key}.png，key 由规格字符串净化而来
pub(crate) fn system_icon_cache_key(spec: &str) -> String {
    spec.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' { c } else { '_' })
        .collect()
}

/// 环境变量展开：把 %name% 替换为环境变量值（未定义的保留原样）
#[cfg(windows)]
pub(crate) fn expand_env_path(path: &str) -> PathBuf {
    let mut out = String::with_capacity(path.len());
    let bytes: Vec<char> = path.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '%' {
            if let Some(end) = bytes[i + 1..].iter().position(|&c| c == '%') {
                let name: String = bytes[i + 1..i + 1 + end].iter().collect();
                if let Ok(v) = std::env::var(&name) {
                    out.push_str(&v);
                    i += end + 2;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(out)
}

/// 解析应用执行别名（%LOCALAPPDATA%\Microsoft\WindowsApps\{name}）。
/// 别名是 IO_REPARSE_TAG_APPEXECLINK (0x8000001B) reparse 点，不是普通符号链接，
/// 需 FSCTL_GET_REPARSE_POINT 读取原始数据后手工解析出包内真实 exe 路径。
#[cfg(windows)]
pub(crate) fn resolve_app_alias(name: &str) -> Option<PathBuf> {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::FSCTL_GET_REPARSE_POINT;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ};

    const APPEXECLINK_TAG: u32 = 0x8000_001B;

    let alias_path = std::env::var("LOCALAPPDATA").ok()?;
    let alias = PathBuf::from(alias_path)
        .join("Microsoft")
        .join("WindowsApps")
        .join(name);
    if !alias.exists() {
        return None;
    }

    let wide: Vec<u16> = alias.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0),
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .ok()?;

    let result = unsafe {
        let mut buf = [0u8; 16 * 1024];
        let mut returned: u32 = 0;
        let ok = DeviceIoControl(
            handle,
            FSCTL_GET_REPARSE_POINT,
            None,
            0,
            Some(buf.as_mut_ptr() as *mut _),
            buf.len() as u32,
            Some(&mut returned),
            None,
        )
        .is_ok();
        if !ok {
            None
        } else {
            // ReparseTag(u32) + ReparseDataLength(u16) + Reserved(u16) + Version(u32) = 12 字节头
            let tag = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
            if tag != APPEXECLINK_TAG || returned < 14 {
                None
            } else {
                // 数据区为 UTF-16LE 的若干个以 \0 结尾的字符串（以空串终止）：
                // 包族名 / EntryPoint / 可执行文件路径 / 应用类型 …
                let data = &buf[12..returned as usize];
                let mut cur: Vec<u16> = Vec::new();
                let mut strings: Vec<String> = Vec::new();
                for chunk in data.chunks_exact(2) {
                    let u = u16::from_le_bytes([chunk[0], chunk[1]]);
                    if u == 0 {
                        if cur.is_empty() {
                            break;
                        }
                        strings.push(String::from_utf16_lossy(&cur));
                        cur.clear();
                    } else {
                        cur.push(u);
                    }
                }
                // 取最后一个形如绝对路径且以 .exe 结尾的字符串（顺序无关）
                strings
                    .into_iter()
                    .rev()
                    .find(|s| {
                        s.len() > 4
                            && s.to_lowercase().ends_with(".exe")
                            && s.contains('\\')
                    })
                    .map(PathBuf::from)
            }
        }
    };
    let _ = unsafe { CloseHandle(handle) };
    let resolved = result?;
    if resolved.exists() { Some(resolved) } else { None }
}

/// 解析 sys: 规格中的文件路径为真实文件：
/// 含分隔符 → 环境变量展开；裸命令名 → System32 / %windir% / PowerShell 目录，
/// 都找不到再尝试应用执行别名（Win11 的 mspaint.exe / snippingtool.exe 属于此类）
#[cfg(windows)]
pub(crate) fn resolve_sys_file(file: &str) -> Option<PathBuf> {
    if file.contains('\\') || file.contains('/') {
        let p = expand_env_path(file);
        return if p.exists() { Some(p) } else { None };
    }
    let windir = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".to_string());
    let candidates = [
        PathBuf::from(&windir).join("System32").join(file),
        PathBuf::from(&windir).join(file),
        PathBuf::from(&windir).join("System32").join("WindowsPowerShell").join("v1.0").join(file),
    ];
    if let Some(p) = candidates.into_iter().find(|p| p.exists()) {
        return Some(p);
    }
    resolve_app_alias(file)
}

/// HICON → RgbaImage（GetIconInfo + GetDIBits，32bpp BGRA → RGBA，含垂直翻转与句柄释放）
#[cfg(windows)]
pub(crate) fn hicon_to_rgba(hicon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<image::RgbaImage> {
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO, DestroyIcon};
    use windows::Win32::Graphics::Gdi::{GetObjectW, BITMAP, DeleteObject, CreateCompatibleDC, DeleteDC, GetDIBits, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS};

    unsafe {
        let mut info = ICONINFO::default();
        if GetIconInfo(hicon, &mut info).is_err() {
            return None;
        }
        // 无论后续成败都要释放的资源
        struct Cleanup(windows::Win32::UI::WindowsAndMessaging::HICON, windows::Win32::Graphics::Gdi::HBITMAP, windows::Win32::Graphics::Gdi::HBITMAP);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyIcon(self.0);
                    if !self.1.is_invalid() { let _ = DeleteObject(self.1); }
                    if !self.2.is_invalid() { let _ = DeleteObject(self.2); }
                }
            }
        }
        let _cleanup = Cleanup(hicon, info.hbmMask, info.hbmColor);

        if info.hbmColor.is_invalid() {
            return None;
        }
        let mut bmp = BITMAP::default();
        if GetObjectW(
            info.hbmColor,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bmp as *mut _ as *mut _),
        ) == 0
        {
            return None;
        }
        let (w, h) = (bmp.bmWidth as u32, bmp.bmHeight as u32);
        if w == 0 || h == 0 {
            return None;
        }

        let hdc = CreateCompatibleDC(None);
        if hdc.is_invalid() {
            return None;
        }
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w as i32;
        bi.bmiHeader.biHeight = -(h as i32); // top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(
            hdc,
            info.hbmColor,
            0,
            h,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(hdc);
        if lines == 0 {
            return None;
        }
        for chunk in pixels.chunks_exact_mut(4) {
            chunk.swap(0, 2); // BGRA → RGBA
        }
        image::RgbaImage::from_raw(w, h, pixels)
    }
}

/// 从系统文件提取图标（PrivateExtractIconsW，按 0-based 图标组索引）
#[cfg(windows)]
pub(crate) fn extract_sys_icon_image(file: &str, index: i32, size: u32) -> Result<image::RgbaImage, String> {
    use windows::Win32::UI::WindowsAndMessaging::{PrivateExtractIconsW, DestroyIcon};

    let path = resolve_sys_file(file).ok_or_else(|| format!("找不到系统文件: {}", file))?;
    // PrivateExtractIconsW 的路径参数为定长 [u16; 260]
    let mut path_buf = [0u16; 260];
    let path_str: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().collect();
    let n_copy = path_str.len().min(259);
    path_buf[..n_copy].copy_from_slice(&path_str[..n_copy]);

    let mut hicons = [windows::Win32::UI::WindowsAndMessaging::HICON::default(); 1];
    let mut icon_ids = [0u32; 1];
    // 注意：windows 0.58 包装签名第 7 参数是 flags（nIcons 由 phicon 切片长度推导）。
    // 传 0 = LR_DEFAULTCOLOR；曾误传 1 = LR_MONOCHROME，导致无原生 256px 帧的
    // 图标走单色转换路径，alpha 全部清零（表现为空图标）。
    let n = unsafe {
        PrivateExtractIconsW(
            &path_buf,
            index,
            size as i32,
            size as i32,
            Some(&mut hicons),
            Some(icon_ids.as_mut_ptr()),
            0,
        )
    };
    let hicon = hicons[0];
    if n == 0 || hicon.is_invalid() {
        return Err(format!("{} @{} 无可提取图标", file, index));
    }
    let img = hicon_to_rgba(hicon);
    unsafe { let _ = DestroyIcon(hicon); }
    img.ok_or_else(|| format!("{} @{} 图标位图读取失败", file, index))
}

/// 渲染 Segoe 图标字体字形为透明底 PNG。
/// 原理：白字形画在黑底 32bpp 内存位图上，亮度即 alpha，再统一着色。
/// 字体族按文件存在性选择：SegoeIcons.ttf（Segoe Fluent Icons，Win11）→ segmdl2.ttf（Segoe MDL2 Assets，Win10），
/// 两者共用同一套 E 区码点。颜色固定 Fluent 蓝 #0F6CBD，明暗主题下均可读。
#[cfg(windows)]
pub(crate) fn render_glyph_image(codepoint: u32, size: u32) -> Result<image::RgbaImage, String> {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateFontW, CreateSolidBrush, DeleteDC,
        DeleteObject, DrawTextW, FillRect, GetDC, ReleaseDC, SelectObject, SetBkColor,
        SetTextColor, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE,
        DT_VCENTER,
    };

    let ch = char::from_u32(codepoint).ok_or_else(|| format!("非法字形码点: U+{:04X}", codepoint))?;
    let mut text: Vec<u16> = ch.to_string().encode_utf16().collect();

    let windir = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".to_string());
    let (family, height) = if Path::new(&windir).join("Fonts").join("SegoeIcons.ttf").exists() {
        ("Segoe Fluent Icons", (size as i32 * 3) / 4)
    } else {
        ("Segoe MDL2 Assets", (size as i32 * 3) / 4)
    };
    let family_w: Vec<u16> = family.encode_utf16().chain([0]).collect();

    unsafe {
        let hdc_screen = GetDC(windows::Win32::Foundation::HWND::default());
        let hdc = CreateCompatibleDC(hdc_screen);

        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = size as i32;
        bi.bmiHeader.biHeight = -(size as i32); // top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let hbmp = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits, None, 0)
            .map_err(|e| format!("CreateDIBSection 失败: {}", e))?;
        let old_bmp = SelectObject(hdc, hbmp);

        // 黑底（后续以像素亮度作为 alpha）
        let black = CreateSolidBrush(COLORREF(0));
        let mut rect = RECT { left: 0, top: 0, right: size as i32, bottom: size as i32 };
        FillRect(hdc, &mut rect, black);
        let _ = DeleteObject(black);

        let hfont = CreateFontW(
            -height, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
            1, // DEFAULT_CHARSET
            0, 0,
            4, // ANTIALIASED_QUALITY
            0,
            PCWSTR(family_w.as_ptr()),
        );
        let old_font = SelectObject(hdc, hfont);

        SetBkColor(hdc, COLORREF(0));
        SetTextColor(hdc, COLORREF(0x00FF_FFFF)); // 白色字形
        let mut rc = RECT { left: 0, top: 0, right: size as i32, bottom: size as i32 };
        DrawTextW(hdc, &mut text, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

        // 必须先拷贝像素再释放位图：bits 指向 DIB 内部内存
        if bits.is_null() {
            let _ = SelectObject(hdc, old_font);
            let _ = SelectObject(hdc, old_bmp);
            let _ = DeleteObject(hbmp);
            let _ = DeleteDC(hdc);
            let _ = ReleaseDC(None, hdc_screen);
            return Err("字形位图读取失败".to_string());
        }
        let n = (size * size * 4) as usize;
        let raw = std::slice::from_raw_parts(bits as *const u8, n);
        // 亮度 → alpha，统一着 Fluent 蓝 #0F6CBD（明暗主题均可读）
        let (r, g, b) = (15u8, 108u8, 189u8);
        let mut pixels = Vec::with_capacity(n);
        for px in raw.chunks_exact(4) {
            // BGRA：白字形黑底，取任一通道即亮度
            let v = px[0];
            pixels.extend_from_slice(&[r, g, b, v]);
        }

        let _ = SelectObject(hdc, old_font);
        let _ = SelectObject(hdc, old_bmp);
        let _ = DeleteObject(hfont);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc);
        let _ = ReleaseDC(None, hdc_screen);

        image::RgbaImage::from_raw(size, size, pixels)
            .ok_or_else(|| "字形位图尺寸错误".to_string())
    }
}

/// 按规格提取系统功能图标为 RgbaImage。
/// 规格："sys:<文件>|<组索引>" 或 "glyph:<码点十六进制>"
#[cfg(windows)]
pub(crate) fn system_icon_image(spec: &str, size: u32) -> Result<image::RgbaImage, String> {
    if let Some(rest) = spec.strip_prefix("glyph:") {
        let cp = u32::from_str_radix(rest.trim(), 16)
            .map_err(|_| format!("glyph 码点非法: {}", rest))?;
        return render_glyph_image(cp, size);
    }
    if let Some(rest) = spec.strip_prefix("sys:") {
        let (file, idx) = rest.rsplit_once('|')
            .ok_or_else(|| format!("sys 规格非法（应为 文件|索引）: {}", rest))?;
        let index: i32 = idx.trim().parse()
            .map_err(|_| format!("sys 组索引非法: {}", idx))?;
        return extract_sys_icon_image(file.trim(), index, size);
    }
    Err(format!("未知图标来源规格: {}", spec))
}


/// 提取系统功能图标并缓存到 assets/icons/extracted/system_{key}.png，返回相对路径。
/// 确定性命名：同规格共享同一份 PNG（force 时删除缓存强制重提取）
pub(crate) fn extract_system_icon_cached(spec: &str, force: bool) -> Result<String, String> {
    let assets_dir = get_assets_dir()?;
    let icon_dir = assets_dir.join("icons").join("extracted");
    ensure_dir(&icon_dir)?;
    let icon_file = icon_dir.join(format!("system_{}.png", system_icon_cache_key(spec)));

    if icon_file.exists() && !force {
        return Ok(rel_to_exe_dir(&icon_file));
    }
    let img = system_icon_image(spec, 256)?;
    img.save(&icon_file).map_err(|e| format!("保存系统图标失败: {}", e))?;
    Ok(rel_to_exe_dir(&icon_file))
}

/// 为系统功能条目提取图标（前端选择器与重提取共用）。
/// spec: "sys:<文件>|<组索引>"（如 sys:%windir%\system32\devmgr.dll|5）或 "glyph:E713"
#[command]
pub fn extract_system_icon(spec: String, force: Option<bool>) -> Result<String, String> {
    extract_system_icon_cached(spec.trim(), force.unwrap_or(false))
}

#[cfg(all(test, windows))]
mod sys_icon_tests {
    use super::*;

    #[test]
    fn extract_dll_icon_by_group_index() {
        // 设备管理器权威来源：devmgr.dll 组索引 5（与 CLSID DefaultIcon -201 哈希一致）
        let img = extract_sys_icon_image("%windir%\\system32\\devmgr.dll", 5, 256).unwrap();
        assert_eq!((img.width(), img.height()), (256, 256));
    }

    #[test]
    fn extract_bare_command_icon() {
        // 裸命令名解析：regedit.exe 在 %windir% 而非 System32
        let img = extract_sys_icon_image("regedit.exe", 0, 256).unwrap();
        assert_eq!(img.width(), 256);
    }

    #[test]
    fn render_settings_glyph() {
        let img = render_glyph_image(0xE713, 256).unwrap();
        // 便于人工核验渲染效果（透明底 + Fluent 蓝字形）
        let _ = img.save(std::env::temp_dir().join("glyph_E713_test.png"));
        // 字形应有足够的不透明像素（齿轮主体）
        let opaque = img.pixels().filter(|p| p[3] > 200).count();
        assert!(opaque > 500, "字形渲染像素不足: {}", opaque);
        // 应存在半透明抗锯齿边缘
        let semi = img.pixels().filter(|p| p[3] > 0 && p[3] < 200).count();
        assert!(semi > 50, "缺少抗锯齿边缘: {}", semi);
    }

    #[test]
    fn resolve_appexec_alias() {
        // Win11 画图：System32 无 mspaint.exe，经应用执行别名解析到商店包内真实路径
        if !Path::new(&std::env::var("LOCALAPPDATA").unwrap())
            .join("Microsoft").join("WindowsApps").join("mspaint.exe").exists()
        {
            return; // 未安装画图时跳过
        }
        let p = resolve_app_alias("mspaint.exe").expect("别名解析失败");
        assert!(p.as_os_str().to_string_lossy().to_lowercase().ends_with("mspaint.exe"));
        assert!(p.exists());
    }

    #[test]
    fn extract_reported_empty_icons() {
        // 用户反馈的四项空图标：逐一提取并断言非透明（alpha 有效内容占比 > 5%）
        let cases: [(&str, i32); 5] = [
            ("%windir%\\system32\\Mycomput.dll", 0),   // 计算机管理
            ("%windir%\\system32\\pmcsnap.dll", 0),    // 打印管理
            ("%windir%\\system32\\gpedit.dll", 0),     // 本地组策略
            ("%windir%\\system32\\wsecedit.dll", 0),   // 本地安全策略
            ("dxdiag.exe", 0),                          // DirectX 诊断
        ];
        for (file, index) in cases {
            let img = extract_sys_icon_image(file, index, 256)
                .unwrap_or_else(|e| panic!("{} @{} 提取失败: {}", file, index, e));
            assert_eq!((img.width(), img.height()), (256, 256));
            let opaque = img.pixels().filter(|p| p[3] > 10).count();
            let pct = (opaque as u64) * 100 / ((img.width() as u64) * (img.height() as u64));
            assert!(pct > 5, "{} @{} 图标全透明（不透明仅 {}%）", file, index, pct);
        }
    }
}

