<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { usePetStore } from "../stores/pet";
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
            ×
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
  border-radius: 8px;
  outline: 1px solid transparent;
  outline-offset: 4px;
  transition: outline-color 0.16s ease, background-color 0.16s ease;
}

.custom-pixel-workshop.dragging {
  background: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.06);
  outline-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.35);
}

.custom-pixel-preview {
  width: 82px;
  height: 82px;
  display: grid;
  place-items: center;
  border: 1px solid rgba(148, 163, 184, 0.28);
  background:
    linear-gradient(45deg, rgba(148, 163, 184, 0.12) 25%, transparent 25%),
    linear-gradient(-45deg, rgba(148, 163, 184, 0.12) 25%, transparent 25%),
    rgba(255, 255, 255, 0.72);
  background-size: 12px 12px;
  border-radius: 8px;
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
  font-size: 18px;
  font-weight: 800;
  color: rgba(51, 65, 85, 0.68);
}

.custom-pixel-main {
  min-width: 0;
}

.custom-pixel-actions,
.custom-pixel-list,
.custom-pixel-options {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.custom-pixel-options {
  margin-top: 8px;
  align-items: center;
}

.custom-pixel-options label {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 28px;
  font-size: 12px;
  font-weight: 700;
  color: rgba(51, 65, 85, 0.78);
}

.custom-pixel-options select {
  min-height: 28px;
  border: 1px solid rgba(148, 163, 184, 0.32);
  border-radius: 7px;
  background: rgba(255, 255, 255, 0.82);
  color: var(--pet-font-color, #334155);
  font-size: 12px;
}

.custom-pixel-checkbox input {
  width: 14px;
  height: 14px;
}

.custom-pixel-list {
  margin-top: 8px;
  align-items: center;
}

.custom-pixel-btn,
.custom-pixel-item,
.custom-pixel-delete {
  border: 1px solid rgba(var(--pet-primary-rgb, 255, 107, 107), 0.22);
  background: rgba(255, 255, 255, 0.78);
  color: var(--pet-font-color, #334155);
  border-radius: 8px;
  cursor: pointer;
}

.custom-pixel-btn {
  min-height: 34px;
  padding: 0 12px;
  font-size: 13px;
  font-weight: 700;
}

.custom-pixel-btn.primary {
  background: var(--pet-bubble-user, linear-gradient(135deg, #ff6b6b, #ff8e8e));
  color: white;
}

.custom-pixel-btn.danger {
  border-color: rgba(239, 68, 68, 0.25);
  color: #dc2626;
}

.custom-pixel-btn:disabled,
.custom-pixel-item:disabled,
.custom-pixel-delete:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.custom-pixel-entry {
  display: flex;
  align-items: stretch;
  gap: 4px;
}

.custom-pixel-item {
  display: grid;
  grid-template-columns: 34px minmax(0, 88px);
  gap: 7px;
  align-items: center;
  min-height: 42px;
  padding: 4px 8px 4px 4px;
  position: relative;
}

.custom-pixel-delete {
  width: 30px;
  min-height: 42px;
  padding: 0;
  color: #dc2626;
  font-size: 16px;
  font-weight: 800;
}

.custom-pixel-item.active {
  border-color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.55);
  box-shadow: 0 0 0 2px rgba(var(--pet-primary-rgb, 255, 107, 107), 0.1);
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
  font-weight: 700;
}

.custom-pixel-item b {
  grid-column: 1 / -1;
  margin-top: -3px;
  font-size: 10px;
  color: rgba(var(--pet-primary-rgb, 255, 107, 107), 0.95);
}

.custom-pixel-empty,
.custom-pixel-status,
.custom-pixel-error {
  margin: 8px 0 0;
  font-size: 12px;
}

.custom-pixel-empty {
  color: rgba(71, 85, 105, 0.72);
}

.custom-pixel-status {
  color: #15803d;
}

.custom-pixel-error {
  color: #dc2626;
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
</style>
