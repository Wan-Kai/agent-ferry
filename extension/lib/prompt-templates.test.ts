import { describe, expect, it } from "vitest";
import {
  DEFAULT_PROMPT,
  PROMPT_TEMPLATE_STORAGE_KEY,
  deleteTemplate,
  effectivePrompt,
  loadPromptTemplateSettings,
  normalizePromptTemplateSettings,
  persistPromptTemplateSettings,
  saveTemplate,
  type ExtensionStorage,
} from "./prompt-templates";

class MemoryStorage implements ExtensionStorage {
  values: Record<string, unknown> = {};

  async get(key: string): Promise<Record<string, unknown>> {
    return { [key]: this.values[key] };
  }

  async set(items: Record<string, unknown>): Promise<void> {
    Object.assign(this.values, items);
  }
}

describe("Prompt Template", () => {
  it("没有选择时始终展示可见默认 Prompt", () => {
    expect(effectivePrompt(normalizePromptTemplateSettings(undefined))).toBe(DEFAULT_PROMPT);
  });

  it("选择模板只生成一份可编辑副本，单次修改不改变模板", () => {
    const saved = saveTemplate(normalizePromptTemplateSettings(undefined), { name: "深度分析", content: "  模板原文\n" }, "template-1");
    const selected = { ...saved, selected_template_id: "template-1" };
    let editor = effectivePrompt(selected);
    editor = "本次单独修改";
    expect(editor).toBe("本次单独修改");
    expect(selected.templates[0]?.content).toBe("  模板原文\n");
  });

  it("新增、编辑、删除和选择状态可以持久化", async () => {
    const storage = new MemoryStorage();
    let settings = saveTemplate(normalizePromptTemplateSettings(undefined), { name: "摘要", content: "先摘要" }, "template-1");
    settings = { ...settings, selected_template_id: "template-1" };
    settings = saveTemplate(settings, { id: "template-1", name: "摘要", content: "更新后的摘要" }, "unused");
    await persistPromptTemplateSettings(storage, settings);
    expect(storage.values[PROMPT_TEMPLATE_STORAGE_KEY]).toEqual(settings);
    const loaded = await loadPromptTemplateSettings(storage);
    expect(effectivePrompt(loaded)).toBe("更新后的摘要");
    expect(deleteTemplate(loaded, "template-1")).toEqual({ version: 1, templates: [], selected_template_id: null });
  });
});
