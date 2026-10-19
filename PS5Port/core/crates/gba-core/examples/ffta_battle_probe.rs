//! Private diagnostic calls original Thumb routines on copied battle snapshots.
//! Never used by the runtime; never writes a ROM or save file.
use gba_core::{ffta_rom, Machine};
fn call(m: &mut Machine, pc: u32, args: &[u32]) -> Result<u32, String> {
    m.bus.io[0x208..0x20a].fill(0);
    m.bus.halted = false;
    m.cpu.cpsr = 0xbf;
    m.cpu.branch = None;
    m.cpu.regs = [0; 16];
    m.cpu.regs[13] = 0x03007c00;
    m.cpu.regs[14] = 0x03007ff1;
    m.cpu.regs[15] = pc;
    for (i, &a) in args.iter().enumerate() {
        if i < 4 {
            m.cpu.regs[i] = a;
        } else {
            let o = 0x7c00 + (i - 4) * 4;
            m.bus.iwram[o..o + 4].copy_from_slice(&a.to_le_bytes());
        }
    }
    for _ in 0..100_000 {
        if m.cpu.regs[15] == 0x03007ff0 {
            return Ok(m.cpu.regs[0]);
        }
        m.step();
    }
    Err(format!("call did not return pc={:08x}", m.cpu.regs[15]))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<_> = std::env::args().collect();
    let rom = std::fs::read(a.get(1).ok_or("ROM")?)?;
    let version = ffta_rom::version(&rom).ok_or("unsupported ROM")?;
    let prefix = a.get(2).ok_or("private snapshot prefix")?;
    let mut m = Machine::new(rom);
    for (name, data) in [
        ("ewram", &mut m.bus.ewram),
        ("iwram", &mut m.bus.iwram),
        ("io", &mut m.bus.io),
        ("vram", &mut m.bus.vram),
        ("oam", &mut m.bus.oam),
        ("palette", &mut m.bus.palette),
    ] {
        data.copy_from_slice(&std::fs::read(format!("{prefix}-{name}.bin"))?);
    }
    if a.get(3).map(String::as_str)==Some("--detect-list") {
        m.bus.ffta_battle_mouse.enable_replay();
        let scene=gba_core::ffta_battle_mouse::MenuScene::read(&m.bus).ok_or("list not detected")?;
        println!("LIVE_LIST {scene:?}");return Ok(());
    }
    if a.get(3).map(String::as_str)==Some("--world-target") {
        m.bus.ffta_battle_mouse.enable_replay();
        assert!(gba_core::ffta_battle_mouse::world_select(&mut m.bus,190.,90.,true));
        assert_eq!(gba_core::ffta_battle_mouse::keys(&mut m.bus,0x3ff),0x36f);
        m.bus.iwram[..2].copy_from_slice(&0x90u16.to_le_bytes());
        call(&mut m,if version==ffta_rom::Version::Europe{0x08038994}else{0x08037428},&[])?;
        let x=u16::from_le_bytes(m.bus.ewram[0x2c16..0x2c18].try_into()?);
        let y=u16::from_le_bytes(m.bus.ewram[0x2c18..0x2c1a].try_into()?);
        let scroll=u16::from_le_bytes(m.bus.ewram[0x2c1c..0x2c1e].try_into()?);
        assert_eq!((x,y),(190,90+scroll));
        println!("WORLD_TARGET original_stores=true world={x},{y} screen=190,90 scroll={scroll}");
        return Ok(());
    }
    if a.get(3).map(String::as_str)==Some("--window-list") {
        m.bus.ffta_battle_mouse.enable_replay();
        let scene=gba_core::ffta_battle_mouse::MenuScene::read(&m.bus).ok_or("window list not detected")?;
        println!("WINDOW_LIST before={scene:?}");
        assert!(gba_core::ffta_battle_mouse::menu_select(&mut m.bus,1,true));
        assert_eq!(gba_core::ffta_battle_mouse::keys(&mut m.bus,0x3ff),0x37f);
        call(&mut m,if version==ffta_rom::Version::Europe {0x080395b4}else{0x08037df0},&[0,1])?;
        assert_eq!(m.bus.ewram[0xfd2e],1);
        println!("WINDOW_LIST original_requested_row=1; drawing updates on owning menu tick");
        return Ok(());
    }
    if a.get(3).map(String::as_str)==Some("--confirm") {
        m.bus.ffta_battle_mouse.enable_replay();
        let scene=gba_core::ffta_battle_mouse::MenuScene::read(&m.bus).ok_or("confirmation not detected")?;
        println!("CONFIRM before={scene:?}");
        assert_eq!(scene.count,2);
        assert!(gba_core::ffta_battle_mouse::aim(&mut m.bus,scene.origin[0]as f32+30.,scene.origin[1]as f32+20.,true));
        let key=gba_core::ffta_battle_mouse::keys(&mut m.bus,0x3ff);
        assert_eq!(key,0x37f);
        m.bus.iwram[2..4].copy_from_slice(&0u16.to_le_bytes());
        m.bus.iwram[4..6].copy_from_slice(&0x80u16.to_le_bytes());
        let base=0x0202dd70;
        call(&mut m,if version==ffta_rom::Version::Usa{0x08029350}else{0x0802a58c},&[base])?;
        let scene=gba_core::ffta_battle_mouse::MenuScene::read(&m.bus);
        println!("CONFIRM original_selected={}",m.bus.ewram[0x2ddd9]);
        assert_eq!(m.bus.ewram[0x2ddd9],1);
        // The original draw publishes OAM on its next update.
        let _=scene;
        println!("CONFIRM original handler toggled Cancel successfully");
        return Ok(());
    }
    if a.get(3).map(String::as_str)==Some("--menu-target") {
        let pc=u32::from_str_radix(a.get(4).ok_or("menu function")?,16)?;
        let base=u32::from_str_radix(a.get(5).ok_or("menu instance")?,16)?;
        let target:u8=a.get(6).ok_or("target row")?.parse()?;
        m.bus.ffta_battle_mouse.enable_replay();
        println!("CONTROLLED_MENU before={:?}",gba_core::ffta_battle_mouse::MenuScene::read(&m.bus));
        if !gba_core::ffta_battle_mouse::menu_select(&mut m.bus,target,false) { return Err("menu request rejected".into()); }
        call(&mut m,pc,&[base,0x80])?;
        let actual=m.bus.ewram[(base-0x02000000)as usize+0x69];
        println!("CONTROLLED_MENU expected={target} actual={actual}");
        if actual!=target {return Err("menu commit mismatch".into());}
        return Ok(());
    }
    if a.get(3).map(String::as_str)==Some("--hover") {
        let x:u8=a.get(4).ok_or("x")?.parse()?;let y:u8=a.get(5).ok_or("y")?.parse()?;
        m.bus.ffta_battle_mouse.enable_replay();
        println!("CONTROLLED_CURSOR original_mode={}",m.bus.ewram[0xf3b0]);
        m.bus.ewram[0xf3b0]=1; // Private isolated function test, not gameplay evidence.
        m.bus.iwram[2..4].copy_from_slice(&0x10u16.to_le_bytes());
        if !gba_core::ffta_battle_mouse::select(&mut m.bus,x,y,false){return Err("target unavailable".into());}
        let pc=if version==ffta_rom::Version::Europe{0x08025cec}else{0x08024ad8};
        call(&mut m,pc,&[0x0200f3ac])?;
        let s=gba_core::ffta_battle_mouse::Scene::read(&m.bus).ok_or("lost scene")?;
        println!("CONTROLLED_HOVER expected={x},{y} actual={},{}",s.selected.0,s.selected.1);
        if s.selected!=(x,y){return Err("hover mismatch".into());}
        return Ok(());
    }
    if let Some(pc) = a.get(3) {
        let pc = u32::from_str_radix(pc.trim_start_matches("0x"), 16)?;
        let args = a[4..]
            .iter()
            .map(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16))
            .collect::<Result<Vec<_>, _>>()?;
        let before = m.bus.ewram.clone();
        let result = call(&mut m, pc, &args)?;
        println!(
            "BATTLE_CALL pc={pc:08x} result={result:08x} output={:02x?}",
            &m.bus.iwram[0x7000..0x7010]
        );
        for (i, (&x, &y)) in before.iter().zip(m.bus.ewram.iter()).enumerate() {
            if x != y {
                println!("WRITE {:08x} {x:02x}->{y:02x}", 0x02000000 + i);
            }
        }
    } else {
        let shift = if version == ffta_rom::Version::Europe {
            0x1170
        } else {
            0
        };
        let width = m.bus.ewram[0x7f18];
        let height = m.bus.ewram[0x7f19];
        println!("BATTLE_MAP width={width} height={height} region={version:?}");
        let mut counts = [0usize; 4];
        let mut valid = 0;
        let mut range_mismatches = 0;
        m.bus.ffta_battle_mouse.enable_replay();
        let scene = gba_core::ffta_battle_mouse::Scene::read(&m.bus);
        println!(
            "BATTLE_SCENE {:?}",
            scene
                .as_ref()
                .map(|s| (s.width, s.height, s.selected, s.cells.len()))
        );
        let mut projection_mismatches = 0;
        if let Some(scene) = &scene {
            for c in &scene.cells {
                call(
                    &mut m,
                    0x0801c918 + shift,
                    &[
                        c.x as u32 * 32 + 16,
                        c.height as u32 * 16,
                        c.y as u32 * 32 + 16,
                        0x03007000,
                        0x03007002,
                        0x03007004,
                    ],
                )?;
                let out = |o| i16::from_le_bytes([m.bus.iwram[o], m.bus.iwram[o + 1]]) as i32;
                let cam = |o| i16::from_le_bytes([m.bus.ewram[o], m.bus.ewram[o + 1]]) as i32;
                let original = [
                    (out(0x7000) - cam(0x7f64)) as f32,
                    (out(0x7002) - cam(0x7f66)) as f32,
                ];
                projection_mismatches += usize::from(original != c.center);
            }
        }
        println!("BATTLE_PROJECTION mismatches={projection_mismatches}");
        for y in 0..height {
            for x in 0..width {
                let exists = call(&mut m, 0x0801cc7c + shift, &[x as u32, y as u32])?;
                let value = call(&mut m, 0x0801d58c + shift, &[x as u32, y as u32])?;
                valid += usize::from(exists != 0);
                range_mismatches += usize::from(
                    value != gba_core::ffta_battle_mouse::range_value(&m.bus, x, y) as u32,
                );
                if value < 4 {
                    counts[value as usize] += 1;
                }
                if value != 0 {
                    println!("BATTLE_CELL x={x} y={y} exists={exists} range={value}");
                }
            }
        }
        println!("BATTLE_SUMMARY existing={valid} range_counts={counts:?} range_mismatches={range_mismatches}");
    }
    Ok(())
}
