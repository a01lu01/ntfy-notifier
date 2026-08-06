# ntfy-Notifier Fluent UI 改造设计

- 日期：2026-08-06
- 状态：待用户审阅
- 范围：主窗口 UI 重构（Fluent 风格 + 深浅色主题 + 托盘交互调整 + 表格列记忆）

## 1. 目标与范围

当前 Tkinter 界面观感偏旧（近似 Win7），不随系统深浅色变化，托盘入口也没有直达页面。本次改造的目标：

- 采用 Windows 11 Fluent 视觉风格（纯色背景，不使用 Mica/毛玻璃）
- 跟随系统深浅色，并支持在设置页手动覆盖
- 单主窗口 + 左侧导航，托盘点击直达“推送”页
- 正确适配 Windows 显示缩放（Per-Monitor V2）
- 推送表格支持拖动列排序、调整列宽，并本地记忆

## 2. 设计决策总览

| 主题 | 决策 |
| --- | --- |
| 技术路线 | Tkinter + pywinstyles（新增依赖，随 exe 打包） |
| 布局 | 单主窗口，左侧导航（推送 / 设置 / 关于）+ 右侧内容区 |
| 窗口结构 | 托盘点击/双击打开主窗口；关闭窗口 = 最小化到托盘；退出仅走托盘菜单 |
| 主题策略 | 跟随系统 + 手动覆盖三选（跟随系统 / 浅色 / 深色） |
| 背景质感 | 纯色背景，不使用 Mica/毛玻璃 |
| 图标 | 不使用 emoji/图标，导航为纯文字 |
| 强调色 | 浅色 #6C357C；深色 #7D3D8E（深色强调文字 #A96BB8） |
| 名称口径 | “推送历史”统一简称为“推送” |

## 3. 视觉与主题系统

### 3.1 色彩 token

| token | 浅色 | 深色 |
| --- | --- | --- |
| 窗口底色 | #F3F3F3 | #202020 |
| 卡片/内容区 | #FFFFFF | #2B2B2B |
| 正文 | #1F1F1F | #FFFFFF |
| 次要文字 | #5D5D5D | #C7C7C7 |
| 悬停 | #E9E9E9 | #333333 |
| 选中 | #E3E3E3 | #3A3A3A |
| 强调色 | #6C357C | #7D3D8E |
| 强调文字 | #6C357C | #A96BB8 |
| 边框 | #E0E0E0 | #3C3C3C |
| 输入框边框 | #C9C9C9 | #4A4A4A |

强调色说明：深色主题按用户指定使用 #7D3D8E；浅色主题使用同色系加深的 #6C357C，保证白色背景上的对比度。

### 3.2 字体与字号

- 优先 `Segoe UI Variable`，回退 `Segoe UI`
- 页面标题 20px（逻辑），正文 14px（逻辑），次要/说明文字 12px（逻辑）
- 字号随 DPI 缩放因子同步放大，保证 125%/150% 缩放下不偏小、不发虚

### 3.3 pywinstyles 用法

- `pywinstyles.apply_style(root, "win11")`：Win11 原生圆角窗口框 + 标题栏
- `pywinstyles.apply_style(root, "dark" / "light")`：标题栏明暗与主题同步
- `pywinstyles.change_header_color(root, 颜色)`：可选，让标题栏融入内容区颜色
- 背景一律用纯色 token，不调用 mica/acrylic

### 3.4 主题引擎与跟随系统

- 新增 `ThemeManager`，内置 LIGHT / DARK 两套 token
- 主题模式存 `config.json` 的 `theme_mode`：`"system"` / `"light"` / `"dark"`，默认 `"system"`
- 跟随系统：读取注册表 `AppsUseLightTheme`（0 = 深色，1 = 浅色）与 `AccentColor`（仅作参考，本项目强调色固定）
- 系统主题变化时即时应用：后台定时器轮询注册表（约 2 秒），变化后通过现有线程安全队列通知主线程
- `ThemeManager.apply()` 负责：重建 ttk 样式、遍历页面更新颜色、同步 pywinstyles 标题栏

## 4. 主窗口与页面

### 4.1 主窗口尺寸与 DPI

- 默认尺寸 1350×800（逻辑单位，参照 optimizerDuck），最小 900×500，启动居中
- 启动时声明 Per-Monitor V2 感知：优先 `SetProcessDpiAwarenessContext(-4)`，失败降级 `SetProcessDpiAwareness(2)` / `SetProcessDPIAware()`
- 用 `GetDpiForWindow / 96` 计算缩放因子，同步设置 `tk scaling`，窗口与控件尺寸乘以缩放因子
- exe 清单同时声明 `dpiAwareness = PerMonitorV2, PerMonitor`
- 窗口关闭（WM_DELETE_WINDOW）= 隐藏到托盘，程序常驻

### 4.2 推送页（默认页）

- 顶部工具栏：刷新 / 清空（清空需二次确认）
- 三列表格：时间 / 标题 / 内容，带垂直与水平滚动条
- 双击任意行复制消息正文；最多保留 1000 条
- 拖动列标题调整列顺序、拖动列边缘调整列宽，自动记忆；界面不显示拖拽说明文字
- 空列表时显示空状态提示（如“暂无推送”）
- 进入设置走左侧导航，页面内不再单独放“设置”按钮

### 4.3 设置页

- 服务器地址
- 用户名 + 密码（并排；密码“显示 / 隐藏”用文字按钮，不使用 emoji 眼睛）
- 主题（订阅频道）
- 界面主题：跟随系统 / 浅色 / 深色（分段选择）
- 开机自启动（勾选）
- 收到短信时自动复制验证码（勾选）
- 底部：保存 / 取消
- 保存时沿用现有行为：HTTP 地址 + 密码非空时弹 HTTPS 安全提醒；保存成功后即时应用主题并重启订阅器

### 4.4 关于页

- 应用名 ntfy-Notifier、版本号、一句话描述、GitHub 仓库链接

### 4.5 页面切换与托盘联动

- 左侧导航三个纯文字项：推送 / 设置 / 关于；选中项用强调色左条 + 选中底色标识
- 页面切换为同一个 Toplevel 内的 Frame 切换，不再打开独立历史/设置窗口
- 托盘点击/双击默认打开主窗口并停在“推送”页；右键菜单“设置”直接切到设置页

## 5. 托盘交互

- 托盘菜单三项：推送 / 设置 / 退出
- “推送”为默认动作：Windows 双击（部分系统单击）托盘图标触发
- pystray 版本锁定 0.19.5；对私有接口（`_message_handlers` / `_hwnd`）保留 `hasattr` 防御，缺失时回退公开 setter

## 6. 表格列记忆

- 存储位置：`%APPDATA%\ntfy-notifier\ui_state.json`（与连接配置分开）
- 内容：`{"column_order": ["time", "title", "message"], "column_widths": {"time": 180, "title": 220, "message": 640}}`
- 写入：鼠标释放时保存，采用临时文件 + `os.replace` 原子写入
- 读取：启动/打开推送页时恢复；文件损坏或字段缺失时回退默认列配置并重建
- 最小列宽：时间 120 / 标题 80 / 内容 160（逻辑单位），防止拖没

## 7. 组件划分

| 文件 | 职责 |
| --- | --- |
| `src/theme.py`（新增） | 色彩 token、`ThemeManager`（解析主题模式、应用样式、系统主题监听） |
| `src/ui_state.py`（新增） | 表格列顺序/宽度读写（`ColumnStateStore`） |
| `src/ui.py`（重构） | 主窗口 + 侧边栏 + 推送/设置/关于三个页面 Frame；移除独立 Toplevel 窗口逻辑 |
| `src/tray.py`（调整） | 默认动作与右键菜单（推送/设置/退出） |
| `src/ntfy_notifier.py`（调整） | 接线：托盘回调 → 页面切换，关闭窗口 → 最小化托盘，主题/配置保存 → 即时生效 |
| `tests/test_theme.py`、`tests/test_ui_state.py`（新增） | 单元测试 |

## 8. 数据流

- 主题切换：设置页 → 保存 `theme_mode` → `ThemeManager.apply()` 即时生效，无需重启
- 系统主题变化：注册表轮询线程 → 队列 → 主线程 → `ThemeManager.apply()`
- 列操作：Treeview 列头拖拽（移动/调宽）→ 释放 → `ColumnStateStore.save()` → 下次启动恢复
- 托盘：点击 → 主窗口显示 + 切“推送”页；右键“设置” → 切设置页；右键“退出” → 保存并退出

## 9. 错误处理与兼容

- pywinstyles 不可用或调用失败：保留原生 Tk 样式，程序功能不受影响
- DPI API 不可用：按 100% 缩放处理
- `ui_state.json` 损坏：回退默认列配置，不阻断启动
- 注册表读取失败：按浅色处理
- 旧配置兼容：已存字段全部保留，仅新增 `theme_mode`（默认 `"system"`）

## 10. 测试计划

单元测试（`python -m unittest discover -s tests -v`）：

- 主题 token 完整性（两套 token 覆盖全部键）
- `resolve_theme("system" / "light" / "dark")` 三种模式结果正确
- `ColumnStateStore`：读写往返、损坏文件回退、列宽 clamp、顺序恢复
- 托盘菜单项构造与默认动作设置

手动验收：

- 125% / 150% 缩放下文字与控件清晰、比例正常
- 系统深浅色切换后程序即时跟随；手动覆盖后不跟随
- 拖动列、调整宽度后重启程序恢复
- 托盘点击打开推送页、右键设置/推送/退出三个入口正常
- 关闭窗口程序仍在托盘；退出仅走托盘菜单

## 11. 打包

- `requirements.txt` 增加 pywinstyles（锁定可复现版本）
- `ntfy-Notifier.spec` 增加 DPI 清单声明（若 PyInstaller 支持），运行时 ctypes 声明作为兜底
- 重新打包后运行诊断 exe 验证界面与托盘

## 12. 明确不做

- Mica / 亚克力 / 毛玻璃背景
- 主窗口位置与大小的记忆（保持固定默认尺寸 + 居中）
- emoji / 图标装饰
- 独立的历史/设置 Toplevel 窗口（全部并入主窗口页面）
- 服务器端或通知逻辑的任何改动
