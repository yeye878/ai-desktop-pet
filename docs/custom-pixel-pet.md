# Custom Pixel Pet

## Goal

Allow users to upload an image and turn it into a personalized pixel desktop pet without replacing the existing `classic` and `daimao-batiao` characters.

## Boundaries

- `pet_character = "custom-pixel"` only selects the renderer type.
- `custom_pixel_pet_asset_id` selects the user's active generated asset.
- Generated assets are stored outside the settings table as files plus metadata.
- Canvas remains the playback surface; image generation and rendering are asset-driven.

## Flow

1. User uploads an image in the custom pixel pet workshop.
2. The frontend creates a 64x64 pixel sprite, removes edge background, quantizes colors, adds a pixel outline, and builds a multi-state spritesheet.
3. Tauri saves `spritesheet.png`, `preview.png`, and manifest metadata in app data under `custom_pets/{id}`.
4. Settings/Dashboard set `custom_pixel_pet_asset_id` and switch `pet_character` to `custom-pixel`.
5. `PetCanvas` loads the active spritesheet and maps pet states to sprite animations.

## Compatibility

- Classic canvas drawing is unchanged.
- Daimao video/still rendering is unchanged.
- Pet window hit testing now uses a shared helper so Canvas click/drag and transparent-window passthrough stay aligned.
- Deleting the active custom asset clears the asset setting and falls back to `classic`.
