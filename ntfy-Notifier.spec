# -*- mode: python ; coding: utf-8 -*-
import sys, os

PY_PREFIX   = sys.prefix
PY_DLLS     = os.path.join(PY_PREFIX, 'DLLs')
PY_TCL      = os.path.join(PY_PREFIX, 'tcl')
PY_SITE     = os.path.join(PY_PREFIX, 'Lib', 'site-packages')
PYWIN32_SYS = os.path.join(PY_SITE, 'pywin32_system32')
WIN32_PKG   = os.path.join(PY_SITE, 'win32')

import PyInstaller.utils.hooks as h

# 收集 pystray 全部数据文件
pystray_all   = h.collect_all('pystray')
pystray_datas = pystray_all[0]
pystray_hid   = pystray_all[2]

# 收集 requests 数据文件
requests_all    = h.collect_all('requests')
requests_datas  = requests_all[0]

# 收集 PIL 数据文件（所有插件 + 字体）
pil_datas = h.collect_data_files('PIL', include_py_files=True)

# 收集 freetype
freetype_datas = h.collect_data_files('freetype')

# ── 二进制文件 ─────────────────────────────────────────────────────────────
_binaries = [
    # SSL / crypto
    (os.path.join(PY_DLLS, 'libssl-3.dll'),       '.'),
    (os.path.join(PY_DLLS, 'libcrypto-3.dll'),    '.'),
    (os.path.join(PY_DLLS, 'libffi-8.dll'),        '.'),
    # stdlib
    (os.path.join(PY_DLLS, '_ctypes.pyd'),   '.'),
    (os.path.join(PY_DLLS, '_tkinter.pyd'),  '.'),
    (os.path.join(PY_DLLS, '_ssl.pyd'),      '.'),
    (os.path.join(PY_DLLS, '_hashlib.pyd'),  '.'),
    (os.path.join(PY_DLLS, '_bz2.pyd'),      '.'),
    (os.path.join(PY_DLLS, '_lzma.pyd'),     '.'),
    (os.path.join(PY_DLLS, 'pyexpat.pyd'),   '.'),
    # tkinter tcl/tk DLL（在 DLLs/ 下，t=threaded 版本）
    (os.path.join(PY_DLLS, 'tk86t.dll'),   '.'),
    (os.path.join(PY_DLLS, 'tcl86t.dll'), '.'),
    # pywin32
    (os.path.join(PYWIN32_SYS, f'pythoncom{sys.version_info[0]}{sys.version_info[1]}.dll'), '.'),
    (os.path.join(PYWIN32_SYS, f'pywintypes{sys.version_info[0]}{sys.version_info[1]}.dll'), '.'),
    (os.path.join(WIN32_PKG, 'win32gui.pyd'),   '.'),
    (os.path.join(WIN32_PKG, 'win32api.pyd'),   '.'),
    (os.path.join(WIN32_PKG, '_win32sysloader.pyd'), '.'),
    (os.path.join(WIN32_PKG, 'lib'), '.'),
    # PIL C 扩展 + freetype DLL
    (os.path.join(PY_SITE, 'PIL', '_imaging.cp314-win_amd64.pyd'),       'PIL'),
    (os.path.join(PY_SITE, 'PIL', '_imagingcms.cp314-win_amd64.pyd'),     'PIL'),
    (os.path.join(PY_SITE, 'PIL', '_imagingft.cp314-win_amd64.pyd'),      'PIL'),
    (os.path.join(PY_SITE, 'PIL', '_imagingmath.cp314-win_amd64.pyd'),    'PIL'),
    (os.path.join(PY_SITE, 'PIL', '_imagingmorph.cp314-win_amd64.pyd'),  'PIL'),
    (os.path.join(PY_SITE, 'PIL', '_imagingtk.cp314-win_amd64.pyd'),      'PIL'),
    (os.path.join(PY_SITE, 'PIL', '_avif.cp314-win_amd64.pyd'),          'PIL'),
    (os.path.join(PY_SITE, 'PIL', '_webp.cp314-win_amd64.pyd'),          'PIL'),
    (os.path.join(PY_SITE, 'freetype', 'libfreetype.dll'),               'freetype'),
]

# ── 数据文件 ────────────────────────────────────────────────────────────────
_datas = [
    ('src', 'src'),
    ('connected.ico',   '.'),
    ('disconnected.ico','.'),
    # tcl/tk 运行时
    (os.path.join(PY_TCL, 'tcl8.6'), 'tcl8.6'),
    (os.path.join(PY_TCL, 'tk8.6'),  'tk8.6'),
    # pywin32
    (WIN32_PKG,   'win32'),
    (PYWIN32_SYS,'pywin32_system32'),
] + list(pystray_datas) + list(requests_datas) + list(pil_datas) + list(freetype_datas)

# ── 隐藏导入 ────────────────────────────────────────────────────────────────
_hidden = [
    # pystray（hook 自动收集）
    *pystray_hid,
    # six（pystray 依赖）
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
    # PIL（Pillow C 扩展 + 插件）
    'PIL', 'PIL.Image', 'PIL.ImageDraw', 'PIL.ImageFont',
    'PIL._imaging', 'PIL._imagingcms', 'PIL._imagingft',
    'PIL._imagingmath', 'PIL._imagingmorph', 'PIL._imagingtk',
    'PIL._avif', 'PIL._webp',
    # plyer（Windows 通知，后备）
    'plyer', 'plyer.platforms', 'plyer.platforms.win',
    'plyer.platforms.win.notification',
    'plyer.utils', 'plyer.compat',
    'plyer.facades', 'plyer.facades.notification',
    # winotify（Windows Toast 通知，首选，支持 AUMID）
    'winotify', 'winotify.audio', 'winotify._notify',
    'winotify._registry', 'winotify._communication', 'winotify._run_ps',
    # re / 标准库
    're',
]

# ── 主程序 ──────────────────────────────────────────────────────────────────
a = Analysis(
    ['src\\ntfy_notifier.py'],
    pathex=[PY_PREFIX],
    binaries=_binaries,
    datas=_datas,
    hiddenimports=_hidden,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz, a.scripts, a.binaries, a.datas, [],
    name='ntfy-Notifier',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    runtime_tmpdir=None,
    console=False,
    icon='connected.ico',
)

# ── 诊断工具（控制台）──────────────────────────────────────────────────────
diag_a = Analysis(
    ['src\\diagnose.py'],
    pathex=[PY_PREFIX],
    binaries=_binaries,
    datas=_datas + [
        ('src\\config.py', 'src'),
        ('src\\notifier.py', 'src'),
    ],
    hiddenimports=_hidden + [
        'src.config', 'src.notifier',
    ],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
diag_pyz = PYZ(diag_a.pure)
diag_exe = EXE(
    diag_pyz, diag_a.scripts, diag_a.binaries, diag_a.datas, [],
    name='ntfy-Notifier-diagnose',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    runtime_tmpdir=None,
    console=True,
)
