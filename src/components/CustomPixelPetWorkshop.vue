<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { usePetStore } from "../stores/pet";
import {
  CUSTOM_PIXEL_PET_SETTING_KEY,
  generateCustomPixelPetFromFile,
  type CustomPixelPetAsset,
} from "../services/customPixelPet";
import { PET_CHARACTER_SETTING_KEY } from "../services/petCharacters";

const emit = defineEmits<{
  selected: [asset: CustomPixelPetAsset | null];
}>();

const pet = usePetStore();
const fileInputRef = ref<HTMLInputElement | null>(null);
const assets = ref<CustomPixelPetAsset[]>([]);
const activeAssetId = ref("");
const isGenerating = ref(false);
const errorMessage = ref("");

const currentAsset = computed(() => {
  return pet.customPixelPetAsset || assets.value.find((asset) => asset.id === activeAssetId.value) || null;
});

const currentPreviewUrl = computed(() => {
  return currentAsset.value ? assetPreviewUrl(currentAsset.value) : "";
});

function assetPreviewUrl(asset: CustomPixelPetAsset) {
  const path = asset.preview_path || asset.sprite_path;
  if (!path || path.startsWith("data:") || path.startsWith("http:") || path.startsWith("https:")) {
    return path;
  }
  return convertFileSrc(path);
}

async function notifyAppearanceChanged() {
  const petWin = await WebviewWindow.getByLabel("pet");
  if (petWin) await petWin.emit("appearance-changed");
}

async function loadCustomPets() {
  try {
    assets.value = await invoke<CustomPixelPetAsset[]>("list_custom_pet_assets");
    activeAssetId.value = await invoke<string>("get_setting_value", {
      key: CUSTOM_PIXEL_PET_SETTING_KEY,
    });
    const active = assets.value.find((asset) => asset.id === activeAssetId.value) || null;
    if (active) {
      pet.customPixelPetAsset = active;
    }
  } catch (err) {
    errorMessage.value = String(err);
  }
}

function chooseFile() {
  fileInputRef.value?.click();
}

async function selectAsset(asset: CustomPixelPetAsset) {
  await invoke("set_setting_value", {
    key: CUSTOM_PIXEL_PET_SETTING_KEY,
    value: asset.id,
  });
  await invoke("set_setting_value", {
    key: PET_CHARACTER_SETTING_KEY,
    value: "custom-pixel",
  });
  activeAssetId.value = asset.id;
  pet.customPixelPetAsset = asset;
  pet.character = "custom-pixel";
  emit("selected", asset);
  await notifyAppearanceChanged();
}

async function onFileSelected(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file) return;

  isGenerating.value = true;
  errorMessage.value = "";
  try {
    const generated = await generateCustomPixelPetFromFile(file);
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
  } catch (err) {
    errorMessage.value = String(err);
  } finally {
    isGenerating.value = false;
  }
}

async function deleteAsset(asset: CustomPixelPetAsset) {
  try {
    await invoke("delete_custom_pet_asset", { id: asset.id });
    assets.value = assets.value.filter((item) => item.id !== asset.id);
    if (activeAssetId.value === asset.id) {
      activeAssetId.value = "";
      pet.customPixelPetAsset = null;
      pet.character = "classic";
      emit("selected", null);
      await notifyAppearanceChanged();
    }
  } catch (err) {
    errorMessage.value = String(err);
  }
}

onMounted(() => {
  void loadCustomPets();
});
</script>

<template>
  <div class="custom-pixel-workshop">
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
          :disabled="!currentAsset"
          @click="currentAsset && selectAsset(currentAsset)"
        >
          使用
        </button>
      </div>

      <div v-if="assets.length" class="custom-pixel-list">
        <button
          v-for="asset in assets"
          :key="asset.id"
          :class="['custom-pixel-item', { active: activeAssetId === asset.id }]"
          @click="selectAsset(asset)"
        >
          <img :src="assetPreviewUrl(asset)" alt="" />
          <span>{{ asset.name }}</span>
        </button>
        <button
          v-if="currentAsset"
          class="custom-pixel-btn danger"
          @click="deleteAsset(currentAsset)"
        >
          删除
        </button>
      </div>

      <p v-if="errorMessage" class="custom-pixel-error">{{ errorMessage }}</p>
    </div>

    <input
      ref="fileInputRef"
      class="custom-pixel-input"
      type="file"
      accept="image/*"
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
.custom-pixel-list {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.custom-pixel-list {
  margin-top: 8px;
  align-items: center;
}

.custom-pixel-btn,
.custom-pixel-item {
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

.custom-pixel-btn:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.custom-pixel-item {
  display: grid;
  grid-template-columns: 34px minmax(0, 88px);
  gap: 7px;
  align-items: center;
  min-height: 42px;
  padding: 4px 8px 4px 4px;
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

.custom-pixel-error {
  margin: 8px 0 0;
  color: #dc2626;
  font-size: 12px;
}

.custom-pixel-input {
  display: none;
}
</style>
