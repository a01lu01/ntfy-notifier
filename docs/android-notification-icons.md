# Android 通知图标与干净打包指南

本文适用于本项目的 Tauri 2 Android 版本，说明通知图标的来源、替换方法、可复现构建流程和 APK 成品验证方法。

## 1. 先区分三类图标

Android 应用中容易混淆的图标有三类：

| 图标 | 本项目资源 | 用途 |
| --- | --- | --- |
| 应用启动图标 | `mipmap-*/ic_launcher*` | 桌面、应用详情页，以及部分系统的通知应用徽标 |
| 通知小图标 | `drawable-*/ic_stat_ntfy.png` | 顶部状态栏和通知卡片左侧的小图标 |
| 通知大图标 | `drawable-*/ic_notification_large.png` | 通知展开后由系统决定是否显示的彩色图标 |

`tauri icon AppIcon.png` 主要生成应用启动图标，不会自动生成符合 Android 规范的通知小图标。

通知小图标必须使用透明背景和纯白色前景。Android 会根据系统主题为它着色，因此不要直接使用带背景、渐变、阴影或多种颜色的完整应用图标。

## 2. 本项目的代码入口

Android 通知由原生 Kotlin 前台服务创建，不是由 `tauri.conf.json` 的 `bundle.icon` 控制：

```text
tauri/src-tauri/gen/android/app/src/main/java/app/ntfy/notifier/NotificationService.kt
```

常驻通知和新消息提醒都必须显式设置小图标和大图标：

```kotlin
.setSmallIcon(R.drawable.ic_stat_ntfy)
.setLargeIcon(
  BitmapFactory.decodeResource(resources, R.drawable.ic_notification_large)
)
```

如果修改了资源名称，例如改成 `ic_stat_my_logo.png`，代码也必须同步改为：

```kotlin
.setSmallIcon(R.drawable.ic_stat_my_logo)
```

资源文件名只能使用小写英文字母、数字和下划线。

## 3. 替换通知图标

当前通知小图标尺寸如下：

| 目录 | `ic_stat_ntfy.png` | `ic_notification_large.png` |
| --- | ---: | ---: |
| `drawable-mdpi` | 24 × 24 | 64 × 64 |
| `drawable-hdpi` | 36 × 36 | 96 × 96 |
| `drawable-xhdpi` | 48 × 48 | 128 × 128 |
| `drawable-xxhdpi` | 72 × 72 | 192 × 192 |
| `drawable-xxxhdpi` | 96 × 96 | 256 × 256 |

需要同时替换五个密度目录中的同名文件：

```text
tauri/src-tauri/gen/android/app/src/main/res/
├── drawable-mdpi/ic_stat_ntfy.png
├── drawable-hdpi/ic_stat_ntfy.png
├── drawable-xhdpi/ic_stat_ntfy.png
├── drawable-xxhdpi/ic_stat_ntfy.png
└── drawable-xxxhdpi/ic_stat_ntfy.png
```

建议使用 Android Studio 的 **New → Image Asset → Notification Icons** 生成通知小图标。图形应当简洁，细节和文字在 24 dp 下通常无法辨认。

`drawable/ic_notification.xml` 当前没有被 `NotificationService.kt` 引用，单独替换它不会改变通知栏图标。

## 4. 干净构建 release APK

为了排除旧 Gradle、Cargo 和 Android 资源产物，最稳妥的方法是在新目录克隆并构建：

```powershell
git clone https://github.com/a01lu01/ntfy-notifier.git ntfy-notifier-clean
cd ntfy-notifier-clean\tauri

# 严格使用 package-lock.json 中锁定的依赖
npm ci

# 全 ABI release 构建；关闭桌面专用默认特性
.\node_modules\.bin\tauri.cmd android build -- --no-default-features
```

已经提交的 `src-tauri/gen/android/` 包含本项目的 Kotlin 服务和定制资源。不要在这个目录上重新运行 `tauri android init`，否则可能覆盖原生 Android 定制。

release APK 位于：

```text
tauri/src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
```

调试构建的 APK 位于另一个目录：

```text
tauri/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

构建 release 后不要误装以前残留的 debug APK。建议每次复制成带版本号的文件名后再分发。

## 5. 安装和真机验证

通过 ADB 安装 release APK：

```powershell
& "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe" install -r `
  "src-tauri\gen\android\app\build\outputs\apk\universal\release\app-universal-release.apk"
```

如果提示签名不一致，说明设备上的旧版由另一证书签名。卸载旧版会清除应用私有数据，应先备份需要保留的配置。

安装后检查：

1. 应用关于页显示预期版本号。
2. 启动应用并授予通知权限。
3. 顶部状态栏出现自定义的单色通知小图标。
4. 展开通知栏，确认常驻通知和新消息提醒均使用正确图标。

部分厂商系统会在通知标题附近额外显示应用启动图标。它与 `setSmallIcon()` 指定的通知小图标不是同一个元素。

## 6. 验证 APK 内的实际资源

仅检查源码或版本号不足以证明 APK 使用了正确资源。可以使用 Android SDK 的 `aapt2` 检查成品：

```powershell
& "$env:LOCALAPPDATA\Android\Sdk\build-tools\36.0.0\aapt2.exe" `
  dump resources `
  "src-tauri\gen\android\app\build\outputs\apk\universal\release\app-universal-release.apk" |
  Select-String "ic_stat_ntfy|ic_notification_large"
```

输出中应同时包含：

```text
drawable/ic_stat_ntfy
drawable/ic_notification_large
```

如需进一步确认 Kotlin 字节码引用，可用 Android SDK 的 `apkanalyzer` 反编译 `app.ntfy.notifier.NotificationService`，检查两处 `setSmallIcon()` 传入的资源 ID 是否与 `drawable/ic_stat_ntfy` 的资源 ID 一致。

## 7. 常见问题

### APK 版本正确，图标为什么仍然可能是旧的？

`versionName` 只是一段配置值，不能证明 APK 来自哪次提交。旧源码、错误的输出目录和残留的 debug APK 都可能具有相同版本号。

### 为什么不能直接把彩色 Logo 用作通知小图标？

Android 会把通知小图标作为遮罩并由系统着色。彩色 Logo 应放在 `ic_notification_large`；小图标应使用适合 24 dp 显示的单色轮廓。

### 修改 `tauri.conf.json` 的 `bundle.icon` 有用吗？

它负责应用打包图标，不控制本项目原生 `NotificationCompat.Builder` 的小图标。通知小图标由 `NotificationService.kt` 的 `setSmallIcon()` 决定。
