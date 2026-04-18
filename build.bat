@echo off
:: ntfy-Notifier 打包脚本
:: ============================================================
:: 前置条件（仅首次需要）:
::   pip install -r requirements.txt
::   pip install pyinstaller
:: ============================================================
:: 打包命令（使用 spec 文件，确保 tkinter 等模块正确包含）:
::   python -m PyInstaller ntfy-Notifier.spec
:: ============================================================
:: 输出文件: dist\ntfy-Notifier.exe
:: ============================================================

echo ntfy-Notifier 打包脚本
echo ========================
echo.
echo 检查依赖...
pip show pyinstaller >nul 2>&1 || pip install pyinstaller --quiet
echo   PyInstaller: OK
pip show pystray >nul 2>&1 || pip install pystray --quiet
echo   pystray: OK
pip show pillow >nul 2>&1 || pip install pillow --quiet
echo   pillow: OK
echo.
echo 开始打包...
python -m PyInstaller ntfy-Notifier.spec
echo.
echo 完成！输出文件: dist\ntfy-Notifier.exe
echo.
pause
