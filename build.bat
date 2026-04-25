@echo off
chcp 65001 >nul 2>&1
:: ntfy-Notifier 打包脚本
:: ============================================================
:: 前置条件：pip install -r requirements.txt & pip install pyinstaller
:: 打包命令：python -m PyInstaller ntfy-Notifier.spec
:: 输出文件：dist\ntfy-Notifier.exe
:: ============================================================

echo ntfy-Notifier 打包脚本
echo ========================`
echo.

:: 检查 Python 版本
python --version 2>&1 | findstr /C:"3.1" >nul || (
    echo [错误] 需要 Python 3.10+，当前版本：
    python --version
    pause
    exit /b 1
)

:: 检查关键依赖
echo 检查依赖...
for %%M in (pyinstaller pystray pillow requests plyer pywin32) do (
    pip show %%M >nul 2>&1 || (
        echo   安装 %%M...
        pip install %%M --quiet
    )
    echo   %%M: OK
)
echo.

:: 检查图标文件
if not exist "connected.ico" (
    echo [警告] connected.ico 不存在，打包后托盘图标将使用后备样式
)
if not exist "disconnected.ico" (
    echo [警告] disconnected.ico 不存在，打包后托盘图标将使用后备样式
)

:: 清理旧构建
if exist "dist\ntfy-Notifier.exe" (
    echo 清理旧构建...
    del /q "dist\ntfy-Notifier.exe" 2>nul
)

:: 开始打包
echo 开始打包...
python -m PyInstaller ntfy-Notifier.spec --noconfirm
echo.

if exist "dist\ntfy-Notifier.exe" (
    echo [成功] 输出文件: dist\ntfy-Notifier.exe
    for %%F in ("dist\ntfy-Notifier.exe") do echo   文件大小: %%~zF bytes
) else (
    echo [失败] 打包未生成 exe，请检查上方错误信息
)
echo.
pause
