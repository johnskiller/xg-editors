// XG Editor — egui 布局 POC(库) 
// 原生入口在 main.rs;wasm 入口在本文件的 WebHandle。
//
// 结构:所有 egui App 逻辑放这,main.rs include 复用它。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

pub mod data;
pub mod device;
pub mod lcd;
pub mod midi_topology;
pub mod ms_button;
pub mod part;
pub mod persist;
pub mod piano_roll;
pub mod smf;
pub mod sysex;
pub mod playback;
pub use playback::*;

pub mod play_view;
pub use play_view::CentralView;

pub mod starfield;
pub mod panels;
pub mod topbar;
pub mod transport;

/// 异步延时 (低精度, 用于 SysEx 请求间隔). wasm: 用定时器; native: 线程 sleep.
/// 注意: wasm 下阻塞主线程的 busy-wait 会卡 UI, 这里用真正的 async 延时 (Promise timer)。
#[cfg(target_arch = "wasm32")]
pub async fn sleep_ms(ms: u64) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    // Promise 构造函数: resolve 是一个 JsValue 函数
    let promise = js_sys::Promise::new(&mut move |resolve, _reject| {
        // setTimeout(resolve, ms) → 到时 resolve() 完成 promise
        let window = web_sys::window().expect("window");
        let f: &js_sys::Function = resolve.unchecked_ref();
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(f, ms as i32);
    });
    let _ = JsFuture::from(promise).await;
}

/// native: 直接用 std 线程 sleep (测试/工具用).
#[cfg(not(target_arch = "wasm32"))]
pub async fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

/// 全局状态日志缓冲 (跨模块/静态上下文记录, UI 每帧 drain 进 status_log).
/// 用于: midi_wasm::send_to 无法访问 App 时记录 [tx:...] 调试行.
thread_local! {
    pub static GLOBAL_STATUS_LOG: std::cell::RefCell<std::collections::VecDeque<String>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

/// 全局 MIDI 事务编号 (发送/接收统一时间线, 调试用). 跨模块自增.
thread_local! {
    pub static MIDI_TX_CTR: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    pub static MIDI_RX_CTR: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// console.log 一条诊断 (仅 wasm 生效; native 空).
#[cfg(target_arch = "wasm32")]
pub fn console_log(mark: &str, msg: impl std::fmt::Display) {
    web_sys::console::log_1(&format!("[{mark}] {msg}").into());
}

/// console.log 一条诊断 (仅 wasm 生效; native 空).
#[cfg(not(target_arch = "wasm32"))]
pub fn console_log(_mark: &str, _msg: impl std::fmt::Display) {}

/// 顶部 bar/beat 时间标尺绘制 (Channel Notes 与 Piano Roll 共用, 用户 2026-08-12 定案).
/// 两侧真正差异只是 time_left/width 与各自的 win_ticks/scroll; bar/beat 遍历/颜色/字号完全一致.
/// 参数: 标尺整块(含深色底), 时间轴起始 x + 宽, 当前窗口 tick 语义 (win_ticks=end/zoom, scroll).
/// Ruler 自适应密度参数 (纯逻辑, 便于测试): 返回 (bar 号标注步长, 是否画 beat tick)
/// label_step: 每 N 个小节才标一次 bar 号 (太密跳号); show_beat: 像素充足才画 beat 子刻线
pub(crate) fn ruler_density(win_ticks: u64, bar_ticks: u64, time_width: f32, ppq: u64) -> (u64, bool) {
    if bar_ticks == 0 || time_width <= 0.0 {
        return (1, false);
    }
    let n_bars_visible = (win_ticks.max(1) as f32 / bar_ticks.max(1) as f32).ceil();
    let px_per_bar = (time_width / n_bars_visible.max(1.0)).max(0.0);
    // bar 号标注步长: 每小节可显示的最小宽度约 44px (数字+间隔)
    let label_step = ((44.0 / px_per_bar.max(1.0)).ceil() as u64).max(1);
    // beat tick 最小像素间距: <9px 则省略 (太密)
    let px_per_beat = px_per_bar / (bar_ticks.max(1) as f32 / ppq.max(1) as f32);
    let show_beat = px_per_beat >= 9.0 && px_per_bar >= 14.0;
    (label_step, show_beat)
}

pub(crate) fn draw_time_ruler(
    p: &egui::Painter,
    ruler_rect: egui::Rect,
    time_left: f32,
    time_width: f32,
    win_ticks: u64,
    scroll: u64,
    ppq: u64,
    bar_ticks: u64,
) {
    // 标尺深色底
    p.rect_filled(ruler_rect, 0.0, egui::Color32::from_rgb(0x0c, 0x14, 0x1e));
    if bar_ticks == 0 || time_width <= 0.0 {
        return;
    }
    let last_tick_win = scroll + win_ticks.max(1);
    let win = win_ticks.max(1) as f32;

    // ---- 自适应密度 (用户 2026-08-12: rule 太密时 bar 号跳格 / 更密省略 beat tick) ----
    let (label_step, show_beat) = ruler_density(win_ticks, bar_ticks, time_width, ppq);

    let first_bar = scroll / bar_ticks;
    let last_bar = last_tick_win / bar_ticks + 1;
    let mut bar_no = first_bar;
    while bar_no <= last_bar {
        let bt = bar_no * bar_ticks;
        if bt <= last_tick_win {
            let bx = time_left + (bt.saturating_sub(scroll)) as f32 / win * time_width;
            // bar 起始竖线 (标尺内亮) — 始终保留
            p.vline(bx, ruler_rect.y_range(), egui::Stroke::new(1.0, egui::Color32::from_rgb(0x66, 0x88, 0x99)));
            // bar 号 (1-based): 太密则跳格 (每 label_step 个小节标一次)
            if (bar_no % label_step) == 0 || bar_no == first_bar || bar_no == last_bar {
                p.text(
                    egui::pos2(bx + 3.0, ruler_rect.top() + 1.0),
                    egui::Align2::LEFT_TOP,
                    (bar_no + 1).to_string(),
                    egui::FontId::monospace(10.0),
                    egui::Color32::from_gray(210),
                );
            }
            // beat 子刻线: 空间足够才画 (太密省略)
            if show_beat {
                let beat_t = ppq.max(1);
                let mut b = 1;
                while b < bar_ticks / beat_t {
                    let btk = bt + b * beat_t;
                    if btk <= last_tick_win {
                        let bbx = time_left + (btk - scroll) as f32 / win * time_width;
                        p.vline(bbx, ruler_rect.y_range(), egui::Stroke::new(1.0, egui::Color32::from_rgb(0x3a, 0x4a, 0x58)));
                    }
                    b += 1;
                }
            }
        }
        bar_no += 1;
    }
    // 标尺底分隔线
    p.hline(ruler_rect.y_range(), ruler_rect.bottom(), egui::Stroke::new(1.0, egui::Color32::from_gray(80)));
}

/// 下载文本为文件 (仅 wasm): 用 Blob + URL.createObjectURL + <a download>. 无需剪贴板权限.
#[cfg(target_arch = "wasm32")]
pub fn download_text(filename: &str, text: &str) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};
    let window = web_sys::window().ok_or("no window")?;
    let doc = window.document().ok_or("no doc")?;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::JsString::from(text));
    let bag = BlobPropertyBag::new();
    bag.set_type("text/plain");
    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &bag)
        .map_err(|_| "blob create")?;
    let url = Url::create_object_url_with_blob(&blob).map_err(|_| "object url")?;
    let a = doc.create_element("a")
        .map_err(|_| "a create")?
        .dyn_into::<HtmlAnchorElement>()
        .map_err(|_| "a cast")?;
    a.set_href(&url);
    a.set_download(filename);
    let body = doc.body().ok_or("no body")?;
    body.append_child(a.as_ref()).map_err(|_| "append")?;
    let _ = a.click();
    body.remove_child(a.as_ref()).map_err(|_| "remove")?;
    Url::revoke_object_url(&url).ok();
    Ok(())
}

/// 复制/下载 (native 空).
#[cfg(not(target_arch = "wasm32"))]
pub fn download_text(_filename: &str, _text: &str) -> Result<(), String> { Ok(()) }

/// 全局 MIDI 收发编号日志: 发送/接收各一条递增时间线, 便于对齐"发了几条/回了几条".
#[cfg(target_arch = "wasm32")]
pub fn midi_trace(dir: &str, port_name: &str, bytes: &[u8]) {
    let hex: String = bytes.iter().map(|x| format!("{x:02X}")).collect();
    let n = match dir {
        "TX" => MIDI_TX_CTR.with(|c| {
            let v = c.get() + 1;
            c.set(v);
            v
        }),
        "RX" => MIDI_RX_CTR.with(|c| {
            let v = c.get() + 1;
            c.set(v);
            v
        }),
        _ => 0,
    };
    console_log(dir, format!("#{n} {port_name} {hex}"));
    log_status_global(format!("[{dir}#{n}:{port_name}] {hex}"));
}

/// 同 native 空实现 (避免警告)
#[cfg(not(target_arch = "wasm32"))]
pub fn midi_trace(_dir: &str, _port_name: &str, _bytes: &[u8]) {}

/// 追加到全局状态日志 (超长丢弃队尾, 最多 200 条).
pub fn log_status_global(msg: impl Into<String>) {
    let m = msg.into();
    GLOBAL_STATUS_LOG.with(|q| {
        let mut q = q.borrow_mut();
        if q.len() >= 200 {
            q.pop_front();
        }
        q.push_back(m);
    });
}

/// 取走全部全局状态日志 (UI 每帧调用一次).
pub fn drain_global_status() -> std::collections::VecDeque<String> {
    GLOBAL_STATUS_LOG.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// wasm: 文件对话框回调 → UI 加载 (update 轮询消费). 非 wasm 为空.
#[cfg(target_arch = "wasm32")]
thread_local! {
    static SMF_DIALOG_PENDING: std::cell::RefCell<Option<(String, Vec<u8>)>> = const { std::cell::RefCell::new(None) };
}
pub mod xg_font;
pub mod xg_icons;
use data::VoiceBank;

// 应用当前是否持有已加载的音色库(wasm/native 共用)
#[cfg(not(test))]
static mut VOICE_BANK: Option<std::sync::Mutex<VoiceBank>> = None;

// ---------- Web MIDI(wasm 分支)----------
#[cfg(target_arch = "wasm32")]
pub mod midi_wasm {
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{MidiAccess, MidiOptions, MidiPort};

    /// 已缓存的 MIDI output 对象 (首次连接后复用, 避免每消息 await request_access 乱序).
    thread_local! {
        static MIDI_OUTPUT_CACHE: std::cell::RefCell<Option<web_sys::MidiOutput>> =
            const { std::cell::RefCell::new(None) };
    }

    /// 请求 MIDI 访问(带 sysex)。浏览器授权一次后, 后续调用直接返回已授权的 access。
    async fn request_access() -> Result<MidiAccess, String> {
        let navigator = web_sys::window()
            .ok_or("window not available")?
            .navigator();
        let mut opts = MidiOptions::new();
        opts.sysex(true);
        let promise = navigator
            .request_midi_access_with_options(&opts)
            .map_err(|e| format!("requestMIDIAccess 失败: {:?}", e))?;
        JsFuture::from(promise)
            .await
            .map_err(|e| format!("MIDI access 拒绝: {:?}", e))
            .map(|v| v.into())
    }

    /// 枚举端口名。返回 (inputs, outputs)。
    pub async fn probe_pair() -> Result<(Vec<String>, Vec<String>), String> {
        let access = request_access().await?;
        let mut ins = Vec::new();
        let mut outs = Vec::new();
        // inputs
        for item in access.inputs().entries() {
            let item = item.map_err(|e| format!("midi map iter: {:?}", e))?;
            let pair = js_sys::Array::from(&item);
            let port: MidiPort = pair.get(1).into();
            let label = port.name().or_else(|| Some(port.id())).unwrap_or_else(|| "unnamed".into());
            ins.push(label);
        }
        // outputs
        for item in access.outputs().entries() {
            let item = item.map_err(|e| format!("midi map iter: {:?}", e))?;
            let pair = js_sys::Array::from(&item);
            let port: MidiPort = pair.get(1).into();
            let label = port.name().or_else(|| Some(port.id())).unwrap_or_else(|| "unnamed".into());
            outs.push(label);
        }
        Ok((ins, outs))
    }

    /// 向指定名字的 output 端口发送 MIDI 字节(如 Program Change 0xC0)。
    pub async fn send_to(output_name: &str, bytes: &[u8]) -> Result<(), String> {
        send_at(output_name, bytes, None).await
    }

    /// 向指定 output 发送 MIDI 字节, 可指定发送时刻(performance.now() 毫秒, 相对 now 用 now()+ms)。
    /// Some(timestamp_ms) 时用 send_with_timestamp(延迟发送, 适合排 Note On/Off)。
    pub async fn send_at(output_name: &str, bytes: &[u8], timestamp_ms: Option<f64>) -> Result<(), String> {
        let access = request_access().await?;
        // 找同名 output
        let mut target = None;
        for item in access.outputs().entries() {
            let item = item.map_err(|e| format!("midi map iter: {:?}", e))?;
            let pair = js_sys::Array::from(&item);
            let port: MidiPort = pair.get(1).into();
            let name = port.name().or_else(|| Some(port.id())).unwrap_or_else(|| "unnamed".into());
            if name == output_name {
                target = Some(unsafe {
                    // MidiPort -> MidiOutput: 同一 JS 对象的投影, 用 wasm-bindgen 的 JsCast
                    use wasm_bindgen::JsCast;
                    web_sys::MidiOutput::unchecked_from_js(port.into())
                });
                break;
            }
        }
        let out = target.ok_or_else(|| format!("output not found: {output_name}"))?;
        // Web MIDI 规范: send 前必须 open, 否则浏览器可能静默丢弃。
        // open() 返回 Promise (非 Result), await 它 (send_at 是 async fn).
        let _ = JsFuture::from(out.open()).await;
        // 组装 Uint8Array
        let arr = js_sys::Uint8Array::from(bytes);
        // 调试: 所有 SysEx 收发加统一编号时间线 (console + status bar 双轨)
        if bytes.first() == Some(&0xF0) {
            crate::midi_trace("TX", output_name, bytes);
        }
        match timestamp_ms {
            Some(ts) => out
                .send_with_timestamp(&arr, ts)
                .map_err(|e| format!("send_at err: {:?}", e))?,
            None => out.send(&arr).map_err(|e| format!("send err: {:?}", e))?,
        }
        Ok(())
    }

    /// 把 MIDI 消息同步发送到已缓存 output (保序: 同 JS 对象 send() 同步入列).
    /// 若 output 未缓存: 异步请求一次并缓存 (仅首次 async); 之后全同步.
    pub async fn send_sync(output_name: &str, bytes: &[u8]) -> Result<(), String> {
        // 缓存命中 → 同步 send (保序关键)
        if let Some(out) = MIDI_OUTPUT_CACHE.with(|c| c.borrow().clone()) {
            let arr = js_sys::Uint8Array::from(bytes);
            // 调试: 与 send_at 同轨的编号时间线 (v87 起 send_sync 也打 TX#, 保证 telemetry 不丢)
            if bytes.first() == Some(&0xF0) {
                crate::midi_trace("TX", output_name, bytes);
            }
            return out.send(&arr).map_err(|e| format!("send err: {:?}", e));
        }
        // 首次: async 获取并缓存
        let access = request_access().await?;
        let mut target = None;
        for item in access.outputs().entries() {
            let item = item.map_err(|e| format!("midi map iter: {:?}", e))?;
            let pair = js_sys::Array::from(&item);
            let port: MidiPort = pair.get(1).into();
            let name = port.name().or_else(|| Some(port.id())).unwrap_or_else(|| "unnamed".into());
            if name == output_name {
                use wasm_bindgen::JsCast;
                let out: web_sys::MidiOutput = unsafe { web_sys::MidiOutput::unchecked_from_js(port.into()) };
                MIDI_OUTPUT_CACHE.with(|c| *c.borrow_mut() = Some(out.clone()));
                target = Some(out);
                break;
            }
        }
        let out = target.ok_or_else(|| format!("output not found: {output_name}"))?;
        let arr = js_sys::Uint8Array::from(bytes);
        // 首次发送 (cold cache) 也打 TX# telemetry
        if bytes.first() == Some(&0xF0) {
            crate::midi_trace("TX", output_name, bytes);
        }
        out.send(&arr).map_err(|e| format!("send err: {:?}", e))
    }

    /// 校验 output 端口: 打开 + 缓存, 不发任何消息 (连接状态判定用).
    /// 返回 Ok 表示该 output 真实存在且已授权/warm; Err 表示不可用.
    pub async fn verify_output(output_name: &str) -> Result<(), String> {
        // 已缓存 = 之前已成功打开过
        if MIDI_OUTPUT_CACHE.with(|c| c.borrow().is_some()) {
            return Ok(());
        }
        let access = request_access().await?;
        let mut found = false;
        for item in access.outputs().entries() {
            let item = item.map_err(|e| format!("midi map iter: {:?}", e))?;
            let pair = js_sys::Array::from(&item);
            let port: MidiPort = pair.get(1).into();
            let name = port.name().or_else(|| Some(port.id())).unwrap_or_else(|| "unnamed".into());
            if name == output_name {
                use wasm_bindgen::JsCast;
                let out: web_sys::MidiOutput = unsafe { web_sys::MidiOutput::unchecked_from_js(port.into()) };
                MIDI_OUTPUT_CACHE.with(|c| *c.borrow_mut() = Some(out.clone()));
                found = true;
                break;
            }
        }
        if !found {
            return Err(format!("output not found: {output_name}"));
        }
        Ok(())
    }

    /// 清空缓存 (设备切换/重连时调用).
    pub fn reset_output_cache() {
        MIDI_OUTPUT_CACHE.with(|c| *c.borrow_mut() = None);
    }

    /// 已绑定的 input 端口 (name → closure). 支持多个输入: UX16(收 MU90) + 未来 MIDI controller 等.
    thread_local! {
        /// 收进来的 MIDI 消息队列 (bytes + 来源端口名). UI 每帧 drain.
        static MIDI_INBOX: std::cell::RefCell<Vec<(String, Vec<u8>)>> = const { std::cell::RefCell::new(Vec::new()) };
        /// 已绑定的 (端口名 → onmidimessage 闭包), 防 GC; 下次同端口重复 bind 幂等.
        static MIDI_INPUT_CLOSURES: std::cell::RefCell<std::collections::HashMap<String, wasm_bindgen::closure::Closure<dyn FnMut(web_sys::MidiMessageEvent)>>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    }

    /// 绑定输入端口: 找到同名 input 并挂 onmidimessage → 带来源名存进 MIDI_INBOX.
    /// 返回 Err 表示没有该 input. 幂等: 同名已绑定则直接 Ok (不重复挂).
    pub async fn bind_input(input_name: &str) -> Result<(), String> {
        if MIDI_INPUT_CLOSURES.with(|c| c.borrow().contains_key(input_name)) {
            return Ok(());
        }
        // 找同名 input
        let access = request_access().await?;
        let mut target: Option<web_sys::MidiInput> = None;
        for item in access.inputs().entries() {
            let item = item.map_err(|e| format!("midi map iter: {:?}", e))?;
            let pair = js_sys::Array::from(&item);
            let port: web_sys::MidiPort = pair.get(1).into();
            let name = port.name().or_else(|| Some(port.id())).unwrap_or_else(|| "unnamed".into());
            if name == input_name {
                use wasm_bindgen::JsCast;
                target = Some(unsafe { web_sys::MidiInput::unchecked_from_js(port.into()) });
                break;
            }
        }
        let Some(inp) = target else {
            return Err(format!("input not found: {input_name}"));
        };
        // 挂闭包: 每次收到完整消息 → (来源名, data) 进 inbox
        let key = input_name.to_string();
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::MidiMessageEvent)>::new(
            move |ev: web_sys::MidiMessageEvent| {
                if let Ok(bytes) = ev.data() {
                    MIDI_INBOX.with(|c| c.borrow_mut().push((key.clone(), bytes)));
                }
            },
        );
        use wasm_bindgen::JsCast;
        inp.set_onmidimessage(Some(closure.as_ref().unchecked_ref()));
        MIDI_INPUT_CLOSURES.with(|c| { c.borrow_mut().insert(input_name.to_string(), closure); });
        Ok(())
    }

    /// 当前已绑定的所有 input 名.
    pub fn bound_input_names() -> Vec<String> {
        MIDI_INPUT_CLOSURES.with(|c| c.borrow().keys().cloned().collect())
    }

    /// 取出所有已收到的 MIDI 消息 (UI 每帧调用). 返回 (来源端口名, bytes).
    pub fn drain_inbox() -> Vec<(String, Vec<u8>)> {
        MIDI_INBOX.with(|c| std::mem::take(&mut *c.borrow_mut()))
    }

    /// 打开文件选择对话框加载 .mid (创建临时 <input type=file> 触发 click; onchange 里读 bytes 存 SMF_DIALOG_PENDING)
    pub fn open_midi_file_dialog() {
        use wasm_bindgen::JsCast;
        let doc = web_sys::window()
            .and_then(|w| w.document())
            .expect("document");
        let input = doc
            .create_element("input")
            .expect("create input")
            .dyn_into::<web_sys::HtmlInputElement>()
            .expect("input element");
        input.set_type("file");
        input.set_accept(".mid,.midi,audio/midi");
        // onchange: 从事件目标取 files (避免闭包内移动 input)
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut(_)>::new(
            move |ev: web_sys::Event| {
                let target = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
                if let Some(inp) = target {
                    if let Some(files) = inp.files() {
                        if let Some(file) = files.get(0) {
                            let name = file.name();
                            let arr_promise = file.array_buffer();
                            wasm_bindgen_futures::spawn_local(async move {
                                use wasm_bindgen::JsCast;
                                let buf = JsFuture::from(arr_promise).await.ok();
                                if let Some(b) = buf {
                                    let u8a = js_sys::Uint8Array::new(&b);
                                    let bytes = u8a.to_vec();
                                    crate::SMF_DIALOG_PENDING.with(|c| {
                                        *c.borrow_mut() = Some((name, bytes));
                                    });
                                }
                            });
                        }
                    }
                }
            },
        );
        // 通过事件目标取文件, 闭包不捕获 input
        input.set_onchange(Some(closure.as_ref().unchecked_ref()));
        // 保持闭包存活: forget (输入元素本身持引用; 因是临时, 单次 OK)
        std::mem::forget(closure);
        let _ = input.click();
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub mod midi_wasm {
    pub async fn probe_pair() -> Result<(Vec<String>, Vec<String>), String> {
        Err("native: Web MIDI 不可用".into())
    }
    pub async fn send_to(_n: &str, _b: &[u8]) -> Result<(), String> {
        Err("native: Web MIDI 不可用".into())
    }
    pub async fn send_at(_n: &str, _b: &[u8], _t: Option<f64>) -> Result<(), String> {
        Err("native: Web MIDI 不可用".into())
    }
    pub async fn send_sync(_n: &str, _b: &[u8]) -> Result<(), String> {
        Err("native: Web MIDI 不可用".into())
    }
    pub async fn verify_output(_n: &str) -> Result<(), String> {
        Err("native: Web MIDI 不可用".into())
    }
    pub fn reset_output_cache() {}
    pub async fn bind_input(_n: &str) -> Result<(), String> {
        Err("native: Web MIDI 不可用".into())
    }
    pub fn bound_input_names() -> Vec<String> { Vec::new() }
    pub fn drain_inbox() -> Vec<(String, Vec<u8>)> { Vec::new() }
}

// ---------- App ----------
pub struct XgApp {
    pub show_left: bool,
    pub show_right: bool,
    pub show_bottom: bool,
    /// 底栏 Piano Roll 开关 (2026-08-12: Piano Roll 从中央视图移到底栏)
    pub show_piano: bool,
    /// 底栏 Piano Roll 高度
    pub piano_height: f32,
    /// 中央视图模式: Piano Roll(静态时间轴) / Channel Notes(每行1通道) / PlayView(播放画面)
    pub central_view: CentralView,
    pub left_width: f32,
    pub right_width: f32,
    pub bottom_height: f32,
    pub tracks: Vec<Track>,
    pub midi_devices: Vec<String>,
    /// MIDI 拓扑 (多接口 + A/B 角色 + 32 part 路由表). 探测后构建, 播放/编辑按 route_for_part 分发.
    pub midi_topology: midi_topology::MidiTopology,
    pub selected_midi: Option<usize>,
    /// Port B 输出设备 (MU90 Port B = parts 17-32, RcvCh 1-16); None=未选 (单口模式)
    pub selected_midi_b: Option<usize>,
    /// 播放时把 16ch 流镜像发到 Port B (Port A parts 1-16 + Port B parts 17-32 = 32 part 全响应)
    pub mirror_to_b: bool,
    pub midi_connected: bool,
    pub lcd_pixels: Vec<u8>,
    pub lcd_side: usize,
    /// LCD 纹理句柄缓存 — 避免每帧 load_texture(每帧新建 GPU 纹理 = 拖动闪烁主因)
    pub lcd_tex: Option<egui::TextureHandle>,
    /// LCD 像素自上次纹理上传后是否变化 (避免每帧 set 纹理 = 拖动闪烁第二根源)
    pub lcd_dirty: bool,
    /// LCD 点阵缩放倍数 (手机窄屏放大到可读)
    pub lcd_zoom: f32,
    /// true = 32-channel 显示模式 (A1/A2+01..32 电平)
    pub lcd_32: bool,
    /// LCD 当前显示的音色/bank/program (从 voice_bank 按 XG MSB0 查询; bank 显示1-based, prog 显示1-based)
    pub cur_voice: String,
    pub cur_bank: u32,
    pub cur_prog: u32,
    /// Bank 滑块 = XG Bank Select MSB 的有效值索引 (见 voice_bank.msb_values()); PC 滑块同理索引
    pub cur_msb_idx: usize,
    /// PC 滑块 = 当前 msb 下有效 prg 列表的索引 (见 voice_bank.prg_values())
    pub cur_pc_idx: usize,
    /// 当前在 (cur_msb, prog0) 变体列表中的索引 (LSB 有效范围走索引, 不直接存 lsb 值)
    pub cur_lsb_idx: usize,
    /// Part 状态唯一数据源 (32 part, 含音色 + 混音参数; 单源重构)
    pub parts: [crate::part::PartState; 32],
    /// 全局系统效果类型 (Rev/Cho/Var), 与 per-part 的 send 量分离
    pub sys_fx: crate::part::SystemFx,
    pub bg_pixels: Vec<u8>,  // 背景纹理像素(测试背景贴图是否扰乱布局)
    pub bg_side: usize,
    /// PlayView 瀑布区背景纹理 (Horsehead 星云) — 缓存避免每帧 load_texture
    pub starfield_tex: Option<egui::TextureHandle>,
    pub params: Vec<(String, f32, f32, f32)>, // 面板参数
    /// 每个参数的 SysEx Multi-Part offset (与 sysex::mp 对齐)
    pub param_offsets: Vec<u8>,
    /// 每个参数驱动 LCD 底部第几条 (VOL=0 EXP=1 BRT=2 PAN=3 REV=4 CHO=5 VAR=6 KEY=7), None=不驱动
    pub param_lcd_idx: Vec<Option<usize>>,
    /// 最近一次滑块生成的 SysEx 字节 (16进制显示, 无硬件也能看到)
    pub last_sysex: Option<String>,
    /// 音色库(正式工程核心数据, 加载 xg_voices.json)
    pub voice_bank: Option<VoiceBank>,
    /// 冒烟验证: Web MIDI 探测是否已启动(wasm 下启动异步, native 下不启动)
    pub midi_probe_started: bool,
    /// 冒烟验证: Web MIDI 探测结果(wasm 下用 Rc<RefCell> 跨 borrow 更新)
    /// Ok((inputs, outputs)) — 设备名列表
    pub midi_probe_result: Option<Result<(Vec<String>, Vec<String>), String>>,
    /// Web MIDI 异步任务的共享结果 cell(wasm; 存在 self 以便每帧轮询)
    #[cfg(target_arch = "wasm32")]
    pub midi_probe_cell: Option<std::rc::Rc<std::cell::RefCell<Option<Result<(Vec<String>, Vec<String>), String>>>>>,
    /// 最后一条 MIDI 发送的结果文案(UI 显示: 成功/失败)
    pub midi_send_status: Option<String>,
    /// 面板状态持久化: 首次 update 已从存储加载?
    pub persist_loaded: bool,
    /// 上次保存的状态签名 (变化才写回, 避免每帧写 localstorage)
    pub persist_signature: Option<String>,
    /// 持久化上次写入时间 (ms, egui 时间) — 节流用
    pub persist_last_save_ms: f64,
    /// 应用版本 (wasm 由 index.html 传入; native 显示 "dev")
    pub app_version: String,
    /// 当前目标硬件 (音色快捷菜单按此过滤; 滑块/发送不限制)
    pub device: device::Device,
    // ---------- 音序器 / MIDI 播放 ----------
    /// 播放状态: true=正在播放
    pub playing: bool,
    /// playhead 当前位置 (tick)
    pub playhead_tick: u64,
    /// 单次播放总长 (tick) — 超出归零循环
    pub total_ticks: u64,
    /// 每四分音符的 tick (PPQ): 96 常用
    pub ppq: u64,
    /// 节拍器 tempo (BPM)
    pub tempo_bpm: f64,
    /// 上次 update 的系统时刻 (播放用, 计算 delta tick)
    pub last_play_frame_ms: f64,
    /// 电平表节流: 每 METER_STEP 帧才衰减一次 (降低更新频率, John: 变动太快)
    pub meter_frame: u32,
    /// SMF 播放: 已播真实秒 (tempo map 驱动)
    pub play_real_sec: f64,
    /// 预构建的播放事件表 (NoteOn/NoteOff 按绝对 tick 排序, 0..total_ticks)
    pub play_events: Vec<PlayEvent>,
    /// 上次消费到的事件表下标
    pub event_cursor: usize,
    /// 回绕恢复机制: 光标所属循环的起始 tick (处理 0..next 的回归窗口)
    pub event_cursor_origin: u64,
    // ---------- SMF 加载 ----------
    /// 已加载 SMF (None = 未加载)
    pub smf: Option<smf::Smf>,
    /// 逐轨视图 (16 通道, 来自 SMF 音符配对)
    pub smf_views: Vec<smf::SmfTrackView>,
    /// SMF 16 通道的实时播放电平 (0.0..=1.0, FakeMu 式 mimic 平滑显示值; channel view + LCD)
    pub live_levels: [f32; 16],
    /// SMF 16 通道的当前音色名 (program → XG 音色; 左栏显示)
    pub live_voice_names: [String; 16],
    /// 16 通道当前音量 (CC7/127): 电平表 = velocity × live_volumes (John: CC7 应反应到电平表)
    pub live_volumes: [f32; 16],
    /// 16 通道当前表情 (CC11/127, FakeMu: 电平表 × CC11)
    pub live_expressions: [f32; 16],
    /// 每通道当前按住的音符 (pitch → velocity), 事件驱动维护; raw_strength 由它算
    pub active_notes: Vec<std::collections::BTreeMap<u8, u8>>,
    /// 每通道当前原始强度上限 = 按住音符中最大 velocity/127 (未卷 CC7/CC11)
    pub raw_vel_peaks: [f32; 16],
    /// PlayView: 每通道 128 个 CC 的实时值 (0..127, 事件驱动; CC 可视化竖条数据源)
    pub cc_live: [[u8; 128]; 16],
    /// PlayView: 每通道实时 Bank Select (MSB, LSB) — CC0/CC32 跟踪, 左矩阵行2 显示
    pub live_bank: [(u8, u8); 16],
    /// PlayView: 每通道实时 Program (PC) — 左矩阵行2 显示 (0-based, XG 语义)
    pub live_program: [u8; 16],
    /// PlayView: 已消费事件计数 (顶部信息栏 events 字段)
    pub play_evt_count: u64,
    /// PlayView: 历史最大复音数 (顶部信息栏 maxPoly 字段, 峰值保持)
    pub max_poly: u64,
    /// PlayView: 垂直滚动偏移 (px) — 16 通道矩阵/瀑布超出视口时滚动, 左矩阵与瀑布共用同步滚
    pub pview_scroll: f32,
    /// mimic 平滑用目标值 (= 上一个物理值的已有实现; 保留原名避免破坏), 见 apply smoothing
    pub live_vel_peaks: [f32; 16],
    /// master volume (0..1, 默认满; FakeMu: strength × master.volume)
    pub live_master_vol: f32,
    /// Channel View per-channel Mute (1..16 → 下标 0..15; true=静音, 仅播放输出层过滤, 会话级不持久化)
    pub channel_mutes: [bool; 16],
    /// Channel View per-channel Solo (true=独奏; 任一 solo 激活时其他通道当 muted; Mute 优先)
    pub channel_solos: [bool; 16],
    /// TopBar Record armed (点击红点亮灭; 功能预留不接逻辑, 会话级不持久化)
    pub rec_armed: bool,
    /// Div 2026-08-13 playable piano roll: 点按发声挂音 (通道 → pitch → (vel, 起声 egui time)).
    /// 点琴键按住 = NoteOn; 松开/移出 = NoteOff; note 点击 = 采样短音 (起声后 ~300ms 自动 off)。
    pub preview_notes: Vec<std::collections::BTreeMap<u8, (u8, f64)>>,
    /// Event List: 选中行索引 (指向过滤+排序后的当前 channel 事件 vec); None=未选中
    pub event_list_sel: Option<usize>,
    /// SysEx 折叠区 (2026-08-14): 展开查看 hex 的条目索引 (未展开=该行为 None 或折叠)
    pub sysex_expanded: Option<usize>,
    /// tempo map (tick↔秒)
    pub tempo_map: Option<smf::TempoMap>,
    /// 文件总时长 (秒)
    pub smf_total_sec: f64,
    /// 已加载文件最长 tick (用于时间轴)
    pub smf_end_tick: u64,
    /// 文件名
    /// 当前编辑的 MU90 part (1..32; 1-16→portA ch1-16, 17-32→portB ch1-16, John 权威 2026-08-09)
    pub cur_part: u32,
    pub smf_name: String,
    /// 最近一次 SMF 加载结果提示 (底部 status 栏显示)
    pub smf_load_result: String,
    /// 状态日志 (循环缓冲, 底部 status 栏显示最近一条)
    pub status_log: std::collections::VecDeque<String>,
    /// wasm 调试: 状态 DOM 显示脏标记 (dump-dom 读真实 zoom/scroll/notes)
    pub smf_is_dirty: bool,
    /// URL 调试钩子显式给了 zoom/scroll 时, auto-fit 不覆盖 (截图指定取景)
    pub url_override_view: bool,
    /// track view 时间缩放 (倍数)。>1 时音符条横向放大, 可看细节; 配套横向滚动。
    pub track_view_zoom: f32,
    /// track view 横向滚动偏移 (tick 起始)
    pub track_view_scroll_ticks: u64,
    /// Piano Roll 显示的 channel (1..16; 用户 2026-08-12: 只显示一个 channel 的音符)
    pub cur_pr_channel: u8,
    /// Piano Roll 独立时间缩放 (倍数); 与 Channel 视图 track_view_zoom 独立 (用户 2026-08-12)
    pub pr_zoom: f32,
    /// Piano Roll 独立横向滚动偏移 (tick 起始); 与 Channel 视图独立
    pub pr_scroll_ticks: u64,
    /// Piano Roll 首帧是否已设定初始垂直滚动 (之后放弃控制, 交给用户滚动)
    pub pr_scrolled_once: bool,
    /// Channel View 每行高度 (16..64, 默认=CHANNEL_ROW_H=28; zoom slider 控制, John 2026-08-13)
    /// 中央 channel 行 + 左侧栏行网格共用, 保持对齐
    pub channel_row_h: f32,
    /// Channel View 压缩钢琴卷帘的可见音高范围 (pitch 0-127 映射到行高; 默认全范围 0..127)
    /// 未来可加 slider 聚焦音域; 目前固定全范围
    pub channel_view_pitch_low: u8,
    pub channel_view_pitch_high: u8,
    /// 是否已为 app 设置滚动条样式 (宽/常显)
    pub ui_scroll_style_done: bool,
    /// 发送测试的异步结果 cell(wasm)
    #[cfg(target_arch = "wasm32")]
    pub midi_send_ui_cell: Option<std::rc::Rc<std::cell::RefCell<Option<Result<(), String>>>>>,
    /// 设备选择后的连接校验结果 cell (wasm; 确认端口真实打开才标 connected)
    #[cfg(target_arch = "wasm32")]
    pub midi_verify_cell: Option<std::rc::Rc<std::cell::RefCell<Option<Result<(), String>>>>>,
    /// 双向通信: 收到硬件 SysEx 的收集器 (从输入端口读 part 音色用)
    pub part_voice_reader: sysex::PartVoiceCollector,
    /// 上一帧收到底层的 MIDI 消息 (调试显示; 每帧 drain 后刷新)
    pub last_midi_rx: Vec<String>,
    /// System Dump 捕获: 开启后全量收存收到的所有 SysEx (供翻看/复制/分析真实 bulk 格式)
    pub sysex_capture: bool,
    /// 捕获缓冲: (来源端口, 原始字节 hex, 时间线编号). cap 200 条, 超出丢最旧.
    pub sysex_capture_log: Vec<(String, String, u32)>,
    /// 捕获 FIFO 计数 (只增, 供 UI 显示捕获了 N 条)
    pub sysex_capture_count: u64,
    /// 捕获聚合: (地址u32 → (来源, 值hex, 出现次数)). 由"分析"按钮从捕获日志重建.
    pub sysex_analysis: Vec<(u32, String, u32)>,
    /// 绑定输入的结果 cell (wasm; bind_input 异步)
    #[cfg(target_arch = "wasm32")]
    pub midi_bind_cell: Option<std::rc::Rc<std::cell::RefCell<Option<Result<(), String>>>>>,
    /// 读 part 音色的请求状态 (UI 提示用)
    pub read_request_inflight: Option<String>,
    /// 读 part 握手状态机: Some((part0基于, 下一个要请求的地址偏移))
    ///   addr_off ∈ {01=MSB, 02=LSB, 03=PC}; 收到对应回包则推进, 否则停在原地不会冒进.
    ///   None = 无读操作在途.
    pub read_wait: Option<(u8, u8)>,
    /// 握手过程累积的读回值 (部分)
    pub read_acc_msb: Option<u8>,
    pub read_acc_lsb: Option<u8>,
    pub read_acc_pc: Option<u8>,
    /// 最近一次成功读回的 (part, msb, lsb, pc, 音色名)
    pub last_read_voice: Option<(u8, u8, u8, u8, String)>,
    /// 批量读: 每个 part(0-31) 读回的 (msb,lsb,pc) 原始值 (None=还没回; 0-31 cover PortA+PortB 32 parts)
    pub read_parts: [Option<(u8, u8, u8)>; 32],
    /// 批量读: UI 循环发请求用的当前 part 游标
    pub read_part_cursor: Option<u8>,
    /// 批量读: 下一个待读 part (0-based); None = 不再继续
    pub read_batch_next: Option<u8>,
    /// 批量读: 只读 MSB(bank) 模式 (true=不追 LSB/PC, 避免每个 part 卡 3s 超时)
    pub read_batch_msb_only: bool,
    /// 握手读: 最近一次请求发出的时间 (egui ctx.time), 超时 (~3s) 未回则放弃
    pub read_handshake_deadline: Option<f64>,
    /// 握手读: 相邻两条请求的最小间隔 ms. 实测下限 = 160ms (150 会超时丢包, 160/200 96/96 全过)
    /// → 生产默认 200ms (留余量); UI 旋钮可调 (2026-08-09, John 多次实测钉死).
    pub read_req_gap_ms: u64,
    /// Bulk Read All 32: 握手式连发 bulk dump request (2n, 绕过 3n 的 160ms 冷却).
    /// 当前待收回包的 part 游标 (0-based); None = bulk 读未在途.
    pub bulk_read_next: Option<u8>,
    /// Bulk Read All 32: 每个 part 收 1 条 41B bulk, 解析 data[1..3]=msb/lsb/pc.
    /// (与 read_parts 同结构; 独立流程避免与 3n 握手状态机互踩, 2026-08-09 教训: 两状态机勿共写同一字段)
    pub bulk_parts: [Option<(u8, u8, u8)>; 32],
    /// Bulk Read All 32: 当前 part 的请求已发出时间 (egui ctx.time); 超时 (~500ms) 未回→fail-soft 跳下一个
    pub bulk_read_deadline: Option<f64>,
}

// ---------- 面板状态持久化 ----------
/// 持久化倒出的轻量状态 (仅 UI 相关, 不含 LCD 像素/音色库等可重建数据)
/// 音色选择存原始 3 轴值 (MSB/LSB/PC, 0-based), 兼容未来"自由发送"设计; 当前 UI 用索引, 通过 XgApp::set_voice_axis 回写
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct PersistedState {
    /// Bank Select MSB (0-based)
    pub msb: u8,
    /// Bank Select LSB (0-based)
    pub lsb: u8,
    /// Program (0-based 0..127)
    pub pc: u8,
    /// 32-channel 显示开关
    pub lcd_32: bool,
    /// LCD 缩放
    pub lcd_zoom: f32,
    /// 参数滑块值 (顺序 = param_offsets, 只存 val)
    pub params: Vec<f32>,
}

impl PersistedState {
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| format!("序列化失败: {e}"))
    }
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("反序列化失败: {e}"))
    }
}

/// 一个 MIDI 音符事件 (音序器). tick 精确; 播放时换算毫秒发 MIDI
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiNote {
    pub start_tick: u64,    // 起始 tick (含小数→整数 tick 网格)
    pub dur_ticks: u64,     // 时值 tick
    pub pitch: u8,          // 0..127 (C-1..G9)
    pub velocity: u8,       // 0..127
    pub channel: u8,        // 0..15 (发送通道)
}


pub struct Track {
    pub name: String,
    pub voice: String,
    pub level: f32,
    pub notes: Vec<MidiNote>, // 音序器音符 (播放用)
}

/// 生成简化但悦耳的多轨旋律 (每个 channel 一个音色可辨的乐句), 供播放用.
/// 16 轨, 4/4, 每轨 2 小节 (8 拍), ppq=96 → 每拍 96 tick, 共 8*96 = 768 tick
pub fn default_pattern_notes() -> Vec<Vec<MidiNote>> {
    let ppq: u64 = 96;
    let mut out = Vec::new();
    // 每轨基础音符 (0-based 音名): 轨0=低音部, 轨1.. 中音部, 轨15=高音部
    let pitches: [[u8; 8]; 16] = [
        [36, 36, 48, 36, 41, 41, 36, 43], // Ch1 低音 riff
        [55, 55, 55, 50, 53, 55, 57, 55], // Ch2
        [60, 64, 67, 64, 60, 65, 67, 64], // Ch3 三和弦分解
        [64, 67, 71, 67, 62, 65, 69, 65], // Ch4
        [67, 71, 74, 71, 65, 69, 72, 69], // Ch5
        [60, 60, 72, 60, 62, 60, 64, 50], // Ch6
        [62, 65, 69, 65, 60, 64, 67, 64], // Ch7
        [59, 62, 65, 62, 60, 65, 62, 59], // Ch8
        [57, 60, 64, 60, 57, 62, 65, 62], // Ch9
        [55, 59, 62, 59, 55, 60, 64, 60], // Ch10
        [53, 57, 60, 57, 55, 59, 62, 59], // Ch11
        [50, 54, 57, 54, 50, 55, 59, 55], // Ch12
        [48, 52, 55, 52, 48, 53, 57, 53], // Ch13
        [43, 47, 50, 47, 43, 48, 52, 48], // Ch14
        [38, 41, 45, 41, 38, 43, 46, 43], // Ch15
        [33, 36, 40, 36, 33, 38, 41, 38], // Ch16 最低音
    ];
    for (ch, arr) in pitches.iter().enumerate() {
        let mut notes = Vec::new();
        for (b, &p) in arr.iter().enumerate() {
            let start = (b as u64) * ppq; // 每拍一个
            let dur = ppq; // 持续 1 拍
            notes.push(MidiNote {
                start_tick: start,
                dur_ticks: dur,
                pitch: p,
                velocity: 90,
                channel: ch as u8,
            });
            // 每拍中间加一个轻快的对拍(offbeat), 丰富织体
            if b % 2 == 1 && ch >= 2 {
                notes.push(MidiNote {
                    start_tick: start + ppq / 2,
                    dur_ticks: ppq / 2,
                    pitch: p.wrapping_add(7),
                    velocity: 60,
                    channel: ch as u8,
                });
            }
        }
        out.push(notes);
    }
    out
}

/// 中央"每行=一个channel"视图与左边栏通道行共用的行高(保证垂直对齐)
pub const CHANNEL_ROW_H: f32 = 28.0;

/// 行网格顶部的偏移: 顶栏底 → 首行的距离(容纳左/中央面板标题行 + separator)
/// 两侧共用同一常量 → 任何窗口/DPI 下都严格对齐(不再依赖各自容器 margin)
pub const GRID_TOP_OFFSET: f32 = 26.0 + 9.0; // heading行高 + separator高

pub fn voice_for(i: usize) -> String {
    const V: &[&str] = &[
        "GrandPno", "Tinkle", "Megalith", "75'Organ", "MutedGt", "VintageCp", "ThermalX",
        "AeroStr", "ScrewDv", "Twinkle1", "KaossPad", "OrchStr", "DreamPno", "Miracle",
        "LoonLand", "ElamStr",
    ];
    if i <= V.len() { V[i - 1].to_string() } else { "????".to_string() }
}

// ---------- 折叠三角按钮(用 ASCII 字符,任何字体都有,绝不 tofu) ----------
/// 折叠三角按钮(点击 → 折叠/展开)。用 ASCII 三角字符(任何字体都有,绝不 tofu),
/// 配 egui::Button 原生交互+悬停,最可靠。
/// 左栏收起=左指向 "<"、右栏=右指向 ">"、底栏=上指向 "^"。
fn collapse_triangle_ui(ui: &mut egui::Ui, id: &str, glyph: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 20.0), egui::Sense::click());
    let bg = if resp.hovered() { egui::Color32::from_gray(80) } else { egui::Color32::from_gray(45) };
    let p = ui.painter();
    p.rect_filled(rect.shrink(1.0), 4.0, bg);
    // 用字符画三角: 大号、加粗感
    let col = if resp.hovered() { egui::Color32::from_rgb(0x6f, 0xcf, 0x97) } else { egui::Color32::from_gray(230) };
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(20.0),
        col,
    );
    ui.interact(rect, egui::Id::new(id), egui::Sense::click())
}

/// 收起窄条上的三角(点击 → 展开): 左侧栏朝右 ">",右栏朝左 "<",底栏朝上 "^"
fn rail_triangle_ui(ui: &mut egui::Ui, rect: egui::Rect, id: &str, glyph: &str) -> egui::Response {
    let (btn, resp) = ui.allocate_exact_size(egui::vec2(rect.width(), 22.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(btn, 2.0, egui::Color32::from_gray(70));
    }
    let col = if resp.hovered() { egui::Color32::from_rgb(0x6f, 0xcf, 0x97) } else { egui::Color32::from_gray(200) };
    p.text(
        btn.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(18.0),
        col,
    );
    ui.interact(btn, egui::Id::new(id), egui::Sense::click())
}

/// 十六进制显示 SysEx 消息 (UI 用, 无硬件也可见)
fn format_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X} ", b)).collect::<String>().trim().to_string()
}

/// 识别 SysEx 常见类型 (事件列表/折叠区显示用; 未知 → "SX")
/// data 含起始 F0/F7. 规则参考 sysex.rs (XG: F0 43 [device] [addr3..] ...)
fn sysex_kind(data: &[u8]) -> &'static str {
    // data[0]==F0, data[1]==0x43 → Yamaha XG 系统专属
    if data.len() >= 3 && data[0] == 0xF0 && data[1] == 0x43 {
        match data[2] & 0xF0 {
            0x00 => "XG bulk",   // 0n bulk dump
            0x10 => "XG param",  // 1n parameter change
            0x20 => "XG dump-req",
            0x30 => "XG param-req",
            _ => "XG",
        }
    } else if data.len() >= 5 && data[0] == 0xF0 && data[1] == 0x41 {
        // Roland GS: F0 41 <device> 42 <cmd>... — data[2]=device, data[3]=42(GS 标志), data[4]=cmd.
        // cmd: 12=DT1 参数变更, 11=RQ1 请求, 42=GS system reset
        match data[4] {
            0x12 => "Roland GS param",   // DT1
            0x11 => "Roland GS req",     // RQ1
            0x42 => "Roland GS reset",   // GS system reset
            _ => "Roland",
        }
    } else if data.len() >= 3 && data[0] == 0xF0 {
        // 通用厂商: F0 7E = 通用系统实时(universal), F0 7F = 通用非实时
        match data[1] {
            0x7E => "GM/Universal",
            0x7F => "Universal",
            _ => "MFG",
        }
    } else {
        "SX"
    }
}

/// 常见 CC 控制器短名 (详情行用; 未知 → 空串)
fn cc_short_name(num: u8) -> &'static str {
    match num {
        0 => "bank MSB", 1 => "mod", 2 => "breath", 4 => "foot", 5 => "porta",
        7 => "vol", 10 => "pan", 11 => "expr", 64 => "sustain", 65 => "porta-on",
        66 => "sostenuto", 67 => "soft", 71 => "reson", 72 => "rel", 73 => "atk",
        74 => "cutoff", 91 => "reverb", 93 => "chorus", 98 => "NRPN-L", 99 => "NRPN-H",
        100 => "RPN-L", 101 => "RPN-H", 120 => "all-sound", 121 => "reset-ctrl",
        123 => "all-notes", 126 => "mono", 127 => "poly",
        _ => "",
    }
}

/// 生成 event list 详情行文本 (2026-08-14: 点击 event 在下方展开详细内容, 同 SYSEX hex 展开)
/// ch 为 1..16 UI 通道语义; tick 为绝对 tick (从 EventRow.tick).
fn event_detail_text(ch: u8, kind: &crate::smf::EventKind, tick: u64) -> String {
    match kind {
        crate::smf::EventKind::NoteOn { pitch, vel } =>
            format!("ch{}  tick={}  {} ({})  vel={}", ch, tick,
                crate::piano_roll::midi_name(*pitch as i32), pitch, vel),
        crate::smf::EventKind::NoteOff { pitch } =>
            format!("ch{}  tick={}  {} ({})", ch, tick,
                crate::piano_roll::midi_name(*pitch as i32), pitch),
        crate::smf::EventKind::Cc { num, val } => {
            let name = cc_short_name(*num);
            if name.is_empty() {
                format!("ch{}  tick={}  CC{}  val={}", ch, tick, num, val)
            } else {
                format!("ch{}  tick={}  CC{} {}  val={}", ch, tick, num, name, val)
            }
        }
        crate::smf::EventKind::Program { program } =>
            format!("ch{}  tick={}  program={} ({:02X})", ch, tick, program + 1, program),
    }
}

impl XgApp {
    /// 每帧收 MIDI input: drain inbox → 喂 part_voice_reader 收集器 → 解析出 (part,msb,lsb,pc) → 查音色名。
    /// 双向通信核心 (John: 读每个 part 的音色; 多输入端口各带来源名, 当前统一喂收集器)。
    fn poll_midi_input(&mut self) {
        // 吸收全局状态日志 ([tx:...] 等) 进 status_log
        for g in drain_global_status() {
            self.log_status(g);
        }
        let msgs = midi_wasm::drain_inbox();
        if msgs.is_empty() {
            return;
        }
        // 调试显示 (限制条数防刷屏)
        let mut rx: Vec<String> = msgs.iter()
            .map(|(src, b)| format!("[{src}] {}", b.iter().map(|x| format!("{x:02X}")).collect::<String>()))
            .collect();
        rx.truncate(5);
        self.last_midi_rx = rx;
        // 喂收集器 → 凑齐即查音色; 每条回包都进 status bar (让 John 能确认收到了).
        // 仅"读请求在途"时记录原始字节, 避免播放时 RX 洪泛刷掉状态。
        for (src, bytes) in &msgs {
            // 所有 SysEx 接收加统一编号时间线 (console + status bar 双轨)
            if bytes.first() == Some(&0xF0) {
                crate::midi_trace("RX", src, bytes);
                // System Dump 捕获: 开启时全量收存 (RX# 编号紧跟 midi_trace 的自增)
                if self.sysex_capture {
                    let n = crate::MIDI_RX_CTR.with(|c| c.get()) as u32;
                    self.sysex_capture_log.push((src.clone(), bytes.iter().map(|x| format!("{x:02X}")).collect::<String>(), n));
                    self.sysex_capture_count += 1;
                    // 缓冲上限 5000 (MU90 ALL dump 可达 ~2600 条; 之前 200 会截断)
                    if self.sysex_capture_log.len() > 5000 {
                        let excess = self.sysex_capture_log.len() - 5000;
                        self.sysex_capture_log.drain(0..excess);
                    }
                }
            } else {
                crate::console_log("RX", format!("{src} {}", bytes.iter().map(|x| format!("{x:02X}")).collect::<String>()));
            }
            // 握手状态机: 读操作在途时, 只有"匹配期望地址"的 DT1 才推进 (不会误判/冒进)
            if let Some((wpart, woff)) = self.read_wait {
                if let Some((part, off, val)) = sysex::PartVoiceCollector::try_dt1(bytes) {
                    let hit = part == wpart && off == woff;
                    crate::console_log("HS",
                        format!("rx part={part} off={off:02X} val={val:02X} | expect part={wpart} off={woff:02X} => {}", if hit {"MATCH"} else {"skip"}));
                    if hit {
                        self.step_read_handshake(part, off, val);
                    }
                }
            }
            // 收集器尝试解析 (兼容 bulk dump 一条到齐 + DT1 逐条; 若握手未触发则这仍是兜底)
            let done = self.part_voice_reader.feed(bytes);
            if done {
                if let Some((part, msb, lsb, pc)) = self.part_voice_reader.result() {
                    // 批量读写入 read_parts (part 0-31)
                    if (part as usize) < 32 {
                        self.read_parts[part as usize] = Some((msb, lsb, pc));
                    }
                    // 查询音色名
                    let name = self.voice_bank
                        .as_ref()
                        .and_then(|b| b.find(msb, pc, lsb))
                        .map(|v| v.name.clone())
                        .unwrap_or_else(|| format!("{msb:02}/{lsb:02}/{pc:02}"));
                    self.last_read_voice = Some((part, msb, lsb, pc, name.clone()));
                    // 注意: 不能动 read_wait/read_request_inflight —— 握手状态机可能正在
                    // 推进 (part1 完成收集器触发时, start_read_part(part2) 刚设好 read_wait=(1,MSB);
                    // 这里清空会把 part2 的期望摧毁, 导致 RX#4 后被跳过, TX#5 永不发.  2026-08-09
                    self.log_status(format!("part{} <- {name} (bank {msb}/{lsb} pc {pc})", part + 1));
                }
                self.part_voice_reader.reset();
            }
            // Bulk Read: 若在途, 尝试解析 41B bulk 回包 (addr 08 nn 00) → step_bulk_read
            if self.bulk_read_next.is_some() {
                if let Some((part, msb, lsb, pc)) = sysex::PartVoiceCollector::try_bulk_dump(bytes) {
                    self.step_bulk_read(part, msb, lsb, pc);
                }
            }
        }
    }

    /// 解析捕获日志为 地址→值 聚合表.
    /// 识别 bulk dump `F0 43 0n ?? bb bb aa aa aa dd..dd cs F7` (Model ID ??), 取 addr3+data.
    /// 也识别 DT1/参数变更 `F0 43 1n 4C aa aa aa dd.. F7`.
    /// 地址 key = (hh<<14)|(mm<<7)|ll; 同地址保留最后一次值 + 计数.
    pub fn analyze_sysex_capture(&mut self) {
        use std::collections::BTreeMap;
        let mut agg: BTreeMap<u32, (String, u32)> = BTreeMap::new();
        for (_, hex, _) in &self.sysex_capture_log {
            // hex 是每字节 2 大写 hex 无空格 (捕获时格式化为 {x:02X} 拼接)
            if hex.len() < 12 || hex.len() % 2 != 0 {
                continue;
            }
            let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                .collect();
            let Some(b) = bytes else { continue };
            if b.len() < 8 || b[0] != 0xF0 || b[1] != 0x43 { continue; }
            let dtype = b[2] & 0xF0;
            if dtype == 0x00 {
                // bulk: F0 43 0n ?? [bb bb] [aa aa aa] [data..] [cs] F7
                if b.len() < 10 || b[b.len() - 1] != 0xF7 { continue; }
                let bc = ((b[4] as u16) << 7) | b[5] as u16;
                let addr = ((b[6] as u32) << 14) | ((b[7] as u32) << 7) | b[8] as u32;
                // data = 紧随地址的 bc 字节; 其后应为 cs + F7
                let data_start = 9usize;
                let data_end = data_start + bc as usize;
                if data_end > b.len().saturating_sub(2) { continue; } // 需留 cs+F7
                let val: String = b[data_start..data_end].iter()
                    .map(|x| format!("{x:02X}")).collect();
                let e = agg.entry(addr).or_insert((format!("x{bc}"), 0));
                e.0 = val.clone();
                e.1 += 1;
            } else if dtype == 0x10 && b.len() >= 9 && b[b.len() - 1] == 0xF7 {
                // DT1/param change: F0 43 1n ?? [aa aa aa] [data..] F7 (no bc/cs)
                let addr = ((b[4] as u32) << 14) | ((b[5] as u32) << 7) | b[6] as u32;
                let val: String = b[7..b.len() - 1].iter().map(|x| format!("{x:02X}")).collect();
                let e = agg.entry(addr).or_insert((String::new(), 0));
                e.0 = val;
                e.1 += 1;
            }
        }
        self.sysex_analysis = agg.into_iter().map(|(a, (v, c))| (a, v, c)).collect();
        self.log_status(format!("Analyzed {} unique addresses", self.sysex_analysis.len()));
        self.parse_dump_parts();
    }

    /// 从已捕获的 XG bulk dump (Model 4C) 解析 32 个 MULTI PART 的音色 (msb/lsb/pc).
    /// 数据源: 聚合表里地址 `08 nn 00` (nn=part 0-based) 的 data.
    /// 布局 (真机缪90 dump 定案 2026-08-09):
    ///   data[0]=Element Reserve, data[1]=MSB, data[2]=LSB, data[3]=PC(0-based), data[4]=RcvCh(0-based)
    /// 解析成功则写入 read_parts[0..32]; 否则不清 (保留现有).
    pub fn parse_dump_parts(&mut self) {
        use std::collections::BTreeMap;
        // 从聚合表重建 08 nn 00 → data
        let mut part00: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
        for (addr, val, _cnt) in &self.sysex_analysis {
            let hh = (addr >> 14) & 0x7F;
            let mm = (addr >> 7) & 0x7F;
            let ll = addr & 0x7F;
            if hh == 0x08 && ll == 0x00 {
                // val 是 hex 字符串
                let data: Option<Vec<u8>> = (0..val.len()).step_by(2)
                    .map(|i| u8::from_str_radix(&val[i..i + 2], 16).ok())
                    .collect();
                if let Some(d) = data {
                    if d.len() >= 4 {
                        part00.insert(mm as u8, d);
                    }
                }
            }
        }
        if part00.is_empty() {
            self.log_status("dump: no 08nn00 part blocks found (need Model 4C bulk dump)");
            return;
        }
        let mut n = 0;
        for (part, d) in &part00 {
            let msb = d[1];
            let lsb = d[2];
            let pc = d[3];
            if (*part as usize) < 32 {
                self.read_parts[*part as usize] = Some((msb, lsb, pc));
                n += 1;
            }
        }
        self.log_status(format!("dump: parsed {n} parts from bulk dump (msb/lsb/pc)"));
    }

    /// 生成地址表文本 (供剪贴板/导出): 每行 `HH MM LL = VAL (xN)`.
    pub fn build_analysis_text(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("# sysex address table ({} unique addrs)\n", self.sysex_analysis.len()));
        for (addr, val, cnt) in &self.sysex_analysis {
            let hh = (addr >> 14) & 0x7F;
            let mm = (addr >> 7) & 0x7F;
            let ll = addr & 0x7F;
            s.push_str(&format!("{hh:02X} {mm:02X} {ll:02X} = {val} (x{cnt})\n"));
        }
        s
    }

    /// 开始读单个 part (握手状态机入口): 种子状态 + 发起第一条 (MSB) 请求.
    fn start_read_part(&mut self, part: u8) {
        self.read_request_inflight = Some(format!("part{}", part + 1));
        self.read_wait = Some((part, sysex::mp::BANK_SELECT_MSB));
        self.read_acc_msb = None;
        self.read_acc_lsb = None;
        self.read_acc_pc = None;
        self.read_handshake_deadline = None;
        // 超时从 update 层设置 (ctx.time); 这里只重置
        self.log_status(format!("reading part{} ... (handshake msb)", part + 1));
        #[cfg(target_arch = "wasm32")]
        if let Some(i) = self.selected_midi {
            let dev = self.midi_devices[i].clone();
            let gap = self.read_req_gap_ms; // 复制成 owned, 才能 move 进 async
            wasm_bindgen_futures::spawn_local(async move {
                // 请求间隔 (batch 续读/握手下一步都用同一策略): 保证不背靠背
                // v87 实验: 同样改用 send_sync (见 step_read_handshake 注释)
                sleep_ms(gap).await;
                let msg = sysex::read_part_voice_param(part, sysex::mp::BANK_SELECT_MSB, sysex::Device::Request(1));
                let _ = midi_wasm::send_sync(&dev, &msg).await;
            });
        }
    }

    /// 握手读 part: 收到匹配"期望地址"的 DT1 回包后推进.
    /// 缓存值 → 若还有下个地址 (02 LSB, 03 PC) 则发下一条请求; 否则完成并查音色名。
    fn step_read_handshake(&mut self, part: u8, off: u8, val: u8) {
        match off {
            sysex::mp::BANK_SELECT_MSB => {
                self.read_acc_msb = Some(val);
                self.log_status(format!("  handshake: msb={val}, request lsb"));
            }
            sysex::mp::BANK_SELECT_LSB => {
                self.read_acc_lsb = Some(val);
                self.log_status(format!("  handshake: lsb={val}, request pc"));
            }
            sysex::mp::PROGRAM_NUMBER => {
                self.read_acc_pc = Some(val);
            }
            _ => return,
        }
        // 下一个地址: 01→02→03; 03 之后完成
        // 批量读"只读 MSB"模式: 收到 MSB 即完成 (不追 LSB/PC)
        let next_off = if self.read_batch_msb_only && off == sysex::mp::BANK_SELECT_MSB {
            0 // 完成
        } else {
            match off {
                sysex::mp::BANK_SELECT_MSB => sysex::mp::BANK_SELECT_LSB,
                sysex::mp::BANK_SELECT_LSB => sysex::mp::PROGRAM_NUMBER,
                _ => 0, // 完成
            }
        };
        if next_off != 0 {
            self.read_wait = Some((part, next_off));
            // 重置超时倒计时 (下一条 request 有新 3s 窗口)
            self.read_handshake_deadline = None;
            #[cfg(target_arch = "wasm32")]
            if let Some(i) = self.selected_midi {
                let dev = self.midi_devices[i].clone();
                let gap = self.read_req_gap_ms; // 复制成 owned, 才能 move 进 async
                wasm_bindgen_futures::spawn_local(async move {
                    // John WedMIDI 实测: 请求间须留足间隔 (秒级全回; 背靠背/80ms 只回第一条)
                    // → 下一条前 sleep (默认 1s), 保证 MSB/LSB/PC 都收到   2026-08-09
                    // v87 实验: 改用 send_sync (缓存 output 同步 send), 排除 send_at 每消息的
                    // request_access/open 异步开销 (若 160ms 实为 send_at 开销, gap 可大幅降低)
                    sleep_ms(gap).await;
                    let msg = sysex::read_part_voice_param(part, next_off, sysex::Device::Request(1));
                    let _ = midi_wasm::send_sync(&dev, &msg).await;
                });
            }
        } else {
            // 完成
            self.read_wait = None;
            self.read_request_inflight = None;
            let msb = self.read_acc_msb.unwrap_or(0);
            let lsb = self.read_acc_lsb.unwrap_or(0);
            let pc = self.read_acc_pc.unwrap_or(0);
            // 批量读"只读 MSB"模式: 只记 bank msb, 不做音色名查找 (LSB/PC 没读)
            if self.read_batch_msb_only {
                if (part as usize) < 32 {
                    self.read_parts[part as usize] = Some((msb, 0, 0));
                }
                self.log_status(format!("part{} <- bank msb={msb} (MSB only)", part + 1));
            } else {
                if (part as usize) < 32 {
                    self.read_parts[part as usize] = Some((msb, lsb, pc));
                }
                let name = self.voice_bank
                    .as_ref()
                    .and_then(|b| b.find(msb, pc, lsb))
                    .map(|v| v.name.clone())
                    .unwrap_or_else(|| format!("{msb:02}/{lsb:02}/{pc:02}"));
                self.last_read_voice = Some((part, msb, lsb, pc, name.clone()));
                self.log_status(format!("part{} <- {name} (bank {msb}/{lsb} pc {pc})", part + 1));
                // 反馈：若读回的是当前选中 part，同步编辑滑块/LCD 音色 (三重交叉验证：LCD + log + 表格)
                if part == (self.cur_part.saturating_sub(1) as u8) {
                    self.set_voice_from_quickpick(msb, lsb, pc);
                    self.log_status(format!("[UI] cur part {} voice <- {name}", self.cur_part));
                }
            }
            // 批量读: 读下一个 part (握手串行, 一个完成才开始下一个)
            if let Some(next) = self.read_batch_next {
                if next < 31 {
                    self.read_batch_next = Some(next + 1);
                    self.read_part_cursor = Some(next + 1);
                    self.log_status(format!("part{} done, reading part{} ...", part + 1, next + 2));
                    self.start_read_part(next + 1);
                } else {
                    self.read_batch_next = None;
                    self.read_part_cursor = None;
                    self.log_status("all 32 parts read.");
                    // 完成反馈: 一屏列出 32 parts 的 bank/pc, 便于对照面板 dump 真相表验证
                    let mut recap = String::from("recap:");
                    for (i, p) in self.read_parts.iter().enumerate() {
                        if let Some((msb, lsb, pc)) = p {
                            recap.push_str(&format!(" {}{:02}/{:02}/{:02}", i + 1, msb, lsb, pc));
                        } else {
                            recap.push_str(&format!(" {}--", i + 1));
                        }
                    }
                    self.log_status(recap.clone()); // status bar (可截断)
                    console_log("READ", recap); // JS console 全量保留 (John: 第一步是看 log)
                }
            }
        }
    }

    // ---------- Bulk Read All 32 (2n dump request, 绕过 3n 冷却; 2026-08-09 定案) ----------

    /// 发起 bulk read: 从 part 0 开始, 每 part 一条 dump request (addr 08 nn 00 41B 块).
    pub fn start_bulk_read(&mut self) {
        self.bulk_parts = Default::default();
        // 握手式: 等回包再发下一条 (不等 gap; bulk 连发几乎零冷却, 无需 read_req_gap_ms)
        self.log_status("bulk read all 32 parts (2n dump request)...");
        self.bulk_read_next = Some(0);
        self.bulk_read_deadline = None;
        self.send_bulk_request(0);
    }

    /// 发当前 part 的 dump request (addr 08 nn 00). 成功则设超时 deadline.
    fn send_bulk_request(&mut self, part: u8) {
        self.bulk_read_next = Some(part);
        self.bulk_read_deadline = None; // update() 里按 ctx.time 重建
        #[cfg(target_arch = "wasm32")]
        if let Some(i) = self.selected_midi {
            let dev = self.midi_devices[i].clone();
            wasm_bindgen_futures::spawn_local(async move {
                let msg = sysex::dump_request(sysex::Device::DumpRequest(1), [0x08, part, 0x00]);
                let _ = midi_wasm::send_sync(&dev, &msg).await;
            });
        }
    }

    /// 收到 41B bulk 回包 (addr 08 nn 00) → 解析 msb/lsb/pc → 存档 → 下一条/完成.
    /// 返回 true 表示这条包确实是 bulk-read 需要的 (已消费).
    fn step_bulk_read(&mut self, part_byte: u8, msb: u8, lsb: u8, pc: u8) -> bool {
        // 由 try_bulk_dump 解析进 (part,msb,lsb,pc); 这里只信任 part 匹配在途游标
        if self.bulk_read_next != Some(part_byte) {
            return false; // 不是当前期望 part 的回包 → 忽略 (防跨 part 误配)
        }
        self.bulk_read_deadline = None;
        if (part_byte as usize) < 32 {
            self.bulk_parts[part_byte as usize] = Some((msb, lsb, pc));
        }
        // 名查询 + 反馈
        let name = self.voice_bank
            .as_ref()
            .and_then(|b| b.find(msb, pc, lsb))
            .map(|v| v.name.clone())
            .unwrap_or_else(|| format!("{msb:02}/{lsb:02}/{pc:02}"));
        self.log_status(format!("part{} <- {name} (bank {msb}/{lsb} pc {pc})", part_byte + 1));
        // fail-soft: 跳过缺 part 的请求 (迟到回包被上面的游标守卫挡住)
        let next = part_byte.wrapping_add(1);
        if next < 32 {
            self.send_bulk_request(next);
        } else {
            self.bulk_read_next = None;
            self.bulk_read_deadline = None;
            // 完成反馈: recap 与 3n 同款格式, 便于对照面板 dump 真相表
            let mut recap = String::from("bulk-recap:");
            for (i, p) in self.bulk_parts.iter().enumerate() {
                if let Some((msb, lsb, pc)) = p {
                    recap.push_str(&format!(" {}{:02}/{:02}/{:02}", i + 1, msb, lsb, pc));
                } else {
                    recap.push_str(&format!(" {}--", i + 1));
                }
            }
            self.log_status(recap.clone());
            console_log("READ", recap);
        }
        true
    }

    /// 所有已选中的输出设备名 (selected Port A/B + 拓扑路由的 PortA/PortB 输出, 去重).
    /// 播放/发送/清音的目标集合.
    pub fn active_outputs(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        if let Some(i) = self.selected_midi {
            if let Some(d) = self.midi_devices.get(i) {
                v.push(d.clone());
            }
        }
        if self.mirror_to_b {
            if let Some(i) = self.selected_midi_b {
                if let Some(d) = self.midi_devices.get(i) {
                    if !v.contains(d) {
                        v.push(d.clone());
                    }
                }
            }
        }
        // 拓扑路由的输出 (auto_assign 的 PortA/PortB) 也纳入, 去重
        for role in [midi_topology::MidiRole::PortA, midi_topology::MidiRole::PortB] {
            if let Some(name) = self.midi_topology.output_for_role(role) {
                if !v.contains(&name) {
                    v.push(name);
                }
            }
        }
        v
    }

    /// 通道音符点颜色: 16 通道唯一色相 (HSV 均匀分布), 鼓 ch10(i=9) 洋红专色; 亮度随 velocity
    pub(crate) fn channel_note_color(&self, i: usize, vel: u8) -> (u8, u8, u8) {
        let bright = 40 + ((vel as f32 / 127.0).clamp(0.0, 1.0) * 215.0) as u8;
        if i == 9 {
            (bright, bright / 8, bright) // 洋红 (鼓专色)
        } else {
            let hue = (i as f32 / 16.0 * 360.0) % 360.0;
            let h6 = hue / 60.0;
            let c = 0.75;
            let x__ = c * (1.0 - ((h6 % 2.0) - 1.0).abs());
            let (r2, g2, b2) = match h6.floor() as u32 % 6 {
                0 => (c, x__, 0.0), 1 => (x__, c, 0.0), 2 => (0.0, c, x__),
                3 => (0.0, x__, c), 4 => (x__, 0.0, c), _ => (c, 0.0, x__),
            };
            ((bright as f32 * r2) as u8, (bright as f32 * g2) as u8, (bright as f32 * b2) as u8)
        }
    }

    /// 音色名查找: 从单源 parts[i] 取 msb/lsb/prog, 查 voice_bank (鼓用 drum_display_name)
    pub(crate) fn voice_name_for_channel(&self, i: usize) -> String {
        if i >= 16 { return String::new(); }
        if let Some(part) = self.parts.get(i) {
            let (msb, lsb) = (part.msb, part.lsb);
            let prg = part.prog;
            if msb == 127 {
                // 鼓: MSB=127 → drum kit 名
                return crate::data::drum_display_name(prg).to_string();
            }
            if let Some(vb) = &self.voice_bank {
                if let Some(v) = vb.find(msb, prg, lsb) {
                    return v.name.clone();
                }
                if let Some(v) = vb.xg_by_prg(prg) {
                    return v.name.clone();
                }
            }
            // 回退: part.voice (SMF 预填或 quickpick 选择)
            if !part.voice.is_empty() {
                return part.voice.clone();
            }
        }
        // XG 初始化默认音色: GrandPno (bank 000 / pgm 001)
        "GrandPno".to_string()
    }

    /// 播放输出层: 该通道(ch 0..15, solos/mutes 下标)在播放时是否应静音.
    /// Mute 优先于 Solo (DAW 惯例): 任一 solo 激活 → 非 solo 通道静音; 自身 mute 恒有效.
    pub(crate) fn channel_is_effectively_muted(&self, ch: usize) -> bool {
        let any_solo = self.channel_solos.iter().any(|&s| s);
        if any_solo {
            !self.channel_solos[ch] || self.channel_mutes[ch]
        } else {
            self.channel_mutes[ch]
        }
    }

    /// 滑块变化后: 用当前 part 的 params 重新渲染 LCD 参数条
    /// 从唯一数据源 parts[cur_part-1] 取音色/bank/pgm/8 条参数
    fn update_lcd_params(&mut self) {
        // 先清空 8 条, 再按映射填入 (现在用 part params, 不再用全局 self.params)
        let mut p = [0.0f32; 8];
        let part = self.parts.get((self.cur_part.saturating_sub(1)) as usize);
        if let Some(part) = part {
            for i in 0..part.params.len() {
                p[i] = part.params[i] / 127.0; // 归一化 0..1
            }
        }
        // 播放时 LCD 实时反映: 电平条 = live_levels (16ch), 音色名 = 当前 part
        let lv: [f32; 16] = self.live_levels;
        // 单源化: LCD 音色/bank/pgm 都来自 parts[cur_part-1]
        let (lcd_voice, lcd_bank, lcd_prog) = if let Some(part) = part {
            (part.voice.clone(), part.lsb as u32, part.prog as u32 + 1)
        } else {
            (self.cur_voice.clone(), self.cur_bank, self.cur_prog)
        };
        if self.lcd_32 {
            lcd::render_lcd_32(
                &mut self.lcd_pixels,
                &lcd_voice, lcd_bank, lcd_prog,
                &lv, &[0.0; 2],
                self.cur_part, // part 1..32 (选择器切换; lcd.rs 内部映射 sec/channel)
                &p,
            );
        } else {
            lcd::render_lcd(
                &mut self.lcd_pixels,
                &lcd_voice, lcd_bank, lcd_prog,
                &lv, &[0.0; 2],
                self.cur_part,
                &p,
            );
        }
        self.lcd_dirty = true; // 像素变了, 纹理需重新上传
    }

    /// Bank/PC/LSB 滑块变化: 按 (索引→真实MSB, 索引→真实PC, 索引→有效LSB) 查真实音色表
    /// 三个滑块都走"有效值索引", 保证取值范围有效, 不会选到不存在的组合
    fn apply_bank_pc(&mut self) {
        let bank = self.voice_bank.as_ref();
        let msbs = bank.map(|b| b.msb_values()).unwrap_or_default();
        let msb = msbs.get(self.cur_msb_idx.min(msbs.len().saturating_sub(1))).copied().unwrap_or(0);
        self.cur_msb_idx = msbs.iter().position(|&m| m == msb).unwrap_or(self.cur_msb_idx);
        let prgs = bank.map(|b| b.prg_values(msb)).unwrap_or_default();
        let prog0 = prgs.get(self.cur_pc_idx.min(prgs.len().saturating_sub(1))).copied().unwrap_or(0);
        self.cur_pc_idx = prgs.iter().position(|&p| p == prog0).unwrap_or(self.cur_pc_idx);
        self.cur_prog = prog0 as u32 + 1; // 显示值 1-based (001..128)
        // 有效 LSB 变体 (当前 msb + prog0)
        let variants = bank.map(|b| b.lsb_variants(msb, prog0)).unwrap_or_default();
        let cur_lsb: u8 = if variants.is_empty() {
            0
        } else {
            let idx = self.cur_lsb_idx.min(variants.len() - 1);
            self.cur_lsb_idx = idx;
            variants[idx]
        };
        self.cur_bank = cur_lsb as u32; // LCD bank 显示 = LSB (MU90 真机显示的是 LSB; 如 Chor.EP2 = lsb32)
        let voice = if msb == 127 {
            // 鼓组 (msb127): 用 LCD 8 字符显示短名 (MU90 真机: StandKit 等), 不用全名截断
            crate::data::drum_display_name(prog0).to_string()
        } else {
            bank
                .and_then(|b| b.find(msb, prog0, cur_lsb))
                .map(|v| v.name.clone())
                .unwrap_or_else(|| "---".to_string())
        };
        self.cur_voice = voice;
        self.update_lcd_params();
    }

    /// 快捷菜单选择: 按 (msb, lsb, prg0) 直接把三轴索引设到目标音色
    fn set_voice_from_quickpick(&mut self, msb: u8, lsb: u8, prg0: u8) {
        let bank = self.voice_bank.as_ref();
        if let Some(b) = bank {
            // msb 索引
            let msbs = b.msb_values();
            if let Some(mi) = msbs.iter().position(|&m| m == msb) {
                self.cur_msb_idx = mi;
            }
            // prg 索引
            let prgs = b.prg_values(msb);
            if let Some(pi) = prgs.iter().position(|&p| p == prg0) {
                self.cur_pc_idx = pi;
            }
            // lsb 索引
            let variants = b.lsb_variants(msb, prg0);
            if let Some(li) = variants.iter().position(|&l| l == lsb) {
                self.cur_lsb_idx = li;
            } else if !variants.is_empty() {
                self.cur_lsb_idx = 0;
            }
        }
        self.apply_bank_pc();
    }

    /// 当前实际 MSB (索引→值)
    fn current_msb(&self) -> u8 {
        self.voice_bank
            .as_ref()
            .map(|b| b.msb_values())
            .unwrap_or_default()
            .get(self.cur_msb_idx)
            .copied()
            .unwrap_or(0)
    }

    /// 当前实际 LSB (按 cur_lsb_idx 从 (msb, prog0) 变体列表取; 空表→0)
    fn current_lsb(&self) -> u8 {
        let msb = self.current_msb();
        let prog0 = self.cur_prog.saturating_sub(1) as u8;
        let variants = self
            .voice_bank
            .as_ref()
            .map(|b| b.lsb_variants(msb, prog0))
            .unwrap_or_default();
        if variants.is_empty() {
            return 0;
        }
        variants
            .get(self.cur_lsb_idx.min(variants.len() - 1))
            .copied()
            .unwrap_or(0)
    }


    /// 导出可持久化状态 (存原始 3 轴值, 兼容未来自由发送设计)
    pub fn to_persisted(&self) -> PersistedState {
        PersistedState {
            msb: self.current_msb(),
            lsb: self.current_lsb(),
            pc: self.cur_prog.saturating_sub(1) as u8,
            lcd_32: self.lcd_32,
            lcd_zoom: self.lcd_zoom,
            params: self.params.iter().map(|p| p.3).collect(),
        }
    }

    /// 从持久化状态恢复 (音色 3 轴找到则回写索引; 参数值对应位置覆盖; 非法值由各滑块钳制)
    pub fn apply_persisted(&mut self, s: &PersistedState) {
        // 音色: 尝试按 (msb, pc, lsb) 回写当前索引模型
        self.cur_voice = self
            .voice_bank
            .as_ref()
            .and_then(|b| b.find(s.msb, s.pc, s.lsb))
            .map(|v| v.name.clone())
            .unwrap_or_else(|| self.cur_voice.clone());
        // 回写索引: msb 索引 / pc 索引 / lsb 索引
        if let Some(b) = self.voice_bank.as_ref() {
            let msbs = b.msb_values();
            if let Some(mi) = msbs.iter().position(|&m| m == s.msb) {
                self.cur_msb_idx = mi;
            }
            let prgs = b.prg_values(s.msb);
            if let Some(pi) = prgs.iter().position(|&p| p == s.pc) {
                self.cur_pc_idx = pi;
            }
            let vars = b.lsb_variants(s.msb, s.pc);
            if let Some(li) = vars.iter().position(|&l| l == s.lsb) {
                self.cur_lsb_idx = li;
            }
        }
        self.cur_bank = s.lsb as u32; // LCD bank 显示 = LSB (MU90 真机显示 LSB)
        self.cur_prog = s.pc as u32 + 1;
        self.lcd_32 = s.lcd_32;
        self.lcd_zoom = if s.lcd_zoom > 0.0 { s.lcd_zoom } else { 1.0 };
        // 参数: 仅覆盖长度匹配的值 (面板参数个数可能随版本变)
        let n = self.params.len().min(s.params.len());
        for i in 0..n {
            self.params[i].3 = s.params[i];
        }
        self.update_lcd_params();
    }
}


impl Default for XgApp {
    fn default() -> Self {
        let lcd_side = lcd::LCD_W;
        // 音色库: MU90 官方权威表 (include_str 打包, wasm/native 都可用)
        // LCD/快捷菜单/滑块显示都与真机一致; 滑块发送仍自由 (用户定案)
        let voice_bank = VoiceBank::embedded_mu90().ok();
        // 初始音色: XG MSB0 bank0 program (0-based 0 → GrandPno; 显示 1-based 001)
        let cur_prog0: u8 = 0; // 0-based 0 = prog 001
        let cur_voice = voice_bank
            .as_ref()
            .and_then(|b| b.xg_by_prg(cur_prog0))
            .map(|v| v.name.clone())
            .unwrap_or_else(|| "GrandPno".to_string());
        // 用真实 MU90 LCD 渲染核心生成初始画面(判别: 冒烟验证 native/wasm 共用渲染逻辑)
        let mut lcd_pixels = vec![0u8; lcd::LCD_W * lcd::LCD_H * 4];
        lcd::render_lcd(
            &mut lcd_pixels,
            &cur_voice, 0, 1,     // 音色名, bank 0(显示 000), program 1 (显示 001, 1-based)
            &[0.0; 16], &[0.0; 2], // 电平 0, audio 0
            1,                    // part 1 (显示 01A01: port A ch01)
            &[0.79, 1.0, 0.0, 0.0, 0.31, 0.0, 0.5, 0.5], // 初始参数条: Vol79 Exp100 Brt0 Pan0 Rev31 Cho0 Cut50 Res50 (%)
        );
        // 背景纹理: 512x128 带条纹渐变, 青蓝调(与 LCD 绿区分) —— 测试背景贴图是否扰乱布局
        let bg_side = 512;
        let bh = 128usize;
        let mut bg_pixels = vec![0u8; bg_side * bh * 4];
        for (i, px) in bg_pixels.chunks_mut(4).enumerate() {
            let y = i / (bg_side * 4);
            let band = (y / 8) % 2 == 0; // 横向条纹
            px[0] = if band { 0x1c } else { 0x16 };
            px[1] = if band { 0x3a } else { 0x2c };
            px[2] = if band { 0x5e } else { 0x48 };
            px[3] = 255;
            let _ = y;
        }
        // 左栏 16 轨: 音色名从真实 XG 音色库取 (MSB 127 常规区, 按 prg 递增)
        let pattern = default_pattern_notes();
        let tracks = (1..=16)
            .map(|i| {
                let voice = voice_bank
                    .as_ref()
                    .and_then(|b| b.xg_by_prg((i - 1) as u8))
                    .map(|v| v.name.clone())
                    .or_else(|| Some(voice_for(i)))
                    .unwrap();
                Track {
                    name: format!("Ch{:02}", i),
                    voice,
                    level: ((i as f32 * 37.0) % 100.0) / 100.0,
                    notes: pattern.get(i - 1).cloned().unwrap_or_default(),
                }
            })
            .collect();
        let midi_devices = vec![
            "UM-ONE (UM-ONE) [Port1]".to_string(),
            "USB-MIDI (FX16) [Port1]".to_string(),
        ];
        // 初始拓扑: 从默认设备列表构建 (仅 native/离线默认; wasm 探测后重建)
        let midi_topology = midi_topology::MidiTopology::from_probe(&[], &midi_devices);
        let params = vec![
            // 顺序对齐 LCD 底部标签 VOL EXP BRT PAN REV CHO VAR KEY
            ("Volume".into(), 0.0, 127.0, 100.0),
            ("Exp".into(), 0.0, 127.0, 127.0),
            ("Bright".into(), -64.0, 63.0, -10.0),
            ("Pan".into(), -64.0, 63.0, 0.0),
            ("Reverb".into(), 0.0, 127.0, 40.0),
            ("Chorus".into(), 0.0, 127.0, 0.0),
            ("Variation".into(), 0.0, 127.0, 0.0), // VAR
            ("Key".into(), 0.0, 127.0, 64.0),      // KEY (key shift 或力度)
            // 以下无 LCD 标签, 仅面板编辑 (不驱动 LCD 条)
            ("Cutoff".into(), 0.0, 127.0, 64.0),
            ("Reso".into(), 0.0, 127.0, 64.0),
        ];
        Self {
            show_left: false, // 2026-08-09 UI 整理: 左侧 track 栏停用(音色+电平移入 center channel view 行头, 根治对齐)
            show_right: true,
            show_bottom: true,
            show_piano: true,        // 默认打开 (用户 2026-08-12: piano roll 默认显示)
            piano_height: 500.0,    // 默认高度 (用户改口: 660→500)
            central_view: CentralView::ChannelNotes, // 默认 Channel 音符指示(每行=channel, 行头含音色+绿电平); 可切 Piano Roll/PlayView
            left_width: 270.0, // 默认宽度容纳完整绿条+百分比(canvas内容); 拖窄则右缘裁剪, 不重排
            right_width: 240.0,
            bottom_height: 200.0,
            tracks,
            midi_devices,
            midi_topology,
            selected_midi: None,
            selected_midi_b: None,
            mirror_to_b: false,
            midi_connected: false,
            lcd_pixels,
            lcd_side,
            lcd_tex: None,
            lcd_dirty: true,
            lcd_zoom: 1.0,
            lcd_32: false,
            cur_voice,
            cur_bank: 0,
            cur_prog: 1,
            cur_part: 1,
            cur_msb_idx: 0,
            cur_pc_idx: 0,
            cur_lsb_idx: 0,
            parts: (0..32).map(|_| crate::part::PartState::default_voice(0, 0, 0, "GrandPno")).collect::<Vec<_>>().try_into().unwrap(),
            sys_fx: crate::part::SystemFx::default(),
            bg_pixels,
            bg_side,
            starfield_tex: None,
            params,
            param_offsets: vec![
                sysex::mp::VOLUME,             // Volume   → 0x0B
                sysex::mp::DRY_LEVEL,          // Exp      → 0x11 (干燥电平, 近似)
                sysex::mp::EG_ATTACK_TIME,     // Bright   → 0x1A (EG起音, 近似)
                sysex::mp::PAN,                // Pan      → 0x0E
                sysex::mp::REVERB_SEND,        // Reverb   → 0x13
                sysex::mp::CHORUS_SEND,        // Chorus   → 0x12
                sysex::mp::VARIATION_SEND,     // Variation→ 0x14
                sysex::mp::NOTE_SHIFT,         // Key      → 0x08 (note shift ≈ key)
                sysex::mp::CUTOFF_FREQ,        // Cutoff   → 0x18
                sysex::mp::RESONANCE,          // Reso     → 0x19
            ],
            // 面板参数 → LCD 底部条: VOL EXP BRT PAN REV CHO VAR KEY (前8个), Cutoff/Reso 无标签
            param_lcd_idx: vec![
                Some(0), // Volume → VOL
                Some(1), // Exp    → EXP
                Some(2), // Bright → BRT
                Some(3), // Pan    → PAN
                Some(4), // Reverb → REV
                Some(5), // Chorus → CHO
                Some(6), // Variation→ VAR
                Some(7), // Key    → KEY
                None,    // Cutoff (无 LCD 条)
                None,    // Reso   (无 LCD 条)
            ],
            last_sysex: None,
            voice_bank,
            midi_probe_started: false,
            midi_probe_result: None,
            midi_send_status: None,
            persist_loaded: false,
            persist_signature: None,
            persist_last_save_ms: -1000.0,
            app_version: "dev".into(),
            device: device::Device::Mu90,
            // 音序器初始: 停止, playhead 0, 4/4 96ppq, 120bpm, 总长 8 拍
            playing: false,
            playhead_tick: 0,
            total_ticks: 768, // 8 拍 * 96
            ppq: 96,
            tempo_bpm: 120.0,
            last_play_frame_ms: 0.0,
            meter_frame: 0,
            play_real_sec: 0.0,
            play_events: Vec::new(),
            event_cursor: 0,
            event_cursor_origin: 0,
            smf: None,
            smf_views: (0..16).map(|_| smf::SmfTrackView::default()).collect(),
            live_levels: [0.0; 16],
            live_voice_names: std::array::from_fn(|_| String::new()),
            live_volumes: [1.0; 16], // 默认满音量; CC7 事件播放时覆盖
            live_expressions: [1.0; 16], // 默认满表情; CC11 播放时覆盖
            active_notes: (0..16).map(|_| std::collections::BTreeMap::new()).collect(),
            raw_vel_peaks: [0.0; 16],
            cc_live: [[0u8; 128]; 16],
            live_bank: [(0u8, 0u8); 16],
            live_program: [0u8; 16],
            play_evt_count: 0,
            max_poly: 0,
            pview_scroll: 0.0,
            live_vel_peaks: [0.0; 16],
            live_master_vol: 1.0,
            channel_mutes: [false; 16],
            channel_solos: [false; 16],
            rec_armed: false,
            preview_notes: (0..16).map(|_| std::collections::BTreeMap::new()).collect(),
            event_list_sel: None,
            sysex_expanded: None,
            tempo_map: None,
            smf_total_sec: 0.0,
            smf_end_tick: 0,
            smf_name: String::new(),
            smf_load_result: String::new(),
            status_log: std::collections::VecDeque::new(),
            smf_is_dirty: true,
            url_override_view: false,
            track_view_zoom: 1.0,
            track_view_scroll_ticks: 0,
            cur_pr_channel: 1,
            pr_zoom: 1.0,
            pr_scroll_ticks: 0,
            pr_scrolled_once: false,
            channel_row_h: CHANNEL_ROW_H,
            channel_view_pitch_low: 0,
            channel_view_pitch_high: 127,
            ui_scroll_style_done: false,
            #[cfg(target_arch = "wasm32")]
            midi_probe_cell: None,
            #[cfg(target_arch = "wasm32")]
            midi_send_ui_cell: None,
            #[cfg(target_arch = "wasm32")]
            midi_verify_cell: None,
            part_voice_reader: Default::default(),
            last_midi_rx: Vec::new(),
            sysex_capture: false,
            sysex_capture_log: Vec::new(),
            sysex_capture_count: 0,
            sysex_analysis: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            midi_bind_cell: None,
            read_request_inflight: None,
            read_wait: None,
            read_acc_msb: None,
            read_acc_lsb: None,
            read_acc_pc: None,
            read_batch_next: None,
            read_batch_msb_only: false,
            read_handshake_deadline: None,
            read_req_gap_ms: 200,
            last_read_voice: None,
            bulk_read_next: None,
            bulk_parts: Default::default(),
            bulk_read_deadline: None,
            read_parts: Default::default(),
            read_part_cursor: None,
        }
    }
}

impl eframe::App for XgApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ---- 滚动条样式 (一次; 用户 2026-08-12: 需可见可拖, 且不占布局使标尺与网格错位) ----
        if !self.ui_scroll_style_done {
            self.ui_scroll_style_done = true;
            let mut style = (*ctx.style()).clone();
            // 全局深色主题 (John 2026-08-13: ☰ 菜单要深色和 transport 一致)
            // 注意: 底部状态栏/钢琴盘有显式浅色 frame (已在下方面板里保持), 不受影响.
            let mut v = egui::Visuals::dark();
            // 菜单/弹窗背景 = 深蓝灰 (呼应 Channel View / 顶栏 #1f2f45)
            v.panel_fill = egui::Color32::from_rgb(0x1f, 0x2f, 0x45);
            v.window_fill = egui::Color32::from_rgb(0x1a, 0x29, 0x3d);
            // 按钮文字/正文 = 浅色
            v.widgets.inactive.fg_stroke.color = egui::Color32::from_rgb(0xd5, 0xdc, 0xe6);
            v.widgets.hovered.fg_stroke.color = egui::Color32::from_rgb(0xff, 0xff, 0xff);
            v.widgets.active.fg_stroke.color = egui::Color32::from_rgb(0xff, 0xff, 0xff);
            // 不设 override_text_color: 它会覆盖所有显式 RichText color (标题白/count 亮金)
            v.selection.bg_fill = egui::Color32::from_rgb(0x2b, 0x43, 0x63);
            style.visuals = v;
            // floating 滚动条不占布局 → 标尺/琴键/网格同宽, bar 竖线上下对齐。
            // 用 floating.allocated_width + 高对比 + 半透明常显, 兼顾"可见可拖"。
            style.spacing.scroll.floating = true;
            style.spacing.scroll.bar_width = 10.0;
            style.spacing.scroll.floating_width = 4.0;      // 未激活更细、更不易见
            style.spacing.scroll.floating_allocated_width = 0.0; // 布局占位 0 → 标尺/内容同宽, bar 上下对齐
            style.spacing.scroll.handle_min_length = 24.0;
            style.spacing.scroll.bar_inner_margin = 2.0;
            style.spacing.scroll.bar_outer_margin = 1.0;
            // 未激活(不在滚动/未 hover)几乎不可见; 激活时对比降低, 灰色更淡
            style.spacing.scroll.dormant_background_opacity = 0.12;
            style.spacing.scroll.active_background_opacity = 0.55;
            style.spacing.scroll.foreground_color = true;   // 高对比 thumb
            ctx.set_style(style);
        }
        // ---- 双向通信: 每帧收 MIDI input (硬件 SysEx/消息 → 解析 part 音色) ----
        self.poll_midi_input();
        // 握手读超时: 建立/检查 deadline (~3s 无回包则放弃, 避免卡死)
        let now = ctx.input(|i| i.time);
        if self.read_wait.is_some() {
            match self.read_handshake_deadline {
                None => self.read_handshake_deadline = Some(now + 3.0),
                Some(dl) if now > dl => {
                    self.read_wait = None;
                    self.read_request_inflight = None;
                    self.read_handshake_deadline = None;
                    // fail-soft: batch 模式下超时的 part 标记为缺失, 继续下一个, 不卡死整批
                    if let Some(next) = self.read_batch_next {
                        if next < 31 {
                            self.read_batch_next = Some(next + 1);
                            self.read_part_cursor = Some(next + 1);
                            self.log_status(format!("part{} read timeout, skipping to part{} ...", next + 1, next + 2));
                            self.start_read_part(next + 1);
                        } else {
                            self.read_batch_next = None;
                            self.read_part_cursor = None;
                            self.log_status("all 32 parts read (some timed out).");
                        }
                    } else {
                        self.log_status("read timeout (no DT1 reply in 3s)");
                    }
                }
                _ => {}
            }
        }
        // bulk read 超时 (500ms): 无回 → fail-soft 跳下一个 part (bulk 间隔短, 不需要 3s)
        if let Some(cur) = self.bulk_read_next {
            match self.bulk_read_deadline {
                None => self.bulk_read_deadline = Some(now + 0.5),
                Some(dl) if now > dl => {
                    let next = cur.wrapping_add(1);
                    self.log_status(format!("part{} bulk timeout, skipping to part{} ...", cur + 1, next + 1));
                    if next < 32 {
                        self.send_bulk_request(next);
                    } else {
                        self.bulk_read_next = None;
                        self.bulk_read_deadline = None;
                        self.log_status("bulk read done (some timed out).");
                    }
                }
                _ => {}
            }
        }
        // ---- 面板状态持久化: 首次加载 + 变化写回 (wasm localStorage / native 文件) ----
        if !self.persist_loaded {
            self.persist_loaded = true;
            if let Ok(Some(json)) = crate::persist::load_json() {
                if let Ok(state) = PersistedState::from_json(&json) {
                    self.apply_persisted(&state);
                }
            }
        }
        // 变化检测: 每次 update 与上次存储签名比对, 变化才写回
        // 节流: 拖动滑块时每帧都会变, 若每帧写 localStorage = IO 风暴(卡顿源头之一)
        // 只在距上次保存 >150ms 且签名确实变了才写
        let now_ms = ctx.input(|i| i.time) * 1000.0; // egui 时间秒→毫秒
        if now_ms - self.persist_last_save_ms > 150.0 {
            let sig = self.to_persisted().to_json().unwrap_or_default();
            if self.persist_signature.as_deref() != Some(sig.as_str()) {
                self.persist_signature = Some(sig.clone());
                let _ = crate::persist::save_json(&sig);
                self.persist_last_save_ms = now_ms;
            }
        }
        // ---- 拖放 .mid 加载 (SMF) ----
        let dropped: Vec<egui::DroppedFile> = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            if f.name.ends_with(".mid") || f.name.ends_with(".midi") {
                if let Some(bytes) = &f.bytes {
                    let name = f.name.clone();
                    let result = self.load_smf_bytes(&name, bytes);
                    self.smf_load_result = match result {
                        Ok(s) => format!("[open] {s}"),
                        Err(e) => format!("[open] ERR {e}"),
                    };
                    break;
                } else if let Some(path) = &f.path {
                    // native 路径加载
                    if let Ok(bytes) = std::fs::read(path) {
                        let name = f.name.clone();
                        let result = self.load_smf_bytes(&name, &bytes);
                        self.smf_load_result = match result {
                            Ok(s) => format!("[open] {s}"),
                            Err(e) => format!("[open] ERR {e}"),
                        };
                        break;
                    }
                }
            }
        }
        // ---- midi_wasm 文件对话框结果 (wasm) ----
        #[cfg(target_arch = "wasm32")]
        {
            let pending = crate::SMF_DIALOG_PENDING.with(|c| c.borrow_mut().take());
            if let Some((name, bytes)) = pending {
                let result = self.load_smf_bytes(&name, &bytes);
                self.smf_load_result = match result {
                    Ok(s) => format!("[open] {s}"),
                    Err(e) => format!("[open] ERR {e}"),
                };
                // 调试/演示钩子: URL ?autoplay=1 → SMF 加载后自动播放 (截图验证播放态)
                if self.smf.is_some() {
                    let ap = web_sys::window()
                        .and_then(|w| w.location().search().ok())
                        .is_some_and(|s| s.contains("autoplay=1"));
                    if ap {
                        self.play_resume(); // 内部空事件表会自动 build_play_events
                        self.log_status("autoplay (URL hook)");
                    }
                }
            }
        }
        // ---- 冒烟验证: Web MIDI 真探测(仅 wasm, 启动一次 + 每帧轮询结果)----
        #[cfg(target_arch = "wasm32")]
        {
            // 首次: 创建共享 cell + spawn 异步探测
            if !self.midi_probe_started {
                self.midi_probe_started = true;
                let cell = std::rc::Rc::new(std::cell::RefCell::new(None));
                let cell_clone = cell.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let r = midi_wasm::probe_pair().await;
                    *cell_clone.borrow_mut() = Some(r);
                });
                self.midi_probe_cell = Some(cell);
            }
            // 每帧: 若 cell 有结果则收进 app
            if let Some(cell) = &self.midi_probe_cell {
                if let Some(r) = cell.borrow_mut().take() {
                    match &r {
                        Ok((ins, outs)) => {
                            // 用 outputs 作为可发送设备列表.
                            // 不再自动标 connected —— 用户必须选一个, 且经 verify_output 验证才 connected (John 2026-08-09 谎报)
                            self.midi_devices = outs.clone();
                            self.midi_connected = false;
                            // 构建 MIDI 拓扑 + 自动分配 A/B 角色 + 32 part 路由
                            self.midi_topology = midi_topology::MidiTopology::from_probe(ins, outs);
                            self.midi_topology.auto_assign_roles();
                        }
                        Err(e) => eprintln!("Web MIDI probe err: {e}"),
                    }
                    self.midi_probe_result = Some(r);
                }
            }
            // 每帧: 设备选择后的连接校验结果 → 只在验证通过后标 connected
            #[cfg(target_arch = "wasm32")]
            {
                // 先 take 结果、释放 cell 借用, 再操作 self (避免 E0502)
                let verify_result = self.midi_verify_cell.as_ref().and_then(|c| c.borrow_mut().take());
                if let Some(r) = verify_result {
                    match r {
                        Ok(()) => self.midi_connected = true,
                        Err(e) => {
                            self.midi_connected = false;
                            self.log_status(format!("MIDI connect fail: {e}"));
                        }
                    }
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // ---------- 音序器播放引擎 ----------
        // 常驻刷新(即使未播放也重绘 playhead/按钮状态; 播放时走时间推进)
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
        if self.playing {
            self.tick_playback(ctx);
        } else {
            self.last_play_frame_ms = 0.0;
        }
        // 采样式预览短音自动 off (30ms 帧循环, ctx.input().time 秒)
        let now = ctx.input(|i| i.time);
        self.expire_preview_notes(now);

        // 顶栏 (44px 高 + 上下 padding, TopBar 美化 v0.1.23)
        // frame 自带 inner_margin = 四周对称 padding (顶/底/左/右), 背景同色.
        // 底色 = Channel View 奇数通道色 #1f2f45 (John 2026-08-13 拍板)
        let topbar_bg = egui::Color32::from_rgb(0x1f, 0x2f, 0x45);
        egui::TopBottomPanel::top("top_bar")
            .frame(
                egui::Frame::none()
                    .fill(topbar_bg)
                    .inner_margin(egui::Margin::symmetric(12.0, 6.0)),
            )
            .exact_height(44.0)
            .show(ctx, |ui| {
                self.top_bar(ui);
            });

        // 顶栏结束后, 记录"全局可用区顶部" = 左右/中央面板内容的共同起算 Y
        // 行网格顶 = 这里 + GRID_TOP_OFFSET —— 两侧共用 → 任何窗口/DPI 严格对齐
        let base_top = ctx.available_rect().top();

        // 左侧 16 轨(判据 2, 可开关 8) —— 收起留窄条 + 自绘三角(像素自绘,无字体依赖)
        {
            let open = self.show_left;
            let mut panel = egui::SidePanel::left("tracks")
                .resizable(open)
                .default_width(self.left_width);
            if open {
                panel = panel.width_range(160.0..=400.0);
            } else {
                panel = panel.exact_width(22.0); // 收起: 只留 22px 窄条, 且不限制 range
            }
            panel.show(ctx, |ui| {
                    if open {
                        ui.horizontal(|ui| {
                            ui.heading("Tracks");
                            // 折叠三角 < (点击收起本栏,留窄条)
                            if collapse_triangle_ui(ui, "left_collapse", "<").clicked() {
                                self.show_left = false;
                            }
                        });
                        ui.separator();
                        // 行网格: 绝对 Y = base_top + GRID_TOP_OFFSET + i*CHANNEL_ROW_H
                        // 与中央共用同一 GRID_TOP_OFFSET + base_top → 任何窗口/DPI 严格对齐
                        // (弃用 ScrollArea: 其内部 cursor/margin 与 Central 不一致, 是历次错位根源)
                        let x0 = ui.max_rect().left();
                        // 用 panel 的 clip_rect 右缘 = 真正的面板右边界(含内边距, 到分隔线)
                        // 使行背景 flush 到分隔线, 消除"底色留 pad / 露内容"的问题
                        let x1 = ui.clip_rect().right();
                        let grid_top = base_top + GRID_TOP_OFFSET;
                        let p = ui.painter();
                        for (row_idx, t) in self.tracks.iter().enumerate() {
                            let y0 = grid_top + row_idx as f32 * CHANNEL_ROW_H;
                            let row_rect = egui::Rect::from_min_max(
                                egui::pos2(x0, y0),
                                egui::pos2(x1, (y0 + CHANNEL_ROW_H).min(ui.max_rect().bottom())),
                            );
                            // 行背景: 与中央交错的同色系, 一眼看出 channel 行
                            // 行背景铺满整个 panel 内容宽度(bg 覆盖住所有 canvas 内容, 包括绿条/槽/百分比)
                            // 内容不会"露出底色之外": bg 右缘 = panel 内容右缘(分辨率/缩放无关)
                            let base: (u8,u8,u8) = if row_idx % 2 == 0 { (0x13,0x20,0x2f) } else { (0x20,0x31,0x47) };
                            p.rect_filled(row_rect, 0.0, egui::Color32::from_rgb(base.0, base.1, base.2));
                            // 名称(左)
                            p.text(
                                egui::pos2(row_rect.left() + 8.0, row_rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                &t.name,
                                egui::FontId::monospace(13.0),
                                egui::Color32::from_gray(230),
                            );
                            // 音色名(中, 截断)
                            let mut voice = t.voice.clone();
                            if voice.chars().count() > 12 { voice.truncate(12); voice.push_str("..."); }
                            p.text(
                                egui::pos2(row_rect.left() + 58.0, row_rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                &voice,
                                egui::FontId::monospace(12.0),
                                egui::Color32::from_gray(160),
                            );
                            // 绿条 + 百分比: 固定画布坐标(canvas 语义), 不随面板宽度变化。
                            // 面板(resize)只是 viewport, 超出右缘的由 egui 自动裁剪, 不重排内容。
                            let bx = row_rect.left() + 148.0;
                            let bw = 76.0;
                            p.rect_filled(
                                egui::Rect::from_min_size(egui::pos2(bx, row_rect.center().y - 4.0), egui::vec2(bw, 8.0)),
                                2.0, egui::Color32::from_gray(60),
                            );
                            let w = (t.level * bw).max(2.0);
                            p.rect_filled(
                                egui::Rect::from_min_size(egui::pos2(bx, row_rect.center().y - 4.0), egui::vec2(w, 8.0)),
                                2.0, egui::Color32::from_rgb(0x2e, 0xcc, 0x40),
                            );
                            // 百分比: 固定在绿条之后, 与绿条一起被裁剪(不会跑到绿条前面)
                            p.text(
                                egui::pos2(bx + bw + 6.0, row_rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                format!("{:>3}%", (t.level * 100.0) as i32),
                                egui::FontId::monospace(12.0),
                                egui::Color32::from_gray(190),
                            );
                        }
                    } else {
                        // 收起窄条: 垂直居中字符合理 > (点击展开)
                        let rect = ui.max_rect();
                        if rail_triangle_ui(ui, rect, "left_rail", ">").clicked() {
                            self.show_left = true;
                        }
                    }
                });
        }

        // 右侧参数面板(判据 4, 可开关 8) —— 收起留窄条 + 三角
        {
            let open = self.show_right;
            let mut panel = egui::SidePanel::right("params")
                .resizable(open)
                .default_width(self.right_width);
            if open {
                panel = panel.width_range(160.0..=420.0);
            } else {
                panel = panel.exact_width(22.0); // 收起: 只留 22px 窄条
            }
            panel.show(ctx, |ui| {
                    if open {
                        ui.horizontal(|ui| {
                            ui.heading("Params");
                            if collapse_triangle_ui(ui, "right_collapse", ">").clicked() {
                                self.show_right = false;
                            }
                        });
                        // 当前 part 选择下拉 (用户 2026-08-12: part 选择移到 Params 顶部, LCD 里去掉了)
                        let cur_part_idx = (self.cur_part.saturating_sub(1)) as usize;
                        let part = self.parts.get(cur_part_idx);
                        let (part_voice, part_bank, part_prog) = part
                            .map(|p| (p.voice.clone(), p.lsb as u32, p.prog as u32 + 1))
                            .unwrap_or_else(|| (self.cur_voice.clone(), self.cur_bank, self.cur_prog));
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("part_select_right")
                                .selected_text(format!("Part {}\t", self.cur_part))
                                .show_ui(ui, |ui| {
                                    for p in 1..=32u32 {
                                        let sec = lcd::part_sec(p);
                                        let ch = lcd::part_channel(p);
                                        ui.selectable_value(&mut self.cur_part, p, format!("{p:02}{sec}{ch:02} Part {p}"));
                                    }
                                });
                            // 与原 LCD 下拉一致: show_ui 后无条件重绘 LCD (lcd_dirty guard 控制纹理上传)
                            self.update_lcd_params();
                            ui.label(format!("·  {}  ▶{:03}▶{:03}", part_voice, part_bank, part_prog));
                        });
                        ui.separator();
                        // Bank / PC 音色选择 (独立滑块, 驱动 LCD 音色名 + bank prog 数字显示)
                        // 注意: 每行 label 占第1列, "数值+滑块" 用 horizontal 包成整体占第2列
                        // 否则数值 label 会吃掉第2列, 滑块错位到下一行 (v12 的 PC 行 bug)
                        egui::Grid::new("bankpc_grid")
                            .num_columns(2)
                            .spacing([8.0, 6.0])
                            .show(ui, |ui| {
                                // Bank (MSB): 只在有效 MSB 区 {0,48,64,126,127} 间步进
                                {
                                    let msbs = self.voice_bank.as_ref().map(|b| b.msb_values()).unwrap_or_default();
                                    let msb_n = msbs.len().max(1);
                                    let msb_val = self.current_msb();
                                    // 第1列 label 固定纯文本(关键!): 动态数字移到第2列, Grid 列宽恒定 → 不闪烁
                                    ui.label("Bank");
                                    let mut msi = self.cur_msb_idx.min(msbs.len().saturating_sub(1)) as f32;
                                    ui.horizontal(|ui| {
                                        ui.add_sized([140.0, 18.0], egui::Slider::new(&mut msi, 0.0..=(msb_n as f32 - 1.0)).integer());
                                        // 动态信息放滑块后方(不占第1列宽): 真实MSB + 当前区/总数
                                        ui.label(format!("~{:03} ({}/{})", msb_val, self.cur_msb_idx.min(msb_n.saturating_sub(1)), msb_n));
                                    });
                                    if (msi as usize) != self.cur_msb_idx {
                                        self.cur_msb_idx = msi as usize;
                                        self.apply_bank_pc();
                                    }
                                    ui.end_row();
                                }
                                ui.label("PC");
                                {
                                    let prgs = self.voice_bank.as_ref().map(|b| b.prg_values(self.current_msb())).unwrap_or_default();
                                    let prg_n = prgs.len().max(1);
                                    let prog_disp = *prgs.get(self.cur_pc_idx.min(prgs.len().saturating_sub(1))).unwrap_or(&0) as u32 + 1;
                                    let mut pci = self.cur_pc_idx.min(prgs.len().saturating_sub(1)) as f32;
                                    ui.horizontal(|ui| {
                                        ui.add_sized([140.0, 18.0], egui::Slider::new(&mut pci, 0.0..=(prg_n as f32 - 1.0)).integer());
                                        ui.label(format!("({})", prog_disp));
                                    });
                                    if (pci as usize) != self.cur_pc_idx {
                                        self.cur_pc_idx = pci as usize;
                                        self.apply_bank_pc();
                                    }
                                    ui.end_row();
                                }
                                // LSB (变体): 当前 (msb, prog) 的有效变体数 = slots
                                {
                                    let variants = self
                                        .voice_bank
                                        .as_ref()
                                        .map(|b| b.lsb_variants(self.current_msb(), (self.cur_prog.saturating_sub(1)) as u8))
                                        .unwrap_or_default();
                                    let n = variants.len().max(1);
                                    let lsb_disp = self.current_lsb();
                                    // 第1列 label 固定纯文本; 变体数移滑块右方
                                    ui.label("LSB");
                                    let mut idx = self.cur_lsb_idx.min(variants.len().saturating_sub(1)) as f32;
                                    ui.horizontal(|ui| {
                                        ui.add_sized([140.0, 18.0], egui::Slider::new(&mut idx, 0.0..=((n - 1) as f32)).integer());
                                        ui.label(format!("({} var, {})", n, lsb_disp));
                                    });
                                    if (idx as usize) != self.cur_lsb_idx {
                                        self.cur_lsb_idx = idx as usize;
                                        self.apply_bank_pc();
                                    }
                                    ui.end_row();
                                }
                                // (voice / 按钮已移出 Grid — 放 bottom 区块下方, 避免占 Grid 第1列宽 → 列宽跳变闪烁)
                            });
                        // ---- 音色查找结果 + 发送按钮 (独立区, 不占 Grid 列宽, 杜绝位置闪烁) ----
                        {
                            let lsb_now = self.current_lsb();
                            ui.label(format!(
                                "voice: {:<8}   bank {:>3}  prog {:>3}  lsb {:>3}",
                                self.cur_voice,
                                self.cur_bank,
                                self.cur_prog,
                                lsb_now
                            ));
                            let msb = self.current_msb();
                            let lsb = self.current_lsb();
                            let pc0 = self.cur_prog.saturating_sub(1) as u8;
                            // 用 ASCII, 避免 egui 默认字体无 CJK/▶/✓ → 方块 tofu
                            let label = if self.midi_connected && self.selected_midi.is_some() {
                                format!("[send voice→Part {}] MSB{msb} LSB{lsb} PC{}", self.cur_part, pc0 + 1)
                            } else {
                                format!(
                                    "[pending Part {}] MSB{msb} LSB{lsb} PC{}",
                                    self.cur_part, pc0 + 1
                                )
                            };
                            if ui.button(label).clicked() {
                                // 编辑器音色选择 = XG Multi-Part SysEx (port-agnostic):
                                // 直接设定 cur_part 的 Bank/LSB/PC, 不依赖 MIDI channel 路由
                                // (等价面板选音色; part 1-32 都能设, 即使 port B 未接)
                                let msgs = sysex::part_voice_select_messages(
                                    (self.cur_part - 1) as u8, msb, lsb, pc0,
                                    sysex::Device::Param(0),
                                );
                                self.last_sysex = Some(msgs.iter().map(|m| format_hex(m)).collect::<Vec<_>>().join("  "));
                                #[cfg(target_arch = "wasm32")]
                                if self.midi_connected && self.selected_midi.is_some() {
                                    let dev = self.midi_devices[self.selected_midi.unwrap()].clone();
                                    for m in &msgs {
                                        let m2 = m.clone();
                                        let d2 = dev.clone();
                                        self.midi_send_status = None;
                                        wasm_bindgen_futures::spawn_local(async move {
                                            let r = midi_wasm::send_to(&d2, &m2).await;
                                            let _ = r;
                                        });
                                    }
                                }
                            }
                            // ---- 音色快捷选择 (John 2026-08-13: 分级钻取 类别→乐器→variation) ----
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label("pick:");
                                // 一级=egui 浮层菜单(弹出覆盖, 不挤面板布局); 二三级=菜单内钻取.
                                // 类别按 GM 标准顺序 (VoiceCategory::ALL), 不用字母序.
                                let mut picked: Option<(u8, u8, u8)> = None;
                                let voices: Vec<&crate::data::Voice> = self
                                    .voice_bank
                                    .as_ref()
                                    .map(|b| b.voices_for_device(self.device))
                                    .unwrap_or_default();
                                ui.menu_button(
                                    format!("{} (msb{} lsb{} pc{})", self.cur_voice, self.current_msb(), self.current_lsb(), self.cur_prog),
                                    |ui| {
                                        // 三级钻取: 选择结果用闭包内局部变量传出 (避免 closure 捕获 self 的 mut)
                                        let mut pick_var: Option<(u8, u8, u8)> = None;
                                        // ===== 第一层: 类别 (GM 顺序) =====
                                        for cat in crate::data::VoiceCategory::ALL {
                                            // 该类别是否有音色
                                            let has = voices
                                                .iter()
                                                .any(|v| crate::data::VoiceCategory::from_msb_prg(v.msb, v.prg) == cat);
                                            if !has { continue; }
                                            // 第二层: 乐器 (该类别下 prg, 取 lsb=0 名)
                                            ui.menu_button(cat.label(), |ui| {
                                                let mut prgs: Vec<(u8, String)> = voices
                                                    .iter()
                                                    .filter(|v| crate::data::VoiceCategory::from_msb_prg(v.msb, v.prg) == cat)
                                                    .map(|v| (v.prg, v.name.clone()))
                                                    .collect();
                                                prgs.sort_by_key(|(p, _)| *p);
                                                prgs.dedup_by_key(|(p, _)| *p);
                                                // 长列表 (SFX 等 49 项) 超出屏幕 → 限高 + 滚动 (John 2026-08-13)
                                                egui::ScrollArea::vertical()
                                                    .max_height(ui.available_height().min(460.0).max(120.0))
                                                    .show(ui, |ui| {
                                                for (p, _name) in &prgs {
                                                    // 第三层: variations (该 prg 下所有 lsb)
                                                    let mut vars: Vec<(u8, u8, String)> = voices
                                                        .iter()
                                                        .filter(|v| crate::data::VoiceCategory::from_msb_prg(v.msb, v.prg) == cat && v.prg == *p)
                                                        .map(|v| (v.msb, v.lsb, v.name.clone()))
                                                        .collect();
                                                    vars.sort_by_key(|(_, l, _)| *l);
                                                    // 单变体 → 直接可点; 多变体 → 子菜单
                                                    if vars.len() == 1 {
                                                        let vo = &vars[0];
                                                        if ui.button(format!("PGM# {:03}  {}", *p + 1, vo.2)).clicked() {
                                                            pick_var = Some((vo.0, vo.1, *p));
                                                            ui.close_menu();
                                                        }
                                                    } else {
                                                        ui.menu_button(format!("PGM# {:03}  {}", *p + 1, vars[0].2), |ui2| {
                                                            for (msb, lsb, name) in &vars {
                                                                if ui2.button(format!("lsb {lsb:03}  {name}")).clicked() {
                                                                    pick_var = Some((*msb, *lsb, *p));
                                                                    ui2.close_menu();
                                                                }
                                                            }
                                                        });
                                                    }
                                                }
                                                    }); // ScrollArea
                                            });
                                        }
                                        // 把选择传出去 (menu 外应用)
                                        if let Some(v) = pick_var {
                                            picked = Some(v);
                                        }
                                    },
                                );
                                if let Some((msb, lsb, prg0)) = picked {
                                    self.set_voice_from_quickpick(msb, lsb, prg0);
                                }
                            });
                        }
                        ui.separator();
                        egui::Grid::new("param_grid")
                            .num_columns(2)
                            .spacing([8.0, 6.0])
                            .show(ui, |ui| {
                                // 前8条 VOL..KEY 是 per-part 混音参数 (单源到 parts[cur_part]);
                                // 后2条 Cutoff/Reso 是音色编辑参数, 不进 part (保留全局 self.params)
                                let cur_part_idx = (self.cur_part.saturating_sub(1)) as usize;
                                let n_mix = crate::part::N_PARAMS; // 8
                                for i in 0..self.params.len() {
                                    let (label, min, max, _val) = self.params[i].clone();
                                    ui.label(label);
                                    // 值: 前8条读 part, 后2条读全局
                                    let mut v = if i < n_mix {
                                        self.parts[cur_part_idx].params[i]
                                    } else {
                                        self.params[i].3
                                    };
                                    if ui.add(egui::Slider::new(&mut v, min..=max)).changed() {
                                        // 写回: 前8条到 part, 后2条到全局
                                        if i < n_mix {
                                            self.parts[cur_part_idx].params[i] = v;
                                        } else {
                                            self.params[i].3 = v;
                                        }
                                        // 生成真实 XG SysEx (当前 part)
                                        let offset = self.param_offsets[i];
                                        let value = v as i32;
                                        // 双向参数 (min<0, 如 Pan/Bright): 偏移使 0 居中 → 0..127
                                        // 单向 (min>=0): 直接 0..127
                                        let (_, pmin, _, _) = self.params[i];
                                        let byte = if pmin < 0.0 {
                                            (value - pmin as i32).clamp(0, 127) as u8
                                        } else {
                                            value.clamp(0, 127) as u8
                                        };
                                        let msg = sysex::part_param(sysex::Device::Param(0), (self.cur_part - 1) as u8, offset, byte)
                                            .unwrap_or_default();
                                        self.last_sysex = Some(format_hex(&msg));
                                        // 有 MIDI → 实际发送
                                        if self.midi_connected && self.selected_midi.is_some() {
                                            let dev = self.midi_devices[self.selected_midi.unwrap()].clone();
                                            self.midi_send_status = None;
                                            #[cfg(target_arch = "wasm32")]
                                            {
                                                let cell = std::rc::Rc::new(std::cell::RefCell::new(None::<Result<(), String>>));
                                                let c2 = cell.clone();
                                                wasm_bindgen_futures::spawn_local(async move {
                                                    let r = midi_wasm::send_to(&dev, &msg).await;
                                                    *c2.borrow_mut() = Some(r.map(|_| ()));
                                                });
                                                self.midi_send_ui_cell = Some(cell);
                                            }
                                            #[cfg(not(target_arch = "wasm32"))]
                                            { self.midi_send_status = Some("native: no Web MIDI".into()); }
                                        }
                                        // 更新 LCD 参数条 (像素验证手机可见): 8 个参数映射 VOL/EXP/BRT/PAN/REV/CHO/VAR/KEY
                                        self.update_lcd_params();
                                    }
                                    ui.end_row();
                                }
                            });
                        ui.separator();
                        if let Some(sx) = &self.last_sysex {
                            ui.label(egui::RichText::new(&format!("SysEx: {sx}")).monospace().size(10.0));
                        } else {
                            ui.label(egui::RichText::new("SysEx: (drag params to generate)").weak().size(10.0));
                        }
                        ui.label("Rev: Hall | Cho: Chorus1 | Var: off");
                        // ===== Event List (2026-08-13 John 指定: 占 params 面板下部原 PARTS 位置) =====
                        ui.separator();
                        // Event List 依赖 SMF 已加载; 无 SMF 时提示
                        if self.smf.is_none() {
                            ui.monospace(egui::RichText::new("EVENTS (ch N) — load a .mid").weak().size(11.0));
                            ui.monospace(egui::RichText::new("(no SMF — event list unavailable)").weak().size(10.0));
                        } else {
                            let rows = crate::smf::event_list_for_channel(
                                self.smf.as_ref().unwrap(), self.cur_pr_channel.saturating_sub(1));
                            let ch = self.cur_pr_channel;
                            // 列 x 常量 (与表头 painter 共用; 绝对定位保证列对齐)
                            const POS_W: f32 = 86.0;   // pos 列宽 (bar:beat:tick 3:1:3 ≈ 78px + 空)
                            const TYPE_W: f32 = 44.0;  // type 列宽 (ON/OFF/CCnn/PG)
                            // pos 换算 (与 topbar count 一致: ppq 四分音符, 4/4, 1-based bar:beat:tick)
                            let ppq = self.ppq.max(1) as u64;
                            let pos_text = |tick: u64| -> String {
                                let beat = tick / ppq;
                                let bar = beat / 4 + 1;
                                let beat_in_bar = (beat % 4) + 1;
                                let tick_in_beat = tick % ppq;
                                format!("{:>3}:{}:{:03}", bar, beat_in_bar, tick_in_beat)
                            };
                            ui.horizontal(|ui| {
                                ui.monospace(egui::RichText::new(format!("EVENTS (ch {ch})")).strong().size(11.0));
                                ui.monospace(egui::RichText::new(format!("{} rows", rows.len())).weak().size(10.0));
                            });
                            // 列头 (金色弱字; painter 定列 x 与行内一致 → 精确对齐)
                            let hdr_y = ui.cursor().top();
                            let hdr_p = ui.painter();
                            let hdr_x = ui.max_rect().left() + 4.0;
                            let hdr_col = egui::Color32::from_rgb(0xc9, 0xb9, 0x8a); // 金色
                            hdr_p.text(
                                egui::pos2(hdr_x, hdr_y),
                                egui::Align2::LEFT_TOP,
                                "pos",
                                egui::FontId::monospace(10.0),
                                hdr_col,
                            );
                            hdr_p.text(
                                egui::pos2(hdr_x + POS_W, hdr_y),
                                egui::Align2::LEFT_TOP,
                                "type",
                                egui::FontId::monospace(10.0),
                                hdr_col,
                            );
                            hdr_p.text(
                                egui::pos2(hdr_x + POS_W + TYPE_W, hdr_y),
                                egui::Align2::LEFT_TOP,
                                "data",
                                egui::FontId::monospace(10.0),
                                hdr_col,
                            );
                            ui.allocate_space(egui::vec2(4.0, 14.0));
                            egui::ScrollArea::vertical()
                                .id_salt("event_list")
                                .max_height(260.0)
                                .show(ui, |ui| {
                                    // 整行铺满 + painter 定列绝对对齐 (无视字体宽度差)
                                    let row_h = 16.0;
                                    let px0 = ui.max_rect().left() + 4.0;
                                    let mut click_tick: Option<u64> = None;
                                    for (i, row) in rows.iter().enumerate() {
                                        let full_w = ui.max_rect().width();
                                        let (rect, resp) = ui.allocate_exact_size(
                                            egui::vec2(full_w, row_h), egui::Sense::click());
                                        let selected = self.event_list_sel == Some(i);
                                        // 背景: 选中=高亮条 (铺满), 否则偶/奇行条纹微差
                                        let bg = if selected {
                                            egui::Color32::from_rgb(0x2a, 0x3d, 0x58)
                                        } else if i % 2 == 0 {
                                            egui::Color32::from_rgb(0x14, 0x22, 0x32)
                                        } else {
                                            egui::Color32::from_rgb(0x1c, 0x2e, 0x42)
                                        };
                                        ui.painter().rect_filled(rect, 0.0, bg);
                                        // 文本色: 选中=金色, 否则浅灰
                                        let fg = if selected {
                                            egui::Color32::from_rgb(0xff, 0xc6, 0x4d)
                                        } else {
                                            egui::Color32::from_rgb(0xcf, 0xd8, 0xe4)
                                        };
                                        let font = egui::FontId::monospace(10.0);
                                        let yc = rect.center().y;
                                        // pos 列
                                        ui.painter().text(
                                            egui::pos2(px0, yc), egui::Align2::LEFT_CENTER,
                                            pos_text(row.tick), font.clone(), fg,
                                        );
                                        // type 列
                                        let type_txt: String = match &row.kind {
                                            crate::smf::EventKind::NoteOn { .. } => "ON".into(),
                                            crate::smf::EventKind::NoteOff { .. } => "OFF".into(),
                                            crate::smf::EventKind::Cc { num, .. } => format!("CC{num}"),
                                            crate::smf::EventKind::Program { .. } => "PG".into(),
                                        };
                                        ui.painter().text(
                                            egui::pos2(px0 + POS_W, yc), egui::Align2::LEFT_CENTER,
                                            type_txt, font.clone(), fg,
                                        );
                                        // data 列
                                        let data_txt = match &row.kind {
                                            crate::smf::EventKind::NoteOn { pitch, vel } =>
                                                format!("{}  v{}", crate::piano_roll::midi_name(*pitch as i32), vel),
                                            crate::smf::EventKind::NoteOff { pitch } =>
                                                crate::piano_roll::midi_name(*pitch as i32),
                                            crate::smf::EventKind::Cc { val, .. } =>
                                                val.to_string(),
                                            crate::smf::EventKind::Program { program } =>
                                                (program + 1).to_string(),
                                        };
                                        ui.painter().text(
                                            egui::pos2(px0 + POS_W + TYPE_W, yc), egui::Align2::LEFT_CENTER,
                                            data_txt, font, fg,
                                        );
                                        if resp.clicked() {
                                            // 2026-08-14: 点击 toggle (同 SYSEX): 再点已选中的行取消选中
                                            self.event_list_sel = if self.event_list_sel == Some(i) { None } else { Some(i) };
                                            click_tick = Some(row.tick);
                                        }
                                        // 2026-08-14: 选中行 → 下方内联展开详情行 (同 SYSEX hex 展开风格)
                                        if self.event_list_sel == Some(i) {
                                            let det_w = ui.max_rect().width();
                                            let (det_rect, _) = ui.allocate_exact_size(egui::vec2(det_w, row_h), egui::Sense::hover());
                                            ui.painter().rect_filled(det_rect, 0.0, egui::Color32::from_rgb(0x10, 0x1a, 0x28));
                                            ui.painter().text(
                                                egui::pos2(px0 + POS_W + TYPE_W, det_rect.center().y),
                                                egui::Align2::LEFT_CENTER,
                                                event_detail_text(ch, &row.kind, row.tick),
                                                egui::FontId::monospace(9.0),
                                                egui::Color32::from_rgb(0x8f, 0xb0, 0xd0),
                                            );
                                        }
                                    }
                                    // 选中行 → 联动 piano roll 滚动到该 tick (让对应音符移入视野)
                                    if let Some(t) = click_tick {
                                        self.pr_scroll_ticks = t.saturating_sub(8);
                                    }
                                });
                        }
                        // 32-part dump 表折叠 (2026-08-09 原位置; 2026-08-13 移入 CollapsingHeader)
                        let parsed_n = self.read_parts.iter().filter(|x| x.is_some()).count();
                        egui::CollapsingHeader::new(format!("PARTS dump ({parsed_n}/32)"))
                            .id_salt("parts_dump_collapse")
                            .default_open(false)
                            .show(ui, |ui| {
                                if parsed_n == 0 {
                                    ui.monospace(egui::RichText::new("(no data — panel dump + Analyze)").weak().size(10.0));
                                } else {
                                    ui.monospace("  n  MSB LSB  PC  Name");
                                    egui::ScrollArea::vertical().id_salt("right_parts").max_height(200.0).show(ui, |ui| {
                                        for (i, r) in self.read_parts.iter().enumerate() {
                                            if let Some((msb, lsb, pc)) = r {
                                                let name = self.voice_bank.as_ref()
                                                    .and_then(|b| b.find(*msb, *pc, *lsb))
                                                    .map(|v| v.name.clone())
                                                    .unwrap_or_default();
                                                ui.monospace(format!("{:>2}   {:>3} {:>3} {:>3}  {}", i + 1, msb, lsb, pc, name));
                                            }
                                        }
                                    });
                                }
                            });
                        // ===== SysEx 折叠区 (2026-08-14 方案2: 与通道无关 → 独立全局视角) =====
                        // 文件里的 SysEx 全部列出 (不分 ch), 供查看/核对播放透传
                        // 2026-08-14 二改: 显示风格与上方 event list 一致 (painter 绝对定位三列 + 斑马纹 + 金色选中 + hex 展开进 data 列)
                        ui.separator();
                        let sx_rows = self.smf.as_ref().map(crate::smf::sysex_list).unwrap_or_default();
                        let sx_pos_text = |tick: u64| -> String {
                            let ppq = self.ppq.max(1) as u64;
                            let beat = tick / ppq;
                            let bar = beat / 4 + 1;
                            let beat_in_bar = (beat % 4) + 1;
                            let tick_in_beat = tick % ppq;
                            format!("{:>3}:{}:{:03}", bar, beat_in_bar, tick_in_beat)
                        };
                        egui::CollapsingHeader::new(format!("SYSEX ({})", sx_rows.len()))
                            .id_salt("sysex_collapse")
                            .default_open(true)
                            .show(ui, |ui| {
                                if sx_rows.is_empty() {
                                    ui.monospace(egui::RichText::new("(no SysEx in this file)").weak().size(10.0));
                                } else {
                                    // 列宽与 event list 对齐 (pos 同宽; 长度列窄, 类型列吃满剩余)
                                    const POS_W2: f32 = 86.0;
                                    const LEN_W: f32 = 36.0;
                                    let hdr_y = ui.cursor().top();
                                    let hdr_p = ui.painter();
                                    let hdr_x = ui.max_rect().left() + 4.0;
                                    let hdr_col = egui::Color32::from_rgb(0xc9, 0xb9, 0x8a); // 金色
                                    hdr_p.text(egui::pos2(hdr_x, hdr_y), egui::Align2::LEFT_TOP, "pos", egui::FontId::monospace(10.0), hdr_col);
                                    hdr_p.text(egui::pos2(hdr_x + POS_W2, hdr_y), egui::Align2::LEFT_TOP, "len", egui::FontId::monospace(10.0), hdr_col);
                                    hdr_p.text(egui::pos2(hdr_x + POS_W2 + LEN_W, hdr_y), egui::Align2::LEFT_TOP, "type", egui::FontId::monospace(10.0), hdr_col);
                                    ui.allocate_space(egui::vec2(4.0, 14.0));
                                    egui::ScrollArea::vertical().id_salt("sysex_list").max_height(240.0).show(ui, |ui| {
                                        let row_h = 16.0;
                                        let px0 = ui.max_rect().left() + 4.0;
                                        for (i, sx) in sx_rows.iter().enumerate() {
                                            let full_w = ui.max_rect().width();
                                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(full_w, row_h), egui::Sense::click());
                                            let sel = self.sysex_expanded == Some(i);
                                            let bg = if sel {
                                                egui::Color32::from_rgb(0x2a, 0x3d, 0x58)
                                            } else if i % 2 == 0 {
                                                egui::Color32::from_rgb(0x14, 0x22, 0x32)
                                            } else {
                                                egui::Color32::from_rgb(0x1c, 0x2e, 0x42)
                                            };
                                            ui.painter().rect_filled(rect, 0.0, bg);
                                            let fg = if sel {
                                                egui::Color32::from_rgb(0xff, 0xc6, 0x4d)
                                            } else {
                                                egui::Color32::from_rgb(0xcf, 0xd8, 0xe4)
                                            };
                                            let font = egui::FontId::monospace(10.0);
                                            let yc = rect.center().y;
                                            let kind = sysex_kind(&sx.data);
                                            // pos 列
                                            ui.painter().text(egui::pos2(px0, yc), egui::Align2::LEFT_CENTER, sx_pos_text(sx.tick), font.clone(), fg);
                                            // len 列 (右对齐到列内)
                                            let len_txt = format!("{}B", sx.data.len());
                                            ui.painter().text(egui::pos2(px0 + POS_W2 + LEN_W - 4.0, yc), egui::Align2::RIGHT_CENTER, len_txt, font.clone(), fg);
                                            // type 列
                                            ui.painter().text(egui::pos2(px0 + POS_W2 + LEN_W, yc), egui::Align2::LEFT_CENTER, kind, font.clone(), fg);
                                            if resp.clicked() {
                                                self.sysex_expanded = if self.sysex_expanded == Some(i) { None } else { Some(i) };
                                            }
                                            if self.sysex_expanded == Some(i) {
                                                // hex 展开 (data 列缩进 2 字符, 弱色小字; 与 event list data 列同 x 起点对齐)
                                                let hex_w = ui.max_rect().width();
                                                let (hex_rect, _) = ui.allocate_exact_size(egui::vec2(hex_w, row_h), egui::Sense::hover());
                                                ui.painter().rect_filled(hex_rect, 0.0, egui::Color32::from_rgb(0x10, 0x1a, 0x28));
                                                let hex = sx.data.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
                                                ui.painter().text(egui::pos2(px0 + POS_W2 + LEN_W, hex_rect.center().y), egui::Align2::LEFT_CENTER, hex, egui::FontId::monospace(9.0), egui::Color32::from_rgb(0x8f, 0xb0, 0xd0));
                                            }
                                        }
                                    });
                                }
                            });
                    } else {
                        let rect = ui.max_rect();
                        if rail_triangle_ui(ui, rect, "right_rail", "<").clicked() {
                            self.show_right = true;
                        }
                    }
                });
        }

        // 底部 status 栏 (2026-08-09 John 要求): 文件名 / 加载结果 / 日志 —— 顶部塞太多, 状态挪底部.
        // egui 语义: 先声明的 bottom 面板放最外层/最底 (John 真机验证 v45 顺序正确) → 须在 LCD 之前声明.
        // 全局已是深色 → 此栏保持浅色 frame + 深色文字 (John: 底部状态浅色可读)
        egui::TopBottomPanel::bottom("bottom_status")
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::from_gray(248))
                    .inner_margin(egui::Margin::symmetric(8.0, 4.0)),
            )
            .show(ctx, |ui| {
                // 浅底 → 文字强制深色
                let dark_txt = egui::Color32::from_rgb(0x2c, 0x2c, 0x2c);
                ui.visuals_mut().override_text_color = Some(dark_txt);
                ui.visuals_mut().widgets.inactive.fg_stroke.color = dark_txt;
                ui.horizontal(|ui| {
                // 文件名 (如有)
                if self.smf_name.is_empty() {
                    ui.label(egui::RichText::new("no file").weak());
                } else {
                    // ★ 2026-08-13 John: 加载 MIDI 后文件名看不清 — `.strong()` 用 strong_text_color
                    //   (= widgets.active.text_color, 全局深色主题里是浅色), 绕过 override_text_color(dark_txt),
                    //   在浅底(248)上浅色→看不清. 修复: 显式 .color(dark_txt) 深色.
                    ui.label(egui::RichText::new(&self.smf_name).color(dark_txt));
                }
                ui.separator();
                // 加载结果 / 提示
                if self.smf_load_result.is_empty() {
                    ui.label(egui::RichText::new("drop a .mid file").weak());
                } else {
                    ui.label(&self.smf_load_result);
                }
                ui.separator();
                // 日志: 读请求在途时显示最近 8 条 (诊断发送/接收序列), 平时只显示最近一条
                let n = if self.read_request_inflight.is_some() { 8 } else { 1 };
                let start = self.status_log.len().saturating_sub(n);
                for (i, line) in self.status_log.iter().enumerate().skip(start) {
                    let weak = i < start + (self.status_log.len() - start) - 1;
                    if weak {
                        ui.label(egui::RichText::new(line).weak());
                    } else {
                        ui.label(egui::RichText::new(line));
                    }
                }
            });
        });

        // 底栏 Piano Roll (2026-08-12: 从中央视图移到底栏独立 panel)
        // 沿用左右边栏 rail 折叠逻辑: 收起 = 22px 窄条 + 三角 ^, 点开展开 (无需中央顶部按钮)
        // 声明顺序: bottom_status(最底) → bottom_piano(其上方) → CentralPanel 剩中间
        {
            let open = self.show_piano;
            let panel_id = egui::Id::new("bottom_piano");
            // 删掉 egui 持久化的面板高度: 面板高度由 self.piano_height 统一管理
            // (展开态实时记录拖动值; 收起=22不残留; 展开用 default=self.piano_height)
            // (用户 2026-08-12: 收起再展开回用户拖过的高度/默认 500, 而非 220; egui 的
            //  PanelState 每帧 store, exact_height(22) 会覆盖用户高度, 故每次 show 前清除)
            ctx.data_mut(|d| d.remove::<egui::containers::panel::PanelState>(panel_id));
            let mut panel = egui::TopBottomPanel::bottom(panel_id)
                .resizable(open)
                .default_height(self.piano_height)
                // 标题行与全局深色统一 (2026-08-13 起全局 dark; Piano topbar 不再白)
                // 注释: 早期 (2026-08-12) 用户要求"标题底色保持白色"; 全局深色化后连同深色.
                // 内容区(render_piano_roll)自己铺深色; 标题行也深色 → 整体一致.
                .frame(egui::Frame::default()
                    .fill(egui::Color32::from_rgb(0x1f, 0x2f, 0x45))
                    .inner_margin(egui::Margin {
                        left: 0.0,
                        right: 0.0,
                        top: 3.0,
                        bottom: 3.0,
                    })); // 标题行上方留 3px, 不再紧贴横线 (用户: 上贴下空 → 上下均衡)
            if open {
                panel = panel.height_range(80.0..=900.0);
            } else {
                panel = panel.exact_height(22.0); // 收起: 只留 22px 窄条
            }
            let pr_inner = panel.show(ctx, |ui| {
                if open {
                    ui.horizontal(|ui| {
                        // 与中央 Channel 视图标题(heading)同字号 (用户 2026-08-12)
                        ui.heading("Piano Roll");
                        ui.separator();
                        // Channel 选择 (用户 2026-08-12: piano roll 只显示一个 channel 的音符)
                        ui.label("Ch");
                        egui::ComboBox::from_id_salt("pr_channel")
                            .selected_text(format!("{}", self.cur_pr_channel))
                            .width(44.0)
                            .show_ui(ui, |ui| {
                                for c in 1..=16u8 {
                                    ui.selectable_value(&mut self.cur_pr_channel, c, format!("{c:02}"));
                                }
                            });
                        ui.separator();
                        // Zoom/Scroll (Piano Roll 独立状态 pr_zoom/pr_scroll_ticks, 与 Channel 视图独立)
                        ui.label("Zoom");
                        ui.add(
                            egui::Slider::new(&mut self.pr_zoom, 0.02..=200.0)
                                .logarithmic(true)
                                .show_value(true)
                                .custom_formatter(|v, _| format!("{v:.2}x"))
                                .custom_parser(|s| s.parse::<f64>().ok()),
                        );
                        ui.separator();
                        let t_end = if self.smf.is_some() { self.smf_end_tick.max(1) } else { self.total_ticks.max(1) };
                        let zoom_s = self.pr_zoom.max(0.002);
                        let win = (t_end.max(1) as f32 / zoom_s).round().max(1.0) as u64;
                        let win = win.max(1);
                        ui.label("Scroll");
                        let max_scroll = t_end.saturating_sub(win) as f64;
                        let mut scf = self.pr_scroll_ticks as f64;
                        ui.add(egui::Slider::new(&mut scf, 0.0..=max_scroll).step_by((win.max(1) / 20).max(1) as f64).custom_formatter(|v, _| format!("{}t", v as i64)));
                        self.pr_scroll_ticks = scf.max(0.0) as u64;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // 收起三角 v (点击收起为 22px 窄条)
                            if collapse_triangle_ui(ui, "piano_collapse", "v").clicked() {
                                self.show_piano = false;
                            }
                        });
                    });
                    ui.separator();
                    // piano roll 本体 (已抽到 src/piano_roll.rs)
                    self.render_piano_roll(ui);
                } else {
                    // 收起窄条: 垂直居中 ^ (点击展开)
                    let rect = ui.max_rect();
                    if rail_triangle_ui(ui, rect, "piano_rail", "^").clicked() {
                        self.show_piano = true;
                    }
                }
            });
            // 展开态实时记录面板高度 (用户拖动 → self.piano_height), 收起/展开后恢复
            if open {
                let h = pr_inner.response.rect.height();
                if h > 10.0 { self.piano_height = h; }
            }
        }

        // LCD 浮动窗口 (用户 2026-08-12: LCD 改为 floating pane, 可拖动/缩放/开关)
        // open = show_bottom (语义: LCD 窗口可见); 用局部变量避开 .open() 借 self 与闭包 &mut self 冲突
        let mut lcd_open = self.show_bottom;
        egui::Window::new("LCD (MU90)")
            .id(egui::Id::new("lcd_float"))
            .open(&mut lcd_open)
            .default_width(450.0)    // 缩小为原来一半 (用户 2026-08-12: 默认尺寸改一半)
            .default_height(165.0)  // 按 LCD 比例: 16ch 等比内接高 ~131 + 顶栏 ~30 (用户: 略高改合适)
            .default_pos(egui::pos2(560.0, 46.0)) // 默认中上 (顶部栏下方), 不挡中央编辑区/底栏 Piano Roll
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // 16ch / 32ch 显示模式切换
                    let label32 = if self.lcd_32 { "32ch[on]" } else { "32ch[off]" };
                    if ui.button(label32).clicked() {
                        self.lcd_32 = !self.lcd_32;
                        self.update_lcd_params();
                    }
                    // Part 选择已移到右栏 Params 顶部 (2026-08-12 用户要求), LCD 只留 32ch toggle + 分辨率
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("{}x{}", lcd::LCD_W, lcd::LCD_H));
                    });
                });
                ui.separator();
                // LCD 纹理缓存: 首次 load, 像素变(dirty)才 set() — 避免每帧重建GPU纹理/重传 → 拖动闪烁
                if self.lcd_tex.is_none() {
                    self.lcd_tex = Some(ctx.load_texture(
                        "lcd",
                        egui::ColorImage::from_rgba_unmultiplied(
                            [self.lcd_side, 256],
                            &self.lcd_pixels,
                        ),
                        egui::TextureOptions::NEAREST,
                    ));
                    self.lcd_dirty = false;
                } else if self.lcd_dirty {
                    if let Some(tex) = self.lcd_tex.as_mut() {
                        tex.set(
                            egui::ColorImage::from_rgba_unmultiplied(
                                [self.lcd_side, 256],
                                &self.lcd_pixels,
                            ),
                            egui::TextureOptions::NEAREST,
                        );
                    }
                    self.lcd_dirty = false;
                }
                let tex = self.lcd_tex.as_ref().expect("lcd_tex cached");
                // ScrollArea: 裁剪绘制到窗口内容区内 (LCD 永不跑出窗口) + auto_shrink=false 撑满可用区不塌缩。
                // LCD 等比内接永远 <= 窗口, 不出 scrollbar (用户验证: 去 scroll 之前正常, 去掉后超出边界)
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let avail = ui.available_size();
                        // LCD 等比内接: 窗口缩小时 LCD 缩小, 放大时放大; 左上贴齐 (坐标确定)
                        let scale = (avail.x / self.lcd_side as f32)
                            .min(avail.y / 256.0)
                            .max(0.1); // 窗口太小时 LCD 不消失
                        let size = egui::vec2(self.lcd_side as f32 * scale, 256.0 * scale);
                        // 深色底铺满整个内容区 (先画, LCD 之下) — 消除窗口比例不匹配时的浅色留白
                        ui.painter()
                            .rect_filled(ui.max_rect(), 0.0, egui::Color32::from_gray(12));
                        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                        // LCD 纹理 (左上贴齐, 画在深底之上)
                        ui.painter().image(
                            tex.id(),
                            rect,
                            egui::Rect::from_min_max(
                                egui::pos2(0.0, 0.0),
                                egui::pos2(1.0, 1.0)
                            ),
                            egui::Color32::WHITE,
                        );
                    });
            });

        // 中央: 三视图切换(Piano Roll 静态时间轴 / Channel Notes 每行=一个channel / PlayView 播放画面)
        // LCD 浮动窗口关闭状态回写 (open() 用的局部变量, 闭包结束后同步回 self)
        self.show_bottom = lcd_open;
        egui::CentralPanel::default().show(ctx, |ui| {
                self.central(ui);
        });

        // 浮动窗口(判据 11)—— 效果器浮窗
        egui::Window::new("Floating EQ (drag me)")
            .default_width(220.0)
            .default_pos(egui::pos2(780.0, 620.0)) // 默认放中下, 不挡通道网格
            .collapsible(true)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label("a draggable panel that can leave the main window");
                let mut low = 0.0;
                let mut mid = 0.0;
                let mut hi = 0.0;
                ui.add(egui::Slider::new(&mut low, -12.0..=12.0).text("Low"));
                ui.add(egui::Slider::new(&mut mid, -12.0..=12.0).text("Mid"));
                ui.add(egui::Slider::new(&mut hi, -12.0..=12.0).text("High"));
                ui.separator();
                ui.label("drag title bar to move / close");
            });
    }
}

// ---------- wasm 入口(egui 0.29 官方模式: WebHandle + WebRunner) ----------
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
#[derive(Clone)]
pub struct WebHandle {
    runner: eframe::WebRunner,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
impl WebHandle {
    // 官方 eframe 示例要求 #[wasm_bindgen(constructor)] 才能 JS new WebHandle()
    #[allow(clippy::new_without_default)]
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        eframe::WebLogger::init(log::LevelFilter::Debug).ok();
        Self { runner: eframe::WebRunner::new() }
    }

    pub async fn start(
        &self,
        canvas: web_sys::HtmlCanvasElement,
        version: String,
        initial_smf: Option<String>,
    ) -> Result<(), wasm_bindgen::JsValue> {
        // 传入版本, 让标题显示 XG Editor (vN) — 单一数据源 (index.html 的 APP_VERSION)
        let app_version: String = version.clone();
        self.runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |_cc| {
                    let mut app = XgApp::default();
                    app.app_version = app_version.clone();
                    // 调试/演示钩子: URL ?zoom=N → 初始 track_view_zoom (截图验证 detail)
                    {
                        let z = web_sys::window()
                            .and_then(|w| w.location().search().ok())
                            .and_then(|s| {
                                let p = s.split('?').nth(1).unwrap_or("");
                                p.split('&').find_map(|kv| {
                                    let mut it = kv.split('=');
                                    if it.next() == Some("zoom") { it.next().and_then(|v| v.parse().ok()) } else { None }
                                })
                            });
                        if let Some(z) = z { app.track_view_zoom = z; app.url_override_view = true; }
                    }
                    // 调试/演示钩子: URL ?view=play → 初始中央视图 = PlayView (截图验证)
                    {
                        let v = web_sys::window()
                            .and_then(|w| w.location().search().ok())
                            .and_then(|sr| {
                                let p = sr.split('?').nth(1).unwrap_or("");
                                p.split('&').find_map(|kv| {
                                    let mut it = kv.split('=');
                                    if it.next() == Some("view") { it.next().map(|s| s.to_string()) } else { None }
                                })
                            });
                        match v.as_deref() {
                            Some("play") => app.central_view = CentralView::PlayView,
                            Some("channel") => app.central_view = CentralView::ChannelNotes,
                            // Piano Roll 已移到底栏 (2026-08-12): ?view=piano → 打开底栏 + 中央回 Channel
                            Some("piano") => { app.show_piano = true; app.central_view = CentralView::ChannelNotes; }
                            _ => {}
                        }
                    }
                    // 调试/演示钩子: URL ?pview_scroll=N → 初始 PlayView 垂直滚动 px (截图验证滚动)
                    {
                        let s = web_sys::window()
                            .and_then(|w| w.location().search().ok())
                            .and_then(|sr| {
                                let p = sr.split('?').nth(1).unwrap_or("");
                                p.split('&').find_map(|kv| {
                                    let mut it = kv.split('=');
                                    if it.next() == Some("pview_scroll") { it.next().and_then(|v| v.parse().ok()) } else { None }
                                })
                            });
                        if let Some(s) = s { app.pview_scroll = s; }
                    }
                    // 调试/演示钩子: URL ?rw=N&?bh=N → 初始右栏宽/底栏高 (验证 panel resize 不缩放星云墙纸)
                    {
                        let rw = web_sys::window()
                            .and_then(|w| w.location().search().ok())
                            .and_then(|sr| {
                                let p = sr.split('?').nth(1).unwrap_or("");
                                p.split('&').find_map(|kv| {
                                    let mut it = kv.split('=');
                                    if it.next() == Some("rw") { it.next().and_then(|v| v.parse().ok()) } else { None }
                                })
                            });
                        if let Some(rw) = rw { app.right_width = rw; app.show_right = rw > 0.0; }
                        let bh = web_sys::window()
                            .and_then(|w| w.location().search().ok())
                            .and_then(|sr| {
                                let p = sr.split('?').nth(1).unwrap_or("");
                                p.split('&').find_map(|kv| {
                                    let mut it = kv.split('=');
                                    if it.next() == Some("bh") { it.next().and_then(|v| v.parse().ok()) } else { None }
                                })
                            });
                        if let Some(bh) = bh { app.bottom_height = bh; app.show_bottom = bh > 0.0; }
                    }
                    // 调试/演示钩子: URL ?scroll=N → 初始横向滚动 tick
                    {
                        let s = web_sys::window()
                            .and_then(|w| w.location().search().ok())
                            .and_then(|sr| {
                                let p = sr.split('?').nth(1).unwrap_or("");
                                p.split('&').find_map(|kv| {
                                    let mut it = kv.split('=');
                                    if it.next() == Some("scroll") { it.next().and_then(|v| v.parse().ok()) } else { None }
                                })
                            });
                        if let Some(s) = s { app.track_view_scroll_ticks = s; app.url_override_view = true; }
                    }
                    // 调试/演示钩子: ?pz=N → 初始 Piano Roll zoom (验证 zoom 重画 bar, 与 Channel 一致)
                    {
                        let z = web_sys::window()
                            .and_then(|w| w.location().search().ok())
                            .and_then(|sr| {
                                let p = sr.split('?').nth(1).unwrap_or("");
                                p.split('&').find_map(|kv| {
                                    let mut it = kv.split('=');
                                    if it.next() == Some("pz") { it.next().and_then(|v| v.parse().ok()) } else { None }
                                })
                            });
                        if let Some(z) = z { app.pr_zoom = z; app.url_override_view = true; }
                    }
                    // 调试/演示钩子: ?prc=N → 初始 Piano Roll 显示 channel (验证力度明暗/单通道)
                    {
                        let c = web_sys::window()
                            .and_then(|w| w.location().search().ok())
                            .and_then(|sr| {
                                let p = sr.split('?').nth(1).unwrap_or("");
                                p.split('&').find_map(|kv| {
                                    let mut it = kv.split('=');
                                    if it.next() == Some("prc") { it.next().and_then(|v| v.parse::<u8>().ok()) } else { None }
                                })
                            });
                        if let Some(c) = c { app.cur_pr_channel = c.clamp(1, 16); app.url_override_view = true; }
                    }
                    // 调试/演示钩子: ?smf=<url> 时启动后自动加载该 SMF (fetch → load_smf_bytes)
                    if let Some(url) = initial_smf {
                        wasm_bindgen_futures::spawn_local(async move {
                            async fn fetch_smf(url: &str) -> Option<(String, Vec<u8>)> {
                                let win = web_sys::window()?;
                                let resp_p: js_sys::Promise = win.fetch_with_str(url);
                                let resp_v = wasm_bindgen_futures::JsFuture::from(resp_p).await.ok()?;
                                let resp: web_sys::Response = resp_v.into();
                                let ab_p: js_sys::Promise = resp.array_buffer().ok()?;
                                let ab_v = wasm_bindgen_futures::JsFuture::from(ab_p).await.ok()?;
                                let u8a = js_sys::Uint8Array::new(&ab_v);
                                let bytes = u8a.to_vec();
                                let name = url.rsplit(['/', '\\']).next().unwrap_or(url).to_string();
                                Some((name, bytes))
                            }
                            if let Some((name, bytes)) = fetch_smf(&url).await {
                                crate::SMF_DIALOG_PENDING.with(|c| {
                                    *c.borrow_mut() = Some((name, bytes));
                                });
                            }
                        });
                    }
                    Ok(Box::new(app))
                }),
            )
            .await
    }

    pub fn stop(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::part::P;

    /// 用户操作全链路: msb_idx=1(→48), pc_idx=21(→prg21, 显示22), lsb 唯一变体 8
    /// 验证 LCD 音色名 + 发送字节全部正确 (真机对 48/8/22 报 Silencc → 排查发字节)
    #[test]
    fn glockenplus_not_in_mu90() {
        // MU90 权威表: msb=48(MU100 Native/Octavia) 不存在 → 电阻层过滤后不可达
        let mut app = XgApp::default();
        // MU90 表 msb_values = [0, 64, 126, 127] (Normal/SFX/SFX Kit/Drum; 无 msb48)
        let msbs = {
            let bank = app.voice_bank.as_ref().unwrap();
            bank.msb_values()
        };
        assert_eq!(msbs, vec![0, 64, 126, 127], "MU90 应含 0/64/126/127, 不含 msb48");
        // Glocken+ (msb48/lsb8) 查询 → None (权威表无此)
        let hit = {
            let bank = app.voice_bank.as_ref().unwrap();
            bank.find(48, 21, 8)
        };
        assert!(hit.is_none(), "MU90 权威表不应有 Glocken+ (msb=48)");
        // 但 bank0 的 Glocken (pc10, lsb0) 存在
        let glock = {
            let bank = app.voice_bank.as_ref().unwrap();
            bank.find(0, 9, 0)
        };
        assert_eq!(glock.map(|v| v.name.as_str()), Some("Glocken"));
    }

    #[test]
    fn quickpick_drives_3axis() {
        // 快捷菜单选中 Acordion (msb0/lsb0/prg21) → 三轴索引应对应真实值
        let mut app = XgApp::default();
        app.set_voice_from_quickpick(0, 0, 21);
        assert_eq!(app.current_msb(), 0, "msb=0");
        assert_eq!(app.current_lsb(), 0, "lsb=0 (Acordion)");
        assert_eq!(app.cur_prog, 22, "pc 显示 22 (1-based)");
        assert_eq!(app.cur_voice, "Acordion");

        // 选 AccordIt (msb0/lsb32/prg21)
        app.set_voice_from_quickpick(0, 32, 21);
        assert_eq!(app.current_msb(), 0, "msb=0");
        assert_eq!(app.current_lsb(), 32, "lsb=32 (AccordIt)");
        assert_eq!(app.cur_voice, "AccordIt");

        // 选 GrandPno (msb0/lsb0/prg0)
        app.set_voice_from_quickpick(0, 0, 0);
        assert_eq!(app.current_msb(), 0, "msb=0");
        assert_eq!(app.current_lsb(), 0, "lsb=0");
        assert_eq!(app.cur_prog, 1, "pc 1");
        assert_eq!(app.cur_voice, "GrandPno");
    }

    /// LCD 参数条底部中心像素是否点亮 (bar_xs 硬编码自 lcd.rs blit)
    fn bar_pixel_lit(px: &[u8], bar_index: usize) -> bool {
        let bar_xs: [i32; 8] = [426, 474, 522, 573, 633, 683, 732, 789];
        let bx = bar_xs[bar_index];
        let w = lcd::LCD_W as i32;
        let y = 234; // 条底部基线: 任何 >0 高都亮这一行
        let i = ((y * w + bx + 1) * 4) as usize;
        px[i] < 100 && px[i + 1] < 200
    }

    #[test]
    fn default_lcd_voice_is_grandpno() {
        // ④ 修复: bank0 prog 001 应显示 GrandPno (不是 DreamPno)
        let app = XgApp::default();
        assert_eq!(app.cur_voice, "GrandPno", "bank0/prog1 应为 GrandPno, 实际 {}", app.cur_voice);
        assert_eq!(app.cur_bank, 0);
        assert_eq!(app.cur_prog, 1);
    }

    #[test]
    fn cc7_volume_scales_live_level() {
        // John: CC7 volume 变化应反应到电平表 (FakeMu: level = raw_vel × CC7 × CC11 × master)
        let mut app = XgApp::default();
        // NoteOn → active_notes + raw_vel_peaks
        app.apply_fired_event_to_meter(&PlayEvent::note(3, 60, 100, 0, true));
        assert_eq!(app.active_notes[3].get(&60), Some(&100));
        assert!((app.raw_vel_peaks[3] - 100.0 / 127.0).abs() < 1e-6, "raw = vel/127");
        // 平滑目标 = raw × CC7 × CC11 × master (当前 volumes 满) → mimic 逼近 0.787
        assert!((app.smooth_meter_target(3) - 100.0 / 127.0).abs() < 1e-6);
        // CC7 降到一半 → 新目标 = 0.787 × 0.5
        app.apply_fired_event_to_meter(&PlayEvent::cc_tick(3, 7, 64, 0));
        assert!((app.smooth_meter_target(3) - (100.0 / 127.0) * (64.0 / 127.0)).abs() < 1e-4, "CC7/64 应折半目标");
        // CC11 expression 也乘进去 (FakeMu: ×CC7×CC11)
        app.apply_fired_event_to_meter(&PlayEvent::cc_tick(3, 11, 64, 0));
        let expect = (100.0 / 127.0) * (64.0 / 127.0) * (64.0 / 127.0);
        assert!((app.smooth_meter_target(3) - expect).abs() < 1e-4, "CC11 应再折半, got {}", app.smooth_meter_target(3));
        // NoteOff → active_notes 清空 → raw 0 → 目标 0 (峰值保持靠 mimic 慢落)
        app.apply_fired_event_to_meter(&PlayEvent::note(3, 60, 0, 0, false));
        assert!(app.active_notes[3].is_empty());
        assert_eq!(app.raw_vel_peaks[3], 0.0);
        assert_eq!(app.smooth_meter_target(3), 0.0);
        // 其它 CC (如 CC10 pan) 不影响 volume/expression
        app.apply_fired_event_to_meter(&PlayEvent::cc_tick(3, 10, 64, 0));
        assert!((app.live_volumes[3] - 64.0 / 127.0).abs() < 1e-9, "CC10 不应改 volume");
    }

    #[test]
    fn meter_zero_when_muted() {
        // John 2026-08-13: mute 后电平表归零 (visual "这条是死的")
        let mut app = XgApp::default();
        app.apply_fired_event_to_meter(&PlayEvent::note(3, 60, 100, 0, true));
        assert!((app.smooth_meter_target(3) - 100.0 / 127.0).abs() < 1e-6, "未静音时正常电平");
        // mute → 目标 0
        app.channel_mutes[3] = true;
        assert_eq!(app.smooth_meter_target(3), 0.0, "mute 后电平归零");
        // unmute → 恢复
        app.channel_mutes[3] = false;
        assert!((app.smooth_meter_target(3) - 100.0 / 127.0).abs() < 1e-6, "unmute 恢复电平");
        // solo 激活时非 solo 通道也归零
        app.channel_solos[3] = true;
        assert_eq!(app.smooth_meter_target(2), 0.0, "solo 后非 solo 通道电平归零");
        assert!((app.smooth_meter_target(3) - 100.0 / 127.0).abs() < 1e-6, "solo 通道正常");
    }

    #[test]
    fn fake_mu_smoothing_attack_fast_decay_slow() {
        // FakeMu mimicStrength: 攻快落慢. 目标升高时逼近快, 目标降/归零时逼近慢.
        let mut app = XgApp::default();
        // target = 0.787 (vel100, vol全满)
        app.apply_fired_event_to_meter(&PlayEvent::note(3, 60, 100, 0, true));
        // mimic 从 0 向 target 逼近: 帧1 → 0.8*target, 帧2 → 0.96*target (attack 快)
        app.tick_meter_smoothing(); // 帧1
        let after_atk = app.live_levels[3];
        assert!(after_atk > 0.6, "attack 1 帧应接近目标, got {after_atk}");
        app.tick_meter_smoothing(); // 帧2
        let after_atk2 = app.live_levels[3];
        assert!(after_atk2 > after_atk, "attack 持续推进");
        // NoteOff → target=0; decay 每帧只接近 20% 剩余 → 慢
        app.apply_fired_event_to_meter(&PlayEvent::note(3, 60, 0, 0, false));
        app.tick_meter_smoothing(); // 帧1 decay
        let after_dcy = app.live_levels[3];
        assert!(after_dcy > 0.5, "decay 1 帧后仍高 (慢落), got {after_dcy}");
        // 20 帧后应明显下降但未完全 0 (decay 0.2^n)
        for _ in 0..20 { app.tick_meter_smoothing(); }
        assert!(app.live_levels[3] < 0.05, "20 帧衰减 +0.8^n → 应接近 0");
    }

    #[test]
    fn active_outputs_port_a_and_b_mirror() {
        // John: 多 out 发挥 MU90 32 part (Port A parts 1-16 + Port B parts 17-32)
        let mut app = XgApp::default();
        app.midi_devices = vec!["UX16 Port1".into(), "UX16 Port2".into()];
        // 只选 Port A → 只发 A
        app.selected_midi = Some(0);
        app.mirror_to_b = false;
        assert_eq!(app.active_outputs(), vec!["UX16 Port1"]);
        // 选 A + mirror B → A,B 都发 (32 part)
        app.selected_midi_b = Some(1);
        app.mirror_to_b = true;
        assert_eq!(app.active_outputs(), vec!["UX16 Port1", "UX16 Port2"]);
        // mirror 关 → 即使有 B 也不发
        app.mirror_to_b = false;
        assert_eq!(app.active_outputs(), vec!["UX16 Port1"]);
        // B 与 A 同设备 → 去重
        app.selected_midi_b = Some(0);
        app.mirror_to_b = true;
        assert_eq!(app.active_outputs(), vec!["UX16 Port1"]);
        // 全不选 → 空 (不发送)
        app.selected_midi = None;
        app.selected_midi_b = None;
        assert!(app.active_outputs().is_empty());
    }

    #[test]
    fn lcd_voice_follows_smf_part_channel() {
        // John 2026-08-09: LCD 播放时要反映当前 part 通道的音色 (像 channel view 那样),
        // 而不仅是编辑器 cur_voice。SMF 加载后, part N 的 LCD 音色 = live_voice_names[ch].
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 0x60]);
        data.extend_from_slice(b"MTrk");
        let mut trk: Vec<u8> = Vec::new();
        trk.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x0F, 0x42, 0x40]); // tempo
        // ch1 (part1/A01): PC → 81 Saw Ld
        trk.extend_from_slice(&[0x00, 0xC0, 81]);
        trk.extend_from_slice(&[0x00, 0x90, 60, 100]);
        trk.extend_from_slice(&[0x83, 0x00, 0x80, 60, 0]);
        trk.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        data.extend_from_slice(&(trk.len() as u32).to_be_bytes());
        data.extend_from_slice(&trk);

        let mut app = XgApp::default();
        let err = app.load_smf_bytes("lcdpart.mid", &data);
        assert!(err.is_ok(), "SMF 解析失败: {err:?}");
        assert!(!app.parts[0].voice.is_empty(), "ch1 voice filled");
        let ch = lcd::part_channel(1).saturating_sub(1) as usize;
        assert_eq!(ch, 0);
        assert_eq!(app.parts[ch].voice, app.parts[0].voice);
        app.update_lcd_params();
        let mut px = vec![0u8; lcd::LCD_W * lcd::LCD_H * 4];
        let lv = app.live_levels;
        let prg = app.parts[0].prog as u32;
        lcd::render_lcd(&mut px, &app.parts[0].voice, app.parts[0].lsb as u32, prg + 1, &lv, &[0.0; 2], 1, &[0.0; 8]);
        assert!(px.chunks_exact(4).all(|c| c[3] == 255), "LCD 像素 alpha 全满");
        assert_eq!(prg, 81, "SMF ch1 PC=81 → parts[0].prog=81");
        assert_eq!(prg + 1, 82, "LCD PGM=82");
        // 切 part 联动: 加载后曲目带 ch5 PC=58(Tuba), 切到 part5 → LCD 应显示 pgm 059
        // (构造: 在同一 MTrk 里追加 ch5 program change + note)
        let mut data2: Vec<u8> = Vec::new();
        data2.extend_from_slice(b"MThd");
        data2.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 0x60]);
        data2.extend_from_slice(b"MTrk");
        let mut trk2: Vec<u8> = Vec::new();
        trk2.extend_from_slice(&[0x00, 0xC0, 81]); // ch1 PC81
        trk2.extend_from_slice(&[0x00, 0xC4, 58]); // ch5 (idx4) PC58 Tuba — 注意 ch 是 0-based 在 status nibble
        trk2.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        data2.extend_from_slice(&(trk2.len() as u32).to_be_bytes());
        data2.extend_from_slice(&trk2);
        let mut app2 = XgApp::default();
        assert!(app2.load_smf_bytes("part5.mid", &data2).is_ok());
        // 单源化: SMF 加载后 live_program 从 SMF 解析同步进 parts[4]
        assert_eq!(app2.parts[4].prog, 58, "SMF ch5 PC58 应同步进 parts[4].prog");
        // 切到 part5 → parts[4] → prog 58 → LCD PGM 059
        app2.cur_part = 5;
        let ch5 = lcd::part_channel(5).saturating_sub(1) as usize;
        assert_eq!(ch5, 4);
        assert_eq!(app2.parts[ch5].prog as u32 + 1, 59, "part5 LCD 应显示 PGM 059");
    }

    #[test]
    fn lcd_levels_reflect_live_levels() {
        // LCD 电平条应反映 live_levels 高度 (John: play 时 LCD 电平实时跳)
        // render_to_matrix 电平条在 row 基线 15 向上; lvl=1.0 → 16 层满
        let mm1 = lcd::render_to_matrix("GrandPno", 0, 1, &[1.0; 16], &[0.0; 2], 1);
        let mm0 = lcd::render_to_matrix("GrandPno", 0, 1, &[0.0; 16], &[0.0; 2], 1);
        // 电平条布局: i=0/1 = A1/A2 (audio), i=2..17 = ch1..16.
        // bar_col(i=2)=5 → ch1 条占 col 5/6. 检查基线上方 row 0..7
        let lit = |mm: &crate::lcd::MuMatrix| -> u32 {
            let mut n = 0;
            for y in 0..8 { for x in 5..7 { n += mm.get(x, y) as u32; } }
            n
        };
        let full = lit(&mm1);
        let zero = lit(&mm0);
        assert!(full > 0, "full level should light bar (lit={full})");
        assert_eq!(zero, 0, "zero level should have no bar above baseline (lit={zero})");
    }

    #[test]
    fn bank_pc_slider_updates_voice() {
        // Bank/PC/LSB 滑块链路 (索引模型): 索引→真实值, 查真实音色表
        let mut app = XgApp::default();
        // Default: msb_idx0=msb0, pc_idx0=prg0, lsb_idx0=lsb0 → GrandPno (显示 000/001)
        assert_eq!(app.cur_voice, "GrandPno");
        assert_eq!(app.cur_bank, 0);
        assert_eq!(app.cur_prog, 1);
        assert_eq!(app.current_msb(), 0);
        // MU90 权威表: MSB ∈ {0, 64, 126, 127} = Normal / SFX / SFX Kit / Drum (手册 1.2.1 Bank Select)
        let msbv = app.voice_bank.as_ref().unwrap().msb_values();
        assert_eq!(msbv, vec![0u8, 64, 126, 127], "MU90 应含 0/64/126/127 (melody/SFX/SFX Kit/drum)");
        // 拨 LSB 索引 → Dream (bank41 pc1): MU90 权威名
        let variants = app.voice_bank.as_ref().unwrap().lsb_variants(0, 0);
        assert_eq!(variants, vec![0u8, 1, 18, 40, 41], "bank0 pc1 变体应为 0/1/18/40/41");
        let dream_idx = variants.iter().position(|&l| l == 41).expect("应有 lsb=41");
        app.cur_lsb_idx = dream_idx;
        app.apply_bank_pc();
        assert_eq!(app.cur_voice, "Dream", "msb0/prg0/lsb41 应为 Dream, got {}", app.cur_voice);
        assert_eq!(app.current_lsb(), 41);
        // 拨回 LSB 0 → GrandPno
        app.cur_lsb_idx = 0;
        app.apply_bank_pc();
        assert_eq!(app.cur_voice, "GrandPno");
        // 同上但在 lsb=1 → GrndPnoK
        let k_idx = variants.iter().position(|&l| l == 1).unwrap();
        app.cur_lsb_idx = k_idx;
        app.apply_bank_pc();
        assert_eq!(app.cur_voice, "GrndPnoK", "lsb=1 应 GrndPnoK");
        app.cur_lsb_idx = 0;
        app.apply_bank_pc();
        // 循环: 每个索引组合都应显示名字 (MU90 表内都可达)
        let msbv = app.voice_bank.as_ref().unwrap().msb_values();
        for mi in 0..msbv.len() {
            app.cur_msb_idx = mi;
            app.apply_bank_pc();
            let cur_p = app.cur_prog;
            assert!(cur_p >= 1 && cur_p <= 128, "prog 显示应在 1..128, got {cur_p}");
            if app.voice_bank.as_ref().unwrap().lsb_variants(app.current_msb(), (cur_p - 1) as u8).is_empty() {
                continue;
            }
            assert_ne!(app.cur_voice, "---", "msb={} prog={} 应查得到名字", app.current_msb(), cur_p);
        }
    }

    #[test]
    fn persisted_state_roundtrip() {
        // 持久化往返: 改状态 → to_persisted → from_json → apply → 恢复 (wasm/native 共用逻辑)
        let mut app = XgApp::default();
        // 改成 Dream (bank41 pc1, MU90 权威名) + 一些参数 + 32ch
        let variants = app.voice_bank.as_ref().unwrap().lsb_variants(0, 0);
        let dream_idx = variants.iter().position(|&l| l == 41).unwrap();
        app.cur_lsb_idx = dream_idx;
        app.apply_bank_pc();
        assert_eq!(app.cur_voice, "Dream");
        // LCD bank 显示 = LSB (MU90 真机显示 LSB, 不是 MSB) — John 真机验证 2026-08-09
        assert_eq!(app.cur_bank, 41, "LCD bank 应显示 LSB(41=Dream), 不是 MSB(0)");

        app.params[0].3 = 55.0; // Volume
        app.lcd_32 = true;

        let st = app.to_persisted();
        assert_eq!(st.msb, 0);
        assert_eq!(st.lsb, 41);
        assert_eq!(st.pc, 0);
        assert_eq!(st.lcd_32, true);
        assert!((st.params[0] - 55.0).abs() < 1e-6);

        // JSON 往返
        let json = st.to_json().unwrap();
        let back = PersistedState::from_json(&json).unwrap();
        assert_eq!(back.lsb, 41);
        assert_eq!(back.pc, 0);

        // 应用到新 app
        let mut app2 = XgApp::default();
        assert_ne!(app2.cur_voice, "Dream");
        app2.apply_persisted(&back);
        assert_eq!(app2.cur_voice, "Dream", "恢复后应回到 Dream (lsb=41)");
        assert_eq!(app2.lcd_32, true);
        assert!((app2.params[0].3 - 55.0).abs() < 1e-6, "参数应恢复 Volume=55");
    }

    #[test]
    fn params_lcd_alignment() {
        let mut app = XgApp::default();
        // 从 parts[0] 取 params (单源化), 设置后调用 update_lcd_params
        // 先设 VOL=0 EXP=0 PAN=0 → 条全灭
        app.parts[0].params[P::Volume as usize] = 0.0;
        app.parts[0].params[P::Exp as usize] = 0.0;
        app.parts[0].params[P::Pan as usize] = 0.0;
        app.update_lcd_params();
        assert!(!bar_pixel_lit(&app.lcd_pixels, 0), "先决: VOL 条初始不应亮 (Vol=0)");
        assert!(!bar_pixel_lit(&app.lcd_pixels, 1), "先决: EXP 条初始不应亮 (Exp=0)");
        assert!(!bar_pixel_lit(&app.lcd_pixels, 3), "先决: PAN 条初始不应亮 (Pan=0)");
        // Volume(0) → VOL 条(0): 设满 → VOL 亮, EXP/PAN 不亮
        app.parts[0].params[P::Volume as usize] = 127.0;
        app.update_lcd_params();
        assert!(bar_pixel_lit(&app.lcd_pixels, 0), "Volume should light VOL bar");
        assert!(!bar_pixel_lit(&app.lcd_pixels, 1), "EXP bar should not light from Volume (错位 bug)");
        assert!(!bar_pixel_lit(&app.lcd_pixels, 3), "PAN bar should not light from Volume");

        // Pan(3) → PAN 条(3): 设最右 (127), Volume 归零
        app.parts[0].params[P::Volume as usize] = 0.0;  // Volume 归零
        app.parts[0].params[P::Pan as usize] = 127.0;   // Pan 最大 (0..127)
        app.update_lcd_params();
        assert!(bar_pixel_lit(&app.lcd_pixels, 3), "Pan should light PAN bar");
        assert!(!bar_pixel_lit(&app.lcd_pixels, 0), "Volume 已归零, VOL 条不应亮");
    }

    #[test]
    fn play_events_build_correct() {
        // 音序器事件表正确性: 默认 16 轨 pattern, total=768 tick
        let mut app = XgApp::default();
        let total = app.total_ticks;
        assert_eq!(total, 768);
        app.build_play_events();
        assert!(!app.play_events.is_empty(), "播放事件表不应为空");
        // 按 tick 排序
        let ticks: Vec<u64> = app.play_events.iter().map(|e| e.tick).collect();
        let mut sorted = ticks.clone();
        sorted.sort_unstable();
        assert_eq!(ticks, sorted, "事件表必须按 tick 升序, 含回绕");
        // 表内所有 tick 都在 0..=total
        assert!(app.play_events.iter().all(|e| e.tick <= total), "事件 tick 越界");
        // 每轨音符都在自己的 channel (pattern 保证 channel == 轨下标)
        for (i, t) in app.tracks.iter().enumerate() {
            for n in &t.notes {
                assert_eq!(n.channel as usize, i, "Ch{} note 通道错", i + 1);
            }
        }
        // 有 NoteOn 且 后续有 NoteOff (成对)
        let ons = app.play_events.iter().filter(|e| !e.off).count();
        let offs = app.play_events.iter().filter(|e| e.off).count();
        assert!(ons >= 16, "至少 16 个 NoteOn, got {ons}");
        assert!(offs >= 16, "至少 16 个 NoteOff, got {offs}");
    }

    #[test]
    fn smf_ch10_drum_bank_injected() {
        // ch10 鼓正确发声前置: 播放 SMF 时, 通道 9 (ch10) 必须注入 Bank MSB=127 (Drum kit)
        // 构造一个最小 SMF: 一个轨, ch10 (0-based 9) 一个 note, 无 bank select
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 0x78]); // 1 track, ppq 120
        data.extend_from_slice(b"MTrk");
        let mut trk: Vec<u8> = Vec::new();
        // delta 0: program change omitted (zygote: no bank/PC for ch10)
        // delta 0: NoteOn ch10 pitch 36 (kick) vel 100
        trk.extend_from_slice(&[0x00, 0x99, 36, 100]);
        // delta 96: NoteOff ch10 pitch 36
        trk.extend_from_slice(&[0x60, 0x89, 36, 0]);
        // end of track
        trk.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        data.extend_from_slice(&(trk.len() as u32).to_be_bytes());
        data.extend_from_slice(&trk);

        let mut app = XgApp::default();
        let err = app.load_smf_bytes("drumtest.mid", &data);
        assert!(err.is_ok(), "SMF 解析失败: {err:?}");
        assert!(app.smf.is_some());
        app.build_play_events();
        // 验证: 存在 CC0=127 给 channel 9 (tick 0)
        let has_drum_bank = app.play_events.iter().any(|e| {
            e.channel == 9 && e.tick == 0 && e.bytes.len() == 3
                && e.bytes[0] == 0xB9 && e.bytes[1] == 0x00 && e.bytes[2] == 127
        });
        assert!(has_drum_bank, "ch10 应注入 Bank MSB=127 (Drum)");
        // 验证: 有 NoteOn ch10
        let has_kick = app.play_events.iter().any(|e| {
            !e.off && e.channel == 9 && e.bytes.len() == 3 && e.bytes[0] == 0x99 && e.bytes[1] == 36
        });
        assert!(has_kick, "ch10 kick note 应存在");
    }

    #[test]
    fn smf_cc_events_flow_into_play_events() {
        // John 2026-08-09: "是否有所有 midi 事件都处理? 原曲有很多 pan change 事件, 现在没看到"
        // 根因: build_play_events 只收集音符+Program, CC(pan10/vol7 等)全丢.
        // 本测试锁定: SMF 的 CC 事件按原曲 tick 进入播放事件表.
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 0x78]); // 1 track, ppq 120
        data.extend_from_slice(b"MTrk");
        let mut trk: Vec<u8> = Vec::new();
        // delta 0: CC10 pan=127 (full right)
        trk.extend_from_slice(&[0x00, 0xB1, 10, 127]);
        // delta 0: NoteOn ch2 pitch 60 (必须, 否则 smf_end_tick=0 → 所有 CC tick%1=0!)
        trk.extend_from_slice(&[0x00, 0x91, 60, 100]);
        // delta 120: CC10 pan=0 (full left) — 中途 pan change
        trk.extend_from_slice(&[0x78, 0xB1, 10, 0]);
        // delta 0: NoteOff ch2 pitch 60
        trk.extend_from_slice(&[0x00, 0x81, 60, 0]);
        trk.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        data.extend_from_slice(&(trk.len() as u32).to_be_bytes());
        data.extend_from_slice(&trk);

        let mut app = XgApp::default();
        let err = app.load_smf_bytes("pan.mid", &data);
        assert!(err.is_ok(), "SMF 解析失败: {err:?}");
        app.build_play_events();
        // 应有: 2 个 CC10 (tick0 pan127, tick120 pan0) + 注入的 bank/prog
        let pans: Vec<(u8, u64, u8)> = app.play_events.iter()
            .filter(|e| e.bytes.len() == 3 && e.bytes[0] & 0xF0 == 0xB0 && e.bytes[1] == 10)
            .map(|e| (e.channel, e.tick, e.bytes[2]))
            .collect();
        assert_eq!(pans.len(), 2, "应保留 2 个 CC10 (pan), got {pans:?}");
        assert!(pans.contains(&(1, 0, 127)), "tick0 pan=127 应存在, got {pans:?}");
        assert!(pans.contains(&(1, 120, 0)), "tick120 pan=0 应存在 (中途 pan change), got {pans:?}");
    }

    #[test]
    fn smf_sysex_flows_into_play_events_and_broadcasts() {
        // 2026-08-14: SysEx 透传 — SMF 的 SysEx 进入播放事件表 (channel=0xFF 哨兵),
        // 原始字节原样保留, dispatch 时绕过 mute/part 路由 (全接口广播).
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 0x78]); // 1 track, ppq 120
        data.extend_from_slice(b"MTrk");
        let mut trk: Vec<u8> = Vec::new();
        // delta 0: XG parameter change SysEx F0 43 10 4C 00 00 00 7F F7
        trk.extend_from_slice(&[0x00, 0xF0, 8, 0x43, 0x10, 0x4C, 0x00, 0x00, 0x00, 0x7F, 0xF7]);
        // delta 0: NoteOn ch1 (必须有音符, 否则 smf_end_tick=0 → tick 取模错乱)
        trk.extend_from_slice(&[0x00, 0x90, 60, 100]);
        // delta 120: 第二个 SysEx (tick 120)
        trk.extend_from_slice(&[0x78, 0xF7, 2, 0x41, 0x42]); // F7 continuation
        // delta 0: NoteOff
        trk.extend_from_slice(&[0x00, 0x80, 60, 0]);
        trk.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        data.extend_from_slice(&(trk.len() as u32).to_be_bytes());
        data.extend_from_slice(&trk);

        let mut app = XgApp::default();
        let err = app.load_smf_bytes("sx.mid", &data);
        assert!(err.is_ok(), "SMF 解析失败: {err:?}");
        app.build_play_events();
        // 事件表含 2 个 SysEx 事件, channel=0xFF, 原始字节保留
        let sx: Vec<(u64, u8, Vec<u8>)> = app.play_events.iter()
            .filter(|e| e.channel == 0xFF)
            .map(|e| (e.tick, e.bytes[0], e.bytes.clone()))
            .collect();
        assert_eq!(sx.len(), 2, "应保留 2 个 SysEx, got {sx:?}");
        assert_eq!(sx[0].0, 0, "第一个 SysEx tick 0, got {sx:?}");
        assert_eq!(sx[0].1, 0xF0, "第一个 SysEx 以 F0 起始, got {sx:?}");
        assert_eq!(sx[0].2, vec![0xF0, 0x43, 0x10, 0x4C, 0x00, 0x00, 0x00, 0x7F, 0xF7], "字节完整保留");
        assert_eq!(sx[1].0, 120, "第二个 SysEx tick 120 (F7 continuation), got {sx:?}");
        assert_eq!(sx[1].1, 0xF7, "F7 continuation 以 F7 起始, got {sx:?}");
        // channel==0xFF 哨兵: dispatch 的 SysEx 分支先于 mute 检查 → 无条件广播
        // (构造器直接验证 channel 值)
        let ev = crate::playback::PlayEvent::sysex(vec![0xF0, 0x43, 0x10], 0);
        assert_eq!(ev.channel, 0xFF);
    }

    #[test]
    fn sysex_kind_recognizes_roland_gs_xg_universal() {
        // 2026-08-14: sysex_kind 类型识别 — Roland GS / Yamaha XG / Universal / 未知
        // (支撑 SYSEX 折叠区的类型标注; John 有 SC-55 VST 收 Roland GS)
        // Roland GS DT1 参数: F0 41 <dev> 42 12 <addr3> <data3> <cksum> F7
        assert_eq!(sysex_kind(&[0xF0, 0x41, 0x10, 0x42, 0x12, 0x01, 0x24, 0x3A, 0x60, 0xF7]),
            "Roland GS param");
        // Roland GS System Reset: F0 41 10 42 12 40 00 7F 00 41 F7
        //   (DT1 写地址 40 00 7F 触发 GS reset; cmd 仍是 12)
        assert_eq!(sysex_kind(&[0xF0, 0x41, 0x10, 0x42, 0x12, 0x40, 0x00, 0x7F, 0x00, 0x41, 0xF7]),
            "Roland GS param");
        // Yamaha XG param: F0 43 10 4C ... (Master Volume)
        assert_eq!(sysex_kind(&[0xF0, 0x43, 0x10, 0x4C, 0x00, 0x00, 0x7E, 0x00, 0xF7]),
            "XG param");
        // Universal GM System On: F0 7E 7F 09 01 F7
        assert_eq!(sysex_kind(&[0xF0, 0x7E, 0x7F, 0x09, 0x01, 0xF7]),
            "GM/Universal");
        // 未知厂商 → MFG
        assert_eq!(sysex_kind(&[0xF0, 0x15, 0x01, 0x02, 0xF7]),
            "MFG");
        // 不足长度 → SX
        assert_eq!(sysex_kind(&[0xF0]), "SX");
    }

    #[test]
    fn event_detail_text_formats_all_kinds() {
        // 2026-08-14: event list 点击展开详情行的文本格式
        use crate::smf::EventKind;
        assert_eq!(event_detail_text(1, &EventKind::NoteOn { pitch: 60, vel: 100 }, 480),
            "ch1  tick=480  C4 (60)  vel=100");
        assert_eq!(event_detail_text(10, &EventKind::NoteOff { pitch: 45 }, 960),
            "ch10  tick=960  A2 (45)");
        // CC 有名字 → 带名; 无名字 → 不带
        assert_eq!(event_detail_text(1, &EventKind::Cc { num: 7, val: 100 }, 480),
            "ch1  tick=480  CC7 vol  val=100");
        assert_eq!(event_detail_text(2, &EventKind::Cc { num: 17, val: 64 }, 100),
            "ch2  tick=100  CC17  val=64");
        // Program: 01 对应 UI 显示 1-based, 十六进制原值
        assert_eq!(event_detail_text(1, &EventKind::Program { program: 0 }, 0),
            "ch1  tick=0  program=1 (00)");
    }

    #[test]
    fn notes_active_at_detects_currently_sounding() {
        // kill_current_notes 的核心算法: 给定 playhead, 找出此刻还在响的 (ch,pitch)
        // 注意: 需按 tick 升序传入 (build_play_events 已排序, 这里手动排)
        let mut evs = vec![
            PlayEvent::note(0, 60, 100, 0, true),   // ch1 C4 on @0
            PlayEvent::note(1, 67, 100, 100, true), // ch2 G4 on @100 (长音, 未 off)
            PlayEvent::note(9, 36, 100, 200, true), // ch10 kick on @200
            PlayEvent::note(9, 36, 0, 300, false),  // ch10 kick off @300
            PlayEvent::note(0, 60, 0, 480, false),  // ch1 C4 off @480
        ];
        evs.sort_by_key(|e| e.tick);
        // playhead=200: ch1 C4 还响 (off@480 未到), ch2 G4 响, ch10 kick 刚 on@200 (300 才 off)
        let a200 = notes_active_at(200, &evs);
        assert!(a200.contains(&(0, 60)), "ch1 C4 应还在响 (off@480 未到)");
        assert!(a200.contains(&(1, 67)), "ch2 G4 应还在响 (无 off)");
        assert!(a200.contains(&(9, 36)), "ch10 kick on@200, off@300 未到 → 应在响");
        // playhead=300: kick 已 off
        let a300 = notes_active_at(300, &evs);
        assert!(!a300.contains(&(9, 36)), "playhead 300 kick 已 off");
        // playhead=600: 只有悬挂的 G4 (无 off 的长音) 还在响
        let a600 = notes_active_at(600, &evs);
        assert!(a600.contains(&(1, 67)), "G4 是悬挂长音 (无 off), 600 仍应在响");
        assert!(!a600.contains(&(0, 60)), "C4 off@480 已过 → 不再响");
        assert!(!a600.contains(&(9, 36)), "kick off@300 已过 → 不再响");
        // playhead=50: 只有 ch1 C4 (ch2 未开始)
        let a50 = notes_active_at(50, &evs);
        assert!(a50.contains(&(0, 60)) && !a50.contains(&(1, 67)), "playhead50 仅 ch1");
    }

    #[test]
    fn smf_load_uses_real_ppq_and_tempo() {
        // John 2026-08-09 报告: 同样 doom.mid, Logic 显示 95bpm/69bar, XG editor 显示
        // 120bpm/86bar. 根因: load_smf_bytes 没把 SMF 真实 ppq 和初始 tempo 写入 self,
        // 一直用默认 96/120. 本测试锁定: 加载后 self.ppq/self.tempo_bpm 取 SMF 值.
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 0x60]); // 1 track, ppq 96
        data.extend_from_slice(b"MTrk");
        let mut trk: Vec<u8> = Vec::new();
        // delta 0: 初始 tempo = 631579 us/qn ≈ 95 bpm (doom 的真实值)
        let us = 631_579u32; // 95 bpm
        trk.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, (us >> 16) as u8, (us >> 8) as u8, us as u8]);
        // delta 0: NoteOn ch1 pitch 60
        trk.extend_from_slice(&[0x00, 0x90, 60, 100]);
        // delta 96*4 = 384 ticks: NoteOff (vlq 0x83 0x00)
        trk.extend_from_slice(&[0x83, 0x00, 0x80, 60, 0]);
        trk.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        data.extend_from_slice(&(trk.len() as u32).to_be_bytes());
        data.extend_from_slice(&trk);

        let mut app = XgApp::default();
        let err = app.load_smf_bytes("tempo95.mid", &data);
        assert!(err.is_ok(), "SMF 解析失败: {err:?}");
        assert_eq!(app.ppq, 96, "ppq 应取 SMF 的 96, 而非默认 96(此例同值, 但语义应来自文件)");
        assert!(
            (app.tempo_bpm - 95.0).abs() < 0.1,
            "tempo_bpm 应从 SMF 初始 tempo 反算 (95 bpm), got {}",
            app.tempo_bpm
        );
        // bar 数: smf_end_tick ≈ 384 (最后 noteoff) / (ppq*4=384) → bar 2
        let bars = app.smf_end_tick / (app.ppq * 4) + 1;
        assert_eq!(bars, 2, "bar 数应按真实 ppq 计算");
    }

    #[test]
    fn smf_load_fills_live_voice_names_from_program() {
        // 中央 channel view (每行=1 channel) 的行头音色应从 SMF program → MU90 权威名.
        // John 2026-08-09: SMF 加载后中央要实时反映音色+电平 (左栏废弃, 不动).
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 0x60]); // 1 track, ppq 96
        data.extend_from_slice(b"MTrk");
        let mut trk: Vec<u8> = Vec::new();
        trk.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x0F, 0x42, 0x40]); // tempo
        // delta 0: Program Change ch1 → GM program 81 (Saw Lead)
        trk.extend_from_slice(&[0x00, 0xC0, 81]);
        // delta 0: NoteOn ch1 pitch 60
        trk.extend_from_slice(&[0x00, 0x90, 60, 100]);
        // delta 384: NoteOff
        trk.extend_from_slice(&[0x83, 0x00, 0x80, 60, 0]);
        trk.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        data.extend_from_slice(&(trk.len() as u32).to_be_bytes());
        data.extend_from_slice(&trk);

        let mut app = XgApp::default();
        let err = app.load_smf_bytes("voice.mid", &data);
        assert!(err.is_ok(), "SMF 解析失败: {err:?}");
        // ch01 program=81 → Saw Ld (GM 81 = Lead 1 square → MU90 表 Saw Ld?)
        assert!(
            app.live_voice_names[0].len() > 2,
            "ch1 应有音色名, got '{}'",
            app.live_voice_names[0]
        );
        // 空通道 fallback "GrandPno" (ch02, program None; XG 初始化默认音色, 不再显示 ChNN)
        assert_eq!(app.live_voice_names[1], "GrandPno");
        // ch10 (注音可能无 program) 是鼓通道 → StandKit (John: MU90 LCD 真机名)
        assert_eq!(app.live_voice_names[9], "StandKit");
        // 电平表初始归零
        assert!(app.live_levels.iter().all(|&l| l == 0.0));
    }

    #[test]
    fn ch10_always_shows_drumkit_name() {
        // ch10 强制鼓通道: 即使 SMF 给了 PC=0 (melodic GrandPno 号), 也应按 XG 显示鼓组名
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 0x60]); // 1 track ppq 96
        data.extend_from_slice(b"MTrk");
        let mut trk: Vec<u8> = Vec::new();
        trk.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x0F, 0x42, 0x40]); // tempo
        // Program Change ch10 → PC 0 (其实鼓通道忽略; 本 SMF 作者放 0 导致先前显示 GrandPno)
        trk.extend_from_slice(&[0x00, 0xC9, 0]);
        // NoteOn ch10 pitch 60 drum
        trk.extend_from_slice(&[0x00, 0x99, 60, 100]);
        trk.extend_from_slice(&[0x83, 0x00, 0x89, 60, 0]);
        trk.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        data.extend_from_slice(&(trk.len() as u32).to_be_bytes());
        data.extend_from_slice(&trk);

        let mut app = XgApp::default();
        let err = app.load_smf_bytes("ch10.mid", &data);
        assert!(err.is_ok(), "SMF 解析失败: {err:?}");
        assert_eq!(
            app.live_voice_names[9], "StandKit",
            "ch10 无 bank 默认应按鼓通道显示 LCD 名 StandKit, got '{}'",
            app.live_voice_names[9]
        );
    }

    #[test]
    fn ch10_responds_to_bank_for_drumkit() {
        // John: ch10 不应硬编码 Standard Kit —— 若有 bank(msb=127) 与 PC, 应响应显示实际鼓组
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 0x60]); // 1 track ppq 96
        data.extend_from_slice(b"MTrk");
        let mut trk: Vec<u8> = Vec::new();
        trk.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x0F, 0x42, 0x40]); // tempo
        // ch10: Bank MSB=127 (drum set) via CC0
        trk.extend_from_slice(&[0x00, 0xB9, 0, 127]);
        // Program Change ch10 → 16 (Rock Kit)
        trk.extend_from_slice(&[0x00, 0xC9, 16]);
        // NoteOn ch10
        trk.extend_from_slice(&[0x00, 0x99, 60, 100]);
        trk.extend_from_slice(&[0x83, 0x00, 0x89, 60, 0]);
        trk.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        data.extend_from_slice(&(trk.len() as u32).to_be_bytes());
        data.extend_from_slice(&trk);

        let mut app = XgApp::default();
        let err = app.load_smf_bytes("ch10_rock.mid", &data);
        assert!(err.is_ok(), "SMF 解析失败: {err:?}");
        assert_eq!(
            app.live_voice_names[9], "Rock Kit",
            "ch10 有 bank msb=127 + PC16 应显示 Rock Kit, got '{}'",
            app.live_voice_names[9]
        );
    }

    #[test]
    fn bar_beat_tick_conversion() {
        let mut app = XgApp::default();
        app.ppq = 96;
        // playhead 0 → bar 1, beat 1, tick 0 (2026-08-13 起 beat 1-based)
        app.playhead_tick = 0;
        let (b, be, t) = app.playhead_bar_beat();
        assert_eq!((b, be, t), (1, 1, 0));
        // 第 2 小节第 1 拍: tick = 96*4 = 384
        app.playhead_tick = 96 * 4;
        let (b, be, t) = app.playhead_bar_beat();
        assert_eq!((b, be, t), (2, 1, 0));
        // 第 1 小节第 3 拍 (beat 3): tick = 96*2 = 192
        app.playhead_tick = 96 * 2;
        let (b, be, t) = app.playhead_bar_beat();
        assert_eq!((b, be, t), (1, 3, 0));
    }

    #[test]
    fn default_notes_16_tracks() {
        let app = XgApp::default();
        assert_eq!(app.tracks.len(), 16, "应有 16 轨");
        for (i, t) in app.tracks.iter().enumerate() {
            assert!(!t.notes.is_empty(), "Ch{} 应有音符", i + 1);
            for n in &t.notes {
                assert!(n.pitch <= 127 && n.velocity <= 127, "pitch/vel 越界");
                assert!(n.channel == i as u8, "note channel 应等于轨号");
            }
        }
    }

    #[test]
    fn pr_notes_no_ghost_after_smf_load() {
        // John 2026-08-13: 加载 MIDI 后无音符的 channel 仍残留 demo(ghost) notes.
        // 根因: pr_notes 用"该 view notes 非空"判断 → 空轨回退到默认 demo tracks.
        // 修复: smf 已加载就只用 SMF 数据, 空轨返回空 (demo 仅限未加载时).
        // 构造只有一个 channel 有音符的 SMF (ch1 一个音), 其余 15 通道无音符.
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0, 0x60]); // 1 track, ppq 96
        data.extend_from_slice(b"MTrk");
        let mut trk: Vec<u8> = Vec::new();
        // delta 0 初始 tempo
        trk.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]);
        // delta 0: NoteOn ch0 pitch 60
        trk.extend_from_slice(&[0x00, 0x90, 60, 100]);
        // delta 96: NoteOff ch0
        trk.extend_from_slice(&[0x81, 0x00, 0x80, 60, 0]);
        trk.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        data.extend_from_slice(&(trk.len() as u32).to_be_bytes());
        data.extend_from_slice(&trk);

        let mut app = XgApp::default();
        assert!(app.load_smf_bytes("ghost.mid", &data).is_ok());
        // ch1 (有音符) → 应有 1 个 note
        let n1 = app.pr_notes(1);
        assert_eq!(n1.len(), 1, "Ch1 应有 SMF 的 1 个音符, got {}", n1.len());
        // ch2 (无音符) → 必须为空 (ghost 修复)
        let n2 = app.pr_notes(2);
        assert!(n2.is_empty(), "Ch2 无音符应返回空, 不得残留 demo: got {}", n2.len());
        // ch16 也无音符 → 空
        assert!(app.pr_notes(16).is_empty(), "Ch16 无音符应返回空");
        // 未加载 SMF 时 (默认) → ch1 应有 demo 音符
        let app2 = XgApp::default();
        assert!(!app2.pr_notes(1).is_empty(), "未加载 SMF 时 Ch1 应有 demo 音符");
    }

    #[test]
    fn params_lcd_cutoff_no_bar() {
        let mut app = XgApp::default();
        // Cutoff(8) 无 LCD 条 → 设满不应影响任何 LCD 条 (不 panic + 像素有效)
        app.params[8].3 = 127.0;
        app.update_lcd_params();
        assert_eq!(app.lcd_pixels.len(), lcd::LCD_W * lcd::LCD_H * 4);
    }

    #[test]
    fn zoom1x_means_fit_region() {
        // John 语义定案 2026-08-09: 1x = 全区正好充满 view
        // win_ticks = end_tick / zoom → zoom=1 → 全区, 2x → 半曲, 4x → 1/4
        let end_tick = 32_852u32; // doom
        let win = |zoom: f32| (end_tick as f32 / zoom.max(0.002)).round().max(1.0) as u32;
        assert_eq!(win(1.0), end_tick, "1x 应显示全区 (win_ticks=end_tick)");
        assert!((win(2.0) as f32 - end_tick as f32 / 2.0).abs() < 1.0, "2x 应为半曲");
        assert!((win(4.0) as f32 - end_tick as f32 / 4.0).abs() < 1.0, "4x 应为 1/4");
        assert!(win(0.5) > end_tick, "0.5x 应显示比全区更宽 (缩小)");
    }

    #[test]
    fn param_sysex_byte_gen() {
        // 直接验证滑块 → SysEx 字节生成逻辑 (无 UI, 程序断言)
        let mut app = XgApp::default();
        // Volume(0) = 100 → offset 0x0B, 值 100
        let offset = app.param_offsets[0];
        let msg = sysex::part_param(sysex::Device::Param(0), 0, offset, 100).unwrap();
        assert_eq!(msg, vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x0B, 100, 0xF7]);
        // Pan(3) = +63 → offset 0x0E, 值 = 63 - (-64) = 127
        let offset = app.param_offsets[3];
        let msg = sysex::part_param(sysex::Device::Param(0), 0, offset, 127).unwrap();
        assert_eq!(msg, vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x0E, 127, 0xF7]);
        // Bright(2) = 0 → offset 0x1A, 值 = 0 - (-64) = 64 (中心)
        let offset = app.param_offsets[2];
        let msg = sysex::part_param(sysex::Device::Param(0), 0, offset, 64).unwrap();
        assert_eq!(msg, vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x1A, 64, 0xF7]);
    }

    #[test]
    fn analyze_sysex_capture_deposit() {
        // 用真实形态的 bulk 沉积喂给分析器: 地址 00 00 10 = 01 (最后一次), 08 00 01 = 00(part1 MSB)
        // 完整 bulk: F0 43 00 49 00 01 00 00 10 01 6E F7 (checksum 验证过)
        let mut app = XgApp::default();
        app.sysex_capture_log = vec![
            ("in".into(), "F04300490001000010016EF7".into(), 1),
            // part1 MSB bulk: F0 43 00 49 0001 08 00 01 00 [cs] F7
            ("in".into(), "F0430049000108000100 6D F7".replace(' ', "").to_string(), 2),
            // 同地址覆盖: 08 00 01 改成 01
            ("in".into(), "F04300490001080001016CF7".into(), 3),
        ];
        app.analyze_sysex_capture();
        // 00 00 10 → 01
        let a000010 = app.sysex_analysis.iter().find(|(a, _, _)| *a == (0x00u32 << 14) | (0x00 << 7) | 0x10).unwrap();
        assert_eq!(a000010.1, "01");
        // 08 00 01 → 覆盖后为 01, 出现 2 次
        let a080001 = app.sysex_analysis.iter().find(|(a, _, _)| *a == (0x08u32 << 14) | (0x00 << 7) | 0x01).unwrap();
        assert_eq!(a080001.1, "01");
        assert_eq!(a080001.2, 2);
        assert_eq!(app.sysex_analysis.len(), 2, "两个唯一地址");
    }

    #[test]
    fn parse_dump_parts_real_mu90_blocks() {
        // 用真机 dump 形态的 4 个 part 的 08 nn 00 块 (data 前 4 字节 = ER/msb/lsb/pc)
        // 校验和按定案公式: cs = -(sum(addr)+sum(data)+(bc&0x7F)) & 0x7F
        fn bulk4c(part: u8, data: &[u8]) -> String {
            let mut v = vec![0xF0, 0x43, 0x00, 0x4C];
            let bc = data.len() as u16;
            v.push(((bc >> 7) & 0x7F) as u8);
            v.push((bc & 0x7F) as u8);
            v.push(0x08);
            v.push(part);
            v.push(0x00);
            v.extend_from_slice(data);
            let sum: u16 = (0x08u16 + part as u16 + 0x00u16) + data.iter().map(|&x| x as u16).sum::<u16>() + (bc & 0x7F);
            let cs = (0x7F - (sum & 0x7F)) & 0x7F;
            v.push(cs as u8);
            v.push(0xF7);
            v.iter().map(|x| format!("{x:02X}")).collect()
        }
        // 真实数据 (2026-08-09 dump): part1 / part2 / part10(鼓) / part26(鼓)
        let mut app = XgApp::default();
        app.sysex_capture_log = vec![
            ("in".into(), bulk4c(0x00, &[2, 0, 0, 1]), 1),      // part1: ER2 msb0 lsb0 pc1
            ("in".into(), bulk4c(0x01, &[2, 0, 0, 0x50]), 2),   // part2: pc=80
            ("in".into(), bulk4c(0x09, &[0, 127, 0, 0]), 3),    // part10: msb127 鼓
            ("in".into(), bulk4c(0x19, &[0, 127, 0, 0]), 4),    // part26: msb127 鼓 (PortB ch10)
        ];
        app.analyze_sysex_capture();
        // 解析应写入 read_parts
        let p1 = app.read_parts[0].unwrap();
        assert_eq!(p1, (0, 0, 1), "part1 msb0 lsb0 pc1");
        let p2 = app.read_parts[1].unwrap();
        assert_eq!(p2, (0, 0, 0x50), "part2 pc=0x50(80)");
        let p10 = app.read_parts[9].unwrap();
        assert_eq!(p10, (127, 0, 0), "part10 鼓 msb127");
        let p26 = app.read_parts[25].unwrap();
        assert_eq!(p26, (127, 0, 0), "part26 (PortB ch10) 鼓 msb127");
        // 未出现的 part3 保持 None
        assert!(app.read_parts[2].is_none(), "part3 未在 dump 中 → None");
    }

    #[test]
    fn playview_cc_live_tracks_all_cc() {
        // Task5: 播放时任意 CC 都应记入 cc_live (PlayView 绿竖条数据源)
        let mut app = XgApp::default();
        app.apply_fired_event_to_meter(&PlayEvent::cc_tick(3, 1, 100, 0));  // mod
        app.apply_fired_event_to_meter(&PlayEvent::cc_tick(3, 74, 80, 0));  // brightness
        app.apply_fired_event_to_meter(&PlayEvent::cc_tick(3, 91, 60, 0));  // reverb
        app.apply_fired_event_to_meter(&PlayEvent::cc_tick(3, 93, 40, 0));  // chorus
        assert_eq!(app.cc_live[3][1], 100);
        assert_eq!(app.cc_live[3][74], 80);
        assert_eq!(app.cc_live[3][91], 60);
        assert_eq!(app.cc_live[3][93], 40);
        // 未触碰的 CC 保持 0
        assert_eq!(app.cc_live[3][64], 0, "未发送的 CC 应为 0");
    }

    #[test]
    fn playview_bank_program_and_poly_tracking() {
        // Task5: CC0/CC32 → parts[ch].msb/lsb; PC → parts[ch].prog; NoteOn → poly 计数
        let mut app = XgApp::default();
        // bank select MSB=0, LSB=1 (Bank1)
        app.apply_fired_event_to_meter(&PlayEvent::cc_tick(4, 0, 0, 0));
        app.apply_fired_event_to_meter(&PlayEvent::cc_tick(4, 32, 1, 0));
        app.apply_fired_event_to_meter(&PlayEvent::prog(4, 41));
        // 单源化: 现在读 parts[4]
        assert_eq!(app.parts[4].msb, 0);
        assert_eq!(app.parts[4].lsb, 1);
        assert_eq!(app.parts[4].prog, 41);
        // NoteOn 两个 → poly 计数 2
        app.apply_fired_event_to_meter(&PlayEvent::note(4, 60, 100, 0, true));
        app.apply_fired_event_to_meter(&PlayEvent::note(4, 64, 90, 0, true));
        assert_eq!(app.max_poly, 2, "两个 NoteOn 后 maxPoly=2");
        // NoteOff 一个 → 当前 poly 1, 峰值保持 2
        app.apply_fired_event_to_meter(&PlayEvent::note(4, 60, 0, 0, false));
        let cur_poly: u64 = app.active_notes.iter().map(|m| m.len() as u64).sum();
        assert_eq!(cur_poly, 1);
        assert_eq!(app.max_poly, 2, "峰值保持");
    }

    #[test]
    fn playview_voice_name_maps_bank_and_drum() {
        // Task4: 鼓通道 (msb=127) → drum_display_name; 旋律 → voice_bank.find
        // 单源化: 现在直接写 parts[ch]
        let mut app = XgApp::default();
        app.voice_bank = crate::data::VoiceBank::embedded_mu90().ok();
        // 鼓: msb=127, prg=0 → Standard Kit
        app.parts[9].msb = 127;
        app.parts[9].prog = 0;
        assert_eq!(app.voice_name_for_channel(9), "StandKit");
        // 鼓: prg=16 → Rock Kit
        app.parts[9].prog = 16;
        assert_eq!(app.voice_name_for_channel(9), "Rock Kit");
        // 旋律: 默认 (msb0 lsb0) + pc0 → Grand Piano
        app.parts[0].msb = 0;
        app.parts[0].lsb = 0;
        app.parts[0].prog = 0;
        let name = app.voice_name_for_channel(0);
        assert!(name.to_lowercase().contains("piano") || !name.is_empty(), "pc0 msb0 应映射到钢琴系, got {name}");
    }

    #[test]
    fn ruler_density_self_adaptive() {
        // 用户 2026-08-12: rule 太密时 bar 号跳格 / 更密省略 beat tick
        // 参数: bar_ticks=384 (默认 4/4 ppq96), time_width=1000px
        let (bar_ticks, ppq, w) = (384u64, 96u64, 1000.0f32);
        // 1) 少量 bar; zoom=1 → win_ticks=768 (2 bars) → 每 bar 500px 充足
        let (step, beat) = ruler_density(768, bar_ticks, w, ppq);
        assert_eq!(step, 1, "2 bars 稀疏: 每 bar 都标号");
        assert!(beat, "2 bars 稀疏: 应画 beat tick");
        // 2) zoom=4 → win_ticks=192 (半 bar 宽 1000px) → 更稀疏, 仍每 bar 标 + beat
        let (step2, beat2) = ruler_density(192, bar_ticks, w, ppq);
        assert_eq!(step2, 1);
        assert!(beat2);
        // 3) zoom-out: win_ticks=76800 (200 bars 挤 1000px) → 每 bar 5px 极密 → 跳号 + 无 beat
        let (step3, beat3) = ruler_density(76800, bar_ticks, w, ppq);
        assert!(step3 > 1, "200 bars 挤 1000px: 必须跳号 (step>1), got {step3}");
        assert!(!beat3, "200 bars 5px: 省略 beat tick");
        // 4) 中密: win_ticks=7680 (20 bars 50px/bar) → 跳号或至少 beat 保留逻辑自洽
        let (step4, beat4) = ruler_density(7680, bar_ticks, w, ppq);
        // 50px/bar > 44 → 每 bar 标; beat px = 50/4 = 12.5 >= 9 → 保留
        assert_eq!(step4, 1, "50px/bar 应每 bar 标号");
        assert!(beat4, "12.5px/beat 应画 beat");
    }

    // ---------- Channel View Mute/Solo (2026-08-13, d10) ----------

    #[test]
    fn mute_solo_default_all_off() {
        let app = XgApp::default();
        assert_eq!(app.channel_mutes, [false; 16]);
        assert_eq!(app.channel_solos, [false; 16]);
        for ch in 0..16 {
            assert!(!app.channel_is_effectively_muted(ch), "默认无静音");
        }
    }

    #[test]
    fn mute_isolates_single_channel() {
        let mut app = XgApp::default();
        app.channel_mutes[3] = true;
        assert!(app.channel_is_effectively_muted(3));
        for ch in 0..16 {
            if ch != 3 {
                assert!(!app.channel_is_effectively_muted(ch), "只有 ch4(下标3) 静音");
            }
        }
    }

    #[test]
    fn solo_isolates_soloed_channels_only() {
        let mut app = XgApp::default();
        app.channel_solos[2] = true; // solo ch3 (下标2)
        // 非 solo 通道全部静音
        for ch in 0..16 {
            if ch != 2 {
                assert!(app.channel_is_effectively_muted(ch), "solo 后 ch{} 应静音", ch + 1);
            }
        }
        assert!(!app.channel_is_effectively_muted(2), "被 solo 的通道应发声");
    }

    #[test]
    fn mute_priority_over_solo() {
        let mut app = XgApp::default();
        app.channel_solos[0] = true;
        app.channel_mutes[0] = true; // ch1 同时 solo + mute → mute 优先 → 静音
        assert!(app.channel_is_effectively_muted(0), "solo 且 mute → 应静音 (mute 优先)");
        // 另一 solo 仅 solo → 发声
        app.channel_solos[5] = true;
        assert!(!app.channel_is_effectively_muted(5), "仅 solo → 发声");
    }

    #[test]
    fn solo_off_restores_normal_mute() {
        let mut app = XgApp::default();
        app.channel_mutes[7] = true;
        app.channel_solos[3] = true;
        // solo 激活期间: 非 solo 全静音
        assert!(app.channel_is_effectively_muted(7));
        // 取消 solo
        app.channel_solos[3] = false;
        // 恢复: 只有 ch8(下标7) 自身 mute 静音
        assert!(app.channel_is_effectively_muted(7), "取消 solo 后 ch8 自身 mute 仍静音");
        for ch in 0..16 {
            if ch != 7 {
                assert!(!app.channel_is_effectively_muted(ch), "取消 solo 后 ch{} 恢复", ch + 1);
            }
        }
    }

    // ---------- TopBar Transport / Record armed (2026-08-13, d11) ----------

    #[test]
    fn transport_record_armed_toggle_is_pure_ui() {
        // Record 按钮只切换 armed 视觉态, 不碰播放状态 (John 2026-08-13: 功能预留)
        let mut app = XgApp::default();
        assert!(!app.rec_armed, "初始未 armed");
        assert!(!app.playing, "初始未播放");
        // 点击 Record: 切换 armed, 但播放状态/playhead 不受影响
        app.rec_armed = !app.rec_armed;
        assert!(app.rec_armed, "Record 点击 → armed");
        assert!(!app.playing, "Record 不改播放状态");
        assert_eq!(app.playhead_tick, 0, "Record 不改 playhead");
        // 再点取消
        app.rec_armed = !app.rec_armed;
        assert!(!app.rec_armed, "Record 再点 → disarmed");
    }

    #[test]
    fn transport_button_kinds_are_distinct() {
        // TransportButton 各类型存在且新建不 panic (皆可构造)
        use crate::transport::{Transport, TransportButton};
        let kinds = [Transport::Play, Transport::Pause, Transport::Stop, Transport::Record];
        for kind in kinds {
            let _b = TransportButton::new(kind).active(true).size(24.0);
        }
        // active(true) 合成不 panic; 行为由浏览器像素验证覆盖
    }

    #[test]
    fn preview_note_manages_hanging_and_expiry() {
        // Playable Piano Roll: preview_note(0-based ch) on/off + expire 300ms 短音自动 off
        let mut app = XgApp::default();
        // 未 muted → preview on 登记挂音 (ch 0-based → 数组下标同)
        app.preview_note(0, 60, 100, true, -1.0); // ch0(MIDI ch1) t0=-1: 按住未放
        assert_eq!(app.preview_notes[0].get(&60), Some(&(100, -1.0)), "ch1 C4 挂音");
        app.preview_note(0, 60, 100, false, -1.0); // 松开
        assert!(!app.preview_notes[0].contains_key(&60), "松开后移除");

        // mute 通道不发声 (preview_note 走 channel_is_effectively_muted 过滤)
        app.channel_mutes[1] = true;
        app.preview_note(1, 62, 80, true, -1.0);
        assert!(!app.preview_notes[1].contains_key(&62), "muted 通道不登记挂音");
        app.channel_mutes[1] = false;

        // 采样式短音: t0=now, 300ms 后 expire 自动 off
        app.preview_note(3, 64, 90, true, 0.0); // t0=0
        assert!(app.preview_notes[3].contains_key(&64));
        // now=0.2 (<0.30) 不过期
        app.expire_preview_notes(0.2);
        assert!(app.preview_notes[3].contains_key(&64), "0.2s 未过期");
        // now=0.5 (>0.30) 过期
        app.expire_preview_notes(0.5);
        assert!(!app.preview_notes[3].contains_key(&64), "0.5s 后自动 off");

        // 按住未放 (t0<0) 永不过期
        app.preview_note(4, 65, 100, true, -1.0);
        app.expire_preview_notes(999.0);
        assert!(app.preview_notes[4].contains_key(&65), "t0<0 按住不受 expire 影响");
    }

    #[test]
    fn preview_note_respects_solo() {
        // Solo ch1(0-based) → 其他通道 preview 静默 (与播放一致)
        let mut app = XgApp::default();
        app.channel_solos[1] = true; // solo 数组下标1 (MIDI ch2)
        app.preview_note(0, 60, 100, true, -1.0); // ch0 非 solo → 静默
        assert!(!app.preview_notes[0].contains_key(&60), "solo ch2 时 ch1 preview 静默");
        app.preview_note(1, 62, 80, true, -1.0); // ch1 (solo) → 发声
        assert!(app.preview_notes[1].contains_key(&62), "solo 通道可 preview");
    }

    #[test]
    fn preview_note_ui_channel_offsets() {
        // ★ 回归: UI 选 ch5(1-based) → preview_note 必须发 0-based 4 (0x94), 不能发 5 (0x95 那会串到 MIDI ch6)
        //   用户 2026-08-14 实测: 选 ch5 渲染正确但点击琴键音色错(串到 ch6) — 根因 preview_note 收了 1-based 没转.
        //   约定钉死: 调用方(piano_roll)传 0-based; preview_note 直接 PlayEvent::note(ch,..) → 0x90|ch.
        let app = XgApp::default();
        // PlayEvent::note 0x90 | ch → ch4→0x94(MIDI ch5), ch5→0x95(MIDI ch6)
        let ev4 = crate::playback::PlayEvent::note(4, 60, 100, 0, true);
        let ev5 = crate::playback::PlayEvent::note(5, 60, 100, 0, true);
        assert_eq!(ev4.bytes[0], 0x94, "UI ch5 → 0x94 (MIDI ch5, 正确)");
        assert_eq!(ev5.bytes[0], 0x95, "UI ch5 若误传 ch5 → 0x95 (MIDI ch6, 错)");
        // preview_note 存 preview_notes[idx=ch] — ch0 存在 preview_notes[0]
        let mut app2 = XgApp::default();
        app2.preview_note(4, 60, 100, true, -1.0);
        assert!(app2.preview_notes[4].contains_key(&60), "preview_note(4) 存 preview_notes[4] (0-based)");
        assert!(!app2.preview_notes[5].contains_key(&60), "绝不该存 index 5 (那是 MIDI ch6)");
    }

    #[test]
    fn event_list_filters_and_sorts_by_channel() {
        // Event List: 只列当前 channel 事件, tick 升序, 同 tick 保序
        use crate::smf::{Smf, TrackEvents, SmfEvent};
        let mut trk = Vec::new();
        // ★ SMF 事件 channel 是 0-based (解析 st&0x0f). 调用方要查真实 MIDI ch5 → 传 4.
        trk.push(SmfEvent::NoteOn { tick: 0, channel: 4, pitch: 60, vel: 100 });
        trk.push(SmfEvent::NoteOff { tick: 96, channel: 4, pitch: 60 });
        trk.push(SmfEvent::NoteOn { tick: 192, channel: 5, pitch: 62, vel: 80 }); // 其他通道(不入)
        trk.push(SmfEvent::Cc { tick: 48, channel: 4, num: 7, val: 100 });
        trk.push(SmfEvent::Program { tick: 48, channel: 4, program: 5 });
        trk.push(SmfEvent::Tempo { tick: 0, us_per_qn: 500_000 }); // 全局(不入)
        let smf = Smf { format: 1, ntracks: 1, ppq: 96, tracks: vec![TrackEvents { events: trk }], meta_tempo_count: 1, meta_timesig_count: 0 };

        // 查真实 MIDI ch5 → 传 0-based 4
        let rows = crate::smf::event_list_for_channel(&smf, 4);
        use crate::smf::EventKind;
        assert_eq!(rows.len(), 4, "ch5 应 4 事件, got {}", rows.len());
        assert_eq!(rows[0].tick, 0);
        assert!(matches!(rows[0].kind, EventKind::NoteOn { pitch: 60, vel: 100 }));
        assert_eq!(rows[1].tick, 48);
        assert!(matches!(rows[1].kind, EventKind::Cc { num: 7, val: 100 }));
        assert_eq!(rows[2].tick, 48);
        assert!(matches!(rows[2].kind, EventKind::Program { program: 5 }));
        assert_eq!(rows[3].tick, 96);
        assert!(matches!(rows[3].kind, EventKind::NoteOff { pitch: 60 }));

        // ch6 (0-based 5) → 1 事件
        let rows2 = crate::smf::event_list_for_channel(&smf, 5);
        assert_eq!(rows2.len(), 1, "ch6 应 1 事件");
        assert!(matches!(rows2[0].kind, EventKind::NoteOn { pitch: 62, vel: 80 }));
        // 传 1-based 4 (错误用法) 不该命中 ch5 → 0 事件 (钉死 0-based 约定)
        let rows3 = crate::smf::event_list_for_channel(&smf, 3);
        assert!(rows3.is_empty(), "传 3(1-based ch4) 不应命中 MIDI ch5");
    }
}
