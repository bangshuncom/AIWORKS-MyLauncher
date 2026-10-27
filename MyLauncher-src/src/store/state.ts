// ═══════════════════════════════════════════
// 全局状态（单一响应式对象）
// ═══════════════════════════════════════════

import { reactive } from 'vue'
import type { AppConfig, Category, Entry, Environment } from '@/types'


interface StoreState {
  loaded: boolean
  config: AppConfig
  environments: Environment[]
  categories: Category[]
  entries: Entry[]

  // exe 所在目录的绝对路径（用于拼接相对路径）
  exeDir: string
  // 当前选中的分类 ID
  currentCategoryId: string | null
  // 当前环境 ID
  currentEnvId: string
  // 搜索关键词
  searchQuery: string
  // 是否处于批量选择模式
  batchMode: boolean
  // 批量选中的条目 ID
  selectedIds: Set<string>
  // 拖拽添加的计数器
  idCounter: number
  // 图标版本号（key = 图标相对路径，value = 单调递增版本）：
  // 重置图标后文件内容更新但 URL 不变，WebView 用缓存的旧图；
  // bump 版本使 resolveAssetUrl 拼上 ?v= 参数强制重新加载
  iconVersions: Map<string, number>
  // 隐藏分类临时解锁标志（LOGO 三击弹关于时置 true，重启/关闭软件自动失效，不持久化）
  hiddenUnlocked: boolean
}

export const state = reactive<StoreState>({
  loaded: false,
  config: {
    version: '1.0.0',
    logo_icon: '🪷',
    logo_icon_type: 'emoji',
    logo_text: 'MyLauncher',
    logo_image: '',
    logo_image_enabled: false,
    close_action: 'tray',
    custom_emojis: [],
    language: 'zh-CN',
    disabled_plugins: [],
    favicon_api_sources: [
      'https://a.favicon.im/{url}?larger=true',
      'https://faviconsnap.com/api/favicon?url={url}',
      '{scheme}://{domain}/favicon.ico',
    ],
    theme: 'system',
    sidebar_collapsed: false,
    card_cover_ratio: '16:9',
    masonry_max_height: 480,
  },
  environments: [],
  categories: [],
  entries: [],
  exeDir: '',
  currentCategoryId: null,
  currentEnvId: 'env_001',
  searchQuery: '',
  batchMode: false,
  selectedIds: new Set(),
  idCounter: 1,
  iconVersions: new Map(),
  hiddenUnlocked: false,
})
