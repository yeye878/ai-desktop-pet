<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { usePetStore } from "../stores/pet";
import AppIcon from "./AppIcon.vue";
import {
  CUSTOM_PIXEL_PET_DEFAULT_OPTIONS,
  generateCustomPixelPetFromFile,
  type CustomPixelPetAsset,
  type CustomPixelPetGenerationOptions,
} from "../services/customPixelPet";
import {
  customPixelPetPreviewUrl,
  deleteCustomPixelPetAsset,
  getActiveCustomPixelPetAsset,
  listCustomPixelPetAssets,
  notifyPetAppearanceChanged,
  setActiveCustomPixelPetAsset,
} from "../services/customPixelPetAssets";

const emit = defineEmits<{
  selected: [asset: CustomPixelPetAsset | null];
}>();

const pet = usePetStore();
const fileInputRef = ref<HTMLInputElement | null>(null);
const assets = ref<CustomPixelPetAsset[]>([]);
const activeAssetId = ref("");
const isGenerating = ref(false);
const isDraggingFile = ref(false);
const statusMessage = ref("");
const errorMessage = ref("");
const generationOptions = ref<CustomPixelPetGenerationOptions>({ ...CUSTOM_PIXEL_PET_DEFAULT_OPTIONS });

const currentAsset = computed(() => {
  return assets.value.find((asset) => asset.id === activeAssetId.value) || pet.customPixelPetAsset || null;
});

const currentPreviewUrl = computed(() => {
  return currentAsset.value ? customPixelPetPreviewUrl(currentAsset.value) : "";
});

const sizeOptions = [
  { label: "48", value: 48 },
  { label: "64", value: 64 },
  { label: "96", value: 96 },
] as const;

const paletteOptions = [
  { label: "柔和", value: 48 },
  { label: "默认", value: 32 },
  { label: "鲜明", value: 24 },
] as const;

function friendlyError(err: unknown) {
  const message = err instanceof Error ? err.message : String(err || "");
  if (message.includes("image file")) return "请选择图片文件。";
  if (message.includes("too large")) return "图片太大了，请换一张 6MB 以内的图片。";
  if (message.includes("not found")) return "这个形象资源已经不存在，请重新生成。";
  if (message.includes("Canvas")) return "当前环境暂时无法处理图片，请稍后再试。";
  return message || "操作失败，请稍后再试。";
}

function clearMessages() {
  errorMessage.value = "";
  statusMessage.value = "";
}

async function loadCustomPets() {
  try {
    assets.value = await listCustomPixelPetAssets();
    const active = await getActiveCustomPixelPetAsset();
    activeAssetId.value = active?.id || "";
    pet.customPixelPetAsset = active;
    if (pet.character === "custom-pixel" && !active) {
      pet.character = "classic";
      await setActiveCustomPixelPetAsset(null).catch(() => null);
    }
  } catch (err) {
    errorMessage.value = friendlyError(err);
  }
}

function chooseFile() {
  fileInputRef.value?.click();
}

async function selectAsset(asset: CustomPixelPetAsset) {
  clearMessages();
  try {
    const active = await setActiveCustomPixelPetAsset(asset.id);
    activeAssetId.value = active?.id || asset.id;
    pet.customPixelPetAsset = active || asset;
    pet.character = "custom-pixel";
    emit("selected", pet.customPixelPetAsset);
    statusMessage.value = "已切换为自定义像素形象。";
    await notifyPetAppearanceChanged();
  } catch (err) {
    errorMessage.value = friendlyError(err);
  }
}

async function generateFromFile(file: File) {
  isGenerating.value = true;
  clearMessages();
  try {
    const generated = await generateCustomPixelPetFromFile(file, generationOptions.value);
    const saved = await invoke<CustomPixelPetAsset>("save_custom_pet_asset", {
      request: {
        name: generated.name,
        kind: "custom-pixel",
        manifest: JSON.stringify(generated.manifest),
        spriteDataUrl: generated.spriteDataUrl,
        previewDataUrl: generated.previewDataUrl,
      },
    });
    await loadCustomPets();
    await selectAsset(saved);
    statusMessage.value = "生成完成，已应用到桌宠。";
  } catch (err) {
    errorMessage.value = friendlyError(err);
  } finally {
    isGenerating.value = false;
  }
}

function imageFilesFromList(files: FileList | File[] | null | undefined) {
  return Array.from(files || []).filter((file) => file.type.startsWith("image/"));
}

async function generateFromFiles(files: File[]) {
  const imageFiles = imageFilesFromList(files);
  if (imageFiles.length === 0) {
    clearMessages();
    errorMessage.value = friendlyError(new Error("Please choose an image file."));
    return;
  }

  if (imageFiles.length === 1) {
    await generateFromFile(imageFiles[0]);
    return;
  }

  isGenerating.value = true;
  clearMessages();
  const savedAssets: CustomPixelPetAsset[] = [];
  try {
    for (let index = 0; index < imageFiles.length; index++) {
      statusMessage.value = `正在生成 ${index + 1} / ${imageFiles.length}`;
      const generated = await generateCustomPixelPetFromFile(imageFiles[index], generationOptions.value);
      const saved = await invoke<CustomPixelPetAsset>("save_custom_pet_asset", {
        request: {
          name: generated.name,
          kind: "custom-pixel",
          manifest: JSON.stringify(generated.manifest),
          spriteDataUrl: generated.spriteDataUrl,
          previewDataUrl: generated.previewDataUrl,
        },
      });
      savedAssets.push(saved);
    }

    await loadCustomPets();
    const lastSaved = savedAssets[savedAssets.length - 1];
    if (lastSaved) await selectAsset(lastSaved);
    statusMessage.value = `已生成 ${savedAssets.length} 个自定义形象，并应用最后一个。`;
  } catch (err) {
    if (savedAssets.length > 0) await loadCustomPets().catch(() => null);
    errorMessage.value = friendlyError(err);
  } finally {
    isGenerating.value = false;
  }
}

async function onFileSelected(event: Event) {
  const input = event.target as HTMLInputElement;
  const files = Array.from(input.files || []);
  input.value = "";
  if (files.length) await generateFromFiles(files);
}

async function onDrop(event: DragEvent) {
  isDraggingFile.value = false;
  const files = Array.from(event.dataTransfer?.files || []);
  if (files.length) await generateFromFiles(files);
}

async function deleteAsset(asset: CustomPixelPetAsset) {
  const confirmed = window.confirm(`删除“${asset.name}”吗？这个自定义形象文件会从本机移除。`);
  if (!confirmed) return;

  clearMessages();
  try {
    const wasActive = activeAssetId.value === asset.id;
    await deleteCustomPixelPetAsset(asset.id);
    assets.value = assets.value.filter((item) => item.id !== asset.id);
    if (wasActive) {
      const nextAsset = assets.value[0] || null;
      if (nextAsset) {
        await selectAsset(nextAsset);
        statusMessage.value = "已删除当前形象，并切换到下一个自定义形象。";
        return;
      }

      activeAssetId.value = "";
      pet.customPixelPetAsset = null;
      pet.character = "classic";
      await setActiveCustomPixelPetAsset(null).catch(() => null);
      emit("selected", null);
      statusMessage.value = "已删除当前形象，并回到默认形象。";
      await notifyPetAppearanceChanged();
    } else {
      statusMessage.value = "已删除自定义形象。";
    }
  } catch (err) {
    errorMessage.value = friendlyError(err);
  }
}

onMounted(() => {
  void loadCustomPets();
});
</script>

<template>
  <div
    class="custom-pixel-workshop"
    :class="{ dragging: isDraggingFile }"
    @dragover.prevent="isDraggingFile = true"
    @dragleave.prevent="isDraggingFile = false"
    @drop.prevent="onDrop"
  >
    <div class="custom-pixel-preview">
      <img v-if="currentPreviewUrl" :src="currentPreviewUrl" alt="" />
      <span v-else>PX</span>
    </div>

    <div class="custom-pixel-main">
      <div class="custom-pixel-actions">
        <button class="custom-pixel-btn primary" :disabled="isGenerating" @click="chooseFile">
          {{ isGenerating ? "生成中" : "上传生成" }}
        </button>
        <button
          class="custom-pixel-btn"
          :disabled="!currentAsset || currentAsset.id === activeAssetId"
          @click="currentAsset && selectAsset(currentAsset)"
        >
          {{ currentAsset?.id === activeAssetId ? "使用中" : "使用" }}
        </button>
      </div>

      <div class="custom-pixel-options" aria-label="生成选项">
        <label>
          尺寸
          <select v-model.number="generationOptions.frameSize" :disabled="isGenerating">
            <option v-for="option in sizeOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
        </label>
        <label>
          调色
          <select v-model.number="generationOptions.paletteStep" :disabled="isGenerating">
            <option v-for="option in paletteOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
        </label>
        <label class="custom-pixel-checkbox">
          <input v-model="generationOptions.removeBackground" type="checkbox" :disabled="isGenerating" />
          去背景
        </label>
        <label class="custom-pixel-checkbox">
          <input v-model="generationOptions.outline" type="checkbox" :disabled="isGenerating" />
          描边
        </label>
      </div>

      <div v-if="assets.length" class="custom-pixel-list">
        <div
          v-for="asset in assets"
          :key="asset.id"
          class="custom-pixel-entry"
        >
          <button
            :class="['custom-pixel-item', { active: activeAssetId === asset.id }]"
            :disabled="isGenerating"
            @click="selectAsset(asset)"
          >
            <img :src="customPixelPetPreviewUrl(asset)" alt="" />
            <span>{{ asset.name }}</span>
            <b v-if="activeAssetId === asset.id">使用中</b>
          </button>
          <button
            class="custom-pixel-delete"
            :disabled="isGenerating"
            aria-label="删除自定义形象"
            @click.stop="deleteAsset(asset)"
          >
            <AppIcon name="trash" :size="13" />
          </button>
        </div>
      </div>
      <p v-else class="custom-pixel-empty">拖入图片或点击上传，生成你的像素桌宠。</p>

      <p v-if="statusMessage" class="custom-pixel-status">{{ statusMessage }}</p>
      <p v-if="errorMessage" class="custom-pixel-error">{{ errorMessage }}</p>
    </div>

    <input
      ref="fileInputRef"
      class="custom-pixel-input"
      type="file"
      accept="image/*"
      multiple
      @change="onFileSelected"
    />
  </div>
</template>

<style scoped>
.custom-pixel-workshop {
  display: grid;
  grid-template-columns: 82px minmax(0, 1fr);
  gap: 12px;
  align-items: start;
  padding: 12px;
  border: 1px dashed var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  border-radius: var(--dash-radius-lg, 13px);
  background: var(--dash-panel-soft, #f3f0e9);
  transition:
    border-color 0.18s cubic-bezier(0.22, 1, 0.36, 1),
    background-color 0.18s cubic-bezier(0.22, 1, 0.36, 1);
}

.custom-pixel-workshop.dragging {
  border-color: var(--dash-accent, #bf7a4e);
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.06));
}

.custom-pixel-preview {
  width: 82px;
  height: 82px;
  display: grid;
  place-items: center;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  border-radius: var(--dash-radius-md, 10px);
  background:
    linear-gradient(45deg, rgba(63, 54, 44, 0.05) 25%, transparent 25%),
    linear-gradient(-45deg, rgba(63, 54, 44, 0.05) 25%, transparent 25%),
    var(--dash-panel-solid, #fffefb);
  background-size: 12px 12px;
  box-shadow: var(--dash-shadow-xs, 0 1px 2px rgba(48, 42, 34, 0.05));
  overflow: hidden;
}

.custom-pixel-preview img,
.custom-pixel-item img {
  width: 64px;
  height: 64px;
  object-fit: contain;
  image-rendering: pixelated;
}

.custom-pixel-preview span {
  font-family: var(--dash-font-mono, monospace);
  font-size: 15px;
  font-weight: 700;
  letter-spacing: 0.12em;
  color: var(--dash-text-muted, #a29a8a);
}

.custom-pixel-main {
  min-width: 0;
}

.custom-pixel-actions,
.custom-pixel-options {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.custom-pixel-options {
  margin-top: 10px;
  align-items: center;
}

.custom-pixel-options label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 28px;
  font-size: 12px;
  font-weight: 650;
  color: var(--dash-text-secondary, #6d6558);
}

.custom-pixel-options select {
  min-height: 28px;
  padding: 3px 26px 3px 10px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  border-radius: 9px;
  background: var(--dash-panel-solid, #fffefb)
    url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='M1 1l4 4 4-4' fill='none' stroke='%23a29a8a' stroke-width='1.6' stroke-linecap='round'/%3E%3C/svg%3E")
    no-repeat right 9px center;
  appearance: none;
  color: var(--dash-text-primary, #2d2922);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  transition:
    border-color 0.16s cubic-bezier(0.22, 1, 0.36, 1),
    box-shadow 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.custom-pixel-options select:focus {
  outline: none;
  border-color: var(--dash-accent, #bf7a4e);
  box-shadow: 0 0 0 3px var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
}

.custom-pixel-checkbox {
  cursor: pointer;
}

.custom-pixel-checkbox input {
  appearance: none;
  width: 36px;
  height: 21px;
  margin: 0;
  flex-shrink: 0;
  border-radius: 999px;
  background: rgba(63, 54, 44, 0.16);
  position: relative;
  cursor: pointer;
  transition: background 0.18s cubic-bezier(0.34, 1.3, 0.64, 1);
}

.custom-pixel-checkbox input::after {
  content: "";
  position: absolute;
  top: 2px;
  left: 2px;
  width: 17px;
  height: 17px;
  border-radius: 50%;
  background: #fffefb;
  box-shadow: 0 1px 3px rgba(48, 42, 34, 0.28);
  transition: transform 0.18s cubic-bezier(0.34, 1.3, 0.64, 1);
}

.custom-pixel-checkbox input:checked {
  background: var(--dash-accent, #bf7a4e);
}

.custom-pixel-checkbox input:checked::after {
  transform: translateX(15px);
}

.custom-pixel-checkbox input:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* 主/次按钮 */
.custom-pixel-btn {
  min-height: 32px;
  padding: 0 14px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  border-radius: 9px;
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-primary, #2d2922);
  font-size: 12.5px;
  font-weight: 650;
  font-family: inherit;
  cursor: pointer;
  transition:
    transform 0.16s cubic-bezier(0.22, 1, 0.36, 1),
    border-color 0.16s cubic-bezier(0.22, 1, 0.36, 1),
    background 0.16s cubic-bezier(0.22, 1, 0.36, 1),
    box-shadow 0.16s cubic-bezier(0.22, 1, 0.36, 1),
    opacity 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.custom-pixel-btn:hover:not(:disabled):not(.primary) {
  background: var(--dash-panel-soft, #f3f0e9);
  transform: translateY(-1px);
}

.custom-pixel-btn.primary {
  border-color: transparent;
  background: var(--dash-accent, #bf7a4e);
  color: #fffaf4;
  box-shadow: 0 2px 10px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.28);
}

.custom-pixel-btn.primary:hover:not(:disabled) {
  background: var(--dash-accent-dark, #a8663c);
  transform: translateY(-1px);
}

.custom-pixel-btn.danger {
  border-color: transparent;
  color: var(--dash-danger, #c05a4d);
}

.custom-pixel-btn.danger:hover:not(:disabled) {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
}

.custom-pixel-btn:active:not(:disabled) {
  transform: scale(0.98);
}

.custom-pixel-btn:focus-visible,
.custom-pixel-item:focus-visible,
.custom-pixel-delete:focus-visible {
  outline: none;
  border-color: var(--dash-accent, #bf7a4e);
  box-shadow: 0 0 0 3px var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
}

.custom-pixel-btn:disabled,
.custom-pixel-item:disabled,
.custom-pixel-delete:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

/* 帧列表：小卡片网格 */
.custom-pixel-list {
  margin-top: 10px;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(136px, 1fr));
  gap: 8px;
}

.custom-pixel-entry {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
}

.custom-pixel-item {
  flex: 1;
  min-width: 0;
  display: grid;
  grid-template-columns: 34px minmax(0, 1fr);
  gap: 7px;
  align-items: center;
  min-height: 44px;
  padding: 5px 8px 5px 5px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  border-radius: var(--dash-radius-md, 10px);
  background: var(--dash-panel-solid, #fffefb);
  box-shadow: var(--dash-shadow-xs, 0 1px 2px rgba(48, 42, 34, 0.05));
  color: var(--dash-text-primary, #2d2922);
  font-family: inherit;
  cursor: pointer;
  transition:
    transform 0.16s cubic-bezier(0.22, 1, 0.36, 1),
    border-color 0.16s cubic-bezier(0.22, 1, 0.36, 1),
    background 0.16s cubic-bezier(0.22, 1, 0.36, 1),
    box-shadow 0.16s cubic-bezier(0.22, 1, 0.36, 1),
    opacity 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.custom-pixel-item:hover:not(:disabled) {
  transform: translateY(-1px);
  border-color: var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  box-shadow: var(--dash-shadow-sm, 0 2px 8px rgba(48, 42, 34, 0.07));
}

.custom-pixel-item:active:not(:disabled) {
  transform: scale(0.98);
}

.custom-pixel-item.active {
  border-color: var(--dash-accent, #bf7a4e);
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.06));
  box-shadow: 0 0 0 3px var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
}

.custom-pixel-item img {
  width: 32px;
  height: 32px;
}

.custom-pixel-item span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  font-weight: 650;
}

.custom-pixel-item b {
  grid-column: 1 / -1;
  margin-top: -3px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.1em;
  color: var(--dash-accent, #bf7a4e);
}

/* 删除：icon 按钮 */
.custom-pixel-delete {
  width: 26px;
  height: 26px;
  flex-shrink: 0;
  display: grid;
  place-items: center;
  padding: 0;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--dash-text-muted, #a29a8a);
  cursor: pointer;
  transition:
    background 0.16s cubic-bezier(0.22, 1, 0.36, 1),
    color 0.16s cubic-bezier(0.22, 1, 0.36, 1),
    opacity 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.custom-pixel-delete:hover:not(:disabled) {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.1));
  color: var(--dash-danger, #c05a4d);
}

.custom-pixel-empty,
.custom-pixel-status,
.custom-pixel-error {
  margin: 10px 0 0;
  font-size: 12px;
  line-height: 1.6;
}

.custom-pixel-empty {
  color: var(--dash-text-muted, #a29a8a);
}

.custom-pixel-status {
  color: var(--dash-success, #5e8f6a);
}

.custom-pixel-error {
  color: var(--dash-danger, #c05a4d);
}

.custom-pixel-input {
  display: none;
}

@media (max-width: 520px) {
  .custom-pixel-workshop {
    grid-template-columns: 64px minmax(0, 1fr);
  }

  .custom-pixel-preview {
    width: 64px;
    height: 64px;
  }

  .custom-pixel-preview img {
    width: 54px;
    height: 54px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .custom-pixel-workshop,
  .custom-pixel-btn,
  .custom-pixel-item,
  .custom-pixel-delete,
  .custom-pixel-checkbox input,
  .custom-pixel-checkbox input::after {
    animation: none !important;
    transition-duration: 1ms !important;
    transform: none !important;
  }
}
</style>
