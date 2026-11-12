<script setup lang="ts">
withDefaults(
  defineProps<{
    name: string;
    size?: number | string;
    strokeWidth?: number | string;
  }>(),
  {
    size: 16,
    strokeWidth: 1.85,
  },
);
</script>

<template>
  <svg
    class="app-icon"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    :stroke-width="strokeWidth"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <!-- Brand / Companion Pet Mark -->
    <template v-if="name === 'brand-pet'">
      <path d="M5.5 11.5L4.5 5.2L9.5 8.2C10.3 7.9 11.1 7.8 12 7.8C12.9 7.8 13.7 7.9 14.5 8.2L19.5 5.2L18.5 11.5C19.3 12.6 19.8 14 19.8 15.5C19.8 19.1 16.3 21.2 12 21.2C7.7 21.2 4.2 19.1 4.2 15.5C4.2 14 4.7 12.6 5.5 11.5Z" />
      <path d="M9 14.5C9.3 15.1 9.8 15.1 10.1 14.5" />
      <path d="M13.9 14.5C14.2 15.1 14.7 15.1 15 14.5" />
      <path d="M11.2 17.2C11.7 17.7 12.3 17.7 12.8 17.2" opacity="0.65" />
      <path d="M19.5 2.8L20.1 4.2L21.5 4.8L20.1 5.4L19.5 6.8L18.9 5.4L17.5 4.8L18.9 4.2Z" fill="currentColor" stroke="none" opacity="0.55" />
    </template>

    <!-- User Portrait Silhouette -->
    <template v-else-if="name === 'user-avatar'">
      <circle cx="12" cy="8.5" r="3.8" />
      <path d="M5.2 19.8C6.3 16.4 8.9 14.8 12 14.8C15.1 14.8 17.7 16.4 18.8 19.8" />
      <path d="M9.8 15.2L12 17.4L14.2 15.2" opacity="0.4" />
    </template>

    <!-- Claude Code Mark (Warm Starburst) -->
    <template v-else-if="name === 'claude'">
      <path d="M12 3.2V20.8" />
      <path d="M3.2 12H20.8" />
      <path d="M5.8 5.8L18.2 18.2" />
      <path d="M18.2 5.8L5.8 18.2" />
      <circle cx="12" cy="12" r="2.6" fill="currentColor" stroke="none" opacity="0.22" />
    </template>

    <!-- Codex Mark (Hexagonal Terminal >_) -->
    <template v-else-if="name === 'codex'">
      <path d="M12 2.8L19.8 7.3V16.7L12 21.2L4.2 16.7V7.3L12 2.8Z" />
      <path d="M8.5 10L11.2 12.2L8.5 14.4" />
      <path d="M12.6 14.5H15.6" />
    </template>

    <!-- DeepSeek Harness Mark (Sleek Wave & Fin Crest) -->
    <template v-else-if="name === 'dsh'">
      <path d="M3.5 15.2C6 15.2 7.4 11.5 10.8 11.5C13.6 11.5 14.8 14.2 17.5 14.2C19 14.2 20.1 13.3 20.8 12.2" />
      <path d="M10.5 11.5C10.5 8.2 12.8 5.6 16.4 5.2C15.2 7.3 15.3 9.4 16.5 11.2" />
      <path d="M5 18.6C7.8 18.6 9.6 17.2 12.4 17.2C15.2 17.2 16.8 18.6 19.2 18.6" opacity="0.42" />
      <circle cx="7.2" cy="7.5" r="1.1" fill="currentColor" stroke="none" opacity="0.5" />
    </template>

    <!-- Direct API Mark (Orbital Node) -->
    <template v-else-if="name === 'api'">
      <circle cx="12" cy="12" r="3" />
      <path d="M12 3.2V6.2" />
      <path d="M12 17.8V20.8" />
      <path d="M3.2 12H6.2" />
      <path d="M17.8 12H20.8" />
      <circle cx="18.2" cy="5.8" r="1.6" />
      <path d="M14.2 9.8L17 7" opacity="0.45" />
      <circle cx="5.8" cy="18.2" r="1.6" />
      <path d="M9.8 14.2L7 17" opacity="0.45" />
    </template>

    <!-- Companion Action: Wave -->
    <template v-else-if="name === 'wave'">
      <path d="M11.2 5.5C11.2 4.7 11.9 4 12.7 4C13.5 4 14.2 4.7 14.2 5.5V11" />
      <path d="M8.2 7.2C8.2 6.4 8.9 5.7 9.7 5.7C10.5 5.7 11.2 6.4 11.2 7.2V11.2" />
      <path d="M14.2 7.5C14.2 6.7 14.9 6 15.7 6C16.5 6 17.2 6.7 17.2 7.5V13.5C17.2 17.1 14.5 19.8 11 19.8C7.8 19.8 5.5 17.5 5.5 14.2V10.5C5.5 9.7 6.2 9 7 9C7.8 9 8.5 9.7 8.5 10.5V12.8" />
      <path d="M18.8 4.8C20 5.8 20.6 7.3 20.5 9" opacity="0.45" />
      <path d="M3.8 6.8C3.2 8 3.1 9.5 3.6 10.8" opacity="0.45" />
    </template>

    <!-- Companion Action: Happy -->
    <template v-else-if="name === 'happy'">
      <circle cx="12" cy="12.5" r="8" />
      <path d="M8.4 11.2C8.9 10.2 10.1 10.2 10.6 11.2" />
      <path d="M13.4 11.2C13.9 10.2 15.1 10.2 15.6 11.2" />
      <path d="M8.8 14.6C9.7 16.2 11 16.8 12 16.8C13 16.8 14.3 16.2 15.2 14.6" />
      <path d="M19.2 3.2L19.8 4.6L21.2 5.2L19.8 5.8L19.2 7.2L18.6 5.8L17.2 5.2L18.6 4.6Z" fill="currentColor" stroke="none" opacity="0.55" />
    </template>

    <!-- Companion Action: Sleep -->
    <template v-else-if="name === 'sleep'">
      <path d="M19.5 14.8C18.2 15.3 16.8 15.5 15.3 15.2C11.2 14.4 8.5 10.4 9.3 6.3C9.5 5.2 10 4.2 10.7 3.4C6.8 4.1 3.8 7.5 3.8 11.7C3.8 16.4 7.6 20.2 12.3 20.2C15.6 20.2 18.5 18 19.5 14.8Z" />
      <path d="M16.2 4.5H19.8L16.2 8.2H19.8" opacity="0.55" />
    </template>

    <!-- Companion Action: Wake -->
    <template v-else-if="name === 'wake'">
      <path d="M6.5 16.5C6.5 13.5 9 11 12 11C15 11 17.5 13.5 17.5 16.5" />
      <path d="M3.5 16.5H20.5" />
      <path d="M7.5 19.5H16.5" opacity="0.45" />
      <path d="M12 5V7.5" />
      <path d="M6.2 8.2L7.9 9.9" />
      <path d="M17.8 8.2L16.1 9.9" />
    </template>

    <!-- New Chat (Bubble + Plus) -->
    <template v-else-if="name === 'chat-plus'">
      <path d="M19.8 12.2C19.8 16.1 16.3 19.2 12 19.2C10.6 19.2 9.3 18.9 8.1 18.3L4.2 19.6L5.3 16.1C4.6 15 4.2 13.6 4.2 12.2C4.2 8.3 7.7 5.2 12 5.2C16.3 5.2 19.8 8.3 19.8 12.2Z" />
      <path d="M12 9.4V15" />
      <path d="M9.2 12.2H14.8" />
    </template>

    <!-- Dashboard Grid -->
    <template v-else-if="name === 'dashboard'">
      <rect x="3.8" y="3.8" width="7.2" height="7.2" rx="2.2" />
      <rect x="13" y="3.8" width="7.2" height="4.8" rx="2" opacity="0.55" />
      <rect x="13" y="10.8" width="7.2" height="9.4" rx="2.2" />
      <rect x="3.8" y="13.2" width="7.2" height="7" rx="2" opacity="0.55" />
    </template>

    <!-- Hide Pet / Minimize to Tray -->
    <template v-else-if="name === 'pet-hide'">
      <path d="M4.2 10.5C6.2 13.4 8.9 14.8 12 14.8C15.1 14.8 17.8 13.4 19.8 10.5" />
      <path d="M7.5 13.5L6 16.2" opacity="0.55" />
      <path d="M12 14.8V17.8" opacity="0.55" />
      <path d="M16.5 13.5L18 16.2" opacity="0.55" />
    </template>

    <!-- Home Shortcut: Chat Cabin -->
    <template v-else-if="name === 'chat-cabin'">
      <path d="M20 11.8C20 15.8 16.4 19 12 19C10.5 19 9.1 18.6 7.9 18L4 19.4L5.2 15.8C4.4 14.6 4 13.3 4 11.8C4 7.8 7.6 4.6 12 4.6C16.4 4.6 20 7.8 20 11.8Z" />
      <path d="M8.5 11.2H15.5" opacity="0.55" />
      <path d="M8.5 14.2H12.8" opacity="0.55" />
    </template>

    <!-- Home Shortcut / Card: Wardrobe / Skin Palette -->
    <template v-else-if="name === 'wardrobe'">
      <path d="M12 3.6C7.3 3.6 3.6 7.2 3.6 11.7C3.6 16.2 7.3 19.8 11.8 19.8C13.3 19.8 14.4 18.7 14.4 17.3C14.4 16.6 14.1 16 13.7 15.5C13.3 15 13.1 14.5 13.1 13.9C13.1 12.6 14.2 11.6 15.5 11.6H17C19 11.6 20.4 10.1 20.4 8.2C20.4 5.5 16.5 3.6 12 3.6Z" />
      <circle cx="8.2" cy="9.8" r="1.1" fill="currentColor" stroke="none" />
      <circle cx="11.5" cy="7.6" r="1.1" fill="currentColor" stroke="none" opacity="0.65" />
      <circle cx="15.2" cy="8.4" r="1.1" fill="currentColor" stroke="none" opacity="0.45" />
    </template>

    <!-- Home Shortcut / Card: Voice Station -->
    <template v-else-if="name === 'voice-station'">
      <rect x="9.2" y="3.8" width="5.6" height="10.2" rx="2.8" />
      <path d="M6.2 11.2C6.2 14.4 8.8 17 12 17C15.2 17 17.8 14.4 17.8 11.2" />
      <path d="M12 17V20.2" />
      <path d="M9 20.2H15" opacity="0.55" />
    </template>

    <!-- Home Shortcut / Card: Agent Hub -->
    <template v-else-if="name === 'agent-hub'">
      <rect x="5" y="6.8" width="14" height="11.4" rx="3.6" />
      <path d="M12 3.6V6.8" />
      <circle cx="12" cy="3.2" r="1" fill="currentColor" stroke="none" />
      <circle cx="9.4" cy="12.2" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="14.6" cy="12.2" r="1.2" fill="currentColor" stroke="none" />
      <path d="M9.8 15.2H14.2" opacity="0.55" />
    </template>

    <!-- Card: Memory Note -->
    <template v-else-if="name === 'memory-note'">
      <path d="M6 3.8H16.5C17.6 3.8 18.5 4.7 18.5 5.8V18.2C18.5 19.3 17.6 20.2 16.5 20.2H6C4.9 20.2 4 19.3 4 18.2V5.8C4 4.7 4.9 3.8 6 3.8Z" />
      <path d="M13.2 3.8V10.4L15.6 8.6L18 10.4V3.8" opacity="0.55" />
      <path d="M7.5 12.8H13.5" />
      <path d="M7.5 16H11.5" opacity="0.55" />
    </template>

    <!-- Card: Clock / Scheduled Task -->
    <template v-else-if="name === 'clock-task'">
      <circle cx="12" cy="12.6" r="7.6" />
      <path d="M12 8.4V12.6L14.8 14.4" />
      <path d="M5.2 4.8L8.2 2.8" opacity="0.55" />
      <path d="M18.8 4.8L15.8 2.8" opacity="0.55" />
    </template>

    <!-- Card / Action: AI Sparkles -->
    <template v-else-if="name === 'ai-spark'">
      <path d="M11.5 3.5L13.1 8.4L18 10L13.1 11.6L11.5 16.5L9.9 11.6L5 10L9.9 8.4L11.5 3.5Z" />
      <path d="M18.2 15.2L19 17.5L21.2 18.2L19 19L18.2 21.2L17.5 19L15.2 18.2L17.5 17.5L18.2 15.2Z" opacity="0.55" />
    </template>

    <!-- Card: Pet Figure -->
    <template v-else-if="name === 'pet-figure'">
      <path d="M6.4 10.6L5.4 5L9.8 7.6C10.5 7.4 11.2 7.2 12 7.2C12.8 7.2 13.5 7.4 14.2 7.6L18.6 5L17.6 10.6C18.5 11.8 19 13.2 19 14.8C19 18.2 15.9 20.2 12 20.2C8.1 20.2 5 18.2 5 14.8C5 13.2 5.5 11.8 6.4 10.6Z" />
      <circle cx="9.5" cy="13.8" r="1" fill="currentColor" stroke="none" />
      <circle cx="14.5" cy="13.8" r="1" fill="currentColor" stroke="none" />
      <path d="M10.8 16.4C11.5 17 12.5 17 13.2 16.4" opacity="0.6" />
    </template>

    <!-- Card: Pixel Grid -->
    <template v-else-if="name === 'pixel-grid'">
      <rect x="4" y="4" width="6.5" height="6.5" rx="1.4" />
      <rect x="13.5" y="4" width="6.5" height="6.5" rx="1.4" opacity="0.5" />
      <rect x="4" y="13.5" width="6.5" height="6.5" rx="1.4" opacity="0.5" />
      <rect x="13.5" y="13.5" width="6.5" height="6.5" rx="1.4" />
    </template>

    <!-- Card: Typography / Font Color -->
    <template v-else-if="name === 'type-color'">
      <path d="M4.8 16.5L9.2 5.2L13.6 16.5" />
      <path d="M6.3 12.8H12.1" />
      <path d="M15.8 10.2C17.5 10.2 18.8 11 18.8 12.6V16.5" />
      <path d="M18.8 13.2H16.4C15.2 13.2 14.4 13.9 14.4 14.9C14.4 15.9 15.2 16.5 16.4 16.5H18.8" />
      <path d="M4.5 19.8H19.5" opacity="0.45" />
    </template>

    <!-- Card: Scene Background -->
    <template v-else-if="name === 'scene-bg'">
      <rect x="3.6" y="4.6" width="16.8" height="14.8" rx="3" />
      <circle cx="8.6" cy="9.4" r="1.5" opacity="0.6" />
      <path d="M4.2 16.8L9.4 12.2L13.2 15.6L16.2 13L19.8 16.4" />
    </template>

    <!-- Card: Voice Mixer / Sliders -->
    <template v-else-if="name === 'voice-mixer'">
      <path d="M6 4.5V19.5" opacity="0.45" />
      <path d="M12 4.5V19.5" opacity="0.45" />
      <path d="M18 4.5V19.5" opacity="0.45" />
      <circle cx="6" cy="9.5" r="2.2" fill="currentColor" />
      <circle cx="12" cy="14.5" r="2.2" fill="currentColor" />
      <circle cx="18" cy="8" r="2.2" fill="currentColor" />
    </template>

    <!-- Card: System Prompt Terminal -->
    <template v-else-if="name === 'prompt-terminal'">
      <rect x="3.6" y="4.4" width="16.8" height="15.2" rx="3" />
      <path d="M7.5 9.6L10.2 12L7.5 14.4" />
      <path d="M11.8 14.4H16.2" opacity="0.6" />
    </template>

    <!-- Card: Skills / Modular Layers -->
    <template v-else-if="name === 'skill-puzzle'">
      <path d="M12 3.6L19.8 8L12 12.4L4.2 8L12 3.6Z" />
      <path d="M4.2 12.2L12 16.6L19.8 12.2" opacity="0.65" />
      <path d="M4.2 16.2L12 20.4L19.8 16.2" opacity="0.38" />
    </template>

    <!-- Card: Weather Sun & Cloud -->
    <template v-else-if="name === 'weather-sun'">
      <path d="M8.2 18.5H16.5C18.7 18.5 20.4 16.8 20.4 14.7C20.4 12.7 18.9 11.1 17 10.9C16.4 7.9 13.8 5.8 10.7 5.8C7.2 5.8 4.4 8.6 4.4 12.1C4.4 12.4 4.4 12.7 4.5 13C3.5 13.6 2.8 14.7 2.8 16C2.8 17.4 4 18.5 5.5 18.5H8.2Z" />
      <path d="M17.2 3.8L18.4 5" opacity="0.5" />
      <path d="M20.8 7.2L19.2 7.6" opacity="0.5" />
    </template>

    <!-- Card: Persona / Heart Spark -->
    <template v-else-if="name === 'persona-heart'">
      <path d="M12 19.6C12 19.6 4.5 14.8 4.5 9.3C4.5 6.8 6.5 4.8 9 4.8C10.5 4.8 11.8 5.6 12 6.8C12.2 5.6 13.5 4.8 15 4.8C17.5 4.8 19.5 6.8 19.5 9.3C19.5 14.8 12 19.6 12 19.6Z" />
      <path d="M12 9.2L12.7 11L14.5 11.7L12.7 12.4L12 14.2L11.3 12.4L9.5 11.7L11.3 11Z" fill="currentColor" stroke="none" opacity="0.45" />
    </template>

    <!-- Card: Role / Compass -->
    <template v-else-if="name === 'role-compass'">
      <circle cx="12" cy="12" r="8.2" />
      <polygon points="14.8,9.2 13.2,13.2 9.2,14.8 10.8,10.8" />
    </template>

    <!-- Background Option: None / Transparent -->
    <template v-else-if="name === 'bg-none'">
      <circle cx="12" cy="12" r="7.8" opacity="0.65" />
      <path d="M6.6 17.4L17.4 6.6" />
    </template>

    <!-- Background Option: Cute (Paw & Heart) -->
    <template v-else-if="name === 'bg-cute'">
      <ellipse cx="12" cy="15.2" rx="3.8" ry="3" />
      <circle cx="7.2" cy="10.5" r="1.5" />
      <circle cx="10.3" cy="8.2" r="1.5" />
      <circle cx="13.7" cy="8.2" r="1.5" />
      <circle cx="16.8" cy="10.5" r="1.5" />
    </template>

    <!-- Background Option: Sci-Fi (Ringed Planet) -->
    <template v-else-if="name === 'bg-scifi'">
      <circle cx="12" cy="12" r="4.8" />
      <path d="M4.2 15.2C2.8 16 2.9 16.9 5.2 16.8C8.6 16.6 14.2 14.3 18.2 11.5C20.8 9.7 21.5 8.2 20.1 7.9C19.3 7.7 18 8.1 16.4 8.8" />
      <circle cx="18.5" cy="5" r="1" fill="currentColor" stroke="none" opacity="0.55" />
    </template>

    <!-- Background Option: Minimal (Concentric Zen Rings) -->
    <template v-else-if="name === 'bg-minimal'">
      <circle cx="12" cy="12" r="7.6" />
      <circle cx="12" cy="12" r="3.4" opacity="0.45" />
    </template>

    <!-- Background Option: Upload Image -->
    <template v-else-if="name === 'bg-upload'">
      <path d="M19.5 13.5V17.5C19.5 18.6 18.6 19.5 17.5 19.5H6.5C5.4 19.5 4.5 18.6 4.5 17.5V6.5C4.5 5.4 5.4 4.5 6.5 4.5H10.5" />
      <path d="M16.5 3.8V9.8" />
      <path d="M13.5 6.8H19.5" />
      <path d="M5.2 16.8L9.6 12.6L12.8 15.6L15.4 13.2L18.8 16.5" opacity="0.55" />
    </template>

    <!-- Common Controls: Play -->
    <template v-else-if="name === 'play'">
      <polygon points="7.5,5.2 18.5,12 7.5,18.8" fill="currentColor" stroke="currentColor" stroke-width="1.5" />
    </template>

    <!-- Common Controls: Pause -->
    <template v-else-if="name === 'pause'">
      <rect x="6.8" y="5.5" width="3.4" height="13" rx="1" fill="currentColor" />
      <rect x="13.8" y="5.5" width="3.4" height="13" rx="1" fill="currentColor" />
    </template>

    <!-- Common Controls: Stop -->
    <template v-else-if="name === 'stop'">
      <rect x="6.5" y="6.5" width="11" height="11" rx="2" fill="currentColor" />
    </template>

    <!-- Common Controls: Trash -->
    <template v-else-if="name === 'trash'">
      <path d="M4.5 6.8H19.5" />
      <path d="M9.2 6.8V4.8C9.2 4.1 9.8 3.5 10.5 3.5H13.5C14.2 3.5 14.8 4.1 14.8 4.8V6.8" />
      <path d="M6.5 6.8L7.3 18.6C7.4 19.6 8.2 20.4 9.2 20.4H14.8C15.8 20.4 16.6 19.6 16.7 18.6L17.5 6.8" />
      <path d="M10.2 10.8V16.2" opacity="0.55" />
      <path d="M13.8 10.8V16.2" opacity="0.55" />
    </template>

    <!-- Common Controls: Plus -->
    <template v-else-if="name === 'plus'">
      <path d="M12 5V19" />
      <path d="M5 12H19" />
    </template>

    <!-- Common Controls: Close / X -->
    <template v-else-if="name === 'close'">
      <path d="M6.5 6.5L17.5 17.5" />
      <path d="M17.5 6.5L6.5 17.5" />
    </template>

    <!-- Common Controls: Check -->
    <template v-else-if="name === 'check'">
      <path d="M5 12.5L9.6 17L19 7.5" />
    </template>

    <!-- Common Controls: Refresh -->
    <template v-else-if="name === 'refresh'">
      <path d="M19.5 5.5V10.2H14.8" />
      <path d="M19.2 14.2C18.2 17.4 15.2 19.6 11.7 19.6C7.3 19.6 3.8 16.1 3.8 11.7C3.8 7.3 7.3 3.8 11.7 3.8C14.8 3.8 17.5 5.6 18.8 8.2L19.5 10.2" />
    </template>

    <!-- Common Controls: Search -->
    <template v-else-if="name === 'search'">
      <circle cx="10.8" cy="10.8" r="6.4" />
      <path d="M15.5 15.5L20 20" />
    </template>

    <!-- Common Controls: Clipboard -->
    <template v-else-if="name === 'clipboard'">
      <path d="M9 4.5H7C5.9 4.5 5 5.4 5 6.5V18.5C5 19.6 5.9 20.5 7 20.5H17C18.1 20.5 19 19.6 19 18.5V6.5C19 5.4 18.1 4.5 17 4.5H15" />
      <rect x="9" y="3" width="6" height="3.4" rx="1.4" />
      <path d="M8.8 11.2H15.2" opacity="0.55" />
      <path d="M8.8 14.8H13.2" opacity="0.55" />
    </template>

    <!-- Common Controls: Pin -->
    <template v-else-if="name === 'pin'">
      <path d="M14.5 4.2L19.8 9.5L18.2 11.1L15.2 10.2L11.6 13.8L12.6 17.2L11 18.8L7.8 15.6L4.2 19.2L7.8 15.6L4.6 12.4L6.2 10.8L9.6 11.8L13.2 8.2L12.3 5.2L14.5 4.2Z" />
    </template>

    <!-- Common Controls: Copy -->
    <template v-else-if="name === 'copy'">
      <rect x="8.8" y="8.8" width="10.8" height="10.8" rx="2.2" />
      <path d="M15.2 8.8V6.2C15.2 5.1 14.3 4.2 13.2 4.2H6.2C5.1 4.2 4.2 5.1 4.2 6.2V13.2C4.2 14.3 5.1 15.2 6.2 15.2H8.8" opacity="0.6" />
    </template>

    <!-- Drag & Drop Overlay: Folder Plus -->
    <template v-else-if="name === 'folder-drop'">
      <path d="M3.5 7.5C3.5 6.1 4.6 5 6 5H9.6L11.8 7.4H18C19.4 7.4 20.5 8.5 20.5 9.9V17.2C20.5 18.6 19.4 19.7 18 19.7H6C4.6 19.7 3.5 18.6 3.5 17.2V7.5Z" />
      <path d="M12 10.8V16.4" />
      <path d="M9.2 13.6H14.8" />
    </template>

    <!-- File Attachment: Generic Document -->
    <template v-else-if="name === 'file-doc'">
      <path d="M14 3.8H7.2C6 3.8 5 4.8 5 6V18C5 19.2 6 20.2 7.2 20.2H16.8C18 20.2 19 19.2 19 18V8.8L14 3.8Z" />
      <path d="M13.8 3.8V9H19" opacity="0.55" />
      <path d="M8.5 13H15.5" opacity="0.55" />
      <path d="M8.5 16.2H13" opacity="0.55" />
    </template>

    <!-- Paperclip -->
    <template v-else-if="name === 'paperclip'">
      <path d="M18.4 10.6L10.8 18.2C8.9 20.1 5.8 20.1 3.9 18.2C2 16.3 2 13.2 3.9 11.3L12.1 3.1C13.4 1.8 15.5 1.8 16.8 3.1C18.1 4.4 18.1 6.5 16.8 7.8L9.2 15.4C8.5 16.1 7.5 16.1 6.8 15.4C6.1 14.7 6.1 13.7 6.8 13L13.8 6" />
    </template>

    <!-- Shield Bolt (Tool Confirm) -->
    <template v-else-if="name === 'shield-bolt'">
      <path d="M12 3.2L19 6.2V11.6C19 16.1 15.9 19.7 12 20.8C8.1 19.7 5 16.1 5 11.6V6.2L12 3.2Z" />
      <path d="M12.6 8.2L9.8 12.4H12.4L11.4 15.8L14.2 11.6H11.6L12.6 8.2Z" />
    </template>

    <!-- Sliders / Settings -->
    <template v-else-if="name === 'sliders'">
      <path d="M4 7H10" />
      <path d="M14 7H20" />
      <circle cx="12" cy="7" r="2.2" />
      <path d="M4 17H8" />
      <path d="M12 17H20" />
      <circle cx="10" cy="17" r="2.2" />
    </template>

    <!-- Chevron Down -->
    <template v-else-if="name === 'chevron-down'">
      <path d="M6.5 9.5L12 15L17.5 9.5" />
    </template>

    <!-- Alert Circle -->
    <template v-else-if="name === 'alert'">
      <circle cx="12" cy="12" r="8.2" />
      <path d="M12 8.2V12.6" />
      <circle cx="12" cy="15.8" r="0.9" fill="currentColor" stroke="none" />
    </template>

    <!-- Diagnostic Pulse / Flask (Deep Compatibility Test) -->
    <template v-else-if="name === 'activity-flask'">
      <path d="M3.5 12H7.2L9.6 6.5L13.8 17.5L16.4 12H20.5" />
    </template>

    <!-- Fallback Minimalist Diamond -->
    <template v-else>
      <rect x="6" y="6" width="12" height="12" rx="3" transform="rotate(45 12 12)" />
    </template>
  </svg>
</template>

<style scoped>
.app-icon {
  display: inline-block;
  vertical-align: middle;
  flex-shrink: 0;
}
</style>
