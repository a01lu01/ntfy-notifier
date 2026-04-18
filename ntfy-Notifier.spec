# -*- mode: python ; coding: utf-8 -*-
import sys, os

PY_PREFIX = sys.prefix
PY_BIN    = os.path.join(PY_PREFIX, 'Library', 'bin')
PY_LIB    = os.path.join(PY_PREFIX, 'Library', 'lib')
PY_DLLS   = os.path.join(PY_PREFIX, 'DLLs')
PY_SITE   = os.path.join(PY_PREFIX, 'lib', 'site-packages')

import PyInstaller.utils.hooks as h

# Properly collect all pystray resources
pystray_all   = h.collect_all('pystray')
pystray_datas = pystray_all[0]   # data files to bundle
pystray_bins  = pystray_all[1]   # binaries (usually empty)
pystray_hid   = pystray_all[2]   # hidden imports

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
    ],
    datas=[
        ('src', 'src'),
        (os.path.join(PY_LIB, 'tcl8.6'), 'tcl8.6'),
        (os.path.join(PY_LIB, 'tk8.6'),  'tk8.6'),
    ] + pystray_datas,
    hiddenimports=[
        # pystray + all submodules
        *pystray_hid,
        # six / six.moves (pystray depends on this)
        'six',
        'six.moves',
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
    name='ntfy-Notifier-v3',
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
