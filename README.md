# ntfy-Notifier

Windows 系统托盘工具，订阅 ntfy 消息并弹出系统通知。

## 功能

- 托盘常驻，最小化不占窗口
- SSE 实时订阅，无轮询延迟
- 系统原生 Toast 通知（plyer），可显示通知中心
- 连接状态托盘图标实时切换（🟢已连接 / 🔴未连接）
- 设置窗口：服务器、用户名、密码、主题、开机自启
- PyInstaller 单文件打包，无需 Python 环境

## 运行

```bash
# 安装依赖
pip install -r requirements.txt

# 直接运行
python -m src.ntfy_notifier
```

首次运行会自动弹出设置窗口，配置好 ntfy 服务器信息后保存即可。

## 打包

```bash
# 安装 PyInstaller
pip install pyinstaller

# 打包（输出 dist/ntfy-Notifier.exe）
python -m PyInstaller ntfy-Notifier.spec
```

或直接双击运行 `build.bat`。

## 配置

ntfy 服务器需要开启 SSE 订阅支持。设置窗口填写：

| 字段 | 说明 |
|------|------|
| 服务器地址 | ntfy 服务器地址，如 `http://114.55.43.156:8080` |
| 用户名 | ntfy 用户名（可选） |
| 密码 | ntfy 密码（可选） |
| 主题 | 订阅的话题名，如 `sms` |
| 开机自启 | 勾选后加入 Windows 开机自启动 |

## 项目结构

```
ntfy-Notifier/
├── src/
│   ├── ntfy_notifier.py   # 主程序入口
│   ├── notifier.py         # SSE 订阅 + 通知发送
│   ├── tray.py             # pystray 托盘图标
│   ├── ui.py               # Tkinter 设置窗口
│   └── config.py           # 配置文件读写
├── connected.ico           # 已连接状态图标
├── disconnected.ico        # 未连接状态图标
├── ntfy-Notifier.spec     # PyInstaller 打包配置
├── build.bat               # 打包脚本
└── requirements.txt        # 依赖列表
```

## 依赖

- Python 3.10+
- pystray
- pillow
- requests
- plyer
- pywin32（Windows）
