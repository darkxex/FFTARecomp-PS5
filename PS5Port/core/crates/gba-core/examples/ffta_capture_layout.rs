//! Layout metadata for read-only diagnostics of a matching desktop build.
use gba_core::mem::MemMap;
fn main() {
    macro_rules! field { ($name:ident) => { println!("{} {}", stringify!($name), std::mem::offset_of!(MemMap, $name)); }; }
    field!(rom); field!(ewram); field!(iwram); field!(vram);
    field!(io); field!(palette); field!(oam);
    let v = vec![0u8; 8];
    let words = unsafe { std::ptr::read((&v as *const Vec<u8>).cast::<[usize; 3]>()) };
    println!("pointer_index {}", words.iter().position(|&w| w == v.as_ptr() as usize).unwrap());
}
