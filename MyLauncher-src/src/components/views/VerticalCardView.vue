<script setup lang="ts">
import { computed } from 'vue'
import { useEntryView, type EntryViewProps, type EntryViewEmits } from '@/composables/useEntryView'
import { onImgError, onImgLoad } from '@/composables/useImageFallback'
import { state } from '@/store'
import EntryCheckbox from '@/components/entry/EntryCheckbox.vue'
import EntryAdminBadge from '@/components/entry/EntryAdminBadge.vue'
import EntryIcon from '@/components/entry/EntryIcon.vue'

const props = defineProps<EntryViewProps>()
const emit = defineEmits<EntryViewEmits>()
const { getCoverPath, onHoverCheck, onContextMenu, pathExists, onDoubleClick, hoverTip } = useEntryView(props, emit)

/** 竖向卡片封面宽高比，用于 padding-top 百分比（如 "16:9" → 56.25%） */
const coverPaddingTop = computed(() => {
  const r = state.config?.card_cover_ratio || '16:9'
  const m = /^(\d+(?:\.\d+)?):(\d+(?:\.\d+)?)$/.exec(r)
  if (!m) return '56.25%'
  const w = parseFloat(m[1])
  const h = parseFloat(m[2])
  if (!(w > 0) || !(h > 0)) return '56.25%'
  return ((h / w) * 100).toFixed(2) + '%'
})
</script>

<template>
  <div class="vertical-card-grid responsive-cols">
    <div
      v-for="entry in entries"
      :key="entry.id"
      class="v-card entry-cell"
      :class="{
        'path-invalid': !pathExists(entry),
        'selected': batchMode && selectedIds.has(entry.id),
      }"
      :title="hoverTip(entry)"
      @dblclick="onDoubleClick(entry)"
      @contextmenu="onContextMenu(entry, $event)"
    >
      <!-- hover 勾选框 -->
      <EntryCheckbox
        :checked="batchMode && selectedIds.has(entry.id)"
        :hover-only="!batchMode"
        @click="onHoverCheck($event, entry)"
      />

      <!-- 管理员标识 -->
      <EntryAdminBadge v-if="entry.run_as_admin" />

      <!-- 封面图 -->
      <div class="v-card-cover">
        <div class="v-card-cover-box" :style="{ paddingTop: coverPaddingTop }">
          <img v-if="getCoverPath(entry)" :src="getCoverPath(entry)" alt="cover" @error="onImgError" @load="onImgLoad" />
          <div v-else class="v-card-cover-fallback">
            <EntryIcon :entry="entry" :size="48" :emoji-size="46" :fallback-size="36" show-initial />
          </div>
        </div>
      </div>

      <!-- 底部信息 -->
      <div class="v-card-footer">
        <EntryIcon :entry="entry" :size="20" />
        <span class="v-card-name">{{ entry.name }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.vertical-card-grid {
  display: grid;
  /* 分档固定列数：基于内容区宽度（容器查询），最小 2 列，最大 6 列 */
  grid-template-columns: repeat(var(--cols, 2), 1fr);
  gap: 16px;
  padding-bottom: 20px;
}

.v-card {
  background: var(--color-bg-card);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  overflow: hidden;
  cursor: pointer;
  transition: all var(--transition);
  position: relative;
}

.v-card:hover {
  box-shadow: 0 4px 12px var(--color-shadow-strong);
  transform: translateY(-2px);
}

.v-card.path-invalid {
  border-color: var(--color-danger);
}

.v-card.selected {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px var(--color-primary);
}

.v-card-cover {
  width: 100%;
  background: var(--color-bg-sidebar);
  overflow: hidden;
  position: relative;
}

/* padding-top 百分比强制锁定高度：高度不随内容变化，保证同排卡片统一 */
.v-card-cover-box {
  position: relative;
  width: 100%;
  height: 0;
}

/* 只作用于封面图本身（必须是 box 的直接子元素）。
   不能用 `.v-card-cover-box img`：Vue scoped 样式会把父组件的 data-v 属性
   加到子组件（EntryIcon）的根节点上，无封面时 EntryIcon 渲染的 <img>
   就会被这条规则命中，被绝对定位到左上角（inline 的宽高会压过 width/height 100%，
   但 left/top:0 生效），从而破坏 .v-card-cover-fallback 的居中。 */
.v-card-cover-box > img {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.v-card-cover-fallback {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}

.v-card-footer {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
}

.v-card-name {
  font-size: 13px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
