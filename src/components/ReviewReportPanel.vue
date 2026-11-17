<script setup lang="ts">
import type { ReviewReport } from "../services/orchestration";

defineProps<{ report: ReviewReport }>();
const verdictLabel = { pass: "通过", rework: "打回返工", blocked: "受阻" } as const;
const statusLabel = { pass: "通过", fail: "不合格", unverified: "待核实" } as const;
</script>

<template>
  <section class="review-report">
    <header>
      <strong>审核报告</strong>
      <span :class="['review-verdict', report.verdict]">{{ verdictLabel[report.verdict] }}</span>
      <span class="review-meta">需求 v{{ report.task_revision }} · 产物 {{ report.artifact_hash.slice(0, 8) }}</span>
    </header>
    <ul class="review-checks">
      <li v-for="check in report.checks" :key="check.criterion_id">
        <span :class="['review-check-status', check.status]">{{ statusLabel[check.status] }}</span>
        <code>{{ check.criterion_id }}</code>
        <span class="review-explain">{{ check.explanation }}</span>
      </li>
    </ul>
    <ul v-if="report.issues.length" class="review-issues">
      <li v-for="issue in report.issues" :key="issue.id" :class="issue.severity">
        <header>
          <span class="issue-severity">{{ issue.severity }}</span>
          <code>{{ issue.criterion_id }}</code>
          <span class="issue-node">→ {{ issue.responsible_node_id }}</span>
        </header>
        <p>{{ issue.description }}</p>
        <p class="issue-fix">修复：{{ issue.required_fix }}；复测：{{ issue.verification }}</p>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.review-report { border: 1px solid #e3e6ea; border-radius: 6px; padding: 8px 10px; margin: 6px 0; font-size: 12px; }
.review-report header { display: flex; gap: 8px; align-items: center; }
.review-verdict { font-size: 10px; border-radius: 3px; padding: 1px 6px; }
.review-verdict.pass { background: rgba(33, 132, 90, .12); color: #21845a; }
.review-verdict.rework { background: rgba(194, 76, 76, .12); color: #c24c4c; }
.review-verdict.blocked { background: rgba(165, 121, 36, .12); color: #a57924; }
.review-meta { margin-left: auto; color: #8a8f98; font-size: 10px; }
.review-checks { list-style: none; margin: 6px 0; padding: 0; }
.review-checks li { display: flex; gap: 6px; align-items: baseline; margin: 3px 0; }
.review-check-status { font-size: 10px; border-radius: 3px; padding: 0 5px; background: rgba(115, 123, 138, .14); color: #5f6368; }
.review-check-status.pass { background: rgba(33, 132, 90, .12); color: #21845a; }
.review-check-status.fail { background: rgba(194, 76, 76, .12); color: #c24c4c; }
.review-explain { color: #5f6368; }
.review-issues { list-style: none; margin: 6px 0 0; padding: 0; }
.review-issues li { border-left: 2px solid #c24c4c; padding: 4px 8px; margin: 6px 0; background: rgba(194, 76, 76, .05); }
.review-issues li.minor { border-left-color: #a57924; background: rgba(165, 121, 36, .05); }
.review-issues header { display: flex; gap: 6px; align-items: center; }
.issue-severity { font-size: 10px; font-weight: 600; color: #c24c4c; }
.issue-node { color: #8a8f98; font-size: 10px; }
.review-issues p { margin: 3px 0; }
.issue-fix { color: #5f6368; }
</style>