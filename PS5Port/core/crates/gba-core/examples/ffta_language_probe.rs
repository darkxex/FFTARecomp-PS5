//! Private language-selector diagnostics using synthetic input and guarded direct requests.
//! User ROM and runtime snapshots stay outside source control; no save is loaded/written.
use gba_core::{machine::Machine,ffta_rom::{self,Version},mem};
fn main()->Result<(),Box<dyn std::error::Error>> {
    let args:Vec<_>=std::env::args().collect();
    let rom=std::fs::read(args.get(1).ok_or("EUR ROM path required")?)?;
    if ffta_rom::version(&rom)!=Some(Version::Europe) {return Err("exact EUR ROM required".into());}
    let prefix=args.get(2).ok_or("private output prefix required")?;
    mem::set_extra(0);mem::set_extra_y(0);
    let mut m=Machine::new(rom);
    if args.iter().any(|s|s=="--direct-probe") {
        for _ in 0..300 {m.bus.keys=0x3ff;m.run_frame(5_000_000);}
        for target in [4u8,0,3,1,2] {
            let pointer=u32::from_le_bytes(m.bus.ewram[0x1c3d4..0x1c3d8].try_into()?);
            let off=(pointer-0x02000000)as usize;
            m.bus.ewram[off+2]=target;
            for tick in 1..=4 {
                m.bus.keys=0x3ff;m.run_frame(5_000_000);
                let y=u16::from_le_bytes([m.bus.oam[0],m.bus.oam[1]])&255;
                let palettes:Vec<_>=(0..5).map(|i|u16::from_le_bytes([m.bus.oam[(3+i*3)*8+4],m.bus.oam[(3+i*3)*8+5]])>>12).collect();
                println!("DIRECT_PROBE target={target} tick={tick} selected={} quill_y={y} palettes={palettes:?}",m.bus.ewram[off+2]);
            }
        }
        return Ok(());
    }
    if args.iter().any(|s|s=="--mouse") {
        let mut mouse=gba_core::ffta_mouse::LanguageMouse::new(&m.bus.rom,true);
        for _ in 0..300 {m.bus.keys=0x3ff;m.run_frame(5_000_000);}
        let mut script=String::from("gba-input v1\n0 3ff\n");let mut last=0x3ff;
        for index in [4u8,2,0,3,1] {
            let scene=mouse.scene(&m.bus).ok_or("language targets not detected")?;
            let b=scene.targets[index as usize].bounds;
            if !mouse.aim(&m.bus,(b[0]+b[2])*0.5,(b[1]+b[3])*0.5,false){return Err("target request failed".into());}
            for tick in 0..4 {
                let keys=mouse.keys(&mut m.bus,0x3ff);
                if let Some(index)=mouse.take_selection_event(){script.push_str(&format!("{} mouse:{index}\n",m.bus.frames));}
                if keys!=0x3ff {return Err("hover emitted a direction key".into());}
                if m.bus.ewram[0xf3d6]!=index {return Err("direct selection was not immediate".into());}
                if keys!=last {script.push_str(&format!("{} {keys:03x}\n",m.bus.frames));last=keys;}
                m.bus.keys=keys;m.run_frame(5_000_000);
                let y=u16::from_le_bytes([m.bus.oam[0],m.bus.oam[1]])&255;
                if y!=38+18*scene.selected as u16 && y!=38+18*index as u16 {return Err("intermediate language appeared".into());}
                if tick>=1 && y!=38+18*index as u16 {return Err("quill did not update within two frames".into());}
                if tick>=1 {for row in 0..5 {let o=(3+row*3)*8+4;let pal=u16::from_le_bytes([m.bus.oam[o],m.bus.oam[o+1]])>>12;
                    if pal!=u16::from(row==index as usize) {return Err("text palette did not follow direct selection".into());}
                }}
            }
            let actual=mouse.scene(&m.bus).ok_or("selector vanished before click")?.selected;
            println!("MOUSE_HOVER requested={index} actual={actual} frame={}",m.bus.frames);
            let mut ppm=b"P6\n240 160\n255\n".to_vec();
            for &p in &m.bus.framebuffer {for c in [p&31,p>>5&31,p>>10&31] {ppm.push((c<<3|c>>2)as u8);}}
            std::fs::write(format!("{prefix}-hover-{index}.ppm"),ppm)?;
            if actual!=index {return Err("mouse navigation failed".into());}
        }
        let (cells,rebuilds)=mouse.grid_metrics();
        println!("MOUSE_GRID cells={cells} rebuilds={rebuilds} cell_size={}",gba_core::interaction_grid::CELL_SIZE);
        if cells!=600 || rebuilds!=1 {return Err("grid rebuilt unnecessarily".into());}
        let scene=mouse.scene(&m.bus).unwrap();let b=scene.targets[1].bounds;
        mouse.aim(&m.bus,(b[0]+b[2])*0.5,(b[1]+b[3])*0.5,true);
        let mut a_edges=0;let mut previous=0x3ff;
        for _ in 0..50 {
            let keys=mouse.keys(&mut m.bus,0x3ff);if keys&1==0&&previous&1!=0 {a_edges+=1;}previous=keys;
            if keys!=last {script.push_str(&format!("{} {keys:03x}\n",m.bus.frames));last=keys;}
            m.bus.keys=keys;m.run_frame(5_000_000);
        }
        println!("MOUSE_CLICK confirm_edges={a_edges} selector_active={}",mouse.scene(&m.bus).is_some());
        if a_edges!=1 || mouse.scene(&m.bus).is_some(){return Err("click confirmation failed".into());}
        let mut hash=0xcbf29ce484222325u64;
        for &p in &m.bus.framebuffer {for b in p.to_le_bytes(){hash^=b as u64;hash=hash.wrapping_mul(0x1_0000_01b3);}}
        println!("MOUSE_REPLAY frames={} framebuffer_hash={hash:016x}",m.bus.frames);
        let mut ppm=b"P6\n240 160\n255\n".to_vec();
        for &p in &m.bus.framebuffer {for c in [p&31,p>>5&31,p>>10&31] {ppm.push((c<<3|c>>2)as u8);}}
        std::fs::write(format!("{prefix}-final.ppm"),ppm)?;
        std::fs::write(format!("{prefix}.input"),script)?;
        return Ok(());
    }
    for frame in 0..410u64 {
        m.bus.keys=if [320,340,360,380,400].iter().any(|&start|(start..start+4).contains(&frame)) {0x37f}else{0x3ff};
        if args.iter().any(|s|s=="--trace-selection") && (320..324).contains(&frame) {
            m.bus.frame_ready=false;let mut steps=0;
            while !m.bus.frame_ready && steps<5_000_000 {
                let pc=m.cpu.regs[15];let lr=m.cpu.regs[14];let thumb=m.cpu.thumb();let before=m.bus.ewram[0xf3d6];
                m.step();steps+=1;
                if m.bus.ewram[0xf3d6]!=before {
                    println!("LANGUAGE_SELECTION_WRITE frame={frame} pc={pc:08x} lr={lr:08x} thumb={thumb} old={before} new={} regs_after={:08x?}",m.bus.ewram[0xf3d6],m.cpu.regs);
                    if thumb && pc>>24==8 {
                        for address in (pc-24..pc+16).step_by(2) {
                            let o=(address-0x08000000)as usize;let h=u16::from_le_bytes([m.bus.rom[o],m.bus.rom[o+1]]);
                            println!("  {address:08x} {}",armv4t::decode_thumb(h,address).disasm());
                        }
                    }
                }
            }
        }else{m.run_frame(5_000_000);}
        if [300,330,350,370,390,410].contains(&m.bus.frames) {
            let index=if m.bus.frames==300 {0}else{(m.bus.frames-310)/20};
            let stem=format!("{prefix}-{index}");
            for (name,data) in [("ewram",&m.bus.ewram),("iwram",&m.bus.iwram),("io",&m.bus.io),("vram",&m.bus.vram),("oam",&m.bus.oam),("palette",&m.bus.palette)] {
                std::fs::write(format!("{stem}-{name}.bin"),data)?;
            }
            let mut ppm=b"P6\n240 160\n255\n".to_vec();
            for &p in &m.bus.framebuffer {for c in [p&31,p>>5&31,p>>10&31] {ppm.push((c<<3|c>>2)as u8);}}
            std::fs::write(format!("{stem}.ppm"),ppm)?;
            println!("LANGUAGE_PROBE frame={} selection_step={index} dispcnt={:04x}",m.bus.frames,u16::from_le_bytes([m.bus.io[0],m.bus.io[1]]));
        }
    }
    Ok(())
}
