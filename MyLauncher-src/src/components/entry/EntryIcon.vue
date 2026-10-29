<script setup lang="ts">
// 条目图标三态渲染：图片 → emoji/类型默认图标 → 首字母兜底。
// 这段 v-if/v-else-if/v-else 原先在 5 个视图里重复了 7 次，统一到此处。
// 各视图的尺寸差异通过 size / emojiSize / fallbackSize 传入，保持原有视觉。
//
// 关键兜底：图片文件缺失或加载失败（如网址 favicon 没抓到、图标文件被删）时，
// 不再把 <img> 简单隐藏（那会留下空白），而是记下失败地址并切到 emoji 兜底：
// 用户 emoji 图标 → 该类型的默认图标（网址固定 🌐）→ 名称首字母。
import { computed, ref, watch } from 'vue'
import type { Entry } from '@/types'
import { getIconPath, getFallbackEmoji } from '@/utils/entryDisplay'

const props = withDefaults(
  defineProps<{
    entry: Entry
    /** 图标边长（px） */
    size?: number
    /** emoji 字号（px），默认取 size 的 90% */
    emojiSize?: number
    /** 兜底首字母字号（px），默认取 size 的 55% */
    fallbackSize?: number
    /** 兜底块内是否显示首字母（卡片封面显示，卡片底部留空占位） */
    showInitial?: boolean
    /** 兜底块附加类名，用于表格视图的带底色徽标样式（定义在 global.css） */
    fallbackClass?: string
  }>(),
  { size: 20, emojiSize: 0, fallbackSize: 0, showInitial: false, fallbackClass: '' },
)

const iconPath = computed(() => getIconPath(props.entry))
const fallbackEmoji = computed(() => getFallbackEmoji(props.entry))

/** 加载失败的图标地址；同一地址不再重试（避免反复请求坏图），直接走兜底 */
const failedPath = ref('')
const showImage = computed(() => !!iconPath.value && failedPath.value !== iconPath.value)

// 图标地址变化（换图标 / 重置后 bump 版本号）时清空失败记录，重新尝试加载
watch(iconPath, () => {
  failedPath.value = ''
})

function handleImgError() {
  failedPath.value = iconPath.value
}

/**
 * 加载"成功"但图片本身是垃圾：favicon 服务对未知域名常返回 1×1 占位黑图，
 * naturalWidth ≤ 2 时同样切到兜底，避免显示一个纯黑小方块。
 */
function handleImgLoad(e: Event) {
  const img = e.target as HTMLImageElement
  if (img.naturalWidth <= 2 || img.naturalHeight <= 2) {
    failedPath.value = iconPath.value
  }
}

const emojiPx = computed(() => props.emojiSize || Math.round(props.size * 0.9))
const fallbackPx = computed(() => props.fallbackSize || Math.round(props.size * 0.55))
</script>

<template>
  <img
    v-if="showImage"
    :src="iconPath"
    class="entry-icon-img"
    :style="{ width: size + 'px', height: size + 'px' }"
    alt="icon"
    @error="handleImgError"
    @load="handleImgLoad"
  />
  <span
    v-else-if="fallbackEmoji"
    class="entry-icon-emoji"
    :style="{ fontSize: emojiPx + 'px' }"
  >{{ fallbackEmoji }}</span>
  <div
    v-else
    class="entry-icon-fallback"
    :class="fallbackClass || 'entry-icon-fallback-default'"
    :style="{ width: size + 'px', height: size + 'px' }"
  >
    <span v-if="showInitial" :style="{ fontSize: fallbackPx + 'px' }">{{ entry.name.charAt(0) || '?' }}</span>
  </div>
</template>

<style scoped>
.entry-icon-img {
  object-fit: contain;
  flex-shrink: 0;
}

.entry-icon-emoji {
  line-height: 1;
  flex-shrink: 0;
}

.entry-icon-fallback {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

/* 默认兜底块（未指定 fallbackClass 时）：带底色圆角，避免无图标时留一片空白 */
.entry-icon-fallback-default {
  background: var(--color-primary-bg);
  border-radius: 6px;
  color: var(--color-primary-dark);
  font-weight: 600;
}
</style>
