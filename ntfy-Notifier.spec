# -*- mode: python ; coding: utf-8 -*-
import sys, os

PY_PREFIX = sys.prefix
PY_BIN    = os.path.join(PY_PREFIX, 'Library', 'bin')
PY_LIB    = os.path.join(PY_PREFIX, 'Library', 'lib')
PY_DLLS   = os.path.join(PY_PREFIX, 'DLLs')

a = Analysis(
    ['src\\ntfy_notifier.py'],
    pathex=[PY_PREFIX],
    binaries=[
        (os.path.join(PY_BIN,   'tcl86t.dll'),  '.'),
        (os.path.join(PY_BIN,   'tk86t.dll'),   '.'),
        (os.path.join(PY_DLLS, '_tkinter.pyd'), '.'),
    ],
    datas=[
        ('src', 'src'),
        (os.path.join(PY_LIB, 'tcl8.6'), 'tcl8.6'),
        (os.path.join(PY_LIB, 'tk8.6'),  'tk8.6'),
    ],
    hiddenimports=[
        'pystray._win32_images',
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
    name='ntfy-Notifier-v2',   # <-- 不同的文件名，绕过文件锁
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
