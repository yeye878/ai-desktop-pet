import fs from '@ohos.file.fs';
import { util } from '@kit.ArkTS';
import { image } from '@kit.ImageKit';
import { picker } from '@kit.CoreFileKit';
import { pasteboard } from '@kit.BasicServicesKit';
import { photoAccessHelper } from '@kit.MediaLibraryKit';
import { textRecognition } from '@kit.CoreVisionKit';
import { Log } from '../commons/Log';
import { KbService } from './KbService';
import { bus, Ev } from '../commons/EventBus';

/**
 * 知识库文档导入器：文件选择（txt/md/csv/代码等文本）+ 拍照/相册图片 OCR。
 */
export class DocImporter {
  /** 打开文档选择器导入文本文件 */
  static async importTextDocs(): Promise<string> {
    const docPicker = new picker.DocumentViewPicker();
    const opts: picker.DocumentSelectOptions = {
      maxSelectNumber: 10
    };
    const uris = await docPicker.select(opts);
    if (!uris || uris.length === 0) {
      return '未选择文件';
    }
    let ok = 0;
    let skipped = 0;
    for (const uri of uris) {
      const title = DocImporter.fileNameOf(uri);
      try {
        const text = DocImporter.readUriText(uri);
        if (text.trim().length < 10) {
          skipped++;
          continue;
        }
        await KbService.ingest('default', title, text, 'file');
        ok++;
      } catch (e) {
        Log.warn(`import ${title} failed: ${(e as Error).message}`);
        skipped++;
      }
    }
    bus.emit(Ev.KB_CHANGED);
    return `已导入 ${ok} 个文档${skipped > 0 ? `，跳过 ${skipped} 个（二进制或空文件请先转文本）` : ''}`;
  }

  /** 剪贴板文本 → 知识库（由用户点按钮触发，满足系统剪贴板读取要求；不经过大模型，零 token） */
  static async importFromClipboard(): Promise<string> {
    let text = '';
    try {
      const data = await pasteboard.getSystemPasteboard().getData();
      const primary = data.getPrimaryText();
      text = primary === undefined || primary === null ? '' : primary;
    } catch (e) {
      return '剪贴板为空或不可读，请先复制一段文本';
    }
    const trimmed = text.trim();
    if (trimmed.length < 10) {
      return '剪贴板里没有足够的文本（至少 10 字），先在别处复制好再来';
    }
    let firstLine = trimmed;
    const nl = trimmed.indexOf('\n');
    if (nl > 0) {
      firstLine = trimmed.substring(0, nl).trim();
    }
    const title = firstLine.length >= 2 && firstLine.length <= 30 ? firstLine : `粘贴_${DocImporter.timeTag()}`;
    await KbService.ingest('default', title, trimmed, 'paste');
    bus.emit(Ev.KB_CHANGED);
    return `已入库《${title}》（${trimmed.length} 字）`;
  }

  /** 打开相册选择图片并 OCR 入库 */
  static async importImagesOcr(): Promise<string> {
    const photoPicker = new photoAccessHelper.PhotoViewPicker();
    const opts: photoAccessHelper.PhotoSelectOptions = {
      MIMEType: photoAccessHelper.PhotoViewMIMETypes.IMAGE_TYPE,
      maxSelectNumber: 5
    };
    const result = await photoPicker.select(opts);
    if (!result.photoUris || result.photoUris.length === 0) {
      return '未选择图片';
    }
    let ok = 0;
    const parts: string[] = [];
    for (const uri of result.photoUris) {
      try {
        const text = await DocImporter.ocrText(uri);
        if (text.trim().length > 5) {
          parts.push(`图片 ${uri.substring(uri.length - 12)}\n${text}`);
          ok++;
        }
      } catch (e) {
        Log.warn(`ocr failed: ${(e as Error).message}`);
      }
    }
    if (ok === 0) {
      return '未能从图片中识别出文字';
    }
    const title = `照片文字_${DocImporter.timeTag()}`;
    await KbService.ingest('default', title, parts.join('\n\n'), 'ocr');
    bus.emit(Ev.KB_CHANGED);
    return `OCR 完成，已入库《${title}》（${ok} 张图片）`;
  }

  /** OCR：图片 → 文本（对应复合指令“拍一下这个文档”的识别环节） */
  static async ocrText(uri: string): Promise<string> {
    try {
      await textRecognition.init();
    } catch (e) {
      // 已初始化会抛重复初始化，忽略
    }
    let source: image.ImageSource | null = null;
    try {
      const isUri = uri.startsWith('file://') || uri.startsWith('datashare://') || uri.startsWith('media://');
      source = isUri ? image.createImageSource(uri) : image.createImageSource(uri);
      const pixelMap = await source.createPixelMap();
      const visionInfo: textRecognition.VisionInfo = { pixelMap: pixelMap };
      const result = await textRecognition.recognizeText(visionInfo);
      await pixelMap.release();
      return result.value;
    } finally {
      if (source !== null) {
        try {
          await source.release();
        } catch (e) {
          // ignore
        }
      }
    }
  }

  static readUriText(uri: string): string {
    const f = fs.openSync(uri, fs.OpenMode.READ_ONLY);
    try {
      // 文档 picker 的 uri 直接 stat 可能失败，改用 fd stat
      let size = 1024 * 1024;
      try {
        const stat = fs.statSync(f.fd);
        size = stat.size > 1024 * 1024 ? 1024 * 1024 : stat.size;
      } catch (e) {
        // 保持默认上限
      }
      const buf = new ArrayBuffer(size);
      fs.readSync(f.fd, buf);
      const decoder = util.TextDecoder.create('utf-8');
      return decoder.decodeToString(new Uint8Array(buf));
    } finally {
      fs.closeSync(f);
    }
  }

  static fileNameOf(uri: string): string {
    const clean = uri.replace(/\\/g, '/');
    const q = clean.indexOf('?');
    const noQuery = q >= 0 ? clean.substring(0, q) : clean;
    const slash = noQuery.lastIndexOf('/');
    let name = slash >= 0 ? noQuery.substring(slash + 1) : noQuery;
    if (name.length > 40) {
      name = `文档_${DocImporter.timeTag()}`;
    }
    return name;
  }

  static timeTag(): string {
    const d = new Date();
    return `${d.getMonth() + 1}${d.getDate()}_${d.getHours()}${String(d.getMinutes()).padStart(2, '0')}`;
  }
}
