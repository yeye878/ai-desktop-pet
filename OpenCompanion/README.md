# OpenCompanion

OpenHarmony 原生智能多模态个人助理（ArkTS + ArkUI，Stage 模型）。
功能特性、命题覆盖、演示脚本与目录结构的完整说明见 [README-HarmonyOS.md](README-HarmonyOS.md)。

本文件补充：**本机构建环境、命令行构建步骤与常见问题**。

## 环境要求

| 依赖 | 本机路径 / 版本 |
|------|----------------|
| DevEco Studio | `C:\Program Files\Huawei\DevEco Studio`（自带 jbr / node / ohpm / hvigorw） |
| HarmonyOS SDK | `C:\Program Files\Huawei\DevEco Studio\sdk`，targetSdkVersion 26.0.0（API 20+） |
| 操作系统 | Windows（`build.cmd` 为 cmd 批处理） |

如果 DevEco Studio 装在其他位置，改 `build.cmd` 顶部的三行路径即可：

```cmd
set PATH=<DevEco>\jbr\bin;<DevEco>\tools\node;<DevEco>\tools\ohpm\bin;%PATH%
set DEVECO_SDK_HOME=<DevEco>\sdk
node "<DevEco>\tools\hvigor\bin\hvigorw.js" ...
```

## 命令行构建

```cmd
build.cmd
```

- 产物：`entry\build\default\outputs\default\entry-default-unsigned.hap`（**未签名**）
- 全量重建：`build.cmd clean assembleHap`
- 安装到模拟器/真机前需签名：DevEco Studio → `File > Project Structure > Signing Configs` → 勾选 Automatically generate signature

## 常见问题（已踩过的坑）

### 1. 双击 build.cmd 报「'CLI' 不是内部或外部命令」等一堆乱码错误

原因：批处理文件被保存成 **LF 行结尾 + UTF-8 中文**，cmd.exe 无法正确解析。
要求：`build.cmd` 必须是 **CRLF 行结尾**。从 Git 检出或被编辑器改成 LF 后会复发，转换方法：

```bash
sed -i 's/$/\r/' build.cmd   # Git Bash 下执行
```

### 2. 模拟器上应用一启动就闪退

原因：模拟器等无 HMS DataAugmentationKit 原生实现的设备上 `rag.ChatLLM` 为 `undefined`，
模块加载期执行 `class X extends rag.ChatLLM` 会抛 TypeError 拖垮整个应用。

修复（`entry/src/main/ets/knowledge/RagBridge.ets`）：类声明放入工厂函数
`createCloudChatLlm()` 内延迟执行，`extends` 只在 `kitAvailable()` 检查通过后求值；
`localAnswer()` 入口也加了同样的能力检查，不可用时抛出可读错误而不是崩溃。

注意：不要写成 `rag.ChatLLM ?? class {}` 这种空基类兜底——ArkTS 不允许类表达式
（`arkts-no-class-literals`），且抽象基类会让 `new` 处报编译错误。

### 3. 编译报大量 `ArkTS:WARN Function may throw exceptions`

是既有代码的静态检查提示，不影响构建产物，可暂不处理。

## 验证状态

- `build.cmd` 命令行构建：**BUILD SUCCESSFUL**（33 tasks，产物约 840 KB）
- 签名安装与真机/模拟器运行验证：待进行
