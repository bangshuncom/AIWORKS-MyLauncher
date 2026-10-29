// ════════════════════════════════════════════
// 条目展示相关的纯函数（图标/封面/提示文案）
// 供 useEntryView 与各视图组件共用，避免同一套判断在多处重写
// ════════════════════════════════════════════

import type { Entry, EntryType } from '@/types'
import { resolveAssetUrl } from '@/store'

/** 各条目类型的默认 emoji 图标：无图片图标 / emoji 图标缺失 / 图片加载失败时兜底，
 *  同时也是新增对话框里类型选择按钮的图标来源（两处共用一份定义，避免漂移） */
export const DEFAULT_TYPE_EMOJI: Record<EntryType, string> = {
  program: '📦',
  url: '🌐',
  folder: '📁',
  file: '📄',
  system: '🛠️',
  appx: '🏪',
  provider: '🎮',
}

/** 图标图片 URL（emoji 类型不返回图片地址） */
export function getIconPath(entry: Entry): string {
  if (entry.icon?.source && entry.icon.type !== 'emoji') return resolveAssetUrl(entry.icon.source)
  return ''
}

/** emoji 图标字符（type 为 emoji 且有 source 时返回，否则空串） */
export function getIconEmoji(entry: Entry): string {
  if (entry.icon?.type === 'emoji' && entry.icon.source) return entry.icon.source
  return ''
}

/** 兜底 emoji：优先用户自定义 emoji，其次按类型取默认图标（如网址永远有 🌐） */
export function getFallbackEmoji(entry: Entry): string {
  return getIconEmoji(entry) || DEFAULT_TYPE_EMOJI[entry.type] || ''
}

/** 封面图 URL（未启用封面时返回空串） */
export function getCoverPath(entry: Entry): string {
  if (entry.cover?.enabled && entry.cover.source) return resolveAssetUrl(entry.cover.source)
  return ''
}

/** 悬停提示：网址显示 URL，平台游戏优先显示主程序路径，其余显示备注/名称 */
export function hoverTip(entry: Entry): string {
  if (entry.type === 'url') return entry.url || entry.name
  if (entry.type === 'provider') {
    // 优先展示主程序路径，无则展示启动链接
    const exe = Object.values(entry.absolute_paths || {})[0] || ''
    return exe || entry.url || entry.name
  }
  if (entry.type === 'system' || entry.type === 'appx') return entry.relative_path || entry.notes || entry.name
  return entry.notes || entry.name
}
