import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";

export interface EvolutionInsight {
  id: string;
  category: "preference" | "work_domain" | "coding_style" | "negative_constraint" | "habit" | string;
  content: string;
  confidence: number;
  hit_count: number;
  status: "active" | "archived" | "rejected" | string;
  created_at: number;
  updated_at: number;
}

export interface EvolutionProposal {
  id: string;
  proposal_type: "create_agent" | "create_skill" | "schedule_task" | "update_guideline" | string;
  title: string;
  rationale: string;
  payload_json: string;
  status: "pending" | "applied" | "rejected" | "rolled_back" | string;
  created_at: number;
  applied_at: number | null;
}

export interface EvolutionLog {
  id: string;
  proposal_id: string | null;
  category: string;
  summary: string;
  before_state_json: string;
  after_state_json: string;
  created_at: number;
}

export interface EvolutionSummary {
  level: number;
  sync_score: number;
  active_insights_count: number;
  pending_proposals_count: number;
  total_evolutions_count: number;
  top_domains: string[];
}

export const useEvolutionStore = defineStore("evolution", {
  state: () => ({
    insights: [] as EvolutionInsight[],
    proposals: [] as EvolutionProposal[],
    logs: [] as EvolutionLog[],
    summary: {
      level: 1,
      sync_score: 50,
      active_insights_count: 0,
      pending_proposals_count: 0,
      total_evolutions_count: 0,
      top_domains: [],
    } as EvolutionSummary,
    isReflecting: false,
    isApplyingId: null as string | null,
    isRollingBackId: null as string | null,
    autoEvolutionEnabled: true,
    error: "" as string,
  }),

  getters: {
    pendingProposals(state): EvolutionProposal[] {
      return state.proposals.filter((p) => p.status === "pending");
    },
    appliedProposals(state): EvolutionProposal[] {
      return state.proposals.filter((p) => p.status === "applied");
    },
    activeInsights(state): EvolutionInsight[] {
      return state.insights.filter((i) => i.status === "active");
    },
    archivedInsights(state): EvolutionInsight[] {
      return state.insights.filter((i) => i.status !== "active");
    },
    hasPendingProposals(state): boolean {
      return state.proposals.some((p) => p.status === "pending");
    },
  },

  actions: {
    async loadAll() {
      try {
        const [insights, proposals, logs, summary, autoEvo] = await Promise.all([
          invoke<EvolutionInsight[]>("get_evolution_insights"),
          invoke<EvolutionProposal[]>("get_evolution_proposals"),
          invoke<EvolutionLog[]>("get_evolution_logs", { limit: 50 }),
          invoke<EvolutionSummary>("get_evolution_summary"),
          invoke<string | null>("get_setting_value", { key: "evolution_auto_enabled" }),
        ]);
        this.insights = insights;
        this.proposals = proposals;
        this.logs = logs;
        this.summary = summary;
        this.autoEvolutionEnabled = autoEvo !== "false";
      } catch (err) {
        this.error = String(err);
        console.error("加载自进化数据失败:", err);
      }
    },

    async setAutoEvolutionEnabled(enabled: boolean) {
      try {
        await invoke("set_setting_value", {
          key: "evolution_auto_enabled",
          value: enabled ? "true" : "false",
        });
        this.autoEvolutionEnabled = enabled;
      } catch (err) {
        this.error = String(err);
        throw err;
      }
    },

    async updateInsight(insight: EvolutionInsight) {
      insight.updated_at = Math.floor(Date.now() / 1000);
      try {
        await invoke("save_evolution_insight", { insight });
        await this.loadAll();
      } catch (err) {
        this.error = String(err);
        throw err;
      }
    },

    async triggerReflection(): Promise<{ success: boolean; message: string }> {
      this.isReflecting = true;
      this.error = "";
      try {
        const res = await invoke<{
          success: boolean;
          message: string;
          new_insights_count: number;
          updated_insights_count: number;
          new_proposals_count: number;
        }>("trigger_evolution_reflection");
        await this.loadAll();
        return {
          success: true,
          message: res.message || "复盘分析完成！",
        };
      } catch (err) {
        this.error = String(err);
        throw err;
      } finally {
        this.isReflecting = false;
      }
    },

    async applyProposal(id: string) {
      this.isApplyingId = id;
      try {
        await invoke("apply_evolution_proposal", { id });
        await this.loadAll();
      } catch (err) {
        this.error = String(err);
        throw err;
      } finally {
        this.isApplyingId = null;
      }
    },

    async rejectProposal(id: string) {
      try {
        await invoke("reject_evolution_proposal", { id });
        await this.loadAll();
      } catch (err) {
        this.error = String(err);
        throw err;
      }
    },

    async toggleInsightStatus(insight: EvolutionInsight) {
      const nextStatus = insight.status === "active" ? "archived" : "active";
      try {
        await invoke("update_evolution_insight_status", {
          id: insight.id,
          status: nextStatus,
        });
        insight.status = nextStatus;
        await this.loadAll();
      } catch (err) {
        this.error = String(err);
        throw err;
      }
    },

    async saveInsight(draft: { category: string; content: string; confidence: number }) {
      const now = Math.floor(Date.now() / 1000);
      const newInsight: EvolutionInsight = {
        id: crypto.randomUUID(),
        category: draft.category,
        content: draft.content.trim(),
        confidence: draft.confidence,
        hit_count: 1,
        status: "active",
        created_at: now,
        updated_at: now,
      };
      try {
        await invoke("save_evolution_insight", { insight: newInsight });
        await this.loadAll();
      } catch (err) {
        this.error = String(err);
        throw err;
      }
    },

    async deleteInsight(id: string) {
      try {
        await invoke("delete_evolution_insight", { id });
        this.insights = this.insights.filter((i) => i.id !== id);
        await this.loadAll();
      } catch (err) {
        this.error = String(err);
        throw err;
      }
    },

    async rollbackLog(id: string) {
      this.isRollingBackId = id;
      try {
        await invoke("rollback_evolution_log", { id });
        await this.loadAll();
      } catch (err) {
        this.error = String(err);
        throw err;
      } finally {
        this.isRollingBackId = null;
      }
    },
  },
});
