//! Host-only observations. Never writes guest memory or advances guest time.
use std::collections::{BTreeMap, BTreeSet};

pub const TRACE_TAG: u64 = 0x4646_5441_4c4f_4732;
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("FFTA_OAM_LOG").as_deref() == Ok("1"))
}

#[derive(Default, Clone, Copy)]
pub struct Context { pub pc: u32, pub lr: u32, pub thumb: bool, pub args: [u32; 4] }
#[derive(Default)]
struct Sprites {
    calls: u64, received: u64, examined: u64, emitted: u64,
    reject_x: u64, reject_y: u64, negative_x: u64, negative_y: u64,
    reject_negative_x: u64, reject_negative_y: u64, limit_hits: u64,
}
#[derive(Default)]
struct Correlation { sum: f64, square: f64, cross_h: f64, cross_v: f64 }
#[derive(Clone)]
struct MapSnapshot { frame: u64, bg: usize, cnt: u16, h: u16, v: u16, tiles: Vec<u16> }
struct Extremes { min: MapSnapshot, max: MapSnapshot }

#[derive(Default)]
pub struct Diagnostics {
    pub observation_frame: u64,
    pub context: Context,
    pub native_pc: Option<u32>,
    pub dma_context: [Context; 4],
    pub dma_depth: usize,
    sprites: [Sprites; 2],
    limit_seen: [bool; 2],
    obj_cpu: u64, obj_dma: u64, obj_ignored8: u64,
    obj_min: Option<u32>, obj_max: u32,
    previous_ram: Vec<u16>, correlations: Vec<Correlation>,
    samples: u64, sum_h: f64, square_h: f64, sum_v: f64, square_v: f64,
    maps: BTreeMap<u16, Extremes>,
    producer_frame: Option<u64>, producer_writes: u64,
    producer_contexts: BTreeSet<(u32, u32, bool)>,
    shadow_frame: Option<u64>,
    shadow_contexts: BTreeSet<(u32, u32)>,
    native_shadow_frame: Option<u64>,
    native_shadow_contexts: BTreeSet<(u32, u32)>,
    movement_frame: Option<u64>,
    movement_contexts: BTreeSet<(u32,u32)>,
}

fn peek16(addr: u32, ewram: &[u8], iwram: &[u8], rom: &[u8]) -> Option<u16> {
    let (data, off) = match addr >> 24 {
        2 => (ewram, (addr & 0x3ffff) as usize),
        3 => (iwram, (addr & 0x7fff) as usize),
        8..=13 => (rom, (addr & 0x1ffffff) as usize),
        _ => return None,
    };
    let b = data.get(off..off + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

impl Diagnostics {
    pub fn native_write_hint(&mut self, frame: u64, addr: u32, regs: &[u32;16], cpsr: u32) {
        if frame < 960 || frame % 30 != 0 || !((0x03000030..0x03000830).contains(&addr) || (0x03002c58..0x03003270).contains(&addr) || addr >> 24 == 7) { return; }
        if self.native_shadow_frame != Some(frame) { self.native_shadow_frame=Some(frame); self.native_shadow_contexts.clear(); }
        if self.native_shadow_contexts.len() < 20 && self.native_shadow_contexts.insert((regs[15],regs[14])) {
            eprintln!("FFTA_NATIVE_SHADOW frame={frame} addr=0x{addr:08X} block_pc=0x{:08X} lr=0x{:08X} mode={} args={:X?} status=BLOCK_PC_NOT_STORE_PC",regs[15],regs[14],if cpsr&32!=0 {"Thumb"} else {"ARM"}, &regs[..4]);
        }
    }
    pub fn instruction(&mut self, pc: u32, regs: &[u32; 16], cpsr: u32, ewram: &[u8], iwram: &[u8], rom: &[u8]) {
        self.context = Context { pc, lr: regs[14], thumb: cpsr & 0x20 != 0, args: regs[..4].try_into().unwrap() };
        if pc==0x0801af14 && std::env::var_os("FFTA_MOVEMENT_DUMP_PREFIX").is_some() {
            let descriptor=(0..24).map(|i|peek16(regs[0]+i*2,ewram,iwram,rom)).collect::<Vec<_>>();
            let stack=(0..4).map(|i|peek16(regs[13]+0x4c+i*2,ewram,iwram,rom)).collect::<Vec<_>>();
            println!("FFTA_BG_WRITER frame={} descriptor={:08x} lr={:08x} args={:08x?} descriptor_halfwords={descriptor:04x?} original_stack={stack:04x?}",self.observation_frame,regs[0],regs[14],&regs[..4]);
        }
        if pc==0x0801d146 && std::env::var_os("FFTA_MOVEMENT_DUMP_PREFIX").is_some() {
            let heights=(0..2).map(|i|peek16(regs[9]+i*2,ewram,iwram,rom)).collect::<Vec<_>>();
            println!("FFTA_RANGE_CELL frame={} x={} y={} projected_x={} projected_y={} style={} heights={heights:04x?} lr={:08x}",self.observation_frame,regs[0] as u8,regs[1] as u8,regs[2] as u8,regs[3] as u8,regs[4] as u8,regs[14]);
        }
        let index = if (0x080014c8..0x080016c8).contains(&pc) { 0 }
            else if (0x080016d4..0x08001882).contains(&pc) { 1 } else { return; };
        let s = &mut self.sprites[index];
        let negative = cpsr >> 31 != 0;
        let overflow = cpsr & 0x10000000 != 0;
        let zero = cpsr & 0x40000000 != 0;
        let greater = !zero && negative == overflow;
        let less = negative != overflow;
        match pc {
            0x080014c8 | 0x080016d4 => {
                self.limit_seen[index] = false;
                s.calls += 1;
                if let Some(n) = peek16(regs[0], ewram, iwram, rom) { s.received += n as u64; }
            }
            0x08001518 | 0x08001722 => s.examined += 1,
            0x0800169a | 0x08001856 => s.emitted += 1,
            0x0800159c => s.reject_negative_x += 1,
            0x08001778 if less => s.reject_negative_x += 1,
            0x080015aa | 0x08001788 if greater => s.reject_x += 1,
            0x080015fe | 0x080017c2 if less => s.reject_negative_y += 1,
            0x0800160c | 0x080017d2 if greater => s.reject_y += 1,
            0x0800158e => s.negative_x += ((regs[1] as i32) < 0) as u64,
            0x0800176a => s.negative_x += ((regs[5] as i32) < 0) as u64,
            0x080015f2 => s.negative_y += ((regs[1] as i32) < 0) as u64,
            0x080017b2 => s.negative_y += ((regs[2] as i32) < 0) as u64,
            0x080014e8 | 0x08001720 => {
                if !self.limit_seen[index] { s.limit_hits += 1; self.limit_seen[index] = true; }
            }
            0x080016a4 if !less => {
                if !self.limit_seen[index] { s.limit_hits += 1; self.limit_seen[index] = true; }
            }
            0x08001870 => {
                if let Some(limit) = peek16(regs[13].wrapping_add(68), ewram, iwram, rom) {
                    if regs[12] >= limit as u32 && !self.limit_seen[index] {
                        s.limit_hits += 1; self.limit_seen[index] = true;
                    }
                }
            }
            _ => {}
        }
    }

    pub fn write(&mut self, frame: u64, addr: u32, width: u32, value: u32) {
        if self.dma_depth==0 && (0x02003cb0..0x02007cb0).contains(&addr)
            && std::env::var_os("FFTA_MOVEMENT_DUMP_PREFIX").is_some() {
            if self.movement_frame!=Some(frame) {self.movement_frame=Some(frame);self.movement_contexts.clear();}
            if self.movement_contexts.len()<32 && self.movement_contexts.insert((self.context.pc,self.context.lr)) {
                println!("FFTA_MOVEMENT_RAM_WRITE frame={frame} addr={addr:08x} value={value:08x} width={width} pc={:08x} lr={:08x} args={:08x?}",self.context.pc,self.context.lr,self.context.args);
            }
        }
        if frame >= 960 && frame % 30 == 0 && self.dma_depth == 0
            && ((0x03000030..0x03000830).contains(&addr) || (0x03002c58..0x03003270).contains(&addr) || addr >> 24 == 7) {
            if self.shadow_frame != Some(frame) { self.shadow_frame = Some(frame); self.shadow_contexts.clear(); }
            if self.shadow_contexts.len() < 20 && self.shadow_contexts.insert((self.context.pc,self.context.lr)) {
                eprintln!("FFTA_SHADOW_WRITE frame={frame} addr=0x{addr:08X} width={width} value=0x{value:08X} pc=0x{:08X} lr=0x{:08X} mode={} args={:X?}", self.context.pc, self.context.lr, if self.context.thumb { "Thumb" } else { "ARM" }, self.context.args);
            }
        }
        if (0x06010000..0x06018000).contains(&addr) {
            if width == 1 { self.obj_ignored8 += 1; }
            else {
                if self.dma_depth == 0 { self.obj_cpu += 1; } else { self.obj_dma += 1; }
                self.obj_min = Some(self.obj_min.map_or(addr, |a| a.min(addr)));
                self.obj_max = self.obj_max.max(addr + width - 1);
            }
        }
        if addr < 0x0200fbde && addr.saturating_add(width) > 0x0200f3de {
            self.producer_writes += 1;
            if self.producer_frame != Some(frame) {
                self.producer_frame = Some(frame);
                self.producer_contexts.clear();
            }
            if self.producer_contexts.insert((self.context.pc,self.context.lr,self.dma_depth != 0)) {
                eprintln!("FFTA_F3DE_WRITE frame={frame} addr=0x{addr:08X} width={width} value=0x{value:08X} pc=0x{:08X} lr=0x{:08X} mode={} dma={}", self.context.pc, self.context.lr, if self.context.thumb { "Thumb" } else { "ARM" }, self.dma_depth != 0);
            }
        }
    }

    pub fn sample(&mut self, frame: u64, io: &[u8], ewram: &[u8], iwram: &[u8], vram: &[u8]) {
        for (i, s) in self.sprites.iter_mut().enumerate() {
            let pc = if i == 0 { 0x080014c8 } else { 0x080016d4 };
            eprintln!("FFTA_SPRITE_COUNTS frame={frame} pc=0x{pc:08X} interval=30 calls={} received={} examined={} emitted={} reject_x_gt239={} reject_y_gt159={} negative_x_evaluated={} negative_y_evaluated={} reject_negative_x={} reject_negative_y={} limit_hits={}", s.calls, s.received, s.examined, s.emitted, s.reject_x, s.reject_y, s.negative_x, s.negative_y, s.reject_negative_x, s.reject_negative_y, s.limit_hits);
            *s = Sprites::default();
        }
        eprintln!("FFTA_OBJ_WRITES frame={frame} cpu_calls={} dma_calls={} ignored8_calls={} first={:?} last=0x{:08X} f3de_write_calls={}", self.obj_cpu, self.obj_dma, self.obj_ignored8, self.obj_min, self.obj_max, self.producer_writes);
        self.obj_cpu = 0; self.obj_dma = 0; self.obj_ignored8 = 0;
        self.obj_min = None; self.obj_max = 0; self.producer_writes = 0;
        let word = |off: usize| u16::from_le_bytes([io[off], io[off + 1]]);
        let dispcnt = word(0);
        // These are candidates identified in capture 1, not a semantic terrain identification.
        let terrain = (0..4).filter_map(|bg| {
            let cnt = word(8 + bg * 2);
            if dispcnt & 7 != 0 || dispcnt & (1 << (8 + bg)) == 0 || !matches!(cnt, 0x4c03 | 0x4e02) { return None; }
            let h = word(0x10 + bg * 4) & 511;
            let v = word(0x12 + bg * 4) & 511;
            Some((bg, cnt, h, v))
        }).collect::<Vec<_>>();
        for &(bg, cnt, h, v) in &terrain {
            let base = ((cnt >> 8) & 31) as usize * 0x800;
            let snapshot = || MapSnapshot {
                frame, bg, cnt, h, v,
                tiles: (0..32).flat_map(|y| (0..64).map(move |x| {
                    let off = base + ((x / 32) * 1024 + y * 32 + x % 32) * 2;
                    u16::from_le_bytes([vram[off], vram[off + 1]])
                })).collect(),
            };
            let entry = self.maps.entry(cnt).or_insert_with(|| { let s = snapshot(); Extremes { min: s.clone(), max: s } });
            if h < entry.min.h { entry.min = snapshot(); }
            if h > entry.max.h { entry.max = snapshot(); }
        }
        let Some(&(_, _, h, v)) = terrain.first() else { return; };
        let ram = ewram.chunks_exact(2).chain(iwram.chunks_exact(2))
            .map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>();
        if self.correlations.is_empty() {
            self.correlations = (0..ram.len()).map(|_| Correlation::default()).collect();
        }
        self.samples += 1;
        self.sum_h += h as f64; self.square_h += (h as f64).powi(2);
        self.sum_v += v as f64; self.square_v += (v as f64).powi(2);
        let changed = ram.iter().zip(&self.previous_ram).filter(|(a,b)| a != b).count();
        for (c, &value) in self.correlations.iter_mut().zip(&ram) {
            // Signed halfwords; other representations require a separate analysis.
            let value = value as i16 as f64;
            c.sum += value; c.square += value * value;
            c.cross_h += value * h as f64; c.cross_v += value * v as f64;
        }
        eprintln!("FFTA_RAM_SUMMARY frame={frame} changed_halfwords={changed} samples={} hofs={h} vofs={v} encoding=i16 correlation=Pearson_raw_registers status=SUPOSICION", self.samples);
        self.previous_ram = ram;
        if self.samples < 8 { return; }
        for (axis, sum, square) in [("HOFS", self.sum_h, self.square_h), ("VOFS", self.sum_v, self.square_v)] {
            let n = self.samples as f64;
            let variance = n * square - sum * sum;
            if variance <= 0.0 { continue; }
            let mut candidates = self.correlations.iter().enumerate().filter_map(|(i, c)| {
                let vy = n * c.square - c.sum * c.sum;
                if vy <= 0.0 { return None; }
                let cross = if axis == "HOFS" { c.cross_h } else { c.cross_v };
                let r = (n * cross - c.sum * sum) / (variance * vy).sqrt();
                (r.abs() >= 0.9).then_some((i, r))
            }).collect::<Vec<_>>();
            candidates.sort_by(|a,b| b.1.abs().total_cmp(&a.1.abs()).then(a.0.cmp(&b.0)));
            for &(i, r) in candidates.iter().take(12) {
                let addr = if i < 0x20000 { 0x02000000 + i * 2 } else { 0x03000000 + (i - 0x20000) * 2 };
                eprintln!("FFTA_CAMERA_CANDIDATE frame={frame} axis={axis} addr=0x{addr:08X} value={} r={r:.6} samples={} status=SUPOSICION", self.previous_ram[i] as i16, self.samples);
            }
        }
    }

    pub fn dump_maps(&mut self) {
        for (_, extremes) in std::mem::take(&mut self.maps) {
            for (endpoint, s) in [("min", extremes.min), ("max", extremes.max)] {
                let first = s.h as usize / 8;
                let last = (s.h as usize + 239) / 8;
                eprintln!("FFTA_MAP_SNAPSHOT endpoint={endpoint} frame={} bg={} cnt=0x{:04X} hofs={} vofs={} width_tiles=64 height_tiles=32 visible_cols={}..{} visible_rows={}..{} status=CONTENTS_ONLY", s.frame,s.bg,s.cnt,s.h,s.v,first,last,s.v/8,(s.v+159)/8);
                for x in 0..64 {
                    let zone = if (first..=last).contains(&x) { "visible" }
                        else if x == (first + 63) % 64 { "adjacent_left" }
                        else if x == (last + 1) % 64 { "adjacent_right" } else { "outside" };
                    let cells = (0..32).map(|y| format!("{:04X}",s.tiles[y*64+x])).collect::<Vec<_>>().join(",");
                    eprintln!("FFTA_MAP_COLUMN endpoint={endpoint} cnt=0x{:04X} column={x} zone={zone} cells={cells}",s.cnt);
                }
            }
        }
    }
}

impl Drop for Diagnostics {
    fn drop(&mut self) { self.dump_maps(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observes_actual_branch_conditions_and_emissions_without_guest_writes() {
        let mut d = Diagnostics::default();
        let mut ram = vec![0; 0x100];
        ram[0] = 4;
        let original = ram.clone();
        let mut regs = [0u32; 16];
        regs[0] = 0x02000000;
        d.instruction(0x080014c8, &regs, 0, &ram, &[], &[]);
        d.instruction(0x08001518, &regs, 0, &ram, &[], &[]);
        d.instruction(0x080015aa, &regs, 0x40000000, &ram, &[], &[]); // EQ: do not reject
        d.instruction(0x080015aa, &regs, 0, &ram, &[], &[]); // GT
        d.instruction(0x0800160c, &regs, 0, &ram, &[], &[]); // GT
        d.instruction(0x080015fe, &regs, 0x80000000, &ram, &[], &[]); // LT
        d.instruction(0x0800159c, &regs, 0, &ram, &[], &[]);
        d.instruction(0x0800169a, &regs, 0, &ram, &[], &[]);
        d.instruction(0x080016a4, &regs, 0, &ram, &[], &[]);
        d.instruction(0x080016a4, &regs, 0, &ram, &[], &[]);
        let s = &d.sprites[0];
        assert_eq!((s.received,s.examined,s.emitted,s.reject_x,s.reject_y), (4,1,1,1,1));
        assert_eq!((s.reject_negative_x,s.reject_negative_y,s.limit_hits), (1,1,1));
        assert_eq!(ram, original);
    }

    #[test]
    fn retains_global_extremes_and_decodes_both_screenblocks() {
        let mut d = Diagnostics::default();
        let mut io = vec![0; 0x400];
        io[1] = 1; // BG0 on, text mode 0
        io[8..10].copy_from_slice(&0x4c03u16.to_le_bytes());
        let mut vram = vec![0; 0x18000];
        for i in 0..2048 { vram[0x6000+i*2..0x6002+i*2].copy_from_slice(&(i as u16).to_le_bytes()); }
        let original = vram.clone();
        for (frame,h) in [(30,105u16),(60,17),(90,264),(120,100)] {
            io[0x10..0x12].copy_from_slice(&h.to_le_bytes());
            d.sample(frame,&io,&[0,0],&[],&vram);
        }
        let e = d.maps.get(&0x4c03).unwrap();
        assert_eq!((e.min.h,e.max.h,e.min.frame,e.max.frame), (17,264,60,90));
        assert_eq!(e.min.tiles[63],1055);
        assert_eq!(e.min.tiles[31*64+63],2047);
        assert_eq!(vram,original);
        d.maps.clear(); // Avoid printing snapshot contents from this synthetic test.
    }
}
