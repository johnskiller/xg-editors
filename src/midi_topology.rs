//! MIDI 拓扑数据结构 (John 2026-08-09: 先把数据结构做好, 多接口 id14/Tascam/蓝牙 + MU90 A/B 口映射).
//!
//! 现实拓扑:
//!   - 多个 MIDI 接口 (UX16, id14, Tascam Model 12, 蓝牙 MIDI), 每个接口有独立的 in/out 端口.
//!   - MU90 有两个物理 MIDI IN 口: Port A (parts 1-16) / Port B (parts 17-32),
//!     每个 port 内是 ch1-16 (RcvCh 默认 = part 号).
//!   - 当前: UX16 单 in/out, 只接了 Port A; Port B 未来用其他接口接.
//!   - 双向通信: 编辑→MU90 (out), MU90→编辑/读 part (in).
//!
//! 本模块只定义纯数据结构 + 路由查询, 不含 wasm/IO. 由 XgApp 持有并驱动。

/// MIDI 接口端口 (一个物理/逻辑 MIDI 端口 = Web MIDI 里的一个 input 或 output).
/// 可以同时有 in + out (如 UX16), 或只有其一 (如只发不发的).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiPort {
    /// 端口唯一名 (Web MIDI port.id 或 name; 用于绑定/查找)
    pub name: String,
    /// 该端口是否作输入 (收 MIDI: MU90 回传 SysEx / 未来 MIDI controller)
    pub is_input: bool,
    /// 该端口是否作输出 (发 MIDI: 编辑 SysEx / 播放事件)
    pub is_output: bool,
}

/// 端口在 MU90 拓扑里的角色.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiRole {
    /// Port A: MU90 物理 MIDI IN A → parts 1-16 (RcvCh 1-16)
    PortA,
    /// Port B: MU90 物理 MIDI IN B → parts 17-32 (RcvCh 1-16)
    PortB,
    /// 未分配 / 未来用途 (如 MIDI controller 输入)
    Unassigned,
}

/// 端口标识: 名字 (输入/输出分开列, 同一接口的 in/out 同名).
/// 路由目标是具体的 (name, direction), 这样 UX16 的 in 和 out 都能独立路由.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PortRef {
    pub name: String,
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    In,
    Out,
}

/// MU90 的 32 part 路由表: part 0-31 → 输出端口 (None = 不发送).
/// part 0-15 = Port A (parts 1-16), part 16-31 = Port B (parts 17-32).
/// 路由可被用户覆盖 (灵活方案 2: 通道→输出路由表).
#[derive(Debug, Clone)]
pub struct PartRouteTable {
    /// part index (0-31) → 输出端口引用
    pub routes: [Option<PortRef>; 32],
}

impl Default for PartRouteTable {
    fn default() -> Self {
        // [None; 32] 需要 PortRef: Copy, 但 PortRef 含 String → 用 array::from_fn 逐项 None
        Self {
            routes: std::array::from_fn(|_| None),
        }
    }
}

/// 完整的 MIDI 拓扑状态.
/// 持有:
///   - 所有可用的端口 (探测结果; wasm 探测到的)
///   - 每端口角色分配 (哪个是 Port A out / Port B out / 输入监听)
///   - part 路由表
#[derive(Debug, Clone, Default)]
pub struct MidiTopology {
    /// 所有已识别的端口 (name 唯一)
    pub ports: Vec<MidiPort>,
    /// 每个端口的角色 (key = port name)
    pub roles: Vec<(String, MidiRole)>,
    /// part 路由表 (32 路)
    pub part_routes: PartRouteTable,
    /// 已绑定监听的输入端口名 (onmidimessage 挂上的)
    pub bound_inputs: Vec<String>,
}

impl MidiTopology {
    /// 从 Web MIDI 探测结果构建拓扑:
    /// inputs: 输入端口名列表, outputs: 输出端口名列表.
    /// 同名端口合并为一个 MidiPort (is_input + is_output 同时标).
    pub fn from_probe(inputs: &[String], outputs: &[String]) -> Self {
        let mut ports: Vec<MidiPort> = Vec::new();
        for name in inputs {
            push_port(&mut ports, MidiPort {
                name: name.clone(),
                is_input: true,
                is_output: false,
            });
        }
        for name in outputs {
            push_port(&mut ports, MidiPort {
                name: name.clone(),
                is_input: false,
                is_output: true,
            });
        }
        Self {
            ports,
            ..Default::default()
        }
    }

    /// 输出端口名列表 (有 is_output)
    pub fn output_ports(&self) -> Vec<&MidiPort> {
        self.ports.iter().filter(|p| p.is_output).collect()
    }

    /// 输入端口名列表 (有 is_input)
    pub fn input_ports(&self) -> Vec<&MidiPort> {
        self.ports.iter().filter(|p| p.is_input).collect()
    }

    /// 按角色找输出端口 (如 PortA out 的端口名, PortB out 的端口名)
    pub fn output_for_role(&self, role: MidiRole) -> Option<String> {
        self.roles.iter()
            .find(|(_, r)| *r == role)
            .map(|(name, _)| name.clone())
    }

    /// 设置某端口角色
    pub fn set_role(&mut self, port_name: &str, role: MidiRole) {
        if let Some(slot) = self.roles.iter_mut().find(|(n, _)| n == port_name) {
            slot.1 = role;
        } else {
            self.roles.push((port_name.to_string(), role));
        }
    }

    /// 默认分配角色: 第一个输出端口 → PortA, 第二个输出端口 → PortB.
    /// 单接口 (UX16) 只有一个 out → PortA; PortB 留空.
    pub fn auto_assign_roles(&mut self) {
        let out_names: Vec<String> = self.output_ports().iter().map(|p| p.name.clone()).collect();
        if let Some(first) = out_names.first() {
            self.set_role(first, MidiRole::PortA);
        }
        if let Some(second) = out_names.get(1) {
            self.set_role(second, MidiRole::PortB);
        }
        // 重建 part 路由: PortA → parts 1-16, PortB → parts 17-32
        self.rebuild_default_routes();
    }

    /// 默认 part 路由: part 0-15 → PortA out, part 16-31 → PortB out.
    pub fn rebuild_default_routes(&mut self) {
        let a = self.output_for_role(MidiRole::PortA);
        let b = self.output_for_role(MidiRole::PortB);
        let mut routes: [Option<PortRef>; 32] = std::array::from_fn(|_| None);
        for part in 0..32 {
            let target = if part < 16 { &a } else { &b };
            routes[part] = target.as_ref().map(|name| PortRef {
                name: name.clone(),
                direction: Direction::Out,
            });
        }
        self.part_routes = PartRouteTable { routes };
    }

    /// 查询某个 part (0-31) 的目标输出端口名
    pub fn route_for_part(&self, part: u8) -> Option<&PortRef> {
        self.part_routes.routes.get(part as usize).and_then(|r| r.as_ref())
    }

    /// 某 part 是否被路由 (可发送)
    pub fn part_is_routed(&self, part: u8) -> bool {
        self.route_for_part(part).is_some()
    }

    /// 某端口是否已绑定输入监听
    pub fn is_input_bound(&self, name: &str) -> bool {
        self.bound_inputs.iter().any(|n| n == name)
    }
}

fn push_port(ports: &mut Vec<MidiPort>, new: MidiPort) {
    if let Some(existing) = ports.iter_mut().find(|p| p.name == new.name) {
        existing.is_input |= new.is_input;
        existing.is_output |= new.is_output;
    } else {
        ports.push(new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_probe_merges_same_name_port() {
        // UX16: 同名 in + out → 合并成一个 MidiPort (is_input + is_output)
        let topo = MidiTopology::from_probe(
            &["UX16 (UX16) [Port1]".into(), "Bluetooth MIDI".into()],
            &["UX16 (UX16) [Port1]".into()],
        );
        assert_eq!(topo.ports.len(), 2);
        let ux = topo.ports.iter().find(|p| p.name.starts_with("UX16")).unwrap();
        assert!(ux.is_input && ux.is_output, "同名 in/out 应合并");
        let bt = topo.ports.iter().find(|p| p.name.starts_with("Bluetooth")).unwrap();
        assert!(bt.is_input && !bt.is_output);
    }

    #[test]
    fn auto_assign_single_interface_port_a_only() {
        // 单 UX16 只有 1 个 out → PortA, PortB 留空; parts 1-16 路由, 17-32 不路由
        let mut topo = MidiTopology::from_probe(&[], &["UX16 (UX16) [Port1]".into()]);
        topo.auto_assign_roles();
        assert_eq!(topo.output_for_role(MidiRole::PortA).as_deref(), Some("UX16 (UX16) [Port1]"));
        assert_eq!(topo.output_for_role(MidiRole::PortB), None);
        assert!(topo.part_is_routed(0), "part1 应路由到 PortA");
        assert!(topo.part_is_routed(15), "part16 应路由到 PortA");
        assert!(!topo.part_is_routed(16), "part17 无 PortB 不应路由");
        assert!(!topo.part_is_routed(31), "part32 无 PortB 不应路由");
    }

    #[test]
    fn auto_assign_two_interfaces_ports_a_and_b() {
        // 两个 out → PortA + PortB; 32 part 全路由
        let mut topo = MidiTopology::from_probe(
            &[],
            &["UX16 (UX16) [Port1]".into(), "Tascam Model 12 MIDI".into()],
        );
        topo.auto_assign_roles();
        assert_eq!(topo.output_for_role(MidiRole::PortA).as_deref(), Some("UX16 (UX16) [Port1]"));
        assert_eq!(topo.output_for_role(MidiRole::PortB).as_deref(), Some("Tascam Model 12 MIDI"));
        assert!(topo.part_is_routed(16), "part17 应路由到 PortB");
        assert!(topo.part_is_routed(31), "part32 应路由到 PortB");
        let r16 = topo.route_for_part(16).unwrap();
        assert_eq!(r16.name, "Tascam Model 12 MIDI");
    }

    #[test]
    fn manual_role_override() {
        // 手动把 PortB 指定到蓝牙
        let mut topo = MidiTopology::from_probe(
            &[],
            &["UX16 (UX16) [Port1]".into(), "BT MIDI".into()],
        );
        topo.set_role("BT MIDI", MidiRole::PortB);
        topo.set_role("UX16 (UX16) [Port1]", MidiRole::PortA);
        topo.rebuild_default_routes();
        assert_eq!(topo.route_for_part(16).map(|r| r.name.as_str()), Some("BT MIDI"));
        assert_eq!(topo.route_for_part(0).map(|r| r.name.as_str()), Some("UX16 (UX16) [Port1]"));
    }

    #[test]
    fn bound_inputs_tracking() {
        let mut topo = MidiTopology::from_probe(&["UX16 (UX16) [Port1]".into()], &[]);
        assert!(!topo.is_input_bound("UX16 (UX16) [Port1]"));
        topo.bound_inputs.push("UX16 (UX16) [Port1]".into());
        assert!(topo.is_input_bound("UX16 (UX16) [Port1]"));
        assert_eq!(topo.input_ports().len(), 1);
    }

    #[test]
    fn part_route_override_single_channel() {
        // 用户手动覆盖: part17 从 PortB 改到 PortA (灵活路由表)
        let mut topo = MidiTopology::from_probe(
            &[],
            &["UX16 (UX16) [Port1]".into(), "Tascam Model 12 MIDI".into()],
        );
        topo.auto_assign_roles();
        topo.part_routes.routes[16] = Some(PortRef {
            name: "UX16 (UX16) [Port1]".into(),
            direction: Direction::Out,
        });
        assert_eq!(topo.route_for_part(16).map(|r| r.name.as_str()), Some("UX16 (UX16) [Port1]"));
    }
}
