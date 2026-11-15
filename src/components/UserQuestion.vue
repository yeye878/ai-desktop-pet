<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { createInteractionSubmission, type AskUserPayload } from "../services/pendingInteractions";

const props = defineProps<{ question: AskUserPayload; compact?: boolean }>();
const emit = defineEmits<{ answered: [id: string] }>();
const selected = ref("");
const answer = ref("");
const request = ref<AskUserPayload | null>(props.question);
const { submittingId, error, submit } = createInteractionSubmission(request, (id, value) => invoke("answer_question", { id, answer: value }));
const submitting = computed(() => submittingId.value === props.question.id);
watch(() => props.question.id, () => {
  request.value = props.question;
  selected.value = "";
  answer.value = "";
  error.value = "";
});
async function send(skip = false) {
  const value = skip ? "" : answer.value.trim() || selected.value;
  if (!skip && !value) return;
  const id = await submit(value);
  if (id && props.question.id === id) emit("answered", id);
}
</script>

<template>
  <section :class="['user-question', { compact }]" role="dialog" aria-modal="true" aria-labelledby="user-question-title">
    <header><span>需要你的回答</span><h2 id="user-question-title">{{ question.question }}</h2></header>
    <div class="question-body">
      <fieldset v-if="question.options.length" :disabled="submitting">
        <legend class="question-options-label">选择答案</legend>
        <label v-for="(option, index) in question.options" :key="index" :class="['question-option', { selected: selected === option.label && !answer.trim() }]">
          <input v-model="selected" type="radio" :name="`question-${question.id}`" :value="option.label" @change="answer = ''" />
          <span><strong>{{ option.label }}</strong><small v-if="option.description">{{ option.description }}</small></span>
        </label>
      </fieldset>
      <label class="question-answer"><span>你的回答</span><textarea v-model="answer" :disabled="submitting" rows="2" placeholder="填写答案" /></label>
      <p v-if="error" class="question-error" role="alert">{{ error }}</p>
    </div>
    <footer>
      <button type="button" :disabled="submitting" @click="send(true)">跳过</button>
      <button type="button" class="question-submit" :disabled="submitting || (!answer.trim() && !selected)" @click="send()">{{ submitting ? "正在提交" : "提交回答" }}</button>
    </footer>
  </section>
</template>

<style scoped>
.user-question { display: flex; flex-direction: column; width: 460px; max-width: 100%; max-height: 100%; min-height: 0; overflow: hidden; border: 1px solid #d8dde3; border-radius: 8px; background: #fff; color: #303741; font: 13px/1.5 "Segoe UI", system-ui, sans-serif; box-shadow: 0 8px 30px #0002; letter-spacing: 0; }
header { padding: 14px 16px 10px; flex-shrink: 0; }
header > span, .question-answer > span, legend { font-size: 11px; color: #65707b; }
h2 { font-size: 15px; line-height: 1.5; margin: 4px 0 0; overflow-wrap: anywhere; }
.question-body { overflow-y: auto; min-height: 0; padding: 0 16px 12px; }
fieldset { border: 0; padding: 0; margin: 0 0 12px; }
legend { padding: 0 0 5px; }
.question-option { display: flex; align-items: flex-start; gap: 8px; padding: 7px 8px; cursor: pointer; border: 1px solid #e0e4e9; border-radius: 4px; margin-bottom: 5px; }
.question-option.selected { border-color: #4b8fb1; background: #f0f8fb; }
.question-option input { margin: 3px 0 0; flex-shrink: 0; accent-color: #267ea4; }
.question-option span { min-width: 0; overflow-wrap: anywhere; }
.question-option strong { font-size: 12px; font-weight: 500; }
.question-option small { display: block; font-size: 11px; color: #697581; }
.question-answer { display: grid; gap: 4px; }
textarea { resize: vertical; min-height: 48px; width: 100%; box-sizing: border-box; font: inherit; padding: 7px 8px; color: inherit; border: 1px solid #d7dde4; border-radius: 4px; }
.question-error { color: #b84040; margin: 7px 0 0; overflow-wrap: anywhere; }
footer { padding: 10px 16px 12px; border-top: 1px solid #e5e8ec; display: flex; gap: 8px; justify-content: flex-end; flex-shrink: 0; }
button { height: 34px; border: 1px solid #d5dbe1; border-radius: 4px; padding: 0 14px; cursor: pointer; background: #fff; color: inherit; font: inherit; }
.question-submit { background: #247f9f; border-color: #247f9f; color: #fff; }
button:disabled { opacity: .5; cursor: default; }
.compact header { padding: 10px 12px 7px; }
.compact .question-body { padding: 0 12px 8px; }
.compact footer { padding: 8px 12px; }
.compact h2 { font-size: 13px; }
input:focus-visible, textarea:focus-visible, button:focus-visible { outline: 2px solid #338fb9; outline-offset: 2px; }
</style>
