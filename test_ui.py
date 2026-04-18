import sys
sys.path.insert(0, r"C:\Users\Why\Downloads\ntfy-notifier")

from src.ui import _theme, SettingsWindow

print("Dark mode:", _theme.is_dark)
c = _theme.colors()
print("Surface:", c["surface"])
print("Accent:", c["accent"])
print("Input bg:", c["input_bg"])
print("All OK")
