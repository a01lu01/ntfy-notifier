# ntfy-Notifier 界面设计规格（Tauri 版）

适用版本：1.0.0（Rust/Tauri 前端）

本文档描述当前界面的字体、颜色、组件尺寸与布局配置。所有 `px` 均为 CSS 逻辑像素，Windows 系统缩放（如 125%、150%）由 WebView2 自动换算，不需要在代码里做二次适配。

## 1. 字体

### 1.1 字体族

```css
font-family: "HarmonyOS Sans SC", "Segoe UI", system-ui, sans-serif;
```

- 首选：`HarmonyOS Sans SC`，字体文件打包在 `tauri/public/assets/fonts/HarmonyOS_Sans_Regular.ttf`。
- 回退：系统 `Segoe UI` → 通用 `system-ui` → 无衬线字体。
- 所有 `button / input / select / textarea` 均显式继承 `font-family: inherit`，保证全局字体一致。

### 1.2 字号与字重

| 用途 | 字号 | 字重 |
| --- | --- | --- |
| 全局基础字号（html/body） | 14px | 常规 |
| 页面标题 `.page-title` | 20px | 600 |
| 页面副标题 `.page-subtitle` | 13px | 常规 |
| 侧边导航 `.nav-item` | 14px（继承） | 常规 |
| 卡片标题 `.card h3` | 14px | 600 |
| 输入框 `input / select` | 14px | 常规 |
| 表单标签 `.field label` | 13px | 常规 |
| 普通按钮 `.btn` | 13px | 常规 |
| 分段按钮 `.seg-btn` | 13px | 常规 |
| 设置项文字 `.switch-row` | 14px | 常规 |
| 表格 `table` | 13px | 常规 |
| 表头 `th` | 13px | 600 |
| 空状态 `.empty` | 14px（继承） | 常规 |

## 2. 颜色主题

颜色通过 CSS 变量定义在 `styles.css` 的 `:root`（浅色）与 `html[data-theme="dark"]`（深色）中，由设置页的“界面主题”切换：

| 变量 | 浅色 | 深色 |
| --- | --- | --- |
| `--window-bg` 窗口背景 | `#f3f3f3` | `#202020` |
| `--card-bg` 卡片/输入背景 | `#ffffff` | `#2b2b2b` |
| `--text` 主文字 | `#1f1f1f` | `#ffffff` |
| `--subtext` 次要文字 | `#5d5d5d` | `#c7c7c7` |
| `--hover` 悬停背景 | `#e9e9e9` | `#333333` |
| `--selected` 选中/滚动条 | `#e3e3e3` | `#3a3a3a` |
| `--accent` 主题色 | `#6c357c` | `#7d3d8e` |
| `--accent-text` 主题色文字/链接 | `#6c357c` | `#a96bb8` |
| `--border` 边框 | `#e0e0e0` | `#3c3c3c` |
| `--input-border` 输入框边框 | `#c9c9c9` | `#4a4a4a` |
| `--danger` 危险色 | `#c42b1c` | `#ff99a0` |

## 3. 组件尺寸

### 3.1 按钮 `.btn`

按钮**没有固定的宽高**，宽度由文字内容加水平内边距决定，高度由字号与内边距决定。

| 属性 | 值 |
| --- | --- |
| 内边距 | `7px 16px` |
| 字号 | 13px |
| 圆角 | 5px |
| 边框 | 无（主按钮）；`1px solid var(--border)`（次按钮） |
| 近似高度 | 约 30–32px（按浏览器默认行高估算） |

变体：

- `.btn-primary`：背景 `--accent`，白色文字，悬停 `brightness(1.08)`。
- `.btn-secondary`：背景 `--card-bg`，主文字色，带边框，悬停 `--hover`。

当前使用位置：推送页“刷新/清空”、设置页“取消/保存”。

### 3.2 分段按钮 `.seg-btn`

用于设置页“界面主题”的“跟随系统 / 浅色 / 深色”三连按钮。

| 属性 | 值 |
| --- | --- |
| 内边距 | `7px 14px` |
| 字号 | 13px |
| 容器圆角 | 6px（外层 `.segmented`） |
| 按钮间分隔 | `1px solid var(--input-border)`（相邻按钮左边框） |
| 选中态 | 背景 `--accent`，白色文字 |

### 3.3 输入框与下拉框

服务器地址、用户名、密码、主题均为**通栏输入框**（`width: 100%`）。

| 属性 | 值 |
| --- | --- |
| 宽度 | 100%（按所在字段宽度） |
| 内边距 | `7px 10px` |
| 字号 | 14px |
| 圆角 | 5px |
| 边框 | `1px solid var(--input-border)` |
| 聚焦态 | 边框变为 `--accent` |
| 近似高度 | 约 33–35px |

用户名/密码在 `.row` 中并排时，每个字段 `flex: 1` 平分宽度。

### 3.4 开关 `.switch`

用于“开机自启动”和“自动复制验证码”：

| 属性 | 值 |
| --- | --- |
| 开关尺寸 | 40 × 22px |
| 圆角 | 11px |
| 滑块尺寸 | 18 × 18px |
| 滑块位置 | 距左 2px、距顶 2px |
| 选中位移 | `translateX(18px)` |
| 选中背景 | `--accent` |
| 动画 | 背景 0.2s 过渡 |

### 3.5 侧边导航

| 属性 | 值 |
| --- | --- |
| 侧栏宽度 | 180px |
| 侧栏内边距 | 8px |
| 导航项内边距 | `9px 12px` |
| 导航项圆角 | 6px |
| 导航项间距 | 2px |
| 选中态 | 背景 `--selected`，文字 `--accent-text`，左侧 3px 主题色指示条 |

### 3.6 卡片 `.card`

| 属性 | 值 |
| --- | --- |
| 内边距 | 16px |
| 圆角 | 8px |
| 边框 | `1px solid var(--border)` |
| 卡片间距 | 下方 14px |

### 3.7 表格

| 属性 | 值 |
| --- | --- |
| 表头 `th` 内边距 | `8px 10px` |
| 表体 `td` 内边距 | `7px 10px` |
| 表格字号 | 13px |
| 表头字重 | 600 |
| 表头行为 | `position: sticky`，滚动时固定 |
| 单元格文本 | 单行省略（`ellipsis`），超出隐藏 |
| 表头鼠标样式 | `grab` |
| 表格外层 | 最大高度 `calc(100vh - 150px)`，可滚动 |

列宽默认值（`ui_state.rs`）：

| 列 | 默认宽度 | 最小宽度 |
| --- | --- | --- |
| 时间 `time` | 180px | 120px |
| 标题 `title` | 220px | 80px |
| 内容 `message` | 640px | 160px |

实际列宽会保存到 `%APPDATA%\ntfy-notifier\ui_state.json`，用户拖宽后以保存值为准。前端按各列最小宽度限制拖宽（与上表一致），后端再做一次最小宽度兜底。

### 3.8 滚动条

| 属性 | 值 |
| --- | --- |
| 宽度/高度 | 8px |
| 轨道 | 透明 |
| 滑块 | 背景 `--selected`，圆角 4px |
| 滑块悬停 | 背景 `--accent` |

## 4. 窗口与布局

窗口尺寸定义在 `tauri/src-tauri/tauri.conf.json`：

| 属性 | 值 |
| --- | --- |
| 默认宽高 | 1350 × 800 |
| 最小宽高 | 900 × 500 |
| 居中 | 是 |
| 主内容内边距 | `20px 28px` |

整体布局为左侧固定 180px 导航 + 右侧弹性内容区：

```text
┌────────────┬──────────────────────────────┐
│ 导航 180px  │  内容区（flex: 1）             │
│ 推送        │  页面标题 → 工具栏 → 卡片/表格   │
│ 设置        │                              │
│ 关于        │                              │
└────────────┴──────────────────────────────┘
```

## 5. 表格交互

- 拖拽表头排序：使用 SortableJS，配置 `forceFallback` + `fallbackOnBody`，避免 WebView2 原生拖拽不稳定。
- 调整列宽：每列表头右侧有 8px 宽的 `.resize-handle`，内部有一条 1px 分隔线，悬停/拖动时变为主题色。拖动分界线时采用**相邻列补偿**：只改变当前列与右侧相邻列（一增一减，总宽不变），其他列不动；拖到最小宽度时自动停止。
- 列顺序与列宽都会保存到 `ui_state.json`，下次启动恢复。

## 6. 修改入口

| 内容 | 文件 |
| --- | --- |
| 字体、颜色变量、所有组件尺寸 | `tauri/src/styles.css` |
| 页面结构、按钮/输入框/开关的 HTML 结构 | `tauri/src/main.js` |
| 列默认宽度、最小宽度 | `tauri/src-tauri/src/ui_state.rs` |
| 窗口默认尺寸 | `tauri/src-tauri/tauri.conf.json` |
| 字体文件 | `tauri/public/assets/fonts/HarmonyOS_Sans_Regular.ttf` |
