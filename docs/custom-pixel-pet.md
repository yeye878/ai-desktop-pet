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
2. The frontend creates a 48x48, 64x64, or 96x96 pixel sprite, applies the selected background, palette, and outline options, and builds a multi-state spritesheet.
3. Tauri saves `spritesheet.png`, `preview.png`, and manifest metadata in app data under `custom_pets/{id}`.
4. Settings/Dashboard activate the asset through `set_active_custom_pet_asset`, which writes `custom_pixel_pet_asset_id` and switches `pet_character` to `custom-pixel` together.
5. `PetCanvas` loads the active spritesheet and maps pet states to sprite animations.

## Commands

- `save_custom_pet_asset`: validates the manifest and PNG data URLs, then stores the asset files and DB metadata.
- `list_custom_pet_assets`: returns saved custom pixel assets newest first.
- `get_custom_pet_asset`: reads one asset by id.
- `get_active_custom_pet_asset`: resolves the current `custom_pixel_pet_asset_id` to an asset, preserving compatibility with the original setting.
- `set_active_custom_pet_asset`: validates and activates an asset, or clears the active asset and falls back to `classic`.
- `delete_custom_pet_asset`: removes metadata and local files; deleting the active asset clears it and falls back to `classic`.

## Manifest

The renderer accepts manifest version `1` with:

- `renderer: "pixel-sprite"`
- `frameSize`: square `48`, `64`, or `96`
- `sheet`: bounded columns and rows
- `animations.idle`: required fallback animation
- optional `generatorVersion`, `source`, and `generationOptions`

Older generated assets remain valid because the added fields are optional.

## Compatibility

- Classic canvas drawing is unchanged.
- Daimao video/still rendering is unchanged.
- Pet window hit testing now uses a shared helper so Canvas click/drag and transparent-window passthrough stay aligned.
- Deleting the active custom asset clears the asset setting and falls back to `classic`.
- If `custom-pixel` is selected but no active asset can be loaded, the real pet window falls back to `classic`; preview surfaces may still show a neutral pixel placeholder.

## UX Checks

- Upload and drag/drop both use the same generator path.
- Generation failure keeps the current pet unchanged and shows a localized message.
- The active asset is marked as in use.
- Delete requires confirmation.
- Settings and Dashboard share the same active asset commands, preventing split state between entry points.
