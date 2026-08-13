#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""分析 M/S 验证截图: 断言 M 按钮变红 & 电平条归零 (程序判定, 不靠眼睛)."""
from PIL import Image
from pathlib import Path

BASE = Path("/tmp/ms_verify")

def row_band(im, y_center):
    """找 M/S 按钮行: y 中心附近的像素"""
    pass

def sample_button(im, cx, cy, r=10):
    """采样按钮区域, 返回 (非灰像素占比, 主要颜色)"""
    non_gray = 0
    total = 0
    colors = []
    for dy in range(-int(r*0.7), int(r*0.7)):
        for dx in range(-int(r*0.7), int(r*0.7)):
            x, y = int(cx+dx), int(cy+dy)
            if 0 <= x < im.width and 0 <= y < im.height:
                p = im.getpixel((x,y))
                total += 1
                # 灰色 = (0x44,0x44,0x44)~; 非灰 = 红/琥珀
                if not (abs(p[0]-p[1])<25 and abs(p[1]-p[2])<25):
                    non_gray += 1
                    colors.append(p)
    return (non_gray/total if total else 0), colors

def main():
    b = BASE / "1_baseline2.png"
    a = BASE / "2_after_mute.png"
    if not b.exists() or not a.exists():
        print("缺截图"); return
    im_b = Image.open(b).convert("RGB")
    im_a = Image.open(a).convert("RGB")
    print(f"baseline {im_b.size} / after {im_a.size}")

    # 第一行电平条: baseline green y≈113, min_x≈188 → 电平条 188..214
    # M 按钮 ≈ (139,113), S 按钮 ≈ (139+22,113)=(161,113)
    mx, my = 139, 113
    sx, sy = 161, 113
    for name, im in [("baseline", im_b), ("after_mute", im_a)]:
        md, mc = sample_button(im, mx, my)
        sd, sc = sample_button(im, sx, sy)
        # 电平条区域 (188..214, y 113±5)
        lv_nonzero = 0
        for xx in range(188, 215):
            p = im.getpixel((xx, 113))
            if p[1] > 0x80 and p[0] < 0x60:  # 绿
                lv_nonzero += 1
        print(f"[{name}] M非灰={md:.2f} S非灰={sd:.2f} 电平条绿像素={lv_nonzero}")
        if mc: print(f"   M色示例: {mc[:3]}")
        if sc: print(f"   S色示例: {sc[:3]}")

    print("\n=== 判定 ===")
    md_a, mc_a = sample_button(im_a, mx, my)
    lv_a = sum(1 for xx in range(188,215) if im_a.getpixel((xx,113))[1] > 0x80 and im_a.getpixel((xx,113))[0] < 0x60)
    md_b, mc_b = sample_button(im_b, mx, my)
    lv_b = sum(1 for xx in range(188,215) if im_b.getpixel((xx,113))[1] > 0x80 and im_b.getpixel((xx,113))[0] < 0x60)
    red_hit = any(p[0]>0x90 and p[1]<0x70 for p in mc_a)
    print(f"M 按钮 baseline 非灰={md_b:.2f} → after {md_a:.2f}; 变红={'✓' if red_hit else '✗'}")
    print(f"电平条绿像素: baseline {lv_b} → after {lv_a}; 归零={'✓' if lv_a==0 else '✗'}")

if __name__ == "__main__":
    main()
