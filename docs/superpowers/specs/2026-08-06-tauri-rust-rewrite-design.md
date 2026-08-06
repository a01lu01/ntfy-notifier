# ntfy-Notifier Tauri/Rust 重写设计

- 日期：2026-08-06
- 状态：待用户审阅
- 范围：用 Tauri 2 + Rust 全量重写 Windows 桌面客户端，功能 1:1 复刻现有 Python 版，v3 视觉落地

## 1. 已确认决策

| 决策点 | 选择 |
| --- | --- |
| 技术路线 | Tauri 2 + 全 Rust 后端（不用 Python 子进程） |
| 前端栈 | 原生 HTML/CSS/JS + Vite，直接采用 v3 页面结构 |
| 平台范围 | 第一版仅 Windows（WebView2 系统自带） |
| 发布形态 | 便携版 exe + NSIS 安装包 |
| 功能范围 | 完全复刻现有功能，不加不减 |
| 代码组织 | 同一仓库新建 `tauri/` 子目录，Python 版保留至功能对齐 |

## 2. 目标

- 用 Tauri 2 重建 ntfy-Notifier：单主窗口 + 侧边栏（推送 / 设置 / 关于）
- 完整还原 v3 视觉（纯色主题、深浅色跟随系统 + 手动覆盖、HarmonyOS Sans）
- 功能与现有 Python 版 1:1 对齐：SSE 订阅、Windows Toast、自动复制验证码、推送历史（SQLite 1000 条）、表格列记忆、托盘、单实例、开机自启、AUMID 通知图标
- 数据无缝兼容：继续使用现有 `%APPDATA%\ntfy-notifier\` 下 config.json / history.db / ui_state.json，无需迁移工具

## 3. 总体架构与目录

```
ntfy-notifier/
├── src/                  # 现有 Python 版（保留，直至验收通过）
├── tauri/
│   ├── frontend/         # Vite + 原生 HTML/CSS/JS
│   │   ├── index.html
│   │   ├── src/
│   │   │   ├── main.js
│   │   │   ├── theme.css / theme.js
│   │   │   ├── pages/push.js / settings.js / about.js
│   │   │   └── table.js          # 列拖拽/列宽记忆
│   │   └── assets/fonts/HarmonyOS_Sans_Regular.ttf
│   └── src-tauri/
│       ├── tauri.conf.json
│       ├── icons/                 # 复用 connected/disconnected 图标
│       └── src/
│           ├── main.rs            # 窗口、托盘、单实例、事件分发
│           ├── config.rs          # 配置读写 + DPAPI
│           ├── history.rs         # SQLite 历史
│           ├── ui_state.rs        # 表格列状态
│           ├── ntfy.rs            # SSE 订阅
│           ├── notify.rs          # Windows Toast + 降级
│           ├── clipboard.rs       # 验证码提取 + 复制
│           └── startup.rs         # 开机自启、AUMID
```

- 前端与后端只通过 Tauri 命令（`invoke`）和事件（`emit/listen`）通信。
- Tauri 2 插件：`tauri-plugin-single-instance`；WebView2 由系统提供。

## 4. 前端设计

- 页面：推送（默认页）、设置、关于；左侧纯文字导航，选中项强调色左条 + 底色。
- 窗口：1350×800，最小 900×500，居中；关闭窗口 = 隐藏到托盘。
- 主题：CSS 变量承载 v3 两套 token（浅 #F3F3F3/#FFFFFF/#6C357C；深 #202020/#2B2B2B/#7D3D8E）；`prefers-color-scheme` 监听系统切换；手动覆盖通过 `data-theme` 生效；主题模式存 `theme_mode`（system/light/dark）。
- 字体：HarmonyOS Sans SC 通过 `@font-face` 内嵌，全局统一 `font-family`，回退 Segoe UI / system-ui。
- 推送页：工具栏（刷新/清空+二次确认）、表格（时间/标题/内容）、双击复制、空状态提示、列拖拽排序与列宽调整（状态经命令持久化）。
- 设置页：连接/行为两张卡片；服务器、用户名+密码（文字按钮显隐）、主题、界面主题三选、开机自启、自动复制验证码；保存前 HTTPS 明文安全提示；取消恢复最近保存值。
- 关于页：应用名、版本号、GitHub 链接。

## 5. Rust 后端模块

### config.rs
- 路径：`%APPDATA%\ntfy-notifier\config.json`
- 密码兼容：旧版 `password_encrypted` 为 base64(DPAPI blob)，用 Windows DPAPI（`CryptUnprotectData`）解密，同一用户可直接读
- 保存：临时文件 + `os::rename` 原子替换；不落明文
- 损坏：改名备份 `config.json.corrupt-时间戳`，重置默认并提示
- 默认值：server 空、username 空、topic sms、theme_mode system、auto_start false、auto_copy_otp false

### history.rs
- 路径：`%APPDATA%\ntfy-notifier\history.db`（WAL）
- 表结构与现有一致：`messages(id TEXT PK, received_at TEXT, topic TEXT, title TEXT, message TEXT)`
- `record_message`：INSERT OR IGNORE 去重；插入后删除超出 1000 条的最旧记录
- 命令：`get_messages(limit=1000)`、`clear_history()`

### ui_state.rs
- 路径：`%APPDATA%\ntfy-notifier\ui_state.json`
- 列顺序/列宽读写；最小宽度约束（时间 120 / 标题 80 / 内容 160）；损坏回退默认

### ntfy.rs
- `reqwest` 流式 GET `{server}/{topic}/sse`，Basic 认证，`stream=true`
- 超时：连接 10s、读取 90s；指数退避 5s→300s；健康检查 60s 间隔 / 120s 无数据主动重连
- 收到 `event: open` 视为连接成功；`event: message` 先 `record_message`（重复则跳过），再发通知
- 连接状态变化通过 Tauri 事件通知前端/托盘图标

### notify.rs
- 首选 Windows Toast（`winrt-notification`，AUMID `ntfy-Notifier`），失败降级 MessageBox（`windows` crate），最后 stderr
- 自动复制验证码：Rust 实现"纯数字 4-8 位 + 关键词优先"提取（与 Python 版一致）

### clipboard.rs
- `arboard` 写入；独立线程执行，失败重试 3 次

### tray / startup / main
- `tray-icon`：菜单 推送（默认动作）/ 设置 / 退出；连接状态绿/红图标
- 开机自启：HKCU Run 写 exe 路径
- 单实例：`tauri-plugin-single-instance`，重复启动聚焦已有窗口
- AUMID：首次运行创建开始菜单快捷方式并写入 `System.AppUserModel.ID`

## 6. 延迟说明

- SSE 长连接端到端延迟预估 50～200ms（国内网络 100ms 内常见），瓶颈为网络 RTT
- 通知弹出感知时间 0.5～1s 属操作系统调度，非网络延迟

## 7. 打包与发布

- `cargo tauri build` 产出 NSIS 安装包 + 便携版 exe
- Win11 无需额外安装 WebView2；Win10 由安装包引导
- 图标复用现有 connected/disconnected 素材并生成 Tauri 所需图标集
- 数据目录与 Python 版共用，切换无感

## 8. 测试与验收

### Rust 单元测试（cargo test）
- 配置 DPAPI 加密/解密往返、旧明文迁移、损坏备份
- 历史去重、1000 条裁剪、清空
- 验证码提取用例（含 "G-000000是您的Google验证码" → 000000）
- 列状态读写、最小宽度、损坏回退

### 手工验收（与 Python 版逐项对照）
- 主题跟随系统/手动覆盖即时生效
- 125% / 150% 缩放下清晰
- 表格拖列排序、调宽、重启恢复
- 托盘：推送（默认动作）/设置/退出、关窗驻留
- 通知弹出、验证码自动复制
- 单实例、开机自启、首次运行配置迁移
- 推送历史与旧数据完整可见

## 9. 实施阶段

1. Tauri 2 脚手架 + 目录结构 + 打包配置
2. Rust 后端：config → history → ui_state → ntfy → notify → clipboard → tray/startup
3. 前端：v3 三页面 + 主题 + 表格交互
4. 前后端联调（命令、事件、托盘联动）
5. 打包（便携 + NSIS）与验收

## 10. 明确不做

- 跨平台（macOS/Linux）
- Mica / 亚克力背景（v3 为纯色）
- 自动更新机制
- 新功能（搜索、导出等）
- 验收通过前停用 Python 版
