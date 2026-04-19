# -*- mode: python ; coding: utf-8 -*-
import sys, os

PY_PREFIX = sys.prefix
PY_BIN    = os.path.join(PY_PREFIX, 'Library', 'bin')
PY_LIB    = os.path.join(PY_PREFIX, 'Library', 'lib')
PY_DLLS   = os.path.join(PY_PREFIX, 'DLLs')
PY_SITE   = os.path.join(PY_PREFIX, 'lib', 'site-packages')
PYWIN32_SYS = os.path.join(PY_SITE, 'pywin32_system32')
WIN32_PKG   = os.path.join(PY_SITE, 'win32')

import PyInstaller.utils.hooks as h

# Properly collect all pystray resources
pystray_all   = h.collect_all('pystray')
pystray_datas = pystray_all[0]   # data files to bundle
pystray_bins  = pystray_all[1]   # binaries (usually empty)
pystray_hid   = pystray_all[2]   # hidden imports

# Properly collect all requests resources
requests_all   = h.collect_all('requests')
requests_datas = requests_all[0]

a = Analysis(
    ['src\\ntfy_notifier.py'],
    pathex=[PY_PREFIX],
    binaries=[
        # All DLLs from PY_BIN (ffi.dll, libssl, libcrypto, expat, etc.)
        (PY_BIN,  '.'),
        # All .pyd modules from PY_DLLS needed by Python stdlib
        (os.path.join(PY_DLLS, '_ctypes.pyd'),   '.'),
        (os.path.join(PY_DLLS, '_tkinter.pyd'),  '.'),
        (os.path.join(PY_DLLS, '_ssl.pyd'),     '.'),
        (os.path.join(PY_DLLS, '_hashlib.pyd'), '.'),
        (os.path.join(PY_DLLS, '_bz2.pyd'),     '.'),
        (os.path.join(PY_DLLS, '_lzma.pyd'),    '.'),
        (os.path.join(PY_DLLS, 'pyexpat.pyd'), '.'),
        # pywin32 DLLs (required by win32gui) - dynamic Python version
        (os.path.join(PYWIN32_SYS, f'pythoncom{sys.version_info[0]}{sys.version_info[1]}0.dll'), '.'),
        (os.path.join(PYWIN32_SYS, f'pywintypes{sys.version_info[0]}{sys.version_info[1]}0.dll'), '.'),
        # pywin32 pyd modules + lib (needed to load pyd at runtime)
        (os.path.join(WIN32_PKG, 'win32gui.pyd'),  '.'),
        (os.path.join(WIN32_PKG, 'win32api.pyd'),  '.'),
        (os.path.join(WIN32_PKG, '_win32sysloader.pyd'), '.'),
        (os.path.join(WIN32_PKG, 'lib'), '.'),
    ],
    datas=[
        ('src', 'src'),
        (os.path.join(PY_LIB, 'tcl8.6'), 'tcl8.6'),
        (os.path.join(PY_LIB, 'tk8.6'),  'tk8.6'),
        (WIN32_PKG, 'win32'),
        (PYWIN32_SYS, 'pywin32_system32'),
    ] + pystray_datas + requests_datas,
    hiddenimports=[
        # pystray + all submodules
        *pystray_hid,
        # six / six.moves (pystray depends on this)
        'six',
        'six.moves',
        # json (standard library, explicit to prevent pystray hook shadowing)
        'json',
        'json.decoder',
        'json.encoder',
        'json.scanner',
        # tkinter
        '_tkinter',
        'tkinter',
        'tkinter.ttk',
        'tkinter.messagebox',
        'tkinter.filedialog',
        'tkinter.colorchooser',
        'tkinter.simpledialog',
        'tkinter.font',
        'tkinter.constants',
        # pywin32 (win32gui for tray)
        'win32gui',
        'win32con',
        'win32api',
        'pywin32_bootstrap',
        # requests + dependencies
        'requests',
        'requests.api',
        'requests.auth',
        'requests.cookies',
        'requests.exceptions',
        'requests.hooks',
        'requests.models',
        'requests.sessions',
        'requests.status_codes',
        'requests.structures',
        'requests.utils',
        'requests.compat',
        'urllib3',
        'urllib3.util',
        'urllib3.exceptions',
        'urllib3.response',
        'urllib3.request',
        'charset_normalizer',
        'certifi',
        'idna',
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
    name='ntfy-Notifier-v3c',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
