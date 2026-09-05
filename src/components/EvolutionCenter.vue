<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { useEvolutionStore, type EvolutionInsight, type EvolutionProposal, type EvolutionLog } from "../stores/evolution";
import AppIcon from "./AppIcon.vue";

const evolutionStore = useEvolutionStore();

const activeTab = ref<"proposals" | "insights" | "logs">("proposals");
const insightCategoryFilter = ref<string>("all");
const toastMessage = ref("");
const toastType = ref<"success" | "error">("success");
let toastTimer: ReturnType<typeof setTimeout> | null = null;

// 手动新增准则模态框
const newInsightModalOpen = ref(false);
const newInsightDraft = ref({
  category: "preference",
  content: "",
  confidence: 0.9,
});

const insightCategories = [
  { value: "all", label: "全部" },
  { value: "preference", label: "偏好习惯" },
  { value: "work_domain", label: "工作领域" },
  { value: "coding_style", label: "代码风格" },
  { value: "negative_constraint", label: "负面约束与避坑" },
  { value: "habit", label: "日常作息" },
];

const categoryLabels: Record<string, { label: string; color: string; icon: string }> = {
  preference: { label: "偏好习惯", color: "#3b82f6", icon: "sparkles" },
  work_domain: { label: "工作领域", color: "#10b981", icon: "folder" },
  coding_style: { label: "代码风格", color: "#8b5cf6", icon: "terminal" },
  negative_constraint: { label: "负面约束", color: "#ef4444", icon: "info" },
  habit: { label: "日常作息", color: "#f59e0b", icon: "clock" },
};

const proposalTypeLabels: Record<string, { label: string; icon: string }> = {
  create_agent: { label: "特化智能体", icon: "agents" },
  create_skill: { label: "专属技能", icon: "zap" },
  schedule_task: { label: "自动化定时任务", icon: "calendar" },
  update_guideline: { label: "协作工作准则", icon: "edit" },
};

const logCategoryLabels: Record<string, string> = {
  reflection_run: "深度复盘",
  create_agent: "智能体衍生",
  create_skill: "技能衍生",
  schedule_task: "定时任务创建",
  update_guideline: "准则更新",
  rollback: "回滚撤销",
};

// 编辑准则模态框
const editInsightModalOpen = ref(false);
const editingInsightDraft = ref<EvolutionInsight | null>(null);

function openEditInsightModal(insight: EvolutionInsight) {
  editingInsightDraft.value = { ...insight };
  editInsightModalOpen.value = true;
}

async function handleSaveEditedInsight() {
  if (!editingInsightDraft.value) return;
  const content = editingInsightDraft.value.content.trim();
  if (!content) {
    showToast("准则内容不能为空", "error");
    return;
  }
  try {
    await evolutionStore.updateInsight({
      ...editingInsightDraft.value,
      content,
    });
    editInsightModalOpen.value = false;
    editingInsightDraft.value = null;
    showToast("已成功更新认知准则！", "success");
  } catch (err) {
    showToast(`更新失败: ${err}`, "error");
  }
}

async function handleToggleAutoEvolution() {
  const next = !evolutionStore.autoEvolutionEnabled;
  try {
    await evolutionStore.setAutoEvolutionEnabled(next);
    showToast(next ? "已开启后台静默自进化" : "已关闭后台静默自进化", "success");
  } catch (err) {
    showToast(`设置失败: ${err}`, "error");
  }
}

const filteredInsights = computed(() => {
  if (insightCategoryFilter.value === "all") {
    return evolutionStore.insights;
  }
  return evolutionStore.insights.filter((i) => i.category === insightCategoryFilter.value);
});

function showToast(msg: string, type: "success" | "error" = "success") {
  toastMessage.value = msg;
  toastType.value = type;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toastMessage.value = "";
  }, 3500);
}

async function handleTriggerReflection() {
  try {
    const res = await evolutionStore.triggerReflection();
    showToast(res.message, "success");
    if (evolutionStore.pendingProposals.length > 0) {
      activeTab.value = "proposals";
    }
  } catch (err) {
    showToast(`复盘失败: ${err}`, "error");
  }
}

async function handleApplyProposal(proposal: EvolutionProposal) {
  try {
    await evolutionStore.applyProposal(proposal.id);
    const extra = proposal.proposal_type === "create_skill"
      ? "技能已创建，请在「技能」页手动激活后才会生效。"
      : "";
    showToast(`已成功采纳并应用进化提案：「${proposal.title}」！${extra}`, "success");
  } catch (err) {
    showToast(`应用提案失败: ${err}`, "error");
  }
}

async function handleRejectProposal(proposal: EvolutionProposal) {
  try {
    await evolutionStore.rejectProposal(proposal.id);
    showToast(`已忽略提案：「${proposal.title}」`, "success");
  } catch (err) {
    showToast(`操作失败: ${err}`, "error");
  }
}

async function handleToggleInsight(insight: EvolutionInsight) {
  try {
    await evolutionStore.toggleInsightStatus(insight);
    showToast(
      insight.status === "active" ? "已重新启用该条准则" : "已归档该条准则（暂不生效）",
      "success",
    );
  } catch (err) {
    showToast(`更新准则状态失败: ${err}`, "error");
  }
}

async function handleDeleteInsight(insight: EvolutionInsight) {
  if (!window.confirm(`确定删除此条认知「${insight.content}」吗？`)) return;
  try {
    await evolutionStore.deleteInsight(insight.id);
    showToast("已删除该认知准则", "success");
  } catch (err) {
    showToast(`删除失败: ${err}`, "error");
  }
}
async function handleCreateInsight() {
  const content = newInsightDraft.value.content.trim();
  if (!content) {
    showToast("请输入准则内容", "error");
    return;
  }
  try {
    await evolutionStore.saveInsight({
      category: newInsightDraft.value.category,
      content,
      confidence: newInsightDraft.value.confidence,
    });
    newInsightDraft.value.content = "";
    newInsightModalOpen.value = false;
    showToast("已成功添加自进化认知准则！", "success");
  } catch (err) {
    showToast(`保存失败: ${err}`, "error");
  }
}

function openAddInsightModal() {
  newInsightDraft.value = {
    category: insightCategoryFilter.value === "all" ? "preference" : insightCategoryFilter.value,
    content: "",
    confidence: 0.9,
  };
  newInsightModalOpen.value = true;
}

async function handleRollback(log: EvolutionLog) {
  if (!window.confirm(`确定回滚操作「${log.summary}」吗？将撤销该进化所做的修改。`)) return;
  try {
    await evolutionStore.rollbackLog(log.id);
    showToast("已成功回滚该进化版本", "success");
  } catch (err) {
    showToast(`回滚失败: ${err}`, "error");
  }
}

function formatTimestamp(ts: number): string {
  if (!ts) return "";
  const d = new Date(ts * 1000);
  return d.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function parsePayload(jsonStr: string): any {
  try {
    return JSON.parse(jsonStr);
  } catch {
    return {};
  }
}

onMounted(() => {
  void evolutionStore.loadAll();
});
</script>

<template>
  <div class="evolution-center">
    <!-- 顶部成长与默契总览卡片 -->
    <div class="evo-hero-card">
      <div class="evo-hero-content">
        <div class="evo-level-badge">
          <span class="evo-dna-icon"><AppIcon name="evolution" :size="26" /></span>
          <div class="evo-level-info">
            <div class="evo-level-title">自进化成长等级 Lv.{{ evolutionStore.summary.level }}</div>
            <div class="evo-level-subtitle">
              {{ evolutionStore.summary.level >= 5 ? "灵犀知己" : evolutionStore.summary.level >= 3 ? "默契搭档" : "探索初生" }} · 默契度指数 {{ evolutionStore.summary.sync_score }}%
            </div>
          </div>
        </div>

        <div class="evo-stats-row">
          <div class="evo-stat-item">
            <span class="evo-stat-num">{{ evolutionStore.summary.active_insights_count }}</span>
            <span class="evo-stat-label">已习得认知</span>
          </div>
          <div class="evo-stat-item highlight">
            <span class="evo-stat-num">{{ evolutionStore.summary.pending_proposals_count }}</span>
            <span class="evo-stat-label">待审批提案</span>
          </div>
          <div class="evo-stat-item">
            <span class="evo-stat-num">{{ evolutionStore.summary.total_evolutions_count }}</span>
            <span class="evo-stat-label">进化记录</span>
          </div>
        </div>
      </div>

      <div class="evo-hero-actions">
        <button
          type="button"
          class="evo-reflect-btn"
          :disabled="evolutionStore.isReflecting"
          @click="handleTriggerReflection"
        >
          <span v-if="evolutionStore.isReflecting" class="evo-spinner"></span>
          <span v-else class="evo-sparkle"><AppIcon name="sparkles" :size="14" /></span>
          <span>{{ evolutionStore.isReflecting ? "正在深度复盘与自我进化..." : "立即深度复盘与进化" }}</span>
        </button>
        <div class="evo-auto-toggle-box">
          <label class="evo-auto-toggle-label" title="开启后，桌宠将在检测到纠错或每 15 轮对话后在后台静默复盘进化">
            <input
              type="checkbox"
              :checked="evolutionStore.autoEvolutionEnabled"
              @change="handleToggleAutoEvolution"
            />
            <span>后台静默自进化</span>
          </label>
        </div>
        <div class="evo-reflect-tip">分析近期对话历史，主动提炼习惯、生成专属特化智能体与工作流</div>
      </div>
    </div>

    <!-- 提示消息 Toast -->
    <Transition name="fade">
      <div v-if="toastMessage" :class="['evo-toast', toastType]">
        <AppIcon :name="toastType === 'success' ? 'check' : 'info'" :size="15" />
        <span>{{ toastMessage }}</span>
      </div>
    </Transition>

    <!-- 导航选项卡 -->
    <div class="evo-nav-tabs">
      <button
        type="button"
        :class="['evo-tab-btn', { active: activeTab === 'proposals' }]"
        @click="activeTab = 'proposals'"
      >
        <span class="evo-tab-label"><AppIcon name="zap" :size="13" />进化提案</span>
        <span v-if="evolutionStore.pendingProposals.length > 0" class="evo-tab-badge">
          {{ evolutionStore.pendingProposals.length }}
        </span>
      </button>
      <button
        type="button"
        :class="['evo-tab-btn', { active: activeTab === 'insights' }]"
        @click="activeTab = 'insights'"
      >
        <span class="evo-tab-label"><AppIcon name="memory" :size="13" />画像准则库</span>
        <span class="evo-tab-count">({{ evolutionStore.activeInsights.length }})</span>
      </button>
      <button
        type="button"
        :class="['evo-tab-btn', { active: activeTab === 'logs' }]"
        @click="activeTab = 'logs'"
      >
        <span class="evo-tab-label"><AppIcon name="clock" :size="13" />进化编年史</span>
      </button>
    </div>

    <!-- Tab 1: 进化提案流 -->
    <div v-if="activeTab === 'proposals'" class="evo-tab-pane">
      <div v-if="evolutionStore.pendingProposals.length === 0" class="evo-empty-state">
        <div class="evo-empty-icon"><AppIcon name="sparkles" :size="36" /></div>
        <div class="evo-empty-title">当前没有待处理的进化提案</div>
        <div class="evo-empty-desc">
          桌宠目前与你的配合十分顺畅。随着更多日常交互进行，桌宠会自动为你构思专属特化智能体或定时自动化提醒。
        </div>
        <button type="button" class="evo-btn-ghost" @click="handleTriggerReflection">
          <AppIcon name="sparkles" :size="13" />
          <span>立即触发一次复盘</span>
        </button>
      </div>

      <div v-else class="evo-proposals-grid">
        <div
          v-for="prop in evolutionStore.pendingProposals"
          :key="prop.id"
          class="evo-proposal-card"
        >
          <div class="evo-prop-header">
            <div class="evo-prop-tag">
              <AppIcon :name="proposalTypeLabels[prop.proposal_type]?.icon || 'sparkles'" :size="11" />
              <span>{{ proposalTypeLabels[prop.proposal_type]?.label || '进化提案' }}</span>
            </div>
            <span class="evo-prop-date">{{ formatTimestamp(prop.created_at) }}</span>
          </div>

          <div class="evo-prop-title">{{ prop.title }}</div>
          <div class="evo-prop-rationale">{{ prop.rationale }}</div>

          <!-- 载荷详情预览 -->
          <div class="evo-prop-payload-box">
            <template v-if="prop.proposal_type === 'create_agent'">
              <div class="evo-agent-preview">
                <span class="evo-agent-avatar">{{ parsePayload(prop.payload_json).avatar || '🤖' }}</span>
                <div class="evo-agent-meta">
                  <div class="evo-agent-name">{{ parsePayload(prop.payload_json).name }}</div>
                  <div class="evo-agent-desc">{{ parsePayload(prop.payload_json).description }}</div>
                </div>
              </div>
              <div v-if="parsePayload(prop.payload_json).system_prompt" class="evo-prop-prompt-preview">
                <small>设定提示词：</small>
                <p>{{ parsePayload(prop.payload_json).system_prompt }}</p>
              </div>
            </template>

            <template v-else-if="prop.proposal_type === 'create_skill'">
              <div class="evo-skill-preview">
                <div class="evo-skill-name"><AppIcon name="zap" :size="13" />{{ parsePayload(prop.payload_json).name }}</div>
                <div class="evo-skill-desc">{{ parsePayload(prop.payload_json).description }}</div>
                <div v-if="parsePayload(prop.payload_json).system_prompt" class="evo-prop-prompt-preview">
                  <small>技能设定：</small>
                  <p>{{ parsePayload(prop.payload_json).system_prompt }}</p>
                </div>
              </div>
            </template>

            <template v-else-if="prop.proposal_type === 'schedule_task'">
              <div class="evo-task-preview">
                <div class="evo-task-title"><AppIcon name="calendar" :size="13" />{{ parsePayload(prop.payload_json).title }}</div>
                <div class="evo-task-note">{{ parsePayload(prop.payload_json).note }}</div>
              </div>
            </template>

            <template v-else>
              <div class="evo-guideline-preview">
                <AppIcon name="edit" :size="13" />{{ parsePayload(prop.payload_json).content }}
              </div>
            </template>
          </div>

          <div class="evo-prop-actions">
            <button
              type="button"
              class="evo-btn-primary"
              :disabled="evolutionStore.isApplyingId === prop.id"
              @click="handleApplyProposal(prop)"
            >
              <span>{{ evolutionStore.isApplyingId === prop.id ? "正在执行..." : "采纳并应用" }}</span>
            </button>
            <button
              type="button"
              class="evo-btn-secondary"
              @click="handleRejectProposal(prop)"
            >
              忽略
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- Tab 2: 已习得画像与准则库 -->
    <div v-else-if="activeTab === 'insights'" class="evo-tab-pane">
      <div class="evo-filter-bar">
        <div class="evo-category-chips">
          <button
            v-for="cat in insightCategories"
            :key="cat.value"
            type="button"
            :class="['evo-chip', { active: insightCategoryFilter === cat.value }]"
            @click="insightCategoryFilter = cat.value"
          >
            {{ cat.label }}
          </button>
        </div>
        <button type="button" class="evo-add-insight-btn" @click="openAddInsightModal">
          <AppIcon name="plus" :size="12" />
          <span>添加准则</span>
        </button>
      </div>

      <div v-if="filteredInsights.length === 0" class="evo-empty-state">
        <div class="evo-empty-icon"><AppIcon name="memory" :size="36" /></div>
        <div class="evo-empty-title">该分类下暂无已习得准则</div>
        <div class="evo-empty-desc">桌宠会在对话中敏锐捕捉你的编码喜好、回答长度与负面约束，自动在此沉淀。</div>
      </div>

      <div v-else class="evo-insights-list">
        <div
          v-for="insight in filteredInsights"
          :key="insight.id"
          :class="['evo-insight-card', { archived: insight.status !== 'active' }]"
        >
          <div class="evo-insight-header">
            <div
              class="evo-category-badge"
              :style="{
                color: categoryLabels[insight.category]?.color || '#4b5563',
                backgroundColor: (categoryLabels[insight.category]?.color || '#4b5563') + '18',
              }"
            >
              <AppIcon :name="categoryLabels[insight.category]?.icon || 'pin'" :size="10" />
              <span>{{ categoryLabels[insight.category]?.label || insight.category }}</span>
            </div>
            <div class="evo-insight-meta">
              <span class="evo-confidence-tag" title="模型置信度">
                置信度 {{ Math.round(insight.confidence * 100) }}%
              </span>
              <span class="evo-hit-tag" title="交互中累计命中/验证次数">
                命中 {{ insight.hit_count }} 次
              </span>
            </div>
          </div>

          <div class="evo-insight-content">
            {{ insight.content }}
          </div>

          <div class="evo-insight-footer">
            <span class="evo-insight-date">更新于 {{ formatTimestamp(insight.updated_at) }}</span>
            <div class="evo-insight-ctrls">
              <button
                type="button"
                :class="['evo-toggle-btn', { active: insight.status === 'active' }]"
                :title="insight.status === 'active' ? '点击归档此条准则（暂不生效）' : '点击启用此条准则'"
                @click="handleToggleInsight(insight)"
              >
                {{ insight.status === "active" ? "已启用" : "已归档" }}
              </button>
              <button
                type="button"
                class="evo-edit-btn"
                title="编辑此条认知准则"
                @click="openEditInsightModal(insight)"
              >
                <AppIcon name="edit" :size="13" />
              </button>
              <button
                type="button"
                class="evo-delete-btn"
                title="永久删除此条认知"
                @click="handleDeleteInsight(insight)"
              >
                <AppIcon name="trash" :size="13" />
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Tab 3: 进化编年史与版本回退 -->
    <div v-else-if="activeTab === 'logs'" class="evo-tab-pane">
      <div v-if="evolutionStore.logs.length === 0" class="evo-empty-state">
        <div class="evo-empty-icon"><AppIcon name="clock" :size="36" /></div>
        <div class="evo-empty-title">暂无进化历史记录</div>
        <div class="evo-empty-desc">每一次深度复盘、准则吸纳或智能体衍生都会在此留下清晰的足迹与快照。</div>
      </div>

      <div v-else class="evo-logs-timeline">
        <div
          v-for="log in evolutionStore.logs"
          :key="log.id"
          class="evo-log-item"
        >
          <div class="evo-log-dot"></div>
          <div class="evo-log-body">
            <div class="evo-log-head">
              <span class="evo-log-time">{{ formatTimestamp(log.created_at) }}</span>
              <span class="evo-log-tag">{{ logCategoryLabels[log.category] || log.category }}</span>
            </div>
            <div class="evo-log-summary">{{ log.summary }}</div>

            <div class="evo-log-actions">
              <button
                v-if="log.category !== 'rollback' && log.category !== 'reflection_run'"
                type="button"
                class="evo-rollback-btn"
                :disabled="evolutionStore.isRollingBackId === log.id"
                @click="handleRollback(log)"
              >
                <span><AppIcon name="refresh" :size="11" />撤销此进化操作</span>
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 手动新增准则模态框 -->
    <div v-if="newInsightModalOpen" class="evo-modal-overlay" @click.self="newInsightModalOpen = false">
      <div class="evo-modal-card">
        <div class="evo-modal-header">
          <div class="evo-modal-title"><AppIcon name="sparkles" :size="15" />添加自进化协作认知</div>
          <button type="button" class="evo-modal-close" @click="newInsightModalOpen = false"><AppIcon name="x" :size="15" /></button>
        </div>

        <div class="evo-modal-body">
          <div class="evo-form-group">
            <label>认知分类</label>
            <select v-model="newInsightDraft.category" class="evo-select">
              <option value="preference">偏好习惯（如：喜欢简洁干练的回复）</option>
              <option value="work_domain">工作领域（如：前端开发、数据分析）</option>
              <option value="coding_style">代码风格（如：使用 Vue 3 组合式 API）</option>
              <option value="negative_constraint">负面约束与避坑（如：禁止长篇寒暄、不生成冗余注释）</option>
              <option value="habit">日常作息（如：晚间 10 点后提醒休息）</option>
            </select>
          </div>

          <div class="evo-form-group">
            <label>准则内容与约束描述</label>
            <textarea
              v-model="newInsightDraft.content"
              class="evo-textarea"
              rows="4"
              placeholder="例如：在编写 TypeScript 代码时，必须定义完整的类型与接口，避免使用 any。"
            ></textarea>
          </div>
        </div>

        <div class="evo-modal-footer">
          <button type="button" class="evo-btn-secondary" @click="newInsightModalOpen = false">取消</button>
          <button type="button" class="evo-btn-primary" @click="handleCreateInsight">保存认知准则</button>
        </div>
      </div>
    </div>

    <!-- 编辑准则模态框 -->
    <div v-if="editInsightModalOpen && editingInsightDraft" class="evo-modal-overlay" @click.self="editInsightModalOpen = false">
      <div class="evo-modal-card">
        <div class="evo-modal-header">
          <div class="evo-modal-title"><AppIcon name="edit" :size="15" />编辑协作认知准则</div>
          <button type="button" class="evo-modal-close" @click="editInsightModalOpen = false"><AppIcon name="x" :size="15" /></button>
        </div>

        <div class="evo-modal-body">
          <div class="evo-form-group">
            <label>认知分类</label>
            <select v-model="editingInsightDraft.category" class="evo-select">
              <option value="preference">偏好习惯</option>
              <option value="work_domain">工作领域</option>
              <option value="coding_style">代码风格</option>
              <option value="negative_constraint">负面约束与避坑</option>
              <option value="habit">日常作息</option>
            </select>
          </div>

          <div class="evo-form-group">
            <label>准则内容与约束描述</label>
            <textarea
              v-model="editingInsightDraft.content"
              class="evo-textarea"
              rows="4"
              placeholder="描述具体的偏好或约束..."
            ></textarea>
          </div>
        </div>

        <div class="evo-modal-footer">
          <button type="button" class="evo-btn-secondary" @click="editInsightModalOpen = false">取消</button>
          <button type="button" class="evo-btn-primary" @click="handleSaveEditedInsight">保存修改</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.evolution-center {
  display: flex;
  flex-direction: column;
  gap: 16px;
  width: 100%;
  max-width: 1100px;
  margin: 0 auto;
  padding-bottom: 32px;
  font-family: var(--dash-font-sans, -apple-system, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif);
  font-size: 13px;
  line-height: 1.6;
  color: var(--dash-text-primary, #2d2922);
}

.evolution-center button {
  font-family: inherit;
}

/* Hero Card */
.evo-hero-card {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 20px 22px;
  border-radius: var(--dash-radius-xl, 16px);
  background:
    radial-gradient(ellipse at top left, rgba(var(--pet-primary-rgb, 191, 122, 78), 0.07), transparent 60%),
    var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  box-shadow: var(--dash-shadow-sm, 0 1px 3px rgba(48, 42, 34, 0.05));
}

.evo-hero-content {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  flex-wrap: wrap;
}

.evo-level-badge {
  display: flex;
  align-items: center;
  gap: 14px;
}

.evo-dna-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 46px;
  height: 46px;
  border-radius: 13px;
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
  color: var(--dash-accent, #bf7a4e);
  border: 1px solid var(--dash-accent-ring, rgba(191, 122, 78, 0.28));
}

.evo-level-title {
  font-size: 16px;
  font-weight: 750;
  color: var(--dash-text-primary, #2d2922);
  letter-spacing: 0.01em;
}

.evo-level-subtitle {
  font-size: 12px;
  color: var(--dash-text-secondary, #6d6558);
  margin-top: 2px;
}

.evo-stats-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.evo-stat-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 8px 16px;
  border-radius: var(--dash-radius-md, 10px);
  background: var(--dash-panel-soft, #f3f0e9);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
}

.evo-stat-item.highlight {
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
  border-color: var(--dash-accent-ring, rgba(191, 122, 78, 0.28));
}

.evo-stat-num {
  font-family: var(--dash-font-mono, "SFMono-Regular", Consolas, monospace);
  font-size: 18px;
  font-weight: 750;
  color: var(--dash-text-primary, #2d2922);
}

.evo-stat-item.highlight .evo-stat-num {
  color: var(--dash-accent-dark, #a05f3d);
}

.evo-stat-label {
  font-size: 11px;
  color: var(--dash-text-secondary, #6d6558);
  margin-top: 2px;
}

.evo-hero-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
  padding-top: 14px;
  border-top: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.08));
}

.evo-reflect-btn {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 9px 20px;
  border-radius: 10px;
  background: var(--dash-accent, #bf7a4e);
  color: #fffaf4;
  border: none;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  box-shadow: 0 2px 10px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.28);
  transition: all 0.18s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-reflect-btn:hover:not(:disabled) {
  background: var(--dash-accent-dark, #a05f3d);
  transform: translateY(-1px);
}

.evo-reflect-btn:active:not(:disabled) {
  transform: scale(0.98);
}

.evo-reflect-btn:disabled {
  opacity: 0.65;
  cursor: not-allowed;
}

.evo-sparkle {
  display: inline-flex;
  align-items: center;
}

.evo-reflect-tip {
  font-size: 12px;
  color: var(--dash-text-muted, #a29a8a);
}

/* Spinner */
.evo-spinner {
  width: 14px;
  height: 14px;
  border: 2px solid rgba(255, 250, 244, 0.35);
  border-top-color: #fffaf4;
  border-radius: 50%;
  animation: evo-spin 0.8s linear infinite;
}

@keyframes evo-spin {
  to { transform: rotate(360deg); }
}

/* Auto evolution toggle */
.evo-auto-toggle-box {
  display: flex;
  align-items: center;
}

.evo-auto-toggle-label {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-radius: 9px;
  background: var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  font-size: 12px;
  font-weight: 600;
  color: var(--dash-text-primary, #2d2922);
  cursor: pointer;
  user-select: none;
  transition: all 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-auto-toggle-label:hover {
  background: var(--dash-panel-soft, #f3f0e9);
  border-color: var(--dash-accent-ring, rgba(191, 122, 78, 0.28));
}

.evo-auto-toggle-label input[type="checkbox"] {
  appearance: none;
  -webkit-appearance: none;
  width: 36px;
  height: 21px;
  border-radius: 999px;
  background: rgba(63, 54, 44, 0.16);
  position: relative;
  cursor: pointer;
  margin: 0;
  flex-shrink: 0;
  transition: background 0.18s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-auto-toggle-label input[type="checkbox"]::after {
  content: "";
  position: absolute;
  top: 2px;
  left: 2px;
  width: 17px;
  height: 17px;
  border-radius: 50%;
  background: #fffefb;
  box-shadow: 0 1px 3px rgba(48, 42, 34, 0.25);
  transition: transform 0.18s cubic-bezier(0.34, 1.3, 0.64, 1);
}

.evo-auto-toggle-label input[type="checkbox"]:checked {
  background: var(--dash-accent, #bf7a4e);
}

.evo-auto-toggle-label input[type="checkbox"]:checked::after {
  transform: translateX(15px);
}

/* Toast */
.evo-toast {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 16px;
  border-radius: var(--dash-radius-md, 10px);
  font-size: 12.5px;
  font-weight: 500;
  box-shadow: var(--dash-shadow-sm, 0 1px 3px rgba(48, 42, 34, 0.05));
}

.evo-toast.success {
  background: var(--dash-success-soft, rgba(94, 143, 106, 0.12));
  color: var(--dash-success, #5e8f6a);
  border: 1px solid rgba(94, 143, 106, 0.25);
}

.evo-toast.error {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.12));
  color: var(--dash-danger, #c05a4d);
  border: 1px solid rgba(192, 90, 77, 0.25);
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s cubic-bezier(0.22, 1, 0.36, 1), transform 0.2s cubic-bezier(0.22, 1, 0.36, 1);
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}

/* Tabs — 分段控件 */
.evo-nav-tabs {
  display: inline-flex;
  align-items: center;
  align-self: flex-start;
  gap: 2px;
  padding: 3px;
  border-radius: 12px;
  background: var(--dash-panel-sunken, #edeae2);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
}

.evo-tab-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 14px;
  border-radius: 9px;
  border: none;
  background: transparent;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
  cursor: pointer;
  transition: all 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-tab-btn:hover {
  color: var(--dash-text-primary, #2d2922);
}

.evo-tab-btn.active {
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-primary, #2d2922);
  box-shadow: var(--dash-shadow-xs, 0 1px 2px rgba(48, 42, 34, 0.08));
}

.evo-tab-label {
  display: inline-flex;
  align-items: center;
  gap: 5px;
}

.evo-tab-btn.active .evo-tab-label {
  color: var(--dash-accent-dark, #a05f3d);
}

.evo-tab-badge {
  min-width: 17px;
  height: 17px;
  padding: 0 5px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 999px;
  background: var(--dash-accent, #bf7a4e);
  color: #fffaf4;
  font-family: var(--dash-font-mono, "SFMono-Regular", Consolas, monospace);
  font-size: 10.5px;
  font-weight: 700;
}

.evo-tab-count {
  font-family: var(--dash-font-mono, "SFMono-Regular", Consolas, monospace);
  font-size: 11px;
  color: var(--dash-text-muted, #a29a8a);
}

/* Proposals Grid */
.evo-proposals-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: 14px;
}

.evo-proposal-card {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px 18px;
  border-radius: var(--dash-radius-lg, 13px);
  background: var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  box-shadow: var(--dash-shadow-sm, 0 1px 3px rgba(48, 42, 34, 0.05));
  transition: all 0.18s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-proposal-card:hover {
  transform: translateY(-1px);
  box-shadow: var(--dash-shadow-md, 0 4px 14px rgba(48, 42, 34, 0.08));
  border-color: var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
}

.evo-prop-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.evo-prop-tag {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 9px;
  border-radius: 999px;
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
  color: var(--dash-accent-dark, #a05f3d);
  font-size: 11px;
  font-weight: 650;
}

.evo-prop-date {
  font-family: var(--dash-font-mono, "SFMono-Regular", Consolas, monospace);
  font-size: 11px;
  color: var(--dash-text-muted, #a29a8a);
}

.evo-prop-title {
  font-size: 14px;
  font-weight: 700;
  color: var(--dash-text-primary, #2d2922);
}

.evo-prop-rationale {
  font-size: 12.5px;
  color: var(--dash-text-secondary, #6d6558);
  line-height: 1.6;
}

.evo-prop-payload-box {
  padding: 12px;
  border-radius: var(--dash-radius-md, 10px);
  background: var(--dash-panel-soft, #f3f0e9);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
}

.evo-agent-preview {
  display: flex;
  align-items: center;
  gap: 10px;
}

.evo-agent-avatar {
  font-size: 24px;
  line-height: 1;
}

.evo-agent-name {
  font-weight: 650;
  font-size: 13px;
  color: var(--dash-text-primary, #2d2922);
}

.evo-agent-desc {
  font-size: 12px;
  color: var(--dash-text-secondary, #6d6558);
}

.evo-skill-name,
.evo-task-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-weight: 650;
  font-size: 13px;
  color: var(--dash-text-primary, #2d2922);
}

.evo-skill-name .app-icon,
.evo-task-title .app-icon,
.evo-guideline-preview .app-icon {
  color: var(--dash-accent, #bf7a4e);
  flex-shrink: 0;
}

.evo-skill-desc,
.evo-task-note {
  font-size: 12px;
  color: var(--dash-text-secondary, #6d6558);
  margin-top: 4px;
}

.evo-guideline-preview {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  font-size: 12.5px;
  color: var(--dash-text-secondary, #6d6558);
  line-height: 1.6;
}

.evo-guideline-preview .app-icon {
  margin-top: 2px;
}

.evo-prop-prompt-preview {
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px dashed var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  font-size: 11px;
  color: var(--dash-text-secondary, #6d6558);
}

.evo-prop-prompt-preview small {
  font-weight: 600;
  color: var(--dash-text-muted, #a29a8a);
}

.evo-prop-prompt-preview p {
  margin: 2px 0 0;
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.evo-prop-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: auto;
  padding-top: 6px;
}

/* Buttons */
.evo-btn-primary {
  flex: 1;
  padding: 8px 14px;
  border-radius: 9px;
  background: var(--dash-accent, #bf7a4e);
  color: #fffaf4;
  border: none;
  font-size: 12.5px;
  font-weight: 600;
  cursor: pointer;
  box-shadow: 0 2px 10px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.28);
  transition: all 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-btn-primary:hover:not(:disabled) {
  background: var(--dash-accent-dark, #a05f3d);
}

.evo-btn-primary:active:not(:disabled) {
  transform: scale(0.98);
}

.evo-btn-primary:disabled {
  opacity: 0.65;
  cursor: not-allowed;
}

.evo-btn-secondary {
  padding: 8px 14px;
  border-radius: 9px;
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-secondary, #6d6558);
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  font-size: 12.5px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-btn-secondary:hover {
  background: var(--dash-panel-soft, #f3f0e9);
  color: var(--dash-text-primary, #2d2922);
}

.evo-btn-ghost {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 8px 18px;
  border-radius: 9px;
  border: 1px solid var(--dash-accent-ring, rgba(191, 122, 78, 0.28));
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-accent-dark, #a05f3d);
  font-size: 12.5px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-btn-ghost:hover {
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.06));
  border-color: var(--dash-accent, #bf7a4e);
}

/* Insights List */
.evo-filter-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  margin-bottom: 14px;
}

.evo-category-chips {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.evo-chip {
  padding: 5px 12px;
  border-radius: 999px;
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  background: var(--dash-panel-solid, #fffefb);
  font-size: 12px;
  font-weight: 500;
  color: var(--dash-text-secondary, #6d6558);
  cursor: pointer;
  transition: all 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-chip:hover {
  border-color: var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  color: var(--dash-text-primary, #2d2922);
}

.evo-chip.active {
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
  border-color: var(--dash-accent-ring, rgba(191, 122, 78, 0.28));
  color: var(--dash-accent-dark, #a05f3d);
  font-weight: 600;
}

.evo-insights-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.evo-insight-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 14px 16px;
  border-radius: var(--dash-radius-lg, 13px);
  background: var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  box-shadow: var(--dash-shadow-xs, 0 1px 2px rgba(48, 42, 34, 0.04));
  transition: border-color 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-insight-card:hover {
  border-color: var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
}

.evo-insight-card.archived {
  opacity: 0.55;
  background: var(--dash-panel-soft, #f3f0e9);
}

.evo-insight-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex-wrap: wrap;
}

.evo-category-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 9px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 650;
}

.evo-insight-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--dash-text-muted, #a29a8a);
}

.evo-confidence-tag,
.evo-hit-tag {
  padding: 2px 7px;
  border-radius: 999px;
  background: var(--dash-panel-sunken, #edeae2);
  font-family: var(--dash-font-mono, "SFMono-Regular", Consolas, monospace);
  font-size: 10.5px;
}

.evo-insight-content {
  font-size: 13px;
  color: var(--dash-text-primary, #2d2922);
  line-height: 1.6;
}

.evo-insight-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding-top: 8px;
  border-top: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.08));
}

.evo-insight-date {
  font-family: var(--dash-font-mono, "SFMono-Regular", Consolas, monospace);
  font-size: 11px;
  color: var(--dash-text-muted, #a29a8a);
}

.evo-insight-ctrls {
  display: flex;
  align-items: center;
  gap: 6px;
}

.evo-toggle-btn {
  padding: 3px 10px;
  border-radius: 999px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-solid, #fffefb);
  color: var(--dash-text-secondary, #6d6558);
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-toggle-btn:hover {
  background: var(--dash-panel-soft, #f3f0e9);
}

.evo-toggle-btn.active {
  background: var(--dash-success-soft, rgba(94, 143, 106, 0.12));
  border-color: rgba(94, 143, 106, 0.3);
  color: var(--dash-success, #5e8f6a);
}

.evo-edit-btn,
.evo-delete-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 8px;
  background: transparent;
  border: none;
  cursor: pointer;
  color: var(--dash-text-muted, #a29a8a);
  transition: all 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-edit-btn:hover {
  background: var(--dash-panel-soft, #f3f0e9);
  color: var(--dash-text-primary, #2d2922);
}

.evo-delete-btn:hover {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.12));
  color: var(--dash-danger, #c05a4d);
}

/* Logs Timeline */
.evo-logs-timeline {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding-left: 20px;
}

.evo-logs-timeline::before {
  content: "";
  position: absolute;
  left: 6px;
  top: 6px;
  bottom: 6px;
  width: 2px;
  background: var(--dash-divider, rgba(63, 54, 44, 0.08));
}

.evo-log-item {
  position: relative;
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

.evo-log-dot {
  position: absolute;
  left: -20px;
  top: 14px;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--dash-accent, #bf7a4e);
  box-shadow: 0 0 0 3px rgba(var(--pet-primary-rgb, 191, 122, 78), 0.2);
}

.evo-log-body {
  flex: 1;
  padding: 12px 16px;
  border-radius: var(--dash-radius-md, 10px);
  background: var(--dash-panel-soft, #f3f0e9);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
}

.evo-log-head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 4px;
}

.evo-log-time {
  font-family: var(--dash-font-mono, "SFMono-Regular", Consolas, monospace);
  font-size: 11px;
  color: var(--dash-text-muted, #a29a8a);
}

.evo-log-tag {
  font-size: 10.5px;
  font-weight: 600;
  padding: 2px 7px;
  border-radius: 999px;
  background: var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  color: var(--dash-text-secondary, #6d6558);
}

.evo-log-summary {
  font-size: 13px;
  color: var(--dash-text-primary, #2d2922);
  line-height: 1.6;
}

.evo-log-actions {
  margin-top: 8px;
}

.evo-rollback-btn {
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px solid rgba(192, 90, 77, 0.28);
  background: transparent;
  color: var(--dash-danger, #c05a4d);
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-rollback-btn span {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.evo-rollback-btn:hover:not(:disabled) {
  background: var(--dash-danger-soft, rgba(192, 90, 77, 0.12));
}

.evo-rollback-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

/* Empty State */
.evo-empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  padding: 44px 24px;
  border-radius: var(--dash-radius-xl, 16px);
  border: 1px dashed var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: transparent;
}

.evo-empty-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--dash-text-muted, #a29a8a);
  margin-bottom: 12px;
}

.evo-empty-title {
  font-size: 14px;
  font-weight: 700;
  color: var(--dash-text-primary, #2d2922);
  margin-bottom: 6px;
}

.evo-empty-desc {
  font-size: 12.5px;
  color: var(--dash-text-secondary, #6d6558);
  max-width: 440px;
  line-height: 1.6;
  margin-bottom: 16px;
}

.evo-add-insight-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 5px 12px;
  border-radius: 999px;
  border: 1px dashed var(--dash-accent-ring, rgba(191, 122, 78, 0.28));
  background: var(--dash-accent-softer, rgba(191, 122, 78, 0.06));
  color: var(--dash-accent-dark, #a05f3d);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-add-insight-btn:hover {
  background: var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
  border-color: var(--dash-accent, #bf7a4e);
}

/* Modal */
.evo-modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(45, 41, 34, 0.42);
  backdrop-filter: blur(4px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.evo-modal-card {
  width: 90%;
  max-width: 480px;
  background: var(--dash-panel-solid, #fffefb);
  border: 1px solid var(--dash-card-border, rgba(63, 54, 44, 0.09));
  border-radius: var(--dash-radius-xl, 16px);
  box-shadow: 0 2px 6px rgba(48, 42, 34, 0.06), 0 20px 48px rgba(48, 42, 34, 0.16);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: evo-modal-pop 0.2s cubic-bezier(0.22, 1, 0.36, 1);
}

@keyframes evo-modal-pop {
  from { opacity: 0; transform: scale(0.96); }
  to { opacity: 1; transform: scale(1); }
}

.evo-modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 15px 20px;
  border-bottom: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.08));
}

.evo-modal-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 700;
  color: var(--dash-text-primary, #2d2922);
}

.evo-modal-title .app-icon {
  color: var(--dash-accent, #bf7a4e);
}

.evo-modal-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 8px;
  background: transparent;
  border: none;
  color: var(--dash-text-muted, #a29a8a);
  cursor: pointer;
  transition: all 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-modal-close:hover {
  background: var(--dash-panel-soft, #f3f0e9);
  color: var(--dash-text-primary, #2d2922);
}

.evo-modal-body {
  padding: 20px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.evo-form-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.evo-form-group label {
  font-size: 12px;
  font-weight: 600;
  color: var(--dash-text-secondary, #6d6558);
}

.evo-select,
.evo-textarea {
  padding: 9px 12px;
  border-radius: 9px;
  border: 1px solid var(--dash-card-border-strong, rgba(63, 54, 44, 0.16));
  background: var(--dash-panel-solid, #fffefb);
  font-family: inherit;
  font-size: 13px;
  color: var(--dash-text-primary, #2d2922);
  outline: none;
  transition: border-color 0.16s cubic-bezier(0.22, 1, 0.36, 1), box-shadow 0.16s cubic-bezier(0.22, 1, 0.36, 1);
}

.evo-select {
  appearance: none;
  -webkit-appearance: none;
  padding-right: 32px;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='M1 1l4 4 4-4' stroke='%236d6558' stroke-width='1.5' fill='none' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 12px center;
  cursor: pointer;
}

.evo-select:focus,
.evo-textarea:focus {
  border-color: var(--dash-accent, #bf7a4e);
  box-shadow: 0 0 0 3px var(--dash-accent-soft, rgba(191, 122, 78, 0.1));
}

.evo-textarea {
  resize: vertical;
  min-height: 80px;
  line-height: 1.6;
}

.evo-modal-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
  padding: 14px 20px;
  border-top: 1px solid var(--dash-divider, rgba(63, 54, 44, 0.08));
  background: var(--dash-panel-soft, #f3f0e9);
}

.evo-modal-footer .evo-btn-primary {
  flex: none;
}
</style>
