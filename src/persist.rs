//! 面板状态持久化 — 统一接口
//! - wasm32: localStorage (浏览器, 存于站点域下)
//! - native: ~/.xg-editor-state.json (开发/测试用)
//! JSON 序列化逻辑与平台无关, 可 cargo test 验证往返。

const KEY: &str = "xg-editor-state-v1";

/// 保存 JSON 字符串到平台的持久存储
pub fn save_json(json: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = web_sys::window()
            .ok_or("no window")?
            .local_storage()
            .map_err(|e| format!("localStorage 访问失败: {e:?}"))?
            .ok_or("no localStorage")?;
        storage
            .set_item(KEY, json)
            .map_err(|e| format!("写入 localStorage 失败: {e:?}"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(".xg-editor-state.json"))
            .unwrap_or_else(|_| std::path::PathBuf::from(".xg-editor-state.json"));
        std::fs::write(&path, json).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
    }
}

/// 读取 JSON 字符串 (未找到返回 Ok(None))
pub fn load_json() -> Result<Option<String>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = web_sys::window()
            .ok_or("no window")?
            .local_storage()
            .map_err(|e| format!("localStorage 访问失败: {e:?}"))?
            .ok_or("no localStorage")?;
        let v = storage
            .get_item(KEY)
            .map_err(|e| format!("读取 localStorage 失败: {e:?}"))?;
        Ok(v)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(".xg-editor-state.json"))
            .unwrap_or_else(|_| std::path::PathBuf::from(".xg-editor-state.json"));
        match std::fs::read_to_string(&path) {
            Ok(s) => Ok(Some(s)),
            Err(_) => Ok(None), // 不存在 → None
        }
    }
}
