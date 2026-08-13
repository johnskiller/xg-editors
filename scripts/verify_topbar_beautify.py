#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""TopBar 美化 完整像素验证 v2 (浅色主题修正后).
判定:
1. 顶栏高 ~44px (包 bar 背景到中央区起点)
2. ☰ 菜单点击弹出 (菜单项文字出现)
3. transport 按钮 (深灰字形) 定位 & 点击 Play → 变绿, Stop, Record armed 变红
4. 播放 count 深金色 (0xe6,0x9d,0x1f)
"""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright
from PIL import Image

CHROME = "/home/john/.cache/ms-playwright/chromium-1208/chrome-linux64/chrome"
OUT = Path("/tmp/topbar_v3")
OUT.mkdir(exist_ok=True)
URL = "http://127.0.0.1:8090/?view=piano"

def px(im, x, y):
    return im.getpixel((x, y))

def count_color(im, target, x0, x1, y0, y1, tol=25):
    n = 0
    for y in range(y0, y1):
        for x in range(x0, x1):
            r, g, b = px(im, x, y)
            if abs(r-target[0])<=tol and abs(g-target[1])<=tol and abs(b-target[2])<=tol:
                n += 1
    return n

def main():
    with sync_playwright() as pw:
        browser = pw.chromium.launch(executable_path=CHROME, headless=True,
                                     args=["--no-sandbox", "--disable-blink-features=AutomationControlled"])
        pg = browser.new_page(viewport={"width": 1600, "height": 1000}, device_scale_factor=1)
        pg.goto(URL, wait_until="networkidle")
        pg.wait_for_timeout(5000)
        p0 = OUT/"0_base.png"; pg.screenshot(path=str(p0)); im0 = Image.open(p0).convert("RGB")
        W, H = im0.size

        # ── 1) 顶栏高度: 顶栏背景 (248,248,248) 纯色行; 中央区从第一个出现内容色的行起
        def is_empty_bar(y):
            # 该行全是 bar 背景 (248,248,248) → 属于顶栏 (顶栏底部无内容)
            return all(px(im0, x, y)==(248,248,248) for x in range(5, 800, 5))
        last_pure_bg = None
        for y in range(0, 70):
            if is_empty_bar(y):
                last_pure_bg = y
        bar_h = (last_pure_bg + 1) if last_pure_bg else 44
        print(f"纯背景行延续到 y={last_pure_bg} → 顶栏高 ~{bar_h}px (期望 ~44)")

        # ── 4) 播放 count 深金色 0xe6,0x9d,0x1f (在顶栏 0..bar_h) ──
        gold = count_color(im0, (230,157,31), 300, 900, 4, bar_h)
        print(f"播放count 深金色像素: {gold} (期望 > 50 = 已渲染, 字体放大)")

        # ── 3a) transport 深灰字形 0x33,0x33,0x33 定位 (字形中心 y~13) ──
        dark = []
        for y in range(6, 24):
            for x in range(200, 900):
                r,g,b = px(im0, x, y)
                if abs(r-0x33)<=12 and abs(g-0x33)<=12 and abs(b-0x33)<=12:
                    dark.append((x, y))
        # 聚类成按钮: 卡片 24px 宽. 只取字形中心行 y 12-16 避免高度噪声
        glyph_cols = sorted({x for x, y in dark if 10<=y<=18})
        segs = []
        for c in glyph_cols:
            if segs and c - segs[-1][-1] <= 2:
                segs[-1].append(c)
            else:
                segs.append([c])
        centers = [((s[0]+s[-1])//2) for s in segs]
        print(f"transport 深灰按钮段: {[(s[0], s[-1]) for s in segs]}")
        print(f"按钮中心 x: {centers}")
        # ☰ menu 图标也是深灰, 会在最左(265)误入; transport 按钮是右边三个 (x>350)
        centers = [c for c in centers if c > 350]
        print(f"过滤后 transport 按钮中心: {centers}")

        # 需要至少 3 个 (play, stop, record). 若不足, 截图看
        if len(centers) < 3:
            print("⚠ transport 按钮不足 3 个 — 布局可能有问题")
            pg.close(); browser.close(); return

        # ── 3b) 点击 Play (第一个按钮 = 最左 = Play) ──
        play_x = centers[0]
        pg.mouse.click(play_x, 20)
        pg.wait_for_timeout(700)  # 等 egui 处理 click + 重绘
        p1 = OUT/"1_play.png"; pg.screenshot(path=str(p1)); im1 = Image.open(p1).convert("RGB")
        # Play 变 Pause + 深绿 0x1e,0x8a,0x3e
        green = count_color(im1, (30,138,62), play_x-20, play_x+20, 4, 34, tol=30)
        print(f"点击 Play 后 绿(激活) 像素: {green} (期望 > 5 = Pause 激活)")

        # ── 3c) Stop (中心索引1) ──
        stop_x = centers[1]
        pg.mouse.click(stop_x, 20)
        pg.wait_for_timeout(500)
        p2 = OUT/"2_stop.png"; pg.screenshot(path=str(p2)); im2 = Image.open(p2).convert("RGB")
        # Stop 后: 无绿 (重新变暗)
        green2 = count_color(im2, (30,138,62), 0, 900, 4, 34, tol=30)
        print(f"Stop 后 绿像素: {green2} (期望 0 = 已停止)")

        # ── 3d) Record armed (索引2 或最后) ──
        rec_x = centers[-1]
        pg.mouse.click(rec_x, 20)
        pg.wait_for_timeout(500)
        p3 = OUT/"3_rec.png"; pg.screenshot(path=str(p3)); im3 = Image.open(p3).convert("RGB")
        red = count_color(im3, (204,34,34), rec_x-20, rec_x+20, 4, 34, tol=35)
        print(f"Record armed 红色像素: {red} (期望 > 5 = armed 红点+圆环)")

        # ── 2) ☰ menu: 点击左上角 (x~18, y~22) 应弹出菜单 ──
        # 但此时 rec armed 状态, 先 Escape
        pg.keyboard.press("Escape"); pg.wait_for_timeout(300)
        pg.mouse.click(20, 22)
        pg.wait_for_timeout(700)
        p4 = OUT/"4_menu.png"; pg.screenshot(path=str(p4)); im4 = Image.open(p4).convert("RGB")
        # 菜单弹出: 对比 baseline, 左上方 (30..300, 40..250) 出现菜单面板 (浅色但非 bar 背景)
        # 菜单面板是白色/浅色; 检测该区是否有 文字 (深色像素)
        menutxt = count_color(im4, (0,0,0), 30, 320, 40, 260, tol=80)  # 深色文字
        menutxt += count_color(im4, (60,60,60), 30, 320, 40, 260, tol=40)
        print(f"☰ 菜单文字像素: {menutxt} (期望 > 30 = 菜单弹出)")

        browser.close()

if __name__ == "__main__":
    main()
