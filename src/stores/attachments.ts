import { defineStore } from "pinia";
import { reactive } from "vue";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { IMAGE_EXTENSIONS, MAX_FILES, MAX_FILE_SIZE, type PendingFile } from "../services/attachments";

interface FileMetadata {
  path: string;
  name: string;
  size: number;
  extension: string;
  exists: boolean;
  is_file: boolean;
  is_dir: boolean;
}

export interface AttachmentDraft {
  files: PendingFile[];
  error: string;
  picking: boolean;
}

/** 草稿按会话保存，独立于窗格的挂载、布局与位置。 */
export const useAttachmentsStore = defineStore("attachments", () => {
  const drafts = reactive<Record<string, AttachmentDraft>>({});
  const attemptedPreviews = new WeakSet<PendingFile>();
  let nextId = 0;
  const pathKey = (path: string) => /^[a-z]:[\\/]|^\\\\/i.test(path) ? path.replace(/\\/g, "/").toLowerCase() : path;

  function ensure(key: string): AttachmentDraft {
    return drafts[key] ?? (drafts[key] = { files: [], error: "", picking: false });
  }

  function remove(key: string, id: string) {
    const draft = ensure(key);
    draft.files = draft.files.filter((file) => file.id !== id);
  }

  function clear(key: string) {
    const draft = ensure(key);
    draft.files = [];
    draft.error = "";
  }

  async function add(key: string, paths: string[]) {
    if (!paths.length) return;
    const draft = ensure(key);
    draft.error = "";
    const errors: string[] = [];
    const reserved: PendingFile[] = [];
    for (const path of paths) {
      if (draft.files.some((file) => pathKey(file.path) === pathKey(path))) continue;
      if (draft.files.length >= MAX_FILES) {
        errors.push(`最多同时添加 ${MAX_FILES} 个文件`);
        break;
      }
      const file: PendingFile = {
        id: `attachment-${++nextId}`, path, name: path.split(/[\\/]/).pop() || path,
        size: 0, extension: "", isImage: false, isDir: false, previewUrl: "", status: "loading",
      };
      draft.files.push(file);
      reserved.push(file);
    }
    if (!reserved.length) {
      draft.error = errors[0] || "文件已在待发送列表中";
      return;
    }
    try {
      const metadata = await invoke<FileMetadata[]>("get_file_metadata", { paths: reserved.map((file) => file.path) });
      for (const reservedFile of reserved) {
        // 用户可在读取元数据期间移除、清空或切换会话；旧请求不得复活已移除的文件。
        const file = draft.files.find((item) => item.id === reservedFile.id);
        if (!file) continue;
        const meta = metadata.find((item) => pathKey(item.path) === pathKey(file.path));
        const error = !meta?.exists ? `路径不存在: ${file.name}`
          : !meta.is_file && !meta.is_dir ? `不是文件或目录: ${file.name}`
          : meta.is_file && meta.size > MAX_FILE_SIZE ? `文件过大: ${meta.name} (最大 50MB)` : "";
        if (error || !meta) {
          remove(key, file.id);
          errors.push(error);
          continue;
        }
        const extension = meta.extension.toLowerCase();
        Object.assign(file, {
          path: meta.path, name: meta.name, size: meta.size, extension,
          isImage: !meta.is_dir && IMAGE_EXTENSIONS.includes(extension), isDir: meta.is_dir, status: "ready",
        });
        if (file.isImage) file.previewUrl = convertFileSrc(meta.path);
      }
    } catch (cause) {
      const stillPresent = reserved.some((file) => draft.files.some((item) => item.id === file.id));
      for (const file of reserved) remove(key, file.id);
      if (stillPresent) errors.push(`获取文件信息失败: ${String(cause)}`);
    }
    if (errors.length) draft.error = [...new Set(errors)].join("；");
  }

  async function pick(key: string, addPaths: (paths: string[]) => Promise<void> = (paths) => add(key, paths)) {
    const draft = ensure(key);
    if (draft.picking) return;
    draft.picking = true;
    try {
      const selected = await open({ multiple: true, directory: false, title: "添加附件" });
      if (selected) await addPaths(Array.isArray(selected) ? selected : [selected]);
    } catch (cause) {
      draft.error = `选择文件失败: ${String(cause)}`;
    } finally {
      draft.picking = false;
    }
  }

  async function previewFallback(key: string, file: PendingFile) {
    // 去掉失效 URL，避免图片错误触发无限重试；文字卡片始终可用。
    file.previewUrl = "";
    if (attemptedPreviews.has(file)) return;
    attemptedPreviews.add(file);
    try {
      const data = await invoke<string>("read_file_as_data_url", { path: file.path });
      if (ensure(key).files.includes(file)) file.previewUrl = data;
    } catch { /* 预览失败不影响附件发送。 */ }
  }

  return { ensure, add, remove, clear, pick, previewFallback };
});
