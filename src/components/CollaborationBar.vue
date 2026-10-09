<script setup lang="ts">
import { useChatStore } from "../stores/chat";
const chat = useChatStore();
</script>

<template>
  <div class="collaboration-bar" aria-live="polite">
    <label class="collaboration-switch">
      <input v-model="chat.collaborationEnabled" type="checkbox" :disabled="chat.isLoading" />
      智能体协同
    </label>
    <span v-if="!chat.collaboration">@claude / @codex / @dsh 可随时召唤；开启后允许角色互相 @ 交接。</span>
    <template v-else>
      <strong v-if="chat.collaboration.status === 'running'">{{ chat.collaboration.active_agent?.name || '准备中' }} · {{ chat.collaboration.turn }}/{{ chat.collaboration.max_turns }}</strong>
      <span>{{ chat.collaboration.message }}</span>
      <span v-if="chat.collaboration.queued.length">等待：{{ chat.collaboration.queued.map(agent => '@' + agent.name).join(' → ') }}</span>
    </template>
  </div>
</template>

<style scoped>
.collaboration-bar { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 12px; padding: 10px 12px; border: 1px solid rgba(24,42,72,.1); border-radius: 10px; background: rgba(255,255,255,.6); color: #64748b; font-size: 12px; line-height: 1.5; }
.collaboration-switch { display: inline-flex; align-items: center; gap: 6px; color: #334155; white-space: nowrap; cursor: pointer; }
.collaboration-bar strong { color: #334155; font-weight: 600; }
.collaboration-switch input { accent-color: var(--pet-primary, #38bdf8); }
</style>
