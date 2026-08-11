//! PlayView 瀑布区背景纹理 — Horsehead 星云 (构建期由 jpg 转换的 raw RGBA).
//! 生成: scripts/gen_starfield.py → app/src/starfield.rgba
//! 用途: 弱化纯色背景, 形成暗色墙纸氛围; 音符/轨道竖条叠在其上保持可读。
//! include_bytes 相对本文件路径.

pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 353;
pub static BYTES: &[u8] = include_bytes!("starfield.rgba");
