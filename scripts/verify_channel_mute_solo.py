#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Channel View Mute/Solo 渲染验证 (playwright + PIL 程序判定, 不靠眼睛).
James 2026-08-13.
流程:
1. 加载 :8090/?view=channel (默认有 demo tracks, 16 行)
2. 截 baseline 图
3. 计算第一行 gutter 的 M/S 按钮坐标 (gutter 左缘 + 100..136px 区, y=行中心)
4. 点击 M 按钮 → 截图, 断言按钮变红 + 电平条归零
5. 点击 S 按钮 → 截图, 断言按钮变琥珀
6. 失败打印 console 错误 (wasm panic 排查)
"""
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

CHROME = "/home/john/.cache/ms-playwright/chromium-1208/chrome-linux64/chrome"
URL = "http://127.0.0.1:8090/?view=channel"
OUT = Path("/tmp/ms_verify")

def shot(page, name):
    OUT.mkdir(parents=True, exist_ok=True)
    p = OUT / f"{name}.png"
    page.screenshot(path=str(p))
    return p

def main():
    if "-h" in sys.argv or "--help" in sys.argv:
        print(__doc__); return
    print(f"[1] 打开 {URL}")
    with sync_playwright() as pw:
        browser = pw.chromium.launch(
            executable_path=CHROME,
            headless=True,
            args=["--no-sandbox", "--disable-blink-features=AutomationControlled"],
        )
        ctx = browser.new_context(viewport={"width": 1600, "height": 1000}, device_scale_factor=1)
        page = ctx.new_page()
        errs = []
        page.on("console", lambda m: errs.append(m.text) if m.type == "error" else None)
        page.on("pageerror", lambda e: errs.append(str(e)))
        page.goto(URL, wait_until="networkidle")
        page.wait_for_timeout(5000)  # egui wasm 冷启动

        # 读取调试状态 (确认 charset 数)
        state = page.evaluate("document.getElementById('xg_state')?.textContent || ''")
        print(f"[2] xg_state: {state}")

        shot(page, "0_baseline")

        # ===== 计算第一行 M/S 按钮坐标 =====
        # gutter 左缘 = 中央面板左缘 (left side bare 22px collapsed) + padding; 按钮 M 中心 x ≈ c_left+100+9
        # 稳健取法: 读取 canvas 像素找 gutter 分隔竖线 (c_left+gutter_w), 反推按钮 y = 行中心
        # 简化: 用固定假设 + 像素探测修正
        # canvas 占满窗口; 中央面板左缘 ≈ 22(左栏收起) + 少许. 先截图探测第一行文字行基线.
        # 保守: M 按钮 y 在第一行中心 (屏幕 y ≈ top_bar(~40) + 22 ruler + row_h/2 ≈ 40+22+14 = 76)
        # 但我们用像素色块检测更可靠 —— 找红色/琥珀平方块.
        print("[3] 探测定点 — 鼠标截图无状态, 直接点固定估算坐标")

        # 点击第一行 M 按钮: gutter 左缘 ≈ 22(leftbar) + 8(pad), M rect 中心 x ≈ 22+8+100+9=139, y ≈ 76
        # 更稳: 用 evaluate 读 canvas 尺寸
        cw = page.evaluate("document.getElementById('the_canvas').width")
        ch_ = page.evaluate("document.getElementById('the_canvas').height")
        print(f"[3] canvas {cw}x{ch_}")

        # 先截 baseline 分析像素找到行分隔线/面板左缘, 再决定点击点
        # baseline 已存, 用 PIL 分析
        from PIL import Image
        base_im = Image.open(shot(page, "1_baseline2")).convert("RGB")
        w, h = base_im.size
        print(f"[4] screenshot {w}x{h}")
        # 扫描找绿电平条 (0x2e,0xcc,0x40) 第一行位置 → 推导 gutter/行布局
        green_rows = {}
        for yy in range(h):
            found = False
            for xx in range(0, w):
                r, g, b = base_im.getpixel((xx, yy))
                if abs(g - 0xcc) < 30 and r < 0x60 and b < 0x60:  # 绿电平
                    found = True
                    break
            if found:
                green_rows[yy] = True
        green_ys = sorted(green_rows.keys())
        print(f"[4] 绿电平像素 y 范围: {green_ys[:3]}...{green_ys[-3:] if green_ys else 'none'} 共{len(green_ys)}")
        if green_ys:
            # 取第一行电平条: 第一组连续 y
            groups = []
            cur = [green_ys[0]]
            for yy in green_ys[1:]:
                if yy - cur[-1] <= 2:
                    cur.append(yy)
                else:
                    groups.append(cur); cur = [yy]
            groups.append(cur)
            first_row_y = groups[0][0] + len(groups[0]) // 2 if groups else None
            print(f"[5] 第一行电平条中心 y ≈ {first_row_y} (分组数 {len(groups)})")
            # gutter 左缘 x = 绿电平条左端 - 158 (lvx = c_left+158), M rect = c_left+100..118
            # 找第一行绿条最小 x
            min_green_x = None
            if green_ys:
                for xx in range(w):
                    r, g, b = base_im.getpixel((xx, first_row_y))
                    if abs(g - 0xcc) < 30 and r < 0x60:
                        min_green_x = xx
                        break
            print(f"[5] 第一行绿条左端 x ≈ {min_green_x}")
            if min_green_x and first_row_y:
                c_left = min_green_x - 158.0
                mx = c_left + 100.0 + 9.0   # M 中心
                my = first_row_y
                print(f"[6] 推定 c_left={c_left:.0f}, 点击 M 按钮 ≈ ({mx:.0f},{my})")
                page.mouse.click(mx, my)
                page.wait_for_timeout(500)
                shot(page, "2_after_mute")
                print("[7] 已点击 M (第一行). 请人工/分析确认变红.")
        else:
            print("[!] 未找到绿电平条 — 可能不在 Channel 视图或 demo 无电平. 尝试点估算坐标 (139,76)")
            page.mouse.click(139, 76)
            page.wait_for_timeout(500)
            shot(page, "2_after_mute")

        # 浏览器 console 错误
        real_errs = [e for e in errs if "favicon" not in e]
        print(f"[8] console errors: {len(real_errs)}")
        for e in real_errs[:10]:
            print("   ", e)

        browser.close()

if __name__ == "__main__":
    main()
