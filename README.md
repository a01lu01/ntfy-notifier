# ntfy-Notifier

Windows 系统托盘 / Android 常驻通知工具，订阅 ntfy 消息并弹出系统通知。

当前版本基于 **Tauri 2 + Rust + 原生 HTML/CSS/JS**，数据继续保存在 `%APPDATA%\ntfy-notifier\` 下，与早期版本兼容。

## 功能

- 系统托盘常驻，关闭窗口隐藏到托盘（Windows）
- 托盘菜单：左键/右键均可呼出，双击托盘图标直接打开推送页（Windows）
- 单主窗口：推送 / 规则 / 设置 / 关于
- SSE 实时订阅，无轮询延迟
- Windows Toast 系统通知；Android 前台服务常驻通知栏
- Android 通知栏常驻"最新推送"，每条新消息高优先级提醒（声音/震动）
- 验证码"一键输入"：通知栏"复制验证码"按钮 + 收到验证码自动复制到剪贴板（由"规则"页驱动）
- 推送历史：SQLite 本地保存最近 1000 条，表格支持拖动列排序、调整列宽并记忆
- 可配置验证码规则：关键词、位数、匹配模式、优先级、启用状态
- 明/暗主题：跟随系统或手动切换
- 开机自启（Windows 注册表 / Android Boot 广播）、单实例运行（Windows）
- 密码使用 Windows DPAPI 加密后保存
- Android 窄屏适配：竖屏/窄屏自动切换底部导航栏，推送历史表格可横向滑动

## 下载

发布版可在 GitHub Releases 页面获取：

<https://github.com/a01lu01/ntfy-notifier/releases>

- `ntfy-notifier.exe`：便携版，直接运行
- `ntfy-Notifier_1.0.0_x64-setup.exe`：NSIS 安装包

## 开发环境

- Windows 10/11（系统自带 WebView2）
- Node.js 20+
- Rust stable（含 Cargo）

### Android 额外要求

- JDK 17+（如 Temurin 21）
- Android SDK / NDK（`ANDROID_HOME` 环境变量指向 SDK；需要 platform-tools、platforms;android-36、build-tools;36.0.0、NDK 27）
- rustup 安卓目标：

```powershell
rustup target add aarch64-linux-android x86_64-linux-android
```

- 首次运行 `npx tauri android init` 生成 `src-tauri/gen/android/`（已提交到仓库）
- Windows 下需要开启"开发者模式"（Tauri CLI 需创建符号链接）
- 构建/联网受限时，可为 Gradle 配置代理（写入 `%USERPROFILE%\.gradle\gradle.properties`）：

```properties
systemProp.http.proxyHost=127.0.0.1
systemProp.http.proxyPort=7897
systemProp.https.proxyHost=127.0.0.1
systemProp.https.proxyPort=7897
```

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

## Android 构建

```powershell
cd tauri
# 调试 APK（x86_64，配合模拟器；--no-default-features 关闭桌面专用特性）
.\node_modules\.bin\tauri.cmd android build --debug -t x86_64 -- --no-default-features
# 全 ABI 发布版
.\node_modules\.bin\tauri.cmd android build -- --no-default-features
```

产物：

- APK：`tauri/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`
- AAB：`tauri/src-tauri/gen/android/app/build/outputs/bundle/universalDebug/app-universal-debug.aab`

在真机上安装调试：

```powershell
& "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe" install -r "tauri/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"
```

Android 端说明：

- 首次启动会请求通知权限；随后启动前台服务，通知栏常驻"最新推送"（不可滑掉），每条新消息额外弹出高优先级提醒。
- 验证码由"规则"页匹配：命中时通知上出现"复制验证码"按钮，开启"自动复制验证码"（设置页）时自动写入剪贴板，配合输入法剪贴板栏实现一键输入。
- 数据目录为应用私有目录（`app.path().app_data_dir()`），包含 `config.json`、`history.db`、`ui_state.json`、`rules.json`。
- Android 端"开机自启"由 Boot 广播接收器实现（需系统允许自启动）。

## 配置

首次运行后在“设置”页填写：

| 字段 | 说明 |
| --- | --- |
| 服务器地址 | ntfy 服务器地址，推荐使用 `https://` |
| 用户名 | ntfy 用户名（可选） |
| 密码 | ntfy 密码（可选） |
| 主题 | 订阅的话题名，如 `your-topic` |
| 界面主题 | 跟随系统 / 浅色 / 深色 |
| 开机自启 | 是否随 Windows 启动 |
| 自动复制验证码 | 收到验证码时自动复制到剪贴板 |

数据文件位于 `%APPDATA%\ntfy-notifier\`：

| 文件 | 用途 |
| --- | --- |
| `config.json` | 配置，密码以 DPAPI 加密保存 |
| `history.db` | SQLite 推送历史（最近 1000 条） |
| `ui_state.json` | 表格列顺序与列宽记忆 |
| `rules.json` | 验证码规则（关键词、位数、匹配模式、优先级） |

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
│   │   ├── rules-model.js    # 验证码规则前端模型
│   │   └── table-model.js    # 表格列排序/调宽模型
│   ├── public/assets/fonts/  # 打包使用的字体
│   ├── tests/                # 前端单元测试
│   ├── src-tauri/            # Rust 后端与打包配置
│   │   ├── src/              # 配置、订阅、历史、规则、通知等模块
│   │   │   ├── ntfy.rs       # SSE 订阅循环
│   │   │   ├── rules.rs      # 验证码规则匹配/持久化
│   │   │   ├── notify_mobile.rs  # 安卓通知插件（Rust 端）
│   │   │   └── appdata.rs    # 跨平台数据目录
│   │   ├── gen/android/      # Android 工程（Kotlin 插件/前台服务）
│   │   └── tauri.conf.json   # 窗口与打包配置
│   └── package.json
├── docs/
│   └── ui-design.md          # 界面设计规格（字体/颜色/尺寸）
└── README.md
```

## 历史说明

仓库根目录下的 `src/`、`tests/` 等为早期 Python 版代码，仅作历史保留，当前版本不再维护。旧文档已随 Tauri 版落地移除，当前设计规格见 `docs/ui-design.md`。
