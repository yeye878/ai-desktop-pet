<script setup lang="ts">
import { computed } from "vue";
import { X } from "@lucide/vue";
import { useAttachmentsStore } from "../stores/attachments";
import { formatFileSize, getFileIcon } from "../services/attachments";

const props = defineProps<{ draftKey: string }>();
const attachments = useAttachmentsStore();
const draft = computed(() => attachments.ensure(props.draftKey));
</script>

<template>
  <div v-if="draft.files.length || draft.error" class="attachment-tray">
    <div v-if="draft.error" class="attachment-error" role="alert">
      <span>{{ draft.error }}</span>
      <button type="button" aria-label="关闭附件提示" @click="draft.error = ''"><X :size="13" /></button>
    </div>
    <template v-if="draft.files.length">
      <div class="attachment-heading">
        <span role="status" aria-live="polite">{{ draft.files.length }} 个文件待发送<span v-if="draft.files.some(file => file.status === 'loading')"> · 读取中…</span></span>
        <button type="button" @click="attachments.clear(draftKey)">全部移除</button>
      </div>
      <TransitionGroup name="attachment" tag="div" class="attachment-list" aria-label="待发送附件">
        <div v-for="file in draft.files" :key="file.id" class="attachment-card" :class="{ loading: file.status === 'loading' }" :title="file.path">
          <img v-if="file.previewUrl" :src="file.previewUrl" alt="" @error="attachments.previewFallback(draftKey, file)" />
          <span v-else class="attachment-icon" aria-hidden="true">{{ file.isDir ? '📂' : getFileIcon(file.extension) }}</span>
          <span class="attachment-info">
            <span class="attachment-name">{{ file.name }}</span>
            <small>{{ file.status === 'loading' ? '读取中…' : file.isDir ? '目录' : formatFileSize(file.size) }}</small>
          </span>
          <button type="button" :aria-label="'移除附件 ' + file.name" :title="'移除 ' + file.name" @click="attachments.remove(draftKey, file.id)"><X :size="13" /></button>
        </div>
      </TransitionGroup>
    </template>
  </div>
</template>

<style scoped>
.attachment-tray { min-width: 0; margin-bottom: 8px; font-size: 11px; color: var(--dash-text-secondary, #7d7267); }
.attachment-heading, .attachment-error { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-bottom: 5px; }
.attachment-heading > span, .attachment-error > span { min-width: 0; overflow-wrap: anywhere; }
.attachment-tray button { display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0; border: 0; border-radius: 5px; background: transparent; color: inherit; font: inherit; cursor: pointer; padding: 4px; }
.attachment-tray button:hover { color: #b7472f; background: rgba(183,71,47,.07); }
.attachment-tray button:focus-visible { outline: 2px solid var(--pet-primary, #7d9fc7); outline-offset: 1px; }
.attachment-list { display: flex; flex-wrap: wrap; gap: 5px; max-height: 98px; overflow-y: auto; overscroll-behavior: contain; padding: 2px; }
.attachment-card { display: flex; align-items: center; gap: 6px; flex: 0 1 196px; min-width: 0; max-width: 100%; box-sizing: border-box; padding: 5px 6px; border: 1px solid rgba(120,110,100,.16); border-radius: 9px; background: rgba(255,255,255,.8); }
.attachment-card.loading { opacity: .65; }
.attachment-card img, .attachment-icon { flex-shrink: 0; width: 28px; height: 28px; border-radius: 5px; object-fit: cover; }
.attachment-icon { display: inline-flex; align-items: center; justify-content: center; font-size: 20px; }
.attachment-info { display: flex; flex-direction: column; flex: 1; gap: 2px; min-width: 0; }
.attachment-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--dash-text-primary, #3d3630); }
.attachment-info small { font-size: 10px; }
.attachment-error { padding: 6px 8px; border-radius: 7px; background: #fff0ed; color: #b7472f; }
.attachment-enter-active { transition: opacity .15s ease, transform .15s ease; }
.attachment-enter-from { opacity: 0; transform: translateY(3px); }
@media (prefers-reduced-motion: reduce) { .attachment-enter-active { transition: none; } }
</style>
