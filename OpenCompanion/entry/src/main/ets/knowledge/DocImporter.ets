import fs from '@ohos.file.fs';
import { util } from '@kit.ArkTS';
import { image } from '@kit.ImageKit';
import { picker } from '@kit.CoreFileKit';
import { photoAccessHelper } from '@kit.MediaLibraryKit';
import { textRecognition } from '@kit.CoreVisionKit';
import { Log } from '../commons/Log';
import { KbService } from './KbService';
import { bus, Ev } from '../commons/EventBus';

/**
 * 知识库文档导入器：文件选择（txt/md/csv/代码等文本）+ 拍照/相册图片 OCR。
 */
export class DocImporter {
  /** OCR 不可用时抛出的错误标识（调用方据此与「识别不出文字」区分） */
  static readonly ERR_OCR_UNAVAILABLE: string = 'OCR_UNAVAILABLE';

  /**
   * 单张图片处理上限：超过则直接拒绝。
   * `recognizeText` 前要把整图解码成 PixelMap，超大图会把内存打爆或长时间卡住，
   * 与其让它在解码阶段失败，不如给用户一句能照着做的提示。
   */
  static readonly MAX_IMAGE_BYTES: number = 12 * 1024 * 1024;

  /**
   * OCR 引擎是否可用：syscap 声明 + 原生模块实际存在，双重验证。
   * 与 RagBridge.kitAvailable 同构 —— syscap 表可能声明支持，但原生模块被裁剪
   * （模拟器常见），只查 canIUse 会漏判。
   */
  static ocrAvailable(): boolean {
    try {
      // syscap 名以 SDK d.ts 的 @syscap 标注为准：
      // @hms.ai.ocr.textRecognition.d.ts → SystemCapability.AI.OCR.TextRecognition
      if (!canIUse('SystemCapability.AI.OCR.TextRecognition')) {
        return false;
      }
      return typeof textRecognition.init === 'function';
    } catch (e) {
      return false;
    }
  }

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

  /** 打开相册选择图片并 OCR 入库 */
  static async importImagesOcr(): Promise<string> {
    // 能力检测放在开选择器之前：设备不支持时直接给出可执行的替代方案，
    // 不必让用户先选完图再被拒
    if (!DocImporter.ocrAvailable()) {
      return '本机不支持文字识别（OCR）。可改用「导入文档」选择文本文件，或直接在对话里输入文字。';
    }
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
    let failed = 0;
    const parts: string[] = [];
    for (const uri of result.photoUris) {
      try {
        const text = await DocImporter.ocrText(uri);
        if (text.trim().length > 5) {
          parts.push(`图片 ${uri.substring(uri.length - 12)}\n${text}`);
          ok++;
        } else {
          failed++;
        }
      } catch (e) {
        failed++;
        Log.warn(`ocr failed: ${(e as Error).message}`);
      }
    }
    if (ok === 0) {
      return `未能从图片中识别出文字（${failed}/${result.photoUris.length} 张未取到有效文本）`;
    }
    const title = `照片文字_${DocImporter.timeTag()}`;
    await KbService.ingest('default', title, parts.join('\n\n'), 'ocr');
    bus.emit(Ev.KB_CHANGED);
    return `OCR 完成，已入库《${title}》（${ok} 张图片${failed > 0 ? `，${failed} 张未识别出文本` : ''}）`;
  }

  /** OCR：图片 → 文本（对应复合指令“拍一下这个文档”的识别环节） */
  static async ocrText(uri: string): Promise<string> {
    // 原实现无条件吞掉 init() 的异常：模块缺失时错误要拖到 recognizeText 才抛出，
    // 调用方只能看到笼统的「识别不出」。这里显式判定，并抛出可区分的错误标识。
    if (!DocImporter.ocrAvailable()) {
      throw new Error(DocImporter.ERR_OCR_UNAVAILABLE);
    }
    let inited = false;
    try {
      inited = await textRecognition.init();
    } catch (e) {
      // init() 重复调用会抛「已初始化」——说明引擎此前初始化成功
      inited = true;
    }
    if (!inited) {
      throw new Error(DocImporter.ERR_OCR_UNAVAILABLE);
    }
    // 解码前先看体积：取不到大小（0）则不阻断，交由解码阶段自行报错
    const bytes = DocImporter.uriSize(uri);
    if (bytes > DocImporter.MAX_IMAGE_BYTES) {
      const mb = Math.round(bytes / 1024 / 1024);
      throw new Error(`图片过大（${mb}MB，上限 ${DocImporter.MAX_IMAGE_BYTES / 1024 / 1024}MB），请压缩后重试`);
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

  /** uri 指向文件的字节数；打不开或取不到则返回 0（调用方据此跳过体积极校验） */
  private static uriSize(uri: string): number {
    try {
      const f = fs.openSync(uri, fs.OpenMode.READ_ONLY);
      try {
        return fs.statSync(f.fd).size;
      } finally {
        fs.closeSync(f);
      }
    } catch (e) {
      return 0;
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
