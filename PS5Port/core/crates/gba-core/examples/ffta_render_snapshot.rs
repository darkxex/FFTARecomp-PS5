//! Offline diagnostic of private RAM/VRAM snapshots; no ROM data is embedded.
use gba_core::{mem::{self,MemMap},ppu};
fn main() -> Result<(),Box<dyn std::error::Error>> {
    let args:Vec<_>=std::env::args().collect();
    let prefix=args.get(1).ok_or("snapshot prefix required")?;
    let wide=args.iter().any(|a|a=="--wide");
    let option=|name:&str|args.windows(2).find(|pair|pair[0]==name).map(|pair|pair[1].as_str());
    mem::set_extra(if wide {option("--extra").unwrap_or("448").parse()?}else{0});
    mem::set_extra_y(if wide {option("--extra-y").unwrap_or("80").parse()?}else{0});
    let rom=if let Some(path)=option("--rom") {std::fs::read(path)?}else{vec![0;192]};
    let mut m=MemMap::new(rom);
    m.ffta_dialogue.separate=args.iter().any(|a|a=="--separate-ui");
    for (name,dst) in [("ewram",&mut m.ewram),("iwram",&mut m.iwram),("vram",&mut m.vram),("io",&mut m.io),("oam",&mut m.oam),("palette",&mut m.palette)] {
        dst.copy_from_slice(&std::fs::read(format!("{prefix}-{name}.bin"))?);
    }
    // Enable only completion of extra rows; no inferred host planes or sprites.
    if let Some(layer)=option("--layer") {
        let layer:usize=layer.parse()?;
        if layer>4 {return Err("--layer expects BG0..BG3 as 0..3, or OBJ as 4".into());}
        let control=u16::from_le_bytes([m.io[0],m.io[1]]);
        m.io[..2].copy_from_slice(&((control&!0x1f00)|(0x100<<layer)).to_le_bytes());
    }
    m.ffta_movement.enabled=wide && !args.iter().any(|a|a=="--original-ui-edge");
    if args.iter().any(|a|a=="--stats") {
        // Explicit diagnostic tagging for the measured frame-13000 snapshot.
        // Production tagging is gated by the exact ROM and producer list.
        m.ffta_sprites.active=Some((8..12).map(|slot| {
            let p=&m.oam[slot*8..slot*8+6];
            let attrs=std::array::from_fn(|i|u16::from_le_bytes([p[i*2],p[i*2+1]]));
            gba_core::ffta_sprites::HostSprite {x:(attrs[1]&511) as i32,y:(attrs[0]&255) as i32,
                attrs,slot,replaces:true,stats_background:true,..Default::default()}
        }).collect());
    }
    for line in 0..160 {ppu::render_scanline(&mut m,line);}
    if wide {ppu::render_ffta_zoom_rows(&mut m);}
    if m.ffta_dialogue.separate {
        println!("SNAPSHOT_UI town={} right={:?} bounds={:?} menu={:?}",m.ffta_dialogue.town_layout,
            m.ffta_dialogue.layer().map(|(_,right)|right),gba_core::ffta_dialogue::pixel_bounds(&m.ffta_dialogue.pixels),
            gba_core::ffta_dialogue::pixel_bounds(&m.ffta_dialogue.extra.pixels));
    }
    let (w,h,pixels)=if wide {(mem::render_width(),mem::render_height(),&m.wide_framebuffer)}else{(240,160,&m.framebuffer)};
    let mut out=format!("P6\n{w} {h}\n255\n").into_bytes();
    for &p in &pixels[..w*h] {for c in [p&31,(p>>5)&31,(p>>10)&31] {out.push((c<<3|c>>2) as u8);}}
    let suffix=if args.iter().any(|a|a=="--stats") {"-stats"}else{""};
    let path=option("--out").map(str::to_owned).unwrap_or_else(||format!("{prefix}-raw-{}{suffix}.ppm",if wide {"wide"}else{"native"}));
    std::fs::write(&path,out)?;
    println!("SNAPSHOT_RENDER {path}");
    Ok(())
}
