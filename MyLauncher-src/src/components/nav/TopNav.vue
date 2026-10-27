<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { state, getVisibleTopCategories, getCategoryById, resolveAssetUrl } from '@/store'
import CategoryIcon from '@/components/CategoryIcon.vue'
import { onImgError } from '@/composables/useImageFallback'
import { useI18n } from '@/composables/useI18n'

const emit = defineEmits<{
  (e: 'search', q: string): void
  (e: 'open-settings'): void
}>()

const { t } = useI18n()

const topCats = computed(() => getVisibleTopCategories())
const searchQuery = ref('')
const searchInputRef = ref<HTMLInputElement | null>(null)

// ─── LOGO 动态渲染 ───
const logoIcon = computed(() => state.config.logo_icon || '🪷')
const logoIconType = computed(() => (state.config.logo_icon_type === 'custom' ? 'custom' : 'emoji'))
const logoIconUrl = computed(() => {
  if (logoIconType.value === 'custom' && state.config.logo_icon) {
    return resolveAssetUrl(state.config.logo_icon)
  }
  return ''
})
const logoText = computed(() => state.config.logo_text || 'MyLauncher')
const logoImageEnabled = computed(() => state.config.logo_image_enabled && !!state.config.logo_image)
const logoImageUrl = computed(() => logoImageEnabled.value ? resolveAssetUrl(state.config.logo_image) : '')

// ─── LOGO 三击检测（800ms 内连续三次点击弹出关于窗口）───
let clickTimes: number[] = []

function onBrandClick() {
  const now = Date.now()
  // 只保留 800ms 内的点击记录
  clickTimes = clickTimes.filter(t => now - t < 800)
  clickTimes.push(now)
  if (clickTimes.length >= 3) {
    clickTimes = []
    emit('open-settings')
  }
}

// 当前选中分类向上追溯到的顶级分类 ID（用于顶部标签高亮）
const currentTopId = computed(() => {
  let cat = state.currentCategoryId ? getCategoryById(state.currentCategoryId) : undefined
  while (cat && cat.parent_id) {
    cat = getCategoryById(cat.parent_id)
  }
  return cat?.id ?? null
})

function selectCategory(id: string) {
  state.currentCategoryId = id
}

// ─── 分类标签区：鼠标按住左右拖动滚动 ───
// 一级分类过多时容器可横向滚动，支持按住拖动（滚动条已隐藏）
const navTabsRef = ref<HTMLElement | null>(null)
let dragState: { downX: number; scrollLeft: number; dragging: boolean } | null = null

function onTabsPointerDown(e: PointerEvent) {
  // 只响应鼠标左键；从输入控件上发起的拖动忽略
  if (e.button !== 0) return
  const el = navTabsRef.value
  if (!el) return
  dragState = { downX: e.clientX, scrollLeft: el.scrollLeft, dragging: false }
}

function onTabsPointerMove(e: PointerEvent) {
  const el = navTabsRef.value
  if (!el || !dragState) return
  const dx = e.clientX - dragState.downX
  // 位移超过阈值才视为拖动，避免和点击冲突
  if (!dragState.dragging && Math.abs(dx) > 4) {
    dragState.dragging = true
  }
  if (dragState.dragging) {
    el.scrollLeft = dragState.scrollLeft - dx
    e.preventDefault()
  }
}

function onTabsPointerUp() {
  // 拖动结束后短暂抑制点击，防止拖完误触发分类切换
  if (dragState?.dragging) {
    suppressClick = true
    window.setTimeout(() => { suppressClick = false }, 50)
  }
  dragState = null
}

let suppressClick = false

function onTabClick(id: string) {
  if (suppressClick) return
  selectCategory(id)
}

onMounted(() => {
  window.addEventListener('pointermove', onTabsPointerMove)
  window.addEventListener('pointerup', onTabsPointerUp)
})

onBeforeUnmount(() => {
  window.removeEventListener('pointermove', onTabsPointerMove)
  window.removeEventListener('pointerup', onTabsPointerUp)
})

function onSearch() {
  emit('search', searchQuery.value)
}

/** 点击搜索按钮 / 输入框回车：执行一次搜索；无输入时聚焦输入框 */
function submitSearch() {
  if (!searchQuery.value.trim()) {
    searchInputRef.value?.focus()
    return
  }
  emit('search', searchQuery.value)
}

function clearSearch() {
  searchQuery.value = ''
  emit('search', '')
}
</script>

<template>
  <div class="top-nav" @contextmenu.prevent>
    <!-- 品牌锚点（三击弹出关于窗口） -->
    <div class="brand-anchor" :class="{ 'has-logo-image': logoImageEnabled }" :title="logoText" @click="onBrandClick">
      <!-- 启用 LOGO 图片：整体显示为图片 -->
      <template v-if="logoImageEnabled">
        <img :src="logoImageUrl" class="brand-image" alt="LOGO" @error="onImgError" />
      </template>
      <!-- 默认：LOGO 图标 + 文字 -->
      <template v-else>
        <div class="brand-logo">
          <img v-if="logoIconUrl" :src="logoIconUrl" class="brand-logo-img" alt="LOGO" @error="onImgError" />
          <span v-else>{{ logoIcon }}</span>
        </div>
        <span class="brand-name">{{ logoText }}</span>
        <span class="brand-divider"></span>
      </template>
    </div>
    <!-- 一级分类（支持按住拖动横向滚动） -->
    <div
      ref="navTabsRef"
      class="nav-tabs"
      :class="{ 'nav-tabs-dragging': dragState?.dragging }"
      @pointerdown="onTabsPointerDown"
    >
      <button
        v-for="cat in topCats"
        :key="cat.id"
        class="nav-tab"
        :class="{ active: currentTopId === cat.id }"
        @click="onTabClick(cat.id)"
      >
        <CategoryIcon :category="cat" class="nav-tab-icon" />
        {{ cat.name }}</button>
    </div>

    <!-- 工具栏（环境切换已移至 左下角菜单 → 环境管理） -->
    <div class="nav-toolbar">
      <!-- 搜索栏：输入框 / 清除 / 搜索按钮合为一个整体 pill，共用一套边框与聚焦态 -->
      <div class="search-bar">
        <input
          ref="searchInputRef"
          v-model="searchQuery"
          class="search-input"
          type="text"
          :placeholder="t('nav.top.searchPlaceholder')"
          @input="onSearch"
          @keydown.enter="submitSearch"
        />
        <button v-if="searchQuery" class="search-clear" :title="t('nav.top.clearSearch')" @click="clearSearch">✕</button>
        <span class="search-divider"></span>
        <button class="search-submit" :title="t('nav.top.search')" @click="submitSearch">
          <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
            <circle cx="7" cy="7" r="4.5" />
            <path d="M10.6 10.6 L14 14" />
          </svg>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.top-nav {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 20px 0 0;
  height: 56px;
  background: var(--gradient-top);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

/* ─── 品牌锚点（与侧导航同宽对齐）─── */
.brand-anchor {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-shrink: 0;
  width: 180px;
  height: 100%;
  padding-left: 18px;
  border-right: 1px solid var(--color-border);
  box-sizing: border-box;
  user-select: none;
}

/* LOGO 图片模式：去掉图标模式预留的左内边距，图片从最左缘铺满整个品牌区 */
.brand-anchor.has-logo-image {
  padding-left: 0;
}

.brand-logo {
  width: 34px;
  height: 34px;
  border-radius: 10px;
  background: linear-gradient(135deg, #4f83f5 0%, #0078d4 100%);
  box-shadow: 0 2px 8px rgba(79, 131, 245, 0.35);
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 18px;
  overflow: hidden;
}

/* 自定义图片图标：铺满圆角底 */
.brand-logo-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.brand-name {
  font-size: 15px;
  font-weight: 700;
  letter-spacing: 0.2px;
  background: linear-gradient(120deg, var(--color-text) 30%, var(--color-primary) 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  white-space: nowrap;
}

.brand-divider {
  display: none;
}

/* LOGO 图片模式：铺满整个品牌区域 */
.brand-image {
  width: 100%;
  height: 100%;
  max-height: 56px;
  object-fit: cover;
  object-position: left center;
  display: block;
}

/* ─── 分类标签（Win11 药丸式高亮）─── */
.nav-tabs {
  display: flex;
  gap: 4px;
  flex: 1;
  min-width: 0;
  overflow-x: auto;
  overflow-y: hidden;
  padding: 10px 2px;
  scrollbar-width: none;
  -ms-overflow-style: none;
}
.nav-tabs::-webkit-scrollbar {
  display: none;
}

/* 拖动中：禁用文字选择并显示抓取光标 */
.nav-tabs-dragging {
  cursor: grabbing;
  user-select: none;
}

.nav-tab {
  padding: 7px 16px;
  border: none;
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 13.5px;
  cursor: pointer;
  border-radius: var(--radius-md);
  white-space: nowrap;
  transition: all var(--transition);
  display: flex;
  align-items: center;
  gap: 6px;
  border: 1px solid transparent;
}

.nav-tab-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  font-size: 14px;
  line-height: 1;
  flex-shrink: 0;
}

.nav-tab:hover {
  background: rgba(0, 120, 212, 0.07);
  color: var(--color-text);
}

.nav-tab.active {
  background: var(--color-primary-bg);
  border-color: rgba(0, 120, 212, 0.25);
  color: var(--color-primary-dark);
  font-weight: 600;
  box-shadow: inset 0 -2px 0 var(--color-primary);
}

.nav-toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

/* ─── 搜索栏：输入框与按钮合成一个 pill（Win11 风格）───
   容器统一负责背景/边框/聚焦态；内部输入框无边框透明底，
   清除与搜索按钮为无边框小热区，视觉上是一个整体控件。 */
.search-bar {
  display: inline-flex;
  align-items: center;
  height: 32px;
  width: 210px;
  padding: 0 3px 0 11px;
  border-radius: var(--radius-md);
  background: var(--color-bg-sidebar);
  border: 1px solid transparent;
  box-sizing: border-box;
  transition: all var(--transition);
}
.search-bar:hover {
  background: var(--color-bg-hover);
}
.search-bar:focus-within {
  background: var(--color-bg-card);
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px rgba(0, 120, 212, 0.12);
  width: 250px;
}

/* 输入框：去掉自身边框背景，融入 pill */
.search-input {
  flex: 1;
  min-width: 0;
  padding: 0;
  border: none;
  outline: none;
  background: transparent;
  /* 文字与顶部导航同色（暗色主题下跟随变浅） */
  color: var(--color-text);
  font-size: 12.5px;
}

.search-input::placeholder {
  color: var(--color-text-tertiary);
}

/* 清除按钮：仅在有输入时出现 */
.search-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  margin-left: 4px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--color-text-tertiary);
  font-size: 10.5px;
  line-height: 1;
  cursor: pointer;
  flex-shrink: 0;
  transition: all var(--transition);
}
.search-clear:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

/* 输入区与按钮区的分隔线 */
.search-divider {
  width: 1px;
  height: 14px;
  margin: 0 3px;
  background: var(--color-border);
  flex-shrink: 0;
}

/* 搜索按钮：放大镜用 SVG 线条图标，与 ✕ 同为细线条文本级视觉，跟随主题变色 */
.search-submit {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
  flex-shrink: 0;
  transition: all var(--transition);
}
.search-submit:hover {
  background: rgba(0, 120, 212, 0.1);
  color: var(--color-primary);
}
.search-submit:active {
  transform: scale(0.92);
}
</style>
