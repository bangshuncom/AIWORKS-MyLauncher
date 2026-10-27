// ════════════════════════════════════════════
// Tauri API 封装层
// ════════════════════════════════════════════

import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import type {
  AllData, AppConfig, AssetCleanResult, AutorunStatus, BatchIconSummary, Category, Entry, Environment,
  ExeInfo, FileInfo, IconRefreshResult, ImportScanSummary, InitResult, LnkInfo, PathInfo, ScanResult, ConvertPathResult,
  BookmarkParseResult, AppxAppInfo, ProviderItemResult, PluginInfo, LangInfo, LangFile
} from '@/types'

// ─── 数据持久化 ───

export async function init_data_dir(): Promise<InitResult> {
  return invoke<InitResult>('init_data_dir')
}

export async function load_all_data(): Promise<AllData> {
  return invoke<AllData>('load_all_data')
}

export async function save_entries(entries: Entry[]): Promise<void> {
  await invoke('save_entries', { entries })
}

export async function save_categories(categories: Category[]): Promise<void> {
  await invoke('save_categories', { categories })
}

export async function save_environments(environments: Environment[]): Promise<void> {
  await invoke('save_environments', { environments })
}

export async function save_config(config: AppConfig): Promise<void> {
  await invoke('save_config', { config })
}

export async function get_exe_dir(): Promise<string> {
  return invoke<string>('get_exe_dir')
}

export async function get_now(): Promise<string> {
  return invoke<string>('get_now')
}

// ─── 路径解析 ───

export async function resolve_path(entry: Entry, currentEnvId: string): Promise<PathInfo> {
  return invoke<PathInfo>('resolve_path', { entry, currentEnvId })
}

export async function check_paths_batch(entries: Entry[], currentEnvId: string): Promise<boolean[]> {
  return invoke<boolean[]>('check_paths_batch', { entries, currentEnvId })
}

// ─── 路径模式转换 ───

export async function convert_path_mode(entry: Entry, currentEnvId: string, targetMode: string): Promise<ConvertPathResult> {
  return invoke<ConvertPathResult>('convert_path_mode', { entry, currentEnvId, targetMode })
}

// ─── 启动程序 ───

export async function launch_program(entry: Entry, currentEnvId: string): Promise<boolean> {
  return invoke<boolean>('launch_program', { entry, currentEnvId })
}

// ─── 打开外部网址（更新地址等，系统默认浏览器） ───

export async function open_external_url(url: string): Promise<boolean> {
  return invoke<boolean>('open_external_url', { url })
}

// ─── 打开所在目录 ───

export async function open_in_explorer(entry: Entry, currentEnvId: string): Promise<boolean> {
  return invoke<boolean>('open_in_explorer', { entry, currentEnvId })
}

// ─── 图标提取 ───

export async function get_exe_info(exePath: string): Promise<ExeInfo> {
  return invoke<ExeInfo>('get_exe_info', { exePath })
}

// ─── 文件/文件夹信息 ───

export async function get_file_info(filePath: string): Promise<FileInfo> {
  return invoke<FileInfo>('get_file_info', { filePath })
}

// ─── 快捷方式解析 ───

export async function resolve_lnk(lnkPath: string): Promise<LnkInfo> {
  return invoke<LnkInfo>('resolve_lnk', { lnkPath })
}

// ─── 文件夹扫描 ───

export async function scan_directory(dirPath: string, maxDepth?: number): Promise<ScanResult> {
  return invoke<ScanResult>('scan_directory', { dirPath, maxDepth })
}

// ─── 平台插件（扫描插件宿主） ───

/** 列出全部插件及加载状态（设置页插件面板 / 平台下拉用） */
export async function list_plugins(): Promise<PluginInfo[]> {
  return invoke<PluginInfo[]>('list_plugins')
}

/** 按平台 ID 查询单个条目（名称/主程序/启动链接/封面/图标） */
export async function scan_provider_item(providerId: string, itemId: string): Promise<ProviderItemResult> {
  return invoke<ProviderItemResult>('scan_provider_item', { providerId, itemId })
}

/** 打开插件目录（资源管理器） */
export async function open_plugins_dir(): Promise<void> {
  await invoke('open_plugins_dir')
}

// ─── 扫描结果批量导入 ───

/** 批量导入扫描结果（后端多线程提取图标/下载封面/构造 Entry 后一次写盘，
 *  逐条推送 import-progress 事件，完成后返回整体统计与已写盘的新 Entry） */
export async function import_scan_items(items: ExeInfo[], categoryId: string, currentEnvId: string): Promise<ImportScanSummary> {
  return invoke<ImportScanSummary>('import_scan_items', { items, categoryId, currentEnvId })
}

// ─── 文件操作 ───

export async function copy_file_to_assets(sourcePath: string, targetSubdir: string): Promise<string> {
  return invoke<string>('copy_file_to_assets', { sourcePath, targetSubdir })
}

// ─── Favicon ───

export async function fetch_favicon(url: string): Promise<string> {
  return invoke<string>('fetch_favicon', { url })
}

// ─── 浏览器书签解析 ───

export async function parse_bookmarks(filePath: string): Promise<BookmarkParseResult> {
  return invoke<BookmarkParseResult>('parse_bookmarks', { filePath })
}

// ─── APPX / 系统功能 ───

export async function list_appx_apps(): Promise<AppxAppInfo[]> {
  return invoke<AppxAppInfo[]>('list_appx_apps')
}

export async function extract_appx_icon(appKey: string, shellUri: string): Promise<string> {
  return invoke<string>('extract_appx_icon', { appKey, shellUri })
}

/** 提取系统功能图标。spec: "sys:<文件>|<组索引>" 或 "glyph:<Segoe 码点>"；force 时绕过缓存重提取 */
export async function extract_system_icon(spec: string, force = false): Promise<string> {
  return invoke<string>('extract_system_icon', { spec, force })
}

// ─── 过期资源清理 ───

/** 扫描未被引用的过期图标/封面（无副作用，仅统计） */
export async function scan_orphan_assets(): Promise<AssetCleanResult> {
  return invoke<AssetCleanResult>('scan_orphan_assets')
}

/** 删除未被引用的过期图标/封面（移入回收站） */
export async function clean_orphan_assets(): Promise<AssetCleanResult> {
  return invoke<AssetCleanResult>('clean_orphan_assets')
}

/** 删除条目后的资源联动清理：传入被删条目的图标/封面相对路径，仍被引用的跳过，未被引用的移入回收站 */
export async function cleanup_deleted_assets(sources: string[]): Promise<number> {
  return invoke<number>('cleanup_deleted_assets', { sources })
}

// ─── 图标重新提取 ───

/** 重新提取条目图标（按类型分派：程序/文件夹/文件/网址/APPX） */
export async function refresh_entry_icon(entry: Entry, currentEnvId: string): Promise<IconRefreshResult> {
  return invoke<IconRefreshResult>('refresh_entry_icon', { entry, currentEnvId })
}

/** 批量重新提取图标（后端多线程，逐条推送 batch-icon-progress 事件，完成后返回整体统计） */
export async function batch_refresh_icons(entries: Entry[], currentEnvId: string): Promise<BatchIconSummary> {
  return invoke<BatchIconSummary>('batch_refresh_icons', { entries, currentEnvId })
}

// ─── 开机自启 ───

/** 设置开机自启（enabled=false 删除注册表值；silent 控制自启时是否隐藏主窗口） */
export async function set_autorun(enabled: boolean, silent: boolean): Promise<void> {
  await invoke('set_autorun', { enabled, silent })
}

/** 查询当前自启状态（读注册表实际值） */
export async function get_autorun(): Promise<AutorunStatus> {
  return invoke<AutorunStatus>('get_autorun')
}

// ─── 文件选择对话框 ───

export async function pick_file(filters?: { name: string; extensions: string[] }[], defaultPath?: string): Promise<string | null> {
  const result = await open({ filters, multiple: false, defaultPath: defaultPath || undefined })
  return result as string | null
}

export async function pick_directory(): Promise<string | null> {
  const result = await open({ directory: true, multiple: false })
  return result as string | null
}

// ─── 窗口标题 ───

/** 同步系统窗口标题为自定义标题（logo_text，传空恢复默认 "MyLauncher"） */
export async function set_window_title(title: string): Promise<void> {
  await invoke('set_window_title', { title })
}

// ─── 多语言（i18n） ───

/** 释放内置默认语言文件到 lang/（幂等：缺哪个补哪个，永不覆盖）。init_data_dir 内部已调用，一般无需手动触发 */
export async function init_lang_dir(): Promise<void> {
  await invoke('init_lang_dir')
}

/** 扫描 lang/ 目录，返回全部可用语言（含损坏标记与完成度） */
export async function list_languages(): Promise<LangInfo[]> {
  return invoke<LangInfo[]>('list_languages')
}

/** 读取指定语言文件的完整内容（meta + messages） */
export async function read_language(code: string): Promise<LangFile> {
  return invoke<LangFile>('read_language', { code })
}

/** 用资源管理器打开 lang/ 目录 */
export async function open_lang_dir(): Promise<void> {
  await invoke('open_lang_dir')
}

