//! Private menu investigation. Only explicitly supplied save copies are read;
//! no save files or ROM bytes are written. Runtime snapshots belong in target/.
use gba_core::{ffta_rom, mem, Machine};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let opt = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].as_str());
    let rom = std::fs::read(args.get(1).ok_or("ROM path required")?)?;
    if ffta_rom::version(&rom).is_none() {
        return Err("exact supported ROM required".into());
    }
    if let Some(address) = opt("--disasm") {
        let start = u32::from_str_radix(address.trim_start_matches("0x"), 16)?;
        let length: u32 = opt("--bytes").unwrap_or("128").parse()?;
        for a in (start..start + length).step_by(2) {
            let o = (a - 0x08000000) as usize;
            let h = u16::from_le_bytes([rom[o], rom[o + 1]]);
            println!("{a:08x} {}", armv4t::decode_thumb(h, a).disasm());
        }
        return Ok(());
    }
    let prefix = opt("--prefix").ok_or("private output prefix required")?;
    let frames: Vec<u64> = opt("--frames")
        .ok_or("frames required")?
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()?;
    let mut input = Vec::new();
    let mut mouse_events = Vec::new();
    let mut click_events = Vec::new();
    let mut battle_click_events = Vec::new();
    let mut pointer_events = Vec::new();
    if let Some(path) = opt("--input") {
        let text = std::fs::read_to_string(path)?;
        for line in text.lines().skip(1).filter(|s| !s.trim().is_empty()) {
            let (f, k) = line.split_once(' ').ok_or("bad input line")?;
            if k.starts_with("point:") || k.starts_with("point-click:") {
                let confirm=k.starts_with("point-click:");
                let point=k.split_once(':').unwrap().1;
                let(x,y)=point.split_once(',').ok_or("bad pointer coordinate")?;
                pointer_events.push((f.parse::<u64>()?,x.parse::<f32>()?,y.parse::<f32>()?,confirm));
            } else if let Some(index) = k.strip_prefix("click:") {
                click_events.push((f.parse::<u64>()?, index.parse::<u16>()?));
            } else if let Some(cell) = k.strip_prefix("battle-click:") {
                let (x, y) = cell.split_once(',').ok_or("bad battle-click cell")?;
                battle_click_events.push((f.parse::<u64>()?, x.parse::<u8>()?, y.parse::<u8>()?));
            } else if let Some(index) = k.strip_prefix("mouse:") {
                mouse_events.push((f.parse::<u64>()?, index.parse::<u16>()?));
            } else {
                input.push((f.parse::<u64>()?, u16::from_str_radix(k, 16)?));
            }
        }
    }
    let probe_extra = std::env::var("FFTA_PROBE_EXTRA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let probe_extra_y = std::env::var("FFTA_PROBE_EXTRA_Y")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    mem::set_extra(probe_extra);
    mem::set_extra_y(probe_extra_y);
    let mut m = Machine::new(rom);
    // Match the desktop runner, which extracts action and stats panels into
    // the independent native UI layer before hit testing.
    m.bus.ffta_dialogue.separate = true;
    if let Some(path) = opt("--sav") {
        m.bus.load_save_data(&std::fs::read(path)?);
    }
    let mut mouse = gba_core::ffta_mouse::LanguageMouse::new(&m.bus.rom, true);
    let mut recorded = String::from("gba-input v1\n0 3ff\n");
    let mut last_keys = 0x3ff;
    let watch = opt("--watch")
        .map(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16))
        .transpose()?;
    let trace_from: u64 = opt("--trace-from").unwrap_or("0").parse()?;
    let trace_to: u64 = opt("--trace-to").unwrap_or("0").parse()?;
    let end = *frames.iter().max().ok_or("no frame")?;
    let pub_trace=std::env::var("FFTA_PUB_TRACE").as_deref()==Ok("1");
    while m.bus.frames < end {
        let f = m.bus.frames;
        m.bus.keys = input
            .iter()
            .rev()
            .find(|(frame, _)| *frame <= f)
            .map_or(0x3ff, |(_, k)| *k);
        for &(_,x,y,confirm) in pointer_events.iter().filter(|(frame,_,_,_)|*frame==f) {
            m.bus.ffta_battle_mouse.enable_replay();
            let handled=gba_core::ffta_battle_mouse::aim(&mut m.bus,x,y,confirm);
            println!("POINTER frame={f} x={x} y={y} confirm={confirm} handled={handled}");
            if !handled {return Err("pointer scene mismatch".into());}
        }
        for &(_, index) in mouse_events.iter().filter(|(frame, _)| *frame == f) {
            if index>=4096 {
                m.bus.ffta_battle_mouse.enable_replay();let point=index-4096;
                if !gba_core::ffta_battle_mouse::world_select(&mut m.bus,(point%240)as f32,(point/240)as f32,false){return Err("world scene mismatch".into());}
            }else if (1024..1030).contains(&index) {
                m.bus.ffta_battle_mouse.enable_replay();
                if !gba_core::ffta_shop_mouse::select_category(&mut m.bus,(index-1024)as u8){return Err(format!("no shop category at {f}").into());}
            }else if index >= 512 {
                m.bus.ffta_battle_mouse.enable_replay();
                if !gba_core::ffta_battle_mouse::menu_select(&mut m.bus,(index-512)as u8,false) { return Err(format!("no menu row at {f}").into()); }
                println!("BATTLE_MENU_REQUEST frame={f} target={}",index-512);
            } else if index >= 256 {
                m.bus.ffta_battle_mouse.enable_replay();
                let cell = index - 256;
                if !gba_core::ffta_battle_mouse::select(
                    &mut m.bus,
                    (cell % 16) as u8,
                    (cell / 16) as u8,
                    false,
                ) {
                    return Err(format!("no battle cell at {f}").into());
                }
                println!("BATTLE_REQUEST frame={f} x={} y={}", cell % 16, cell / 16);
            } else {
                let before = mouse.scene(&m.bus).ok_or("no mouse scene")?;
                if !mouse.select(&mut m.bus, index as u8) {
                    return Err("invalid mouse target".into());
                }
                println!(
                    "MENU_MOUSE frame={f} target={index} selected_before={} selected_after={}",
                    before.selected,
                    mouse.scene(&m.bus).unwrap().selected
                );
            }
        }
        for &(_, index) in click_events.iter().filter(|(frame, _)| *frame == f) {
            if index>=4096 {
                m.bus.ffta_battle_mouse.enable_replay();let point=index-4096;
                if !gba_core::ffta_battle_mouse::world_select(&mut m.bus,(point%240)as f32,(point/240)as f32,true){return Err("world click mismatch".into());}
                continue;
            }
            if (1024..1030).contains(&index) {
                m.bus.ffta_battle_mouse.enable_replay();
                if !gba_core::ffta_shop_mouse::select_category(&mut m.bus,(index-1024)as u8){return Err(format!("no shop category click at {f}").into());}
                continue;
            }
            if index >= 512 {
                m.bus.ffta_battle_mouse.enable_replay();
                if !gba_core::ffta_battle_mouse::menu_select(&mut m.bus,(index-512)as u8,true) { return Err(format!("no menu click at {f}").into()); }
                println!("BATTLE_MENU_CLICK frame={f} target={}",index-512);
                continue;
            }
            let scene = mouse.scene(&m.bus).ok_or("no click scene")?;
            let t = scene
                .targets
                .iter()
                .find(|t| t.index as u16 == index)
                .ok_or("no click target")?;
            if !mouse.aim(
                &m.bus,
                (t.bounds[0] + t.bounds[2]) * 0.5,
                (t.bounds[1] + t.bounds[3]) * 0.5,
                true,
            ) {
                return Err("click rejected".into());
            }
            println!("MENU_CLICK frame={f} target={index}");
        }
        for &(_, x, y) in battle_click_events.iter().filter(|(frame, _, _)| *frame == f) {
            m.bus.ffta_battle_mouse.enable_replay();
            if !gba_core::ffta_battle_mouse::select(&mut m.bus, x, y, true) {
                return Err(format!("no clickable battle cell {x},{y} at {f}").into());
            }
            println!("BATTLE_CLICK frame={f} x={x} y={y}");
        }
        let physical = m.bus.keys;
        m.bus.keys = mouse.keys(&mut m.bus, physical);
        gba_core::ffta_battle_mouse::replay_keys(&mut m.bus);
        if let Some(index) = mouse.take_selection_event() {
            recorded.push_str(&format!("{f} mouse:{index}\n"));
        }
        if let Some(index) = m.bus.ffta_battle_mouse.take_event() {
            recorded.push_str(&format!("{f} mouse:{index}\n"));
        }
        if m.bus.keys != last_keys {
            recorded.push_str(&format!("{f} {:03x}\n", m.bus.keys));
            last_keys = m.bus.keys;
        }

        if let Some(addr) = watch.filter(|_| f >= trace_from && f < trace_to) {
            let offset = (addr & 0x3ffff) as usize;
            m.bus.frame_ready = false;
            let mut steps = 0;
            while !m.bus.frame_ready && steps < 5_000_000 {
                let before = m.bus.ewram[offset];
                let pc = m.cpu.regs[15];
                let lr = m.cpu.regs[14];
                let thumb = m.cpu.thumb();
                if let Some(address) = opt("--entry") {
                    if pc == u32::from_str_radix(address, 16)? {
                        println!(
                            "MENU_ENTRY frame={f} pc={pc:08x} lr={lr:08x} regs={:08x?}",
                            m.cpu.regs
                        );
                    }
                }
                m.step();
                steps += 1;
                if m.bus.ewram[offset] != before {
                    println!("MENU_WRITE frame={f} addr={addr:08x} old={before} new={} pc={pc:08x} lr={lr:08x} thumb={thumb} regs={:08x?}",m.bus.ewram[offset],m.cpu.regs);
                    if pc >> 24 == 8 && thumb {
                        for a in (pc - 12..pc + 12).step_by(2) {
                            let o = (a - 0x08000000) as usize;
                            let h = u16::from_le_bytes([m.bus.rom[o], m.bus.rom[o + 1]]);
                            println!("  {a:08x} {}", armv4t::decode_thumb(h, a).disasm());
                        }
                    }
                }
            }
        } else {
            m.run_frame(5_000_000);
        }
        if pub_trace && m.bus.frames>=4800 {
            println!("FFTA_PUB frame={} main={} side={:?} extra={:?}",m.bus.frames,
                m.bus.ffta_dialogue.town_layout,m.bus.ffta_dialogue.layer().map(|(_,right)|right),m.bus.ffta_dialogue.extra.kind);
        }
        if frames.contains(&m.bus.frames) {
            println!(
                "MENU_SCENE frame={} scene={:?}",
                m.bus.frames,
                mouse.scene(&m.bus)
            );
            println!("BATTLE_MENU_SCENE frame={} scene={:?}",m.bus.frames,gba_core::ffta_battle_mouse::MenuScene::read(&m.bus));
            println!(
                "BATTLE_SCENE frame={} {:?}",
                m.bus.frames,
                gba_core::ffta_battle_mouse::Scene::read(&m.bus).map(|s| (
                    s.width,
                    s.height,
                    s.selected,
                    s.cells.len()
                ))
            );
            let stem = format!("{prefix}-{}", m.bus.frames);
            for (name, data) in [
                ("ewram", &m.bus.ewram),
                ("iwram", &m.bus.iwram),
                ("io", &m.bus.io),
                ("vram", &m.bus.vram),
                ("oam", &m.bus.oam),
                ("palette", &m.bus.palette),
            ] {
                std::fs::write(format!("{stem}-{name}.bin"), data)?;
            }
            let mut ppm = b"P6\n240 160\n255\n".to_vec();
            for &p in &m.bus.framebuffer {
                for c in [p & 31, p >> 5 & 31, p >> 10 & 31] {
                    ppm.push((c << 3 | c >> 2) as u8);
                }
            }
            std::fs::write(format!("{stem}.ppm"), ppm)?;
            println!(
                "MENU_PROBE frame={} dispcnt={:04x} cnt={:04x?}",
                m.bus.frames,
                u16::from_le_bytes([m.bus.io[0], m.bus.io[1]]),
                (0..4)
                    .map(|i| u16::from_le_bytes([m.bus.io[8 + i * 2], m.bus.io[9 + i * 2]]))
                    .collect::<Vec<_>>()
            );
            let ui = &m.bus.ffta_dialogue;
            let bounds = gba_core::ffta_dialogue::pixel_bounds(&ui.pixels);
            println!(
                "MENU_UI frame={} action={} kind={:?} bounds={:?} panel_bounds={:?} shop={}",
                m.bus.frames,
                ui.action_layout,
                ui.extra.kind,
                bounds,
                gba_core::ffta_dialogue::panel_bounds(&ui.extra.pixels, &ui.extra.backing),ui.shop_layout
            );
            if ui.layer().is_some() {
                let mut active_rows = Vec::new();
                for y in 0..gba_core::ffta_dialogue::LAYER_HEIGHT {
                    let row = &ui.pixels[y * gba_core::ffta_dialogue::LAYER_WIDTH
                        ..(y + 1) * gba_core::ffta_dialogue::LAYER_WIDTH];
                    let backing = ui.backing.get(y * gba_core::ffta_dialogue::LAYER_WIDTH
                        ..(y + 1) * gba_core::ffta_dialogue::LAYER_WIDTH);
                    let mut foreground = Vec::new();
                    for (x, pixel) in row.iter().enumerate() {
                        if pixel.visible()
                            && backing.is_none_or(|b| b[x] != *pixel)
                        {
                            foreground.push(x as i32 - gba_core::ffta_dialogue::LAYER_INSET);
                        }
                    }
                    if !foreground.is_empty() {
                        active_rows.push((y, *foreground.first().unwrap(), *foreground.last().unwrap(), foreground.len()));
                    }
                }
                println!("MENU_UI_ROWS frame={} foreground={:?}", m.bus.frames, active_rows);
                let ui_path = format!("{prefix}-{}-ui.bin", m.bus.frames);
                let ui_bytes: Vec<u8> = ui.pixels.iter().flat_map(|p| p.packed()).collect();
                std::fs::write(&ui_path, ui_bytes)?;
                let backing_path = format!("{prefix}-{}-ui-backing.bin", m.bus.frames);
                let backing_bytes: Vec<u8> = ui.backing.iter().flat_map(|p| p.packed()).collect();
                std::fs::write(backing_path, backing_bytes)?;
                let extra_path = format!("{prefix}-{}-ui-extra.bin", m.bus.frames);
                let extra_bytes: Vec<u8> = ui.extra.pixels.iter().flat_map(|p| p.packed()).collect();
                std::fs::write(extra_path, extra_bytes)?;
                for (slot, entry) in m.bus.oam.chunks_exact(8).enumerate() {
                    let a0 = u16::from_le_bytes([entry[0], entry[1]]);
                    let a1 = u16::from_le_bytes([entry[2], entry[3]]);
                    let a2 = u16::from_le_bytes([entry[4], entry[5]]);
                    if (0x124..=0x127).contains(&(a2 & 1023)) {
                        println!("MENU_CURSOR frame={} slot={} x={} y={} a0={:04x} a1={:04x} a2={:04x}",m.bus.frames,slot,a1&511,a0&255,a0,a1,a2);
                    }
                }
            }
        }
    }
    if let Some(path) = opt("--record") {
        std::fs::write(path, recorded)?;
    }
    Ok(())
}
