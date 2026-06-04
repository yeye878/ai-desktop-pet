# 呆猫八条形象修改计划

## 背景
用户反馈呆猫八条形象中的视频处理存在问题，需要暂时删除视频功能，同时延长图片切换时间。

## 当前实现分析
在 `src/components/PetCanvas.vue` 中：
1. **媒体数组**：`daimaoMedia` 包含图片和视频（第47-50行）
2. **视频处理**：`ensureDaimaoVideo()` 函数创建视频元素（第288-298行）
3. **视频绘制**：`drawDaimaoVideo()` 函数处理视频绘制（第315-352行）
4. **媒体选择**：`pickDaimaoMedia()` 函数随机选择媒体（第300-313行）
5. **主绘制函数**：`drawDaimaoPet()` 函数处理绘制逻辑（第354-379行）

## 修改方案

### 1. 注释掉视频处理功能
**目标文件**：`src/components/PetCanvas.vue`

**修改内容**：
- 注释掉 `daimaoMedia` 数组中的视频项（第49行）
- 注释掉 `ensureDaimaoVideo()` 函数（第288-298行）
- 注释掉 `drawDaimaoVideo()` 函数（第315-352行）
- 在 `drawDaimaoPet()` 函数中注释掉视频相关逻辑（第368-372行）

### 2. 延长图片切换时间
**目标文件**：`src/components/PetCanvas.vue`

**修改内容**：
- 修改 `pickDaimaoMedia()` 函数中的切换时间计算（第309行）
- 当前：`240 + Math.floor(Math.random() * 240)` 帧（约4-8秒，假设60fps）
- 修改为：`480 + Math.floor(Math.random() * 480)` 帧（约8-16秒）

## 验证方法
1. 运行开发服务器：`npm run tauri dev`
2. 切换到呆猫八条形象
3. 观察图片切换是否正常，无视频播放
4. 确认切换时间是否延长
5. 构建生产版本并测试：`npm run tauri build`

## 风险评估
- **低风险**：仅删除视频功能，不影响图片显示
- **无依赖**：视频功能独立，删除不影响其他功能
- **可恢复**：代码可从版本控制恢复