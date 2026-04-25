# -*- mode: python ; coding: utf-8 -*-
import sys, os

PY_PREFIX   = sys.prefix
PY_DLLS     = os.path.join(PY_PREFIX, 'DLLs')
PY_TCL      = os.path.join(PY_PREFIX, 'tcl')
PY_SITE     = os.path.join(PY_PREFIX, 'Lib', 'site-packages')
PYWIN32_SYS = os.path.join(PY_SITE, 'pywin32_system32')
WIN32_PKG   = os.path.join(PY_SITE, 'win32')

import PyInstaller.utils.hooks as h

# Collect pystray + requests data files
pystray_all   = h.collect_all('pystray')
pystray_datas = pystray_all[0]
pystray_bins  = pystray_all[1]
pystray_hid   = pystray_all[2]

requests_all   = h.collect_all('requests')
requests_datas = requests_all[0]

a = Analysis(
    ['src\\ntfy_notifier.py'],
    pathex=[PY_PREFIX],
    binaries=[
        # SSL / crypto DLLs（标准 CPython 在 DLLs/ 下，非 Library/bin）
        (os.path.join(PY_DLLS, 'libssl-3.dll'),   '.'),
        (os.path.join(PY_DLLS, 'libcrypto-3.dll'), '.'),
        (os.path.join(PY_DLLS, 'libffi-8.dll'),    '.'),
        # stdlib .pyd
        (os.path.join(PY_DLLS, '_ctypes.pyd'),   '.'),
        (os.path.join(PY_DLLS, '_tkinter.pyd'),  '.'),
        (os.path.join(PY_DLLS, '_ssl.pyd'),      '.'),
        (os.path.join(PY_DLLS, '_hashlib.pyd'),  '.'),
        (os.path.join(PY_DLLS, '_bz2.pyd'),      '.'),
        (os.path.join(PY_DLLS, '_lzma.pyd'),     '.'),
        (os.path.join(PY_DLLS, 'pyexpat.pyd'),   '.'),
        # pywin32
        (os.path.join(PYWIN32_SYS, f'pythoncom{sys.version_info[0]}{sys.version_info[1]}.dll'), '.'),
        (os.path.join(PYWIN32_SYS, f'pywintypes{sys.version_info[0]}{sys.version_info[1]}.dll'), '.'),
        (os.path.join(WIN32_PKG, 'win32gui.pyd'),  '.'),
        (os.path.join(WIN32_PKG, 'win32api.pyd'),  '.'),
        (os.path.join(WIN32_PKG, '_win32sysloader.pyd'), '.'),
        (os.path.join(WIN32_PKG, 'lib'), '.'),
    ],
    datas=[
        ('src', 'src'),
        ('connected.ico', '.'),
        ('disconnected.ico', '.'),
        # tcl/tk 在标准 CPython 下是 {prefix}\tcl\，不是 {prefix}\lib\
        (os.path.join(PY_TCL, 'tcl8.6'), 'tcl8.6'),
        (os.path.join(PY_TCL, 'tk8.6'),  'tk8.6'),
        (WIN32_PKG, 'win32'),
        (PYWIN32_SYS, 'pywin32_system32'),
    ] + pystray_datas + requests_datas,
    hiddenimports=[
        # pystray
        *pystray_hid,
        'six', 'six.moves',
        # json
        'json', 'json.decoder', 'json.encoder', 'json.scanner',
        # tkinter
        '_tkinter', 'tkinter', 'tkinter.ttk',
        'tkinter.messagebox', 'tkinter.filedialog',
        'tkinter.font', 'tkinter.constants',
        # pywin32
        'win32gui', 'win32con', 'win32api', 'pywin32_bootstrap',
        # requests
        'requests', 'requests.api', 'requests.auth',
        'requests.cookies', 'requests.exceptions',
        'requests.hooks', 'requests.models',
        'requests.sessions', 'requests.status_codes',
        'requests.structures', 'requests.utils', 'requests.compat',
        'urllib3', 'urllib3.util', 'urllib3.exceptions',
        'urllib3.response', 'urllib3.request',
        'charset_normalizer', 'certifi', 'idna',
        # Pillow（托盘图标后备绘图）
        'PIL', 'PIL.Image', 'PIL.ImageDraw',
        # plyer（Windows Toast 通知）
        'plyer', 'plyer.platforms', 'plyer.platforms.win',
        'plyer.platforms.win.notification',
        'plyer.utils', 'plyer.compat',
    ],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name='ntfy-Notifier',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    icon='connected.ico',
)
