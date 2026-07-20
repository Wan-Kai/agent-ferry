export type AgentProductId = "hermes" | "claude_code" | "open_code" | "codex_cli" | "codex_app";
export type AgentTargetKind = "remote_hermes" | "local_open_code" | "local_claude_code" | "local_codex_cli" | "local_codex_app";
export type AgentTargetState = "ready" | "credential_missing" | "authentication_failed" | "connection_failed" | "incompatible";

export type AgentTarget = {
  id: string;
  name: string;
  kind: AgentTargetKind;
  state: AgentTargetState;
  capabilities: string[];
};

export type AgentWorkspace = { id: string; name: string; path: string; ready: boolean };

export type AgentProduct = {
  id: AgentProductId;
  title: string;
  icon: string;
  ready: boolean;
};

export type RunLocation = {
  id: string;
  title: string;
  meta: string;
  locality: "local" | "remote";
  state: AgentTargetState | "not_detected";
  disabled: boolean;
  targetId: string | null;
};

const PRODUCTS: Array<Omit<AgentProduct, "ready"> & { kind: AgentTargetKind }> = [
  { id: "hermes", title: "Hermes", icon: "/icons/agents/hermes.svg", kind: "remote_hermes" },
  { id: "claude_code", title: "Claude Code", icon: "/icons/agents/claude.svg", kind: "local_claude_code" },
  { id: "open_code", title: "OpenCode", icon: "/icons/agents/opencode.svg", kind: "local_open_code" },
  { id: "codex_cli", title: "Codex CLI", icon: "/icons/agents/codex.svg", kind: "local_codex_cli" },
  { id: "codex_app", title: "Codex App", icon: "/icons/agents/codex.svg", kind: "local_codex_app" },
];

const KIND_TO_PRODUCT: Partial<Record<AgentTargetKind, AgentProductId>> = {
  remote_hermes: "hermes",
  local_claude_code: "claude_code",
  local_open_code: "open_code",
  local_codex_cli: "codex_cli",
  local_codex_app: "codex_app",
};

export function buildAgentProducts(targets: AgentTarget[]): AgentProduct[] {
  return PRODUCTS.map(({ kind, ...product }) => ({
    ...product,
    ready: targets.some((target) => target.kind === kind && target.state === "ready"),
  }));
}

function productKind(productId: AgentProductId): AgentTargetKind {
  return PRODUCTS.find((product) => product.id === productId)?.kind ?? "remote_hermes";
}

function targetWorkspace(target: AgentTarget, workspaces: AgentWorkspace[]): AgentWorkspace | undefined {
  return workspaces.find((workspace) => target.id.endsWith(workspace.id));
}

export function buildRunLocations(productId: AgentProductId, targets: AgentTarget[], workspaces: AgentWorkspace[]): RunLocation[] {
  const matching = targets.filter((target) => target.kind === productKind(productId));
  if (productId === "hermes") {
    return [
      ...matching.map((target) => ({
        id: target.id,
        title: target.name,
        meta: target.state === "ready" ? "远端 · 实时输出" : `远端 · ${targetStateLabel(target.state)}`,
        locality: "remote" as const,
        state: target.state,
        disabled: target.state !== "ready",
        targetId: target.id,
      })),
      {
        id: "local-hermes-unavailable",
        title: "Local Hermes",
        meta: "本机 · 未检测到",
        locality: "local" as const,
        state: "not_detected" as const,
        disabled: true,
        targetId: null,
      },
    ];
  }

  return matching.map((target) => {
    const workspace = targetWorkspace(target, workspaces);
    return {
      id: target.id,
      title: workspace?.name ?? target.name.split(" · ").at(-1) ?? target.name,
      meta: workspace?.path ?? `本机 · ${targetStateLabel(target.state)}`,
      locality: "local" as const,
      state: target.state,
      disabled: target.state !== "ready" || workspace?.ready === false,
      targetId: target.id,
    };
  });
}

export function migrateSelectedProduct(value: string, targets: AgentTarget[]): AgentProductId | "" {
  if (PRODUCTS.some((product) => product.id === value)) return value as AgentProductId;
  if (value in KIND_TO_PRODUCT) return KIND_TO_PRODUCT[value as AgentTargetKind] ?? "";
  const target = targets.find((candidate) => candidate.id === value);
  return target ? KIND_TO_PRODUCT[target.kind] ?? "" : "";
}

/**
 * V1 将远端连接 ID 和本地 Workspace ID 分存在两个字段中；V2 的运行位置统一使用 Target ID。
 * 迁移只在新字段不存在时执行，避免旧配置覆盖用户在新界面中的最新选择。
 */
export function migrateLocationSelections(
  legacySelectedAgent: string,
  legacyWorkspaceByAgent: Record<string, string>,
  targets: AgentTarget[],
): Partial<Record<AgentProductId, string>> {
  const migrated: Partial<Record<AgentProductId, string>> = {};
  const selectedTarget = targets.find((target) => target.id === legacySelectedAgent);
  if (selectedTarget) {
    const product = KIND_TO_PRODUCT[selectedTarget.kind];
    if (product) migrated[product] = selectedTarget.id;
  }
  for (const [legacyKind, workspaceId] of Object.entries(legacyWorkspaceByAgent)) {
    const product = KIND_TO_PRODUCT[legacyKind as AgentTargetKind];
    if (!product) continue;
    const target = targets.find((candidate) => candidate.kind === legacyKind && candidate.id.endsWith(workspaceId));
    if (target) migrated[product] = target.id;
  }
  return migrated;
}

export function targetStateLabel(state: AgentTargetState | "not_detected"): string {
  return ({
    ready: "可用",
    credential_missing: "凭据缺失",
    authentication_failed: "认证失败",
    connection_failed: "无法连接",
    incompatible: "不可用",
    not_detected: "未检测到",
  })[state];
}
