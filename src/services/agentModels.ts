import { invoke } from "@tauri-apps/api/core";

/** 三个内置 agent 窗口的 slot；与后端 get_agent_model_catalog 的参数一致。 */
export type AgentModelSlot = "claude" | "codex" | "dsh";

/** 后端给出的思考强度选项；id 由后端目录决定，前端不做白名单限制。 */
export interface AgentEffortOption {
  id: string;
  /** 后端展示名（可能是英文）；UI 用 effortLabel 做中文兜底映射。 */
  name: string;
  description?: string;
}

/** 后端给出的单个可选模型。 */
export interface AgentModelOption {
  /** 传给 CLI 的模型 id / 别名；空串表示跟随本机。 */
  id: string;
  name: string;
  description?: string;
  /** 仅 dsh：provider 路由 id。 */
  provider?: string;
  /** 仅 dsh：provider 展示名。 */
  provider_label?: string;
  /** 后端默认思考强度 id；"" = 无/未知。 */
  default_effort: string;
  /** 该模型可选的思考强度；空数组 = 不支持选择。 */
  efforts: AgentEffortOption[];
}

/** 与 docs/agent-model-catalog-contract.md 第 2 节一致的模型目录。 */
export interface AgentModelCatalog {
  slot: string;
  backend: string;
  /** 本机默认 provider 路由 id；"" = 未知/不区分渠道。v2 起用于「跟随本机」判定。 */
  local_provider: string;
  local_model: string;
  local_model_label: string;
  local_effort: string;
  models: AgentModelOption[];
  error: string;
  fetched_at: number;
}

/**
 * 已知思考强度 id 的中文展示映射。后端 crate 可能新增 id，因此这里只做
 * 「已知 id 更好看，未知 id 原样显示」的兜底，不参与合法性校验。
 */
export const EFFORT_LABELS: Record<string, string> = {
  off: "关闭",
  minimal: "极低",
  low: "低",
  medium: "中",
  high: "高",
  xhigh: "极高",
  max: "最高",
  ultra: "极限",
};

/** 思考强度 id → 中文展示名；未知 id 回退 fallback（缺省为 id 本身）。 */
export function effortLabel(id: string, fallback?: string): string {
  const key = (id || "").trim().toLowerCase();
  if (!key) return fallback || "";
  return EFFORT_LABELS[key] ?? fallback ?? id;
}

/**
 * 按 (provider, id) 找模型条目：
 * - provider 非空：先精确匹配 provider+id；
 * - provider 为空：先匹配 local_provider+id（视为本机渠道）；
 * - 都找不到时退化为同 id 的任一模型（兼容旧目录/手输 id）；
 * - modelId 为空：用 local_provider+local_model 的条目。
 */
export function modelOptionFor(
  catalog: AgentModelCatalog | null | undefined,
  modelId: string,
  provider?: string,
): AgentModelOption | undefined {
  const models = catalog?.models ?? [];
  const id = (modelId || "").trim();
  const localProvider = (catalog?.local_provider || "").trim();
  const wantedProvider = (provider || "").trim();
  if (!id) {
    const localId = (catalog?.local_model || "").trim();
    if (!localId) return undefined;
    if (localProvider) {
      const localExact = models.find((model) => model.id === localId && (model.provider || "") === localProvider);
      if (localExact) return localExact;
    }
    return models.find((model) => model.id === localId);
  }
  if (wantedProvider) {
    const exact = models.find((model) => model.id === id && (model.provider || "") === wantedProvider);
    if (exact) return exact;
  } else if (localProvider) {
    const localExact = models.find((model) => model.id === id && (model.provider || "") === localProvider);
    if (localExact) return localExact;
  }
  return models.find((model) => model.id === id);
}

/** 某个 (provider, model) 可选的思考强度；modelId 为空时用本机默认模型的能力。 */
export function effortOptionsFor(
  catalog: AgentModelCatalog | null | undefined,
  modelId: string,
  provider?: string,
): AgentEffortOption[] {
  const option = modelOptionFor(catalog, modelId, provider);
  return Array.isArray(option?.efforts) ? option.efforts : [];
}

/** provider id → 展示名；provider 为空视为本机渠道（用 local_provider）。找不到时回退原始 id。 */
export function providerLabel(catalog: AgentModelCatalog | null | undefined, provider?: string): string {
  const wanted = (provider || "").trim();
  const target = wanted || (catalog?.local_provider || "").trim();
  if (!target) return "";
  const match = (catalog?.models ?? []).find((model) => (model.provider || "") === target);
  if (match) return (match.provider_label || "").trim() || target;
  return wanted;
}

/** 是否本机默认渠道：空 provider 或与 catalog.local_provider 相同都算本机。 */
export function isLocalProvider(catalog: AgentModelCatalog | null | undefined, provider?: string): boolean {
  const wanted = (provider || "").trim();
  if (!wanted) return true;
  const local = (catalog?.local_provider || "").trim();
  return !!local && wanted === local;
}

/** 目录缓存有效期；与后端 60s 缓存保持一致，force 刷新可绕过。 */
const CACHE_TTL_MS = 60_000;
const cache = new Map<string, { catalog: AgentModelCatalog; at: number }>();
const inFlight = new Map<string, Promise<AgentModelCatalog>>();

function text(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function normalizeEffort(raw: unknown): AgentEffortOption | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  const id = text(value.id).trim();
  if (!id) return null;
  const name = text(value.name).trim();
  const description = text(value.description);
  return { id, name: name || id, ...(description ? { description } : {}) };
}

function normalizeModel(raw: unknown): AgentModelOption | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  const id = text(value.id).trim();
  if (!id) return null;
  const models: AgentModelOption = {
    id,
    name: text(value.name).trim() || id,
    default_effort: text(value.default_effort),
    efforts: Array.isArray(value.efforts)
      ? value.efforts.map(normalizeEffort).filter((effort): effort is AgentEffortOption => !!effort)
      : [],
  };
  const description = text(value.description);
  if (description) models.description = description;
  const provider = text(value.provider);
  if (provider) models.provider = provider;
  const providerLabel = text(value.provider_label);
  if (providerLabel) models.provider_label = providerLabel;
  return models;
}

/** 把 invoke 回来的任意 JSON 归一化成契约结构，避免脏数据打断渲染。 */
function normalizeCatalog(raw: unknown, slot: AgentModelSlot): AgentModelCatalog {
  const value = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  return {
    slot: text(value.slot) || slot,
    backend: text(value.backend),
    local_provider: text(value.local_provider),
    local_model: text(value.local_model),
    local_model_label: text(value.local_model_label),
    local_effort: text(value.local_effort),
    models: Array.isArray(value.models)
      ? value.models.map(normalizeModel).filter((model): model is AgentModelOption => !!model)
      : [],
    error: text(value.error),
    fetched_at: typeof value.fetched_at === "number" && Number.isFinite(value.fetched_at) ? value.fetched_at : 0,
  };
}

/**
 * 读取某个 agent 窗口的模型目录。
 * 内存缓存 + 进行中请求去重：同一 slot 并发打开选择器只会探测一次；
 * force = true 供弹层里的刷新按钮强制重新探测（对应 Rust 端 force 参数）。
 */
export async function fetchAgentModelCatalog(slot: AgentModelSlot, force = false): Promise<AgentModelCatalog> {
  const cached = cache.get(slot);
  if (!force && cached && Date.now() - cached.at < CACHE_TTL_MS) return cached.catalog;
  const pending = inFlight.get(slot);
  if (!force && pending) return pending;
  const request: Promise<AgentModelCatalog> = invoke<unknown>("get_agent_model_catalog", { slot, force })
    .then((raw) => {
      const catalog = normalizeCatalog(raw, slot);
      cache.set(slot, { catalog, at: Date.now() });
      return catalog;
    })
    .finally(() => {
      if (inFlight.get(slot) === request) inFlight.delete(slot);
    });
  inFlight.set(slot, request);
  return request;
}

/** 测试/调试用：清空目录缓存（可选只清某个 slot）。 */
export function clearAgentModelCatalogCache(slot?: AgentModelSlot): void {
  if (slot) {
    cache.delete(slot);
    inFlight.delete(slot);
    return;
  }
  cache.clear();
  inFlight.clear();
}
