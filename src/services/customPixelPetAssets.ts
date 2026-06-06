import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { CustomPixelPetAsset } from "./customPixelPet";

export function customPixelPetPreviewUrl(asset: CustomPixelPetAsset) {
  const path = asset.preview_path || asset.sprite_path;
  if (!path || path.startsWith("data:") || path.startsWith("http:") || path.startsWith("https:")) {
    return path;
  }
  return convertFileSrc(path);
}

export async function listCustomPixelPetAssets() {
  return await invoke<CustomPixelPetAsset[]>("list_custom_pet_assets");
}

export async function getActiveCustomPixelPetAsset() {
  return await invoke<CustomPixelPetAsset | null>("get_active_custom_pet_asset");
}

export async function setActiveCustomPixelPetAsset(id: string | null) {
  return await invoke<CustomPixelPetAsset | null>("set_active_custom_pet_asset", { id });
}

export async function deleteCustomPixelPetAsset(id: string) {
  await invoke("delete_custom_pet_asset", { id });
}

export async function notifyPetAppearanceChanged() {
  const petWin = await WebviewWindow.getByLabel("pet");
  if (petWin) await petWin.emit("appearance-changed");
}
