export interface PendingFile {
  id: string;
  path: string;
  name: string;
  size: number;
  extension: string;
  isImage: boolean;
  isDir: boolean;
  previewUrl: string;
  status: "loading" | "ready";
}

export const MAX_FILES = 5;
export const MAX_FILE_SIZE = 50 * 1024 * 1024;
export const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

export function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function getFileIcon(ext: string): string {
  if (["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"].includes(ext)) return "📷";
  if (["doc", "docx"].includes(ext)) return "📝";
  if (["xls", "xlsx", "csv"].includes(ext)) return "📊";
  if (["ppt", "pptx"].includes(ext)) return "📑";
  if (["zip", "rar", "7z"].includes(ext)) return "📦";
  if (["mp3", "wav", "flac"].includes(ext)) return "🎵";
  if (["mp4", "avi", "mkv"].includes(ext)) return "🎬";
  if (["js", "ts", "py", "rs", "go", "java"].includes(ext)) return "💻";
  return "📄";
}

export function buildMessageWithFiles(text: string, files: PendingFile[]): string {
  if (!files.length) return text;
  const fileLines = files.map((file, index) => {
    const type = file.isDir ? "目录" : file.isImage ? "图片" : "文件";
    return `${index + 1}. ${type}: "${file.name}" (路径: ${file.path}, 大小: ${file.isDir ? "目录" : formatFileSize(file.size)})`;
  }).join("\n");
  const hints = [
    files.some((file) => file.isImage) ? "图片会随消息作为视觉输入发送，请直接观察图片内容。" : "",
    files.some((file) => !file.isImage && !file.isDir) ? "普通文件可使用 read_file 工具读取内容。" : "",
    files.some((file) => file.isDir) ? "目录可先用 list_directory 查看结构，再用 read_file 读取其中的文件。" : "",
  ].filter(Boolean).join(" ");
  return `${text || "[用户附加了文件，但没有输入文字]"}\n\n---\n[用户附加了以下本地文件]\n${fileLines}\n\n提示: ${hints}`;
}

export function attachmentPayload(files: PendingFile[]) {
  return files.map(({ path, name, size, extension, isImage }) => ({ path, name, size, extension, isImage }));
}

export function attachmentTags(files: PendingFile[]) {
  return files.map(({ name, isImage, extension, isDir }) => ({ name, isImage, extension, isDir }));
}

interface DropPane {
  paneId: string;
  sessionId: string;
  rect: { left: number; right: number; top: number; bottom: number };
}

/** Tauri 原生拖放使用物理像素；DOM 的窗格边界使用 CSS 像素。 */
export function attachmentDropTarget(position: { x: number; y: number }, scale: number, panes: DropPane[]) {
  const x = position.x / (scale > 0 ? scale : 1);
  const y = position.y / (scale > 0 ? scale : 1);
  return panes.find(({ rect }) => x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom) ?? null;
}
