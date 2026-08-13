#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""验证三修复: count 固定宽度 / transport 按钮同尺寸 / topbar 底色."""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright
from PIL import Image

CHROME = "/home/john/.cache/ms-playwright/chromium-1208/chrome-linux64/chrome"
OUT = Path("/tmp/tb3")
OUT.mkdir(exist_ok=True)
URL = "http://127.0.0.1:8090/?view=piano"

def count_color(im, target, x0, x1, y0, y1, tol=25):
    return sum(1 for y in range(y0, y1) for x in range(x0, x1)
               if abs(im.getpixel((x,y))[0]-target[0])<=tol
               and abs(im.getpixel((x,y))[1]-target[1])<=tol
               and abs(im.getpixel((x,y))[2]-target[2])<=tol)

def main():
    with sync_playwright() as pw:
        browser = pw.chromium.launch(executable_path=CHROME, headless=True,
                                     args=["--no-sandbox", "--disable-blink-features=AutomationControlled"])
        pg = browser.new_page(viewport={"width": 1600, "height": 1000}, device_scale_factor=1)
        pg.goto(URL, wait_until="networkidle")
        pg.wait_for_timeout(5000)

        # ── 1) topbar 底色: 检查 y=40 (顶栏底部) 是 0xf0f3f7 vs 中央区 y=56 不同 ──
        p0 = OUT/"0.png"; pg.screenshot(path=str(p0)); im = Image.open(p0).convert("RGB")
        print("y=3 采样:", im.getpixel((800, 3)), "| y=40 采样:", im.getpixel((800, 40)), "| y=60:", im.getpixel((800, 60)))
        bar_bg = im.getpixel((800, 3))
        is_bg = abs(bar_bg[0]-0xf0)<6 and abs(bar_bg[1]-0xf3)<6 and abs(bar_bg[2]-0xf7)<6
        print(f"topbar 底色 0xf0f3f7: {'✓' if is_bg else '✗ (实际' + str(bar_bg) + ')'}")

        # ── 2) transport 按钮位置 (深灰几何轮廓) ──
        # 手绘图标是 深灰 0x333333 且按钮 24px 高; 找所有非背景深色
        dark_cols = set()
        for y in range(6, 34):
            for x in range(350, 900):
                r,g,b = im.getpixel((x,y))
                if abs(r-0x33)<=20 and abs(g-0x33)<=20 and abs(b-0x33)<=20:
                    dark_cols.add(x)
        cols = sorted(dark_cols)
        segs = []
        for c in cols:
            if segs and c - segs[-1][-1] <= 2:
                segs[-1].append(c)
            else:
                segs.append([c])
        # transport 图标是几何填充 → 深灰覆盖范围应较大 (非 1px 字形的边缘)
        print("深灰 icon 段:", [(s[0], s[-1]) for s in segs])
        # 三个 transport 按钮应宽度接近 (段宽相似)
        widths = [(s[-1]-s[0]) for s in segs if s[-1]-s[0] > 3]
        print(f"图标段宽: {widths} (应大致相等, 相差 <5px = 按钮同尺寸)")

        # ── 3) count 固定宽度: 采样播放前后 count 区域 gold 像素 x 范围 ──
        def gold_extent(im):
            xs = [x for y in range(4, 26) for x in range(200, 500)
                  if abs(im.getpixel((x,y))[0]-0xe6)<25 and abs(im.getpixel((x,y))[1]-0x9d)<25 and abs(im.getpixel((x,y))[2]-0x1f)<25]
            return (min(xs), max(xs)) if xs else None
        g0 = gold_extent(im)
        print(f"count gold x范围: {g0}")

        # 点 Play 播放几帧 → count 变化但宽度不变
        # transport Play 在 350..900 深灰段第一个 (中心 ~490)
        if segs and segs[0][-1]-segs[0][0] > 3:
            play_x = (segs[0][0]+segs[0][-1])//2
        else:
            play_x = 490
        pg.mouse.click(play_x, 20)
        pg.wait_for_timeout(900)
        p1 = OUT/"1_play.png"; pg.screenshot(path=str(p1)); im1 = Image.open(p1).convert("RGB")
        g1 = gold_extent(im1)
        print(f"播放后 count gold x范围: {g1}")
        if g0 and g1:
            w0, w1 = g0[1]-g0[0], g1[1]-g1[0]
            print(f"count 宽度: before={w0} after={w1} → {'✓ 恒定' if abs(w0-w1)<=1 else '✗ 宽度变化(会抖)'}")
        # 播放时 playhead 数值应变化 (tick != 0)
        # 停
        pg.keyboard.press("Escape"); pg.wait_for_timeout(200)

        # ── 4) Record 红色 / Play 绿色状态 (手绘几何仍应工作) ──
        green = count_color(im1, (30,138,62), 350, 900, 4, 34, tol=30)
        print(f"播放时 Play 深绿像素: {green} (期望 > 0 = 手绘 Pause 绿)")

        browser.close()

if __name__ == "__main__":
    main()
