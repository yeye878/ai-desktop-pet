# 系统设置页 · 左侧栏子项导航（v2）

日期：2026-11-13

## 改动

v1 是「一页三分类 + 顶部锚点跳转」，用户反馈要改成**左边栏子项目**那样。
现在和「对话互动」的分组交互完全一致。

### 侧栏

```
系统设置  ⌄          ← 父项，点文字进页面，点箭头展开/收起
  ▸ 对话设定
  ▸ AI 连接
  ▸ 服务与提醒
```

- 父项带展开箭头（`.sidebar-item-caret`），展开状态会记住
- 三个子项用 `.sidebar-subitem`，和 agent 对话窗口的子项同一套样式
- 当前选中的子项高亮，父项同时保持 active

### 内容区

一次只渲染一个分类。原来三组卡片连续排在一页（要滚 500 多行），现在按
`systemSection` 条件渲染，进入子项后只看得到该类内容。

页头保留当前分类的标题行（图标 + kicker + 标题 + 说明），
让用户始终知道自己在哪里。

### 记住选择

分类写到 `localStorage["dash.system-section"]`，刷新或下次打开应用都回到上次那一类。

## 实现要点

| 项 | 做法 |
| --- | --- |
| 状态 | `systemSection` ref + `SYSTEM_SECTIONS` 常量表（id/label/kicker/icon/desc 单一来源） |
| 侧栏 | 复用现有 `.sidebar-group` / `.sidebar-item-parent` / `.sidebar-subnav` 样式，不新增组件 |
| 内容区 | 三个 `<template v-if="systemSection === '...'">` 包裹原卡片 |
| 持久化 | `watch(systemSection)` 写 localStorage，初值经 `readStoredSystemSection()` 校验合法性 |
| 清理 | 删掉了 v1 的 `scrollToLabSection()` 与 `.lab-jump` 锚点样式 |

## 验证

浏览器实测（1320×900），逐项断言：

- 侧栏子项 3 个且顺序为 `对话设定 / AI 连接 / 服务与提醒` ✓
- 点「对话设定」：当前标题=对话设定、侧栏激活子项=对话设定、**可见卡片正好 4 张** ✓
- 点「AI 连接」：标题更新、**可见卡片正好 1 张** ✓
- 点「服务与提醒」：标题更新、**可见卡片正好 1 张** ✓
- 刷新后 `localStorage` 记住 `service`，再点父项进入仍是「服务与提醒」 ✓
- 点箭头折叠后子项不可见 ✓

「可见卡片数」是关键断言：它同时验证了「只渲染当前分类」和「没有漏掉卡片」。

回归：`verify:palette` 12/12、`verify:chat-backgrounds` 13/13、
multi-agent / chat-sessions / agent-activity / pending-interactions / markdown /
custom-pixel-bg 全部 fail 0；交互回归、12 个主题、6 个背景浏览器实测通过；
`vue-tsc` 与 `vite build` 通过。

## 截图

- `nav-conversation.png` — 对话设定（4 张卡片）
- `nav-connection.png` — AI 连接（1 张卡片）
- `nav-service.png` — 服务与提醒（1 张卡片）
