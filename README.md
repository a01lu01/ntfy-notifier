# ntfy-Notifier

Windows 系统托盘工具，订阅 ntfy 消息并弹出系统通知。

当前版本基于 **Tauri 2 + Rust + 原生 HTML/CSS/JS**，数据继续保存在 `%APPDATA%\ntfy-notifier\` 下，与早期版本兼容。

## 功能

- 系统托盘常驻，关闭窗口隐藏到托盘
- 单主窗口：推送 / 设置 / 关于
- SSE 实时订阅，无轮询延迟
- Windows Toast 系统通知
- 推送历史：SQLite 本地保存最近 1000 条，表格支持拖动列排序、调整列宽并记忆
- 收到验证码消息时自动复制 4-8 位纯数字验证码到剪贴板
- 明/暗主题：跟随系统或手动切换
- 开机自启、单实例运行
- 密码使用 Windows DPAPI 加密后保存

## 下载

发布版可在 GitHub Releases 页面获取：

<https://github.com/a01lu01/ntfy-notifier/releases>

- `ntfy-notifier.exe`：便携版，直接运行
- `ntfy-Notifier_1.0.0_x64-setup.exe`：NSIS 安装包

## 开发环境

- Windows 10/11（系统自带 WebView2）
- Node.js 20+
- Rust stable（含 Cargo）

## 开发运行

```powershell
cd tauri
npm install
npm run tauri dev
```

## 打包

```powershell
cd tauri
npm run tauri build
```

产物：

- 便携版：`tauri/src-tauri/target/release/ntfy-notifier.exe`
- 安装包：`tauri/src-tauri/target/release/bundle/nsis/ntfy-Notifier_1.0.0_x64-setup.exe`

## 配置

首次运行后在“设置”页填写：

| 字段 | 说明 |
| --- | --- |
| 服务器地址 | ntfy 服务器地址，推荐使用 `https://` |
| 用户名 | ntfy 用户名（可选） |
| 密码 | ntfy 密码（可选） |
| 主题 | 订阅的话题名，如 `sms` |
| 界面主题 | 跟随系统 / 浅色 / 深色 |
| 开机自启 | 是否随 Windows 启动 |
| 自动复制验证码 | 收到验证码时自动复制到剪贴板 |

数据文件位于 `%APPDATA%\ntfy-notifier\`：

| 文件 | 用途 |
| --- | --- |
| `config.json` | 配置，密码以 DPAPI 加密保存 |
| `history.db` | SQLite 推送历史（最近 1000 条） |
| `ui_state.json` | 表格列顺序与列宽记忆 |

## 测试

```powershell
# 前端单元测试
cd tauri
npm test

# 后端 Rust 测试
cd tauri/src-tauri
cargo test
```

## 项目结构

```text
ntfy-notifier/
├── tauri/
│   ├── src/                  # 前端：HTML/CSS/JS
│   ├── public/assets/fonts/  # 打包使用的字体
│   ├── tests/                # 前端单元测试
│   ├── src-tauri/            # Rust 后端与打包配置
│   │   ├── src/              # 配置、订阅、历史、通知等模块
│   │   └── tauri.conf.json   # 窗口与打包配置
│   └── package.json
├── docs/
│   └── ui-design.md          # 界面设计规格（字体/颜色/尺寸）
└── README.md
```

## 历史说明

仓库根目录下的 `src/`、`tests/` 等为早期 Python 版代码，仅作历史保留，当前版本不再维护。旧文档已随 Tauri 版落地移除，当前设计规格见 `docs/ui-design.md`。

