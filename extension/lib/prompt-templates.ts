export const DEFAULT_PROMPT = "请分析这篇内容，提炼核心观点、关键证据和可执行的启发，并将值得长期保留的信息自行沉淀到你的文档或记忆中。";
export const PROMPT_TEMPLATE_STORAGE_KEY = "prompt_template_settings_v1";
export const MAX_PROMPT_TEMPLATES = 50;
export const MAX_TEMPLATE_NAME_LENGTH = 80;
export const MAX_PROMPT_LENGTH = 16 * 1024;

export type PromptTemplate = {
  id: string;
  name: string;
  content: string;
};

export type PromptTemplateSettings = {
  version: 1;
  templates: PromptTemplate[];
  selected_template_id: string | null;
};

export type ExtensionStorage = {
  get(key: string): Promise<Record<string, unknown>>;
  set(items: Record<string, unknown>): Promise<void>;
};

export const EMPTY_PROMPT_TEMPLATE_SETTINGS: PromptTemplateSettings = {
  version: 1,
  templates: [],
  selected_template_id: null,
};

export function normalizePromptTemplateSettings(value: unknown): PromptTemplateSettings {
  if (!value || typeof value !== "object") return { ...EMPTY_PROMPT_TEMPLATE_SETTINGS };
  const candidate = value as Partial<PromptTemplateSettings>;
  const templates = Array.isArray(candidate.templates)
    ? candidate.templates
        .filter((template): template is PromptTemplate => {
          if (!template || typeof template !== "object") return false;
          const item = template as Partial<PromptTemplate>;
          return typeof item.id === "string"
            && item.id.length > 0
            && typeof item.name === "string"
            && item.name.trim().length > 0
            && item.name.trim().length <= MAX_TEMPLATE_NAME_LENGTH
            && typeof item.content === "string"
            && item.content.trim().length > 0
            && new TextEncoder().encode(item.content).byteLength <= MAX_PROMPT_LENGTH;
        })
        .slice(0, MAX_PROMPT_TEMPLATES)
        .filter((template, index, all) => all.findIndex((item) => item.id === template.id) === index)
        .map((template) => ({ ...template, name: template.name.trim() }))
    : [];
  const selected = typeof candidate.selected_template_id === "string"
    && templates.some((template) => template.id === candidate.selected_template_id)
    ? candidate.selected_template_id
    : null;
  return { version: 1, templates, selected_template_id: selected };
}

export function saveTemplate(
  settings: PromptTemplateSettings,
  draft: { id?: string; name: string; content: string },
  newId: string,
): PromptTemplateSettings {
  const name = draft.name.trim();
  const content = draft.content;
  if (!name || name.length > MAX_TEMPLATE_NAME_LENGTH) throw new Error("模板名称不能为空且不得超过 80 个字符");
  if (!content.trim() || new TextEncoder().encode(content).byteLength > MAX_PROMPT_LENGTH) throw new Error("模板内容不能为空且不得超过 16 KiB");
  if (settings.templates.some((template) => template.name === name && template.id !== draft.id)) {
    throw new Error("模板名称不能重复");
  }
  if (!draft.id && settings.templates.length >= MAX_PROMPT_TEMPLATES) throw new Error("最多保存 50 个模板");
  const template = { id: draft.id ?? newId, name, content };
  const templates = draft.id
    ? settings.templates.map((item) => item.id === draft.id ? template : item)
    : [...settings.templates, template];
  return { ...settings, templates };
}

export function deleteTemplate(settings: PromptTemplateSettings, id: string): PromptTemplateSettings {
  return {
    ...settings,
    templates: settings.templates.filter((template) => template.id !== id),
    selected_template_id: settings.selected_template_id === id ? null : settings.selected_template_id,
  };
}

export function effectivePrompt(settings: PromptTemplateSettings): string {
  return settings.templates.find((template) => template.id === settings.selected_template_id)?.content ?? DEFAULT_PROMPT;
}

export async function loadPromptTemplateSettings(storage: ExtensionStorage): Promise<PromptTemplateSettings> {
  const stored = await storage.get(PROMPT_TEMPLATE_STORAGE_KEY);
  return normalizePromptTemplateSettings(stored[PROMPT_TEMPLATE_STORAGE_KEY]);
}

export async function persistPromptTemplateSettings(storage: ExtensionStorage, settings: PromptTemplateSettings): Promise<void> {
  await storage.set({ [PROMPT_TEMPLATE_STORAGE_KEY]: settings });
}
