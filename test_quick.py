import sys, os
sys.path.insert(0, r"C:\Users\Why\Downloads\ntfy-notifier")

import tkinter as tk
from src.ui import SettingsWindow

root = tk.Tk()
root.withdraw()

def on_save(cfg):
    print("Saved config:", list(cfg.keys()))
    root.quit()

win = SettingsWindow(
    {"server": "http://", "username": "", "password": "", "topic": "mytopic", "auto_start": False},
    on_save, None, root
)
win.show()
root.destroy()
print("Done")
