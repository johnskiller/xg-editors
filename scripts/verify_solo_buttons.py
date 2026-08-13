#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""验证 Solo 按钮: 点击 S → 琥珀; 且非 solo 通道的 M 变红/电平归零 (DAW 行为)."""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright
from PIL import Image

CHROME = "/home/john/.cache/ms-playwright/chromium-1208/chrome-linux64/chrome"
URL = "http://127.0.0.1:8090/?view=channel"
OUT = Path("/tmp/ms_verify")

def shot(page, name):
    OUT.mkdir(parents=True, exist_ok=True)
    p = OUT / f"{name}.png"
    page.screenshot(path=str(p)); return p

def red_ratio(img, cx, cy, r=7):
    n=red=0
    for dy in range(-r,r):
        for dx in range(-r,r):
            x,y=int(cx+dx),int(cy+dy)
            if 0<=x<img.width and 0<=y<img.height:
                n+=1
                p=img.getpixel((x,y))
                if p[0]>0x90 and p[1]<0x70: red+=1
    return red/n if n else 0

def amber_ratio(img, cx, cy, r=7):
    n=am=0
    for dy in range(-r,r):
        for dx in range(-r,r):
            x,y=int(cx+dx),int(cy+dy)
            if 0<=x<img.width and 0<=y<img.height:
                n+=1
                p=img.getpixel((x,y))
                if p[0]>0xd0 and p[1]>0x80 and p[2]<0x70: am+=1
    return am/n if n else 0

def green_px(img, y, x0, x1):
    return sum(1 for xx in range(x0,x1) if img.getpixel((xx,y))[1]>0x80 and img.getpixel((xx,y))[0]<0x60)

def main():
    with sync_playwright() as pw:
        b = pw.chromium.launch(executable_path=CHROME, headless=True,
            args=["--no-sandbox","--disable-blink-features=AutomationControlled"])
        pg = b.new_page()
        pg.goto(URL, wait_until="networkidle")
        pg.wait_for_timeout(5000)
        shot(pg, "solo_0")
        # 行1 y=113, c_left=30 → S 中心 (161,113), M (139,113); 行2 y=141
        pg.mouse.click(161, 113)  # S (行1)
        pg.wait_for_timeout(400)
        shot(pg, "solo_1")
        pg.mouse.click(161, 141)  # S (行2) — 第二个 solo
        pg.wait_for_timeout(400)
        shot(pg, "solo_2")
        b.close()

    im1 = Image.open(OUT/"solo_1.png").convert("RGB")
    im2 = Image.open(OUT/"solo_2.png").convert("RGB")
    # 行1 (y=113), 行2 (y=141): S 按钮 x=161, M x=139, 电平条 188..214
    print("=== solo_1 (只有行1 solo) ===")
    print(f"行1 S 琥珀={amber_ratio(im1,161,113):.2f}  (期望>0.8)")
    print(f"行2 S 琥珀={amber_ratio(im1,161,141):.2f}  (期望~0, 未 solo)")
    print(f"行2 M 红={red_ratio(im1,139,141):.2f}  (期望>0.5, 非solo被静音但M按钮本身不红 — 注意: M按钮红只表示自身mute=on)")
    print(f"行1 电平绿像素={green_px(im1,113,188,214)} 行2={green_px(im1,141,188,214)}")
    # 判定: 非solo通道电平该归零
    print("=== solo_2 (行1+行2 都 solo) ===")
    print(f"行1 S 琥珀={amber_ratio(im2,161,113):.2f}  行2 S 琥珀={amber_ratio(im2,161,141):.2f}")
    print(f"行3 S 琥珀={amber_ratio(im2,161,169):.2f} (未solo, 期望~0)")
    print(f"行1 电平绿像素={green_px(im2,113,188,214)} 行2={green_px(im2,141,188,214)}")

if __name__ == "__main__":
    main()
