//! Regression driver: original and separated UI must be identical at native size.
//! Reads the user's ROM; writes no ROM, save or captured game data.
use gba_core::{mem,Machine};
fn hash(bytes:impl IntoIterator<Item=u8>,mut h:u64)->u64 {
    for b in bytes {h=(h^b as u64).wrapping_mul(0x100000001b3);}h
}
fn main()->Result<(),Box<dyn std::error::Error>> {
    let path=std::env::args().nth(1).ok_or("ROM required")?;
    let rom=std::fs::read(path)?;
    if gba_core::ffta_rom::version(&rom).is_none(){return Err("exact supported ROM required".into());}
    mem::set_extra(0);mem::set_extra_y(0);
    let mut original=Machine::new(rom.clone());let mut patched=Machine::new(rom);
    patched.bus.ffta_dialogue.separate=true;
    let mut fb=0xcbf29ce484222325;let mut oam=fb;
    for frame in 0..600 {
        let key=if (240..248).contains(&frame)||(500..508).contains(&frame){0x3f7}else{0x3ff};
        original.bus.keys=key;patched.bus.keys=key;
        original.run_frame(5_000_000);patched.run_frame(5_000_000);
        assert_eq!(original.bus.frames,frame+1,"original frame did not complete");
        assert_eq!(patched.bus.frames,frame+1,"patched frame did not complete");
        assert_eq!(original.bus.framebuffer,patched.bus.framebuffer,"framebuffer frame={frame}");
        assert_eq!(original.bus.oam,patched.bus.oam,"OAM frame={frame}");
        fb=hash(original.bus.framebuffer.iter().flat_map(|p|p.to_le_bytes()),fb);
        oam=hash(original.bus.oam.iter().copied(),oam);
    }
    println!("EXTRA0 frames=600 framebuffer_and_oam_equal=true framebuffer={fb:016x} oam={oam:016x}");
    Ok(())
}
