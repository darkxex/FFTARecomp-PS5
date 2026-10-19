//! Opt-in battle mouse. Original cursor routine commits the target and refreshes UI.
//! No ROM code, map, range, pathfinding or unit position is patched.
use crate::{
    ffta_rom::{self, Version},
    Machine, MemMap,
};
const RELEASED: u16 = 0x3ff;
#[derive(Clone, Copy, Debug)]
struct Profile {
    base: usize,
    start: u32,
    commit: u32,
    end: u32,
}
#[derive(Clone, Copy, Debug)]
struct Request {
    x: u8,
    y: u8,
    base: usize,
    deadline: u64,
    confirm: bool,
    wake_sent: bool,
    committed: Option<u64>,
}
#[derive(Default)]
pub struct State {
    pub(crate) shop:crate::ffta_shop_mouse::State,
    profile: Option<Profile>,
    pub enabled: bool,
    request: Option<Request>,
    event: Option<u16>,
    pulse: u8,
    release: u8,
    key: u16,
    menu_profile: Option<MenuProfile>,
    menu_request: Option<MenuRequest>,
    menu_cache: std::sync::atomic::AtomicUsize,
    buttons: std::collections::VecDeque<u16>,
    button_key: u16,
    button_hold: u8,
    button_release: u8,
    facing_key: Option<u16>,
    world_profile: Option<Profile>,
    world_request: Option<Request>,
}
#[derive(Clone, Copy)]
struct MenuProfile { start: u32, commit: u32, end: u32, cursor: u32 }
#[derive(Clone, Copy)]
struct MenuRequest {
    base: usize, target: u8, confirm: bool,
    last_selected: u8,
    context: [u16;5],
    committed: Option<u64>, deadline: u64,
}
/// A live instance of the game's vertical list, matched to its visible cursor.
/// Count and placement come from the instance, never the names of its entries.
#[derive(Clone, Copy, Debug,PartialEq,Eq)]
pub struct MenuScene {
    pub count: u8,
    pub selected: u8,
    pub origin: [i32; 2],
    base: usize,
    binary: bool,
    horizontal: bool,
    split: i32,
    polling: bool,
    pub(crate) right: i32,
}
impl MenuProfile {
    fn new(rom: &[u8], version: Version) -> Option<Self> {
        let p = match version {
            Version::Usa => Self { start: 0x08028708, commit: 0x080287a0, end: 0x080287c0, cursor: 0x08017e81 },
            Version::Europe => Self { start: 0x08029944, commit: 0x080299dc, end: 0x080299fc, cursor: 0x08018fc5 },
        };
        let bytes = |pc: u32, n| rom.get((pc - 0x08000000) as usize..(pc - 0x08000000) as usize + n);
        (bytes(p.start, 12) == Some(&[0x70,0xb5,5,0x1c,9,4,9,0x0c,0x2e,0x1c,0x58,0x36])
            && bytes(p.commit, 6) == Some(&[0x24,6,0x21,0x0e,0x30,0x1c])
            && bytes(p.cursor & !1, 12) == Some(&[0x70,0xb5,0x87,0xb0,4,0x1c,0x23,0x1c,0x26,0x7c,4,0x20]))
            .then_some(p)
    }
}
impl MenuScene {
    fn cursor_offsets<'a>(mem:&'a MemMap,callback:u32,start:usize,end:usize,
        known:Option<&'a[usize]>)->impl Iterator<Item=usize>+'a {
        let mut index=0;let mut next=start;
        std::iter::from_fn(move||{
            if let Some(offsets)=known {
                while let Some(&offset)=offsets.get(index) {
                    index+=1;if (start..end).contains(&offset){return Some(offset);}
                }
                return None;
            }
            while next<end {
                let offset=next;next+=4;
                if w(&mem.ewram,offset)==Some(callback){return Some(offset);}
            }
            None
        })
    }
    pub fn read(mem: &MemMap) -> Option<Self> {
        if mem.ffta_dialogue.extra.kind==crate::ffta_dialogue::PanelKind::Notice && mem.ffta_dialogue.extra.active() {return Self::notice_menu(mem);}
        let s = &mem.ffta_battle_mouse;
        let p = s.menu_profile.filter(|_| s.enabled)?;
        if h(&mem.io,0)?&0x1007!=0x1000 {return None;}
        if let Some(scene)=Self::bank_menu(mem) {return Some(scene);}
        if let Some(scene)=Self::parent_window(mem) {return Some(scene);}
        if let Some(scene)=Self::shop_inventory(mem) {return Some(scene);}
        if let Some(scene)=Self::window_list(mem,p.cursor==0x08018fc5) {return Some(scene);}
        let battle_layers=!(h(&mem.io, 0)? & 0x0f07 != 0x0f00
            || h(&mem.io, 8)? != 0x4c03 || h(&mem.io, 10)? != 0x4e02
            || !matches!((h(&mem.io,12)?, h(&mem.io,14)?), (0x5c08,0x4a09) | (0x0b08,0x0a08)));
        let mut cursor = None;
        for obj in mem.oam.chunks_exact(8) {
            let a = h(obj,0)?; let b = h(obj,2)?; let c = h(obj,4)?;
            let tile=c&0x3ff;
            let marker=(a&0xff00==0 && (0x124..=0x127).contains(&tile))
                || (a&0xff00==0x4000 && (0x120..=0x123).contains(&tile));
            if marker && b & 0xfe00 == 0
                && a & 255 < 160 && b & 511 < 240 {
                if cursor.is_some() { return None; }
                cursor = Some([(b & 511) as i32, (a & 255) as i32]);
            }
        }
        let cursor = cursor.or_else(|| Self::town_menu(mem).and_then(|scene| {
            Some([h(&mem.ewram,scene.base+8)? as i16 as i32,
                h(&mem.ewram,scene.base+10)? as i16 as i32+h(&mem.ewram,scene.base+14)? as i16 as i32])
        })).or_else(|| {
            // The game's arrow blinks. A pending request was armed while it
            // was visible; retain its verified instance through that blink.
            let r=s.menu_request?;
            if mem.frames>r.deadline || r.context!=[0,8,10,12,14].map(|o|h(&mem.io,o).unwrap_or(0)){return None;}
            let c=if w(&mem.ewram,r.base)==Some(p.cursor){r.base}else{r.base+0x58};
            if w(&mem.ewram,c)!=Some(p.cursor){return None;}
            let flags=*mem.ewram.get(c+0x10)?;
            if flags&15==0{return None;}
            Some([h(&mem.ewram,c+8)? as i16 as i32+if flags&4!=0{15}else if flags&8!=0{24}else{0},
                h(&mem.ewram,c+10)? as i16 as i32+h(&mem.ewram,c+14)? as i16 as i32+if flags&4!=0{1}else{0}])
        })?;
        let shared=||Self::town_menu(mem).or_else(||Self::text_list(mem,p.cursor,Some(cursor)));
        if !battle_layers {return shared();}
        // The two-button confirmation has no text-list pointer or count.
        // Its original handler toggles cursor+0x11 and moves to base+0x80/88.
        let binary_pc = if p.cursor==0x08017e81 {0x29350}else{0x2a58c};
        if mem.rom.get(binary_pc..binary_pc+8)==Some(&[0xf0,0xb5,4,0x1c,0x26,0x1c,0x58,0x36]) {
            for base in (0..mem.ewram.len().saturating_sub(0x90)).step_by(4) {
                if mem.ewram[base+8]!=10 || w(&mem.ewram,base+0x58)!=Some(p.cursor)
                    || w(&mem.ewram,base+0x5c)!=Some(0) || mem.ewram[base+0x68]&4==0 {continue;}
                let selected=mem.ewram[base+0x69];
                if selected>1 {continue;}
                let origin=[h(&mem.ewram,base+0x84)? as i16 as i32,h(&mem.ewram,base+0x86)? as i16 as i32];
                let row=base+0x80+selected as usize*8;
                let x=h(&mem.ewram,row+4)? as i16 as i32;
                let y=h(&mem.ewram,row+6)? as i16 as i32;
                if origin[0]<0 || origin[0]>160 || origin[1]<0 || origin[1]>128
                    || h(&mem.ewram,base+0x8e)? as i32!=origin[1]+16
                    || (cursor!=[x+9,y+4] && !s.menu_request.is_some_and(|r|r.base==base && r.target==selected)) {continue;}
                return Some(Self{count:2,selected,origin,base,binary:true,horizontal:false,split:0,polling:false,right:origin[0]+80});
            }
        }
        let read = |base: usize| -> Option<Self> {
            let count = *mem.ewram.get(base+9)?;
            let selected = *mem.ewram.get(base+0x69)?;
            if count == 0 || count > 32 || selected >= count
                || w(&mem.ewram,base+0x58)? != p.cursor
                || w(&mem.ewram,base+0x5c)? != 0x02000000+base as u32+0x38
                || mem.ewram.get(base+0x68)? & 4 == 0
                || h(&mem.ewram,base+0x66)? != selected as u16*16 { return None; }
            let origin = [h(&mem.ewram,base+0x60)? as i16 as i32, h(&mem.ewram,base+0x62)? as i16 as i32];
            let publishing = s.menu_request.is_some_and(|r| r.base==base
                && r.committed.is_some_and(|f| mem.frames<=f+2));
            if origin[0] < 0 || origin[0] >= 224 || origin[1] < 0
                || origin[1]+count as i32*16 > 160
                || (!publishing && cursor != [origin[0]+15,origin[1]+selected as i32*16+1]) { return None; }
            Some(Self { count, selected, origin, base, binary:false,horizontal:false,split:0,polling:false,right:240 })
        };
        if let Some(scene) = s.menu_request.filter(|r|r.committed.is_some()).and_then(|r|read(r.base)) {
            return Some(scene);
        }
        // A cached instance must still match the current visible cursor. A
        // submenu can reuse its parent's coordinates, so prefer child lists.
        let mut candidates = Vec::new();
        if let Some(scene) = s.menu_cache.load(std::sync::atomic::Ordering::Relaxed).checked_sub(1).and_then(read) {
            if w(&mem.ewram,scene.base+4) != Some(0) { return Some(scene); }
        }
        for base in (0..mem.ewram.len().saturating_sub(0x70)).step_by(4) {
            if w(&mem.ewram,base+0x58) == Some(p.cursor) {
                if let Some(scene) = read(base) { candidates.push(scene); }
            }
        }
        candidates.sort_by_key(|v| w(&mem.ewram,v.base+4) != Some(0));
        let Some(result) = candidates.pop()else{return shared();};
        if candidates.last().is_some_and(|v| w(&mem.ewram,v.base+4) != Some(0)) { return None; }
        s.menu_cache.store(result.base+1,std::sync::atomic::Ordering::Relaxed);
        Some(result)
    }
    /// Other users of the same original cursor/text-list component (shop,
    /// pub). Use the live visible row count and text geometry, not labels.
    pub fn town_menu(mem:&MemMap)->Option<Self> {
        let p=mem.ffta_battle_mouse.menu_profile?;
        if h(&mem.io,0)?&0x0f07!=0x0f00 || h(&mem.io,8)?!=0x4c03
            || h(&mem.io,10)?!=0x4e02 || h(&mem.io,12)?!=0x0b08 || h(&mem.io,14)?!=0x0a08 {return None;}
        Self::text_list(mem,p.cursor,None)
    }
    /// Measured Pub root window, not its Rumors/Missions LIST subwindows.
    pub(crate) fn pub_menu(mem:&MemMap)->Option<Self> {
        let menu=Self::town_menu(mem)?;
        (mem.ewram[menu.base+0x10]&7==7 && menu.count==4
            && menu.origin==[8,92]).then_some(menu)
    }
    /// Shop root uses the same window component, with Buy/Sell/Leave rows.
    pub(crate) fn shop_menu(mem:&MemMap)->Option<Self> {
        let menu=Self::town_menu(mem)?;
        (mem.ewram[menu.base+0x10]&7==7 && menu.count==3
            && menu.origin==[8,92]).then_some(menu)
    }
    /// Other active town-root windows share the list callback and style, but
    /// may use a different row count and placement (for example Monster Bank).
    /// LIST and Pub submenus use styles 03 and 0F and remain unchanged.
    pub(crate) fn site_menu(mem:&MemMap)->Option<Self> {
        if let Some(menu)=Self::bank_window(mem,false) {return Some(menu);}
        if let Some(menu)=Self::parent_window(mem) {return Some(menu);}
        let menu=Self::town_menu(mem)?;
        (mem.ewram[menu.base+0x10]&0x0f==7 && (2..=12).contains(&menu.count))
            .then_some(menu)
    }
    /// One immutable scan for the three layout queries made at scanline zero.
    /// Recompute every frame; never retain a negative result across guest writes.
    pub(crate) fn layout_menus(mem:&MemMap)->(Option<Self>,Option<Self>,Option<Self>) {
        let Some(p)=mem.ffta_battle_mouse.menu_profile else{return (None,None,None);};
        if h(&mem.io,0).is_none_or(|v|v&0x0f07!=0x0f00)
            || [8,10,12,14].map(|o|h(&mem.io,o))!=[Some(0x4c03),Some(0x4e02),Some(0x0b08),Some(0x0a08)] {
            return (None,None,None);
        }
        let mut offsets=[0usize;256];let mut count=0;
        for offset in Self::cursor_offsets(mem,p.cursor,0x20,mem.ewram.len().saturating_sub(0x18),None) {
            if count==offsets.len(){return (Self::site_menu(mem),Self::pub_menu(mem),Self::shop_menu(mem));}
            offsets[count]=offset;count+=1;
        }
        let known=Some(&offsets[..count]);
        let parent=Self::bank_window_candidates(mem,false,known).or_else(||Self::parent_window_candidates(mem,known));
        let town=Self::text_list_candidates(mem,p.cursor,None,known);
        let site=parent.or_else(||town.filter(|m|mem.ewram[m.base+0x10]&15==7 && (2..=12).contains(&m.count)));
        let pub_menu=town.filter(|m|mem.ewram[m.base+0x10]&7==7 && m.count==4 && m.origin==[8,92]);
        let shop_menu=town.filter(|m|mem.ewram[m.base+0x10]&7==7 && m.count==3 && m.origin==[8,92]);
        (site,pub_menu,shop_menu)
    }
    /// Measured Monster Bank root. Unlike Pub/Shop this is a parent window
    /// with its cursor at +58; the options sit LEFT of its cursor anchor.
    /// The handler, updater and short ROM signatures identify both regions.
    pub(crate) fn bank_menu(mem:&MemMap)->Option<Self> {
        Self::bank_window(mem,true)
    }
    /// Town parent-window lists (including service menus), with the same
    /// verified cursor component as Monster Bank. Read geometry from their
    /// owner instead of mistaking the cursor anchor for the text's left edge.
    fn parent_window(mem:&MemMap)->Option<Self> {
        Self::parent_window_candidates(mem,None)
    }
    fn parent_window_candidates(mem:&MemMap,known:Option<&[usize]>)->Option<Self> {
        let p=mem.ffta_battle_mouse.menu_profile?;
        if p.cursor!=0x08017e81{return None;} // New service fallback is USA-only.
        if h(&mem.io,0)?&0x0f07!=0x0f00 || [8,10,12,14].map(|o|h(&mem.io,o))
            !=[Some(0x4c03),Some(0x4e02),Some(0x0b08),Some(0x0a08)] {return None;}
        let function=|pc:u32|->bool {
            if pc&1==0{return false;}
            pc.checked_sub(0x08000001).and_then(|o|h(&mem.rom,o as usize))
                .is_some_and(|op|op&0xff00==0xb500)
        };
        let mut result=None;
        for cursor in Self::cursor_offsets(mem,p.cursor,0x58,mem.ewram.len()-0x18,known) {
            if w(&mem.ewram,cursor)!=Some(p.cursor) {continue;}
            let base=cursor-0x58;
            if w(&mem.ewram,cursor+4)!=Some(0x02000000+base as u32+0x38)
                || !function(w(&mem.ewram,base)?) || !function(w(&mem.ewram,base+4)?)
                || !matches!(mem.ewram[cursor+0x10]&15,7|15) {continue;}
            let count=mem.ewram[base+9];let selected=mem.ewram[cursor+17];
            let x=mem.ewram[base+0x1c]as i32*8;
            let right=x+mem.ewram[base+0x1e]as i32*8;
            let y=h(&mem.ewram,cursor+10)?as i16 as i32;
            if !(2..=12).contains(&count) || selected>=count || right<=x+8 || right>240
                || y<0 || y+count as i32*16>160
                || h(&mem.ewram,cursor+14)?!=selected as u16*16 {continue;}
            if result.is_some(){return None;}
            result=Some(Self{count,selected,origin:[x,y],base,binary:false,horizontal:false,split:0,polling:true,right});
        }
        result
    }
    fn bank_window(mem:&MemMap,active_only:bool)->Option<Self> {
        Self::bank_window_candidates(mem,active_only,None)
    }
    fn bank_window_candidates(mem:&MemMap,active_only:bool,known:Option<&[usize]>)->Option<Self> {
        let p=mem.ffta_battle_mouse.menu_profile?;
        if h(&mem.io,0)?&0x0f07!=0x0f00 || [8,10,12,14].map(|o|h(&mem.io,o))
            !=[Some(0x4c03),Some(0x4e02),Some(0x0b08),Some(0x0a08)] {return None;}
        let (handler,updater,stack)=if p.cursor==0x08018fc5 {(0x5aee4,0x5e7b4,0x85)}else{(0x58d40,0x5c594,0x83)};
        if mem.rom.get(handler..handler+12)!=Some(&[0xf0,0xb5,stack,0xb0,0x0e,0x1c,0,6,5,0x0e,0x31,0x78])
            || mem.rom.get(updater..updater+12)!=Some(&[0xf0,0xb5,0x57,0x46,0x4e,0x46,0x45,0x46,0xe0,0xb4,0x8e,0xb0]) {return None;}
        let read=|base:usize|->Option<Self> {
            let callback=w(&mem.ewram,base)?;
            let root=callback==0x08000001+handler as u32;
            // Measured EUR empty-list message switches handler while retaining
            // the same window. Keep its frame and text on the same UI canvas.
            let list_pc=if p.cursor==0x08018fc5 {0x5b624}else{0x59454};
            let list_sig=if p.cursor==0x08018fc5 {[0xf0,0xb5,0x57,0x46,0x4e,0x46,0x45,0x46,0xe0,0xb4,0x86,0xb0]}
                else{[0xf0,0xb5,0x4f,0x46,0x46,0x46,0xc0,0xb4,0x84,0xb0,0x0f,0x1c]};
            let list=!active_only && callback==0x08000001+list_pc as u32
                && mem.rom.get(list_pc..list_pc+12)==Some(&list_sig);
            if (!root && !list)
                || w(&mem.ewram,base+4)?!=0x08000001+updater as u32
                || w(&mem.ewram,base+0x58)?!=p.cursor
                || w(&mem.ewram,base+0x5c)?!=0x02000000+base as u32+0x38
                || (active_only && *mem.ewram.get(base+0x68)?&4==0) {return None;}
            let count=*mem.ewram.get(base+9)?;let selected=*mem.ewram.get(base+0x69)?;
            let x=*mem.ewram.get(base+0x1c)? as i32*8;
            let right=x+*mem.ewram.get(base+0x1e)? as i32*8;
            let y=h(&mem.ewram,base+0x62)? as i16 as i32;
            if count==0 || count>12 || selected>=count || h(&mem.ewram,base+0x66)?!=selected as u16*16
                || right<=x+8 || right>240 || y<0 || y+count as i32*16>160 {return None;}
            Some(Self{count,selected,origin:[x,y],base,binary:false,horizontal:false,split:0,polling:true,right})
        };
        let cache=&mem.ffta_battle_mouse.menu_cache;
        if let Some(menu)=cache.load(std::sync::atomic::Ordering::Relaxed).checked_sub(1).and_then(read) {return Some(menu);}
        let mut found=None;
        for cursor in Self::cursor_offsets(mem,p.cursor,0x58,mem.ewram.len().saturating_sub(0x18),known) {
            let base=cursor-0x58;
            if let Some(menu)=read(base) {if found.is_some(){return None;}found=Some(menu);}
        }
        if let Some(menu)=found {cache.store(menu.base+1,std::sync::atomic::Ordering::Relaxed);}
        found
    }
    pub(crate) fn shop_inventory(mem:&MemMap)->Option<Self> {
        let p=mem.ffta_battle_mouse.menu_profile?;
        let pc=if p.cursor==0x08018fc5 {0x71030}else{0x6eb48};
        if mem.rom.get(pc..pc+12)!=Some(&[0xf0,0xb5,0x87,0xb0,6,0x1c,0x11,0x4f,0x39,0x68,0x11,0x4a])
            || h(&mem.io,0)?&0x4f07!=0x0f00 || h(&mem.io,8)?!=0x4c03 || h(&mem.io,10)?!=0x4e02
            || h(&mem.io,12)?!=0x0b08 || h(&mem.io,14)?!=0x0a08 {return None;}
        let mut found=None;
        for cursor in (0x20..mem.ewram.len()-0x20).step_by(4) {
            let list=cursor-0x20;
            if w(&mem.ewram,cursor)!=Some(0x08000001+pc as u32)
                || w(&mem.ewram,cursor+4)!=Some(0x02000000+list as u32)
                || mem.ewram[cursor+16]&1==0 {continue;}
            let count=h(&mem.ewram,list+0x18)?;
            let visible=mem.ewram[list+0x16] as u16;
            let selected=mem.ewram[cursor+17];
            let first=h(&mem.ewram,list+0x1a)?;
            if count==0 || visible==0 || visible>8 || first>=count {continue;}
            // Cursor indices are page-relative; list+1A is its first item.
            let count=(count-first).min(visible);
            if selected as u16>=count {continue;}
            let x=mem.ewram[list+0x13]as i32*8;
            let y=h(&mem.ewram,cursor+10)? as i16 as i32;
            let right=(x+(mem.ewram[list+0x15]as i32+2)*8).min(240);
            if x<0 || right<=x || y<0 || y+count as i32*16>160 {continue;}
            if found.is_some(){return None;}
            found=Some(Self{count:count as u8,selected,origin:[x,y],base:cursor,binary:false,horizontal:false,split:0,polling:true,right});
        }
        found
    }
    fn text_list(mem:&MemMap,callback:u32,visible_cursor:Option<[i32;2]>)->Option<Self> {
        Self::text_list_candidates(mem,callback,visible_cursor,None)
    }
    fn text_list_candidates(mem:&MemMap,callback:u32,visible_cursor:Option<[i32;2]>,known:Option<&[usize]>)->Option<Self> {
        let mut result=None;
        for cursor in Self::cursor_offsets(mem,callback,0x20,mem.ewram.len().saturating_sub(0x20),known) {
            if w(&mem.ewram,cursor)!=Some(callback)
                || w(&mem.ewram,cursor+4)!=Some(0x02000000+cursor as u32-0x20)
                || mem.ewram[cursor+0x10]&0x0f==0 {continue;}
            let list=cursor-0x20;
            let count=mem.ewram[list+0x16];let selected=mem.ewram[cursor+0x11];
            if count==0 || count>12 || selected>=count {continue;}
            let x=mem.ewram[list+0x13]as i32*8;
            let y=h(&mem.ewram,cursor+0xa)? as i16 as i32;
            let flags=mem.ewram[cursor+0x10];
            // Measured town styles: wide LIST arrow (03), compact MENU arrow
            // (0F). Do not mistake the battle cursor style (07) for a town UI.
            let root_window=(matches!(count,3|4) && x==8 && y==92 && flags&7==7)
                || (flags&0x0f==7 && (2..=12).contains(&count));
            if visible_cursor.is_none() && !matches!(flags&15,3|15) && !root_window {continue;}
            let arrow_x=h(&mem.ewram,cursor+8)? as i16 as i32
                +if flags&4!=0 {15}else if flags&8!=0 {24}else{0};
            let arrow_y=y+h(&mem.ewram,cursor+0xe)? as i16 as i32+if flags&4!=0 {1}else{0};
            let visible_cursor=visible_cursor.unwrap_or([arrow_x,arrow_y]);
            // The wide arrow's animation resource shifts its OAM piece up
            // to eight pixels left of the logical anchor (measured in pub).
            let same_x=if flags&4!=0 {visible_cursor[0]==arrow_x}else{(-8..=0).contains(&(visible_cursor[0]-arrow_x))};
            let publishing=mem.ffta_battle_mouse.menu_request.is_some_and(|r|r.base==cursor&&mem.frames<=r.deadline)
                && same_x && (visible_cursor[1]-y).rem_euclid(16)==if flags&4!=0{1}else{0}
                && visible_cursor[1]>=y && visible_cursor[1]<y+count as i32*16;
            let right=(x+(mem.ewram[list+0x15]as i32+2)*8).min(240);
            if x<0 || x>=right || y<0 || y+count as i32*16>160
                || (!publishing && (!same_x || visible_cursor[1]!=arrow_y)) {continue;}
            if result.is_some(){return None;}
            result=Some(Self{count,selected,origin:[x,y],base:cursor,binary:false,horizontal:false,split:0,polling:true,right});
        }
        result
    }
    /// Match the two identical choice bullets in the actual BG2 glyph pixels.
    /// The original cursor supplies the template and selected option; no labels,
    /// language widths or assumed Yes/No coordinates enter the hit targets.
    fn notice_menu(mem:&MemMap)->Option<Self> {
        if !mem.ffta_battle_mouse.enabled || mem.ffta_battle_mouse.menu_profile.is_none(){return None;}
        let bounds=mem.ffta_dialogue.extra.bounds?;
        let mut marker=None;
        for obj in mem.oam.chunks_exact(8) {
            let attrs=[h(obj,0)?,h(obj,2)?,h(obj,4)?];
            let x=(attrs[1]&511)as i32;let y=(attrs[0]&255)as i32;
            if crate::ffta_dialogue::notice_cursor(attrs,x,y,Some(bounds)) {
                if marker.is_some(){return None;}marker=Some((x,y));
            }
        }
        let (cx,cy)=marker?;
        let cnt=h(&mem.io,12)?;
        if cnt&0x80!=0{return None;}
        let map=((cnt>>8)&31)as usize*0x800;
        let chars=((cnt>>2)&3)as usize*0x4000;
        let pixel=|x:i32,y:i32|->Option<u8>{
            if !(0..240).contains(&x)||!(0..160).contains(&y){return None;}
            let wide=cnt&0x4000!=0;let tall=cnt&0x8000!=0;
            let x=(x+h(&mem.io,0x18)?as i32)&if wide{511}else{255};
            let y=(y+h(&mem.io,0x1a)?as i32)&if tall{511}else{255};
            let block=(y/256)*if wide{2}else{1}+x/256;
            let a=h(&mem.vram,map+block as usize*0x800+(((y/8%32)*32+x/8%32)*2)as usize)?;
            let tx=if a&0x400!=0{7-x%8}else{x%8};
            let ty=if a&0x800!=0{7-y%8}else{y%8};
            let v=*mem.vram.get(chars+(a&1023)as usize*32+ty as usize*4+tx as usize/2)?;
            Some((v>>(4*(tx%2)))&15)
        };
        let sample=|x|->Option<[u8;64]>{let mut out=[0;64];for y in 0..8 {for dx in 0..8 {out[(y*8+dx)as usize]=pixel(x+dx,cy+y)?;}}Some(out)};
        let template=sample(cx)?;
        let ink=template.iter().filter(|p|**p!=0).count();
        if !(8..=56).contains(&ink){return None;}
        let choices:Vec<_>=(bounds[0]..bounds[2]-7).filter(|x|sample(*x).as_ref()==Some(&template)).collect();
        if choices.len()!=2 || choices[1]-choices[0]<16{return None;}
        let selected=choices.iter().position(|x|*x==cx)?as u8;
        let right=(choices[1]+8..bounds[2]-8).filter(|x|(0..12).any(|dy|pixel(*x,cy-1+dy).is_some_and(|p|p!=0))).last()?+2;
        Some(Self{count:2,selected,origin:[choices[0],cy-4],base:0x40000,
            binary:true,horizontal:true,split:choices[1],polling:false,right})
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<u8> {
        let left=if self.binary || self.polling {self.origin[0]}else{self.origin[0]+8};
        let right=self.right;
        if !x.is_finite() || !y.is_finite() || x < left as f32 || x >= right as f32
            || y < self.origin[1] as f32 || y >= (self.origin[1]+if self.horizontal {16}else{self.count as i32*16}) as f32 { return None; }
        if self.horizontal {return Some(u8::from(x>=self.split as f32));}
        Some(((y-self.origin[1] as f32)/16.) as u8)
    }
    /// Shared window-list component, observed in the world-map town menu.
    /// Only visible, unscrolled instances with verified callback/layout qualify.
    fn window_list(mem:&MemMap,europe:bool)->Option<Self> {
        let pc=if europe {0x397ec}else{0x38028};
        if mem.rom.get(pc..pc+12)!=Some(&[0xf0,0xb5,0x87,0xb0,0x84,0x46,0x0e,0x1c,0x34,0x1c,0x31,0x7c]) {return None;}
        let manager=offset(w(&mem.iwram,0x2810)?,0x9cc+0xcc)?;
        let mut found=None;
        for slot in 0..8 {
            let list=manager+0x9cc+slot*0xcc;
            let cursor=list+0x20;
            if w(&mem.ewram,cursor)!=Some(0x08000001+pc as u32)
                || w(&mem.ewram,cursor+4)!=Some(0x02000000+list as u32) {continue;}
            let window=*mem.ewram.get(list-0x48)? as usize;
            if window>=8 {continue;}
            let win=0x9e0+window*0x4c;
            if mem.iwram.get(win)?&2==0 {continue;}
            let count=h(&mem.ewram,list+0x18)?;
            let visible=*mem.ewram.get(list+0x16)? as u16;
            let selected=*mem.ewram.get(cursor+0x11)?;
            if count==0 || count>visible || count>16 || selected as u16>=count {continue;}
            let x=mem.iwram[win+0x34]as i8 as i32*8;
            let y=(mem.iwram[win+0x35]as i8 as i32+mem.iwram[win+0x49]as i8 as i32-1)*8
                +h(&mem.ewram,cursor+0xa)? as i16 as i32-4;
            let right=x+mem.iwram[win+0x36]as i32*8;
            if x<0 || right>240 || right<=x || y<0 || y+count as i32*16>160 {continue;}
            if found.is_some(){return None;}
            found=Some(Self{count:count as u8,selected,origin:[x,y],base:cursor,binary:false,horizontal:false,split:0,polling:true,right});
        }
        found
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub x: u8,
    pub y: u8,
    pub height: u8,
    pub range: u8,
    pub center: [f32; 2],
}
#[derive(Debug)]
pub struct Scene {
    pub width: u8,
    pub height: u8,
    pub selected: (u8, u8),
    pub cells: Vec<Cell>,
    base: usize,
}
fn h(data: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(o..o + 2)?.try_into().ok()?))
}
fn w(data: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(o..o + 4)?.try_into().ok()?))
}
fn offset(pointer: u32, length: usize) -> Option<usize> {
    let o = pointer.checked_sub(0x02000000)? as usize;
    (o.checked_add(length)? <= 0x40000).then_some(o)
}
impl State {
    pub fn new(rom: &[u8]) -> Self {
        let p = match ffta_rom::version(rom) {
            Some(Version::Usa) => Profile {
                base: 0xf3ac,
                start: 0x08024ad8,
                commit: 0x08025132,
                end: 0x0802516e,
            },
            Some(Version::Europe) => Profile {
                base: 0xf3ac,
                start: 0x08025cec,
                commit: 0x08026346,
                end: 0x08026382,
            },
            _ => return Self::default(),
        };
        if rom.get((p.start - 0x08000000) as usize..(p.start - 0x08000000) as usize + 4)
            != Some(&[0xf0, 0xb5, 0x57, 0x46])
            || rom.get((p.commit - 0x08000000) as usize..(p.commit - 0x08000000) as usize + 4)
                != Some(&[0, 0x99, 0x88, 0x89])
        {
            return Self::default();
        }
        Self {
            shop:crate::ffta_shop_mouse::State::new(rom,ffta_rom::version(rom).unwrap()),
            profile: Some(p),
            world_profile: world_profile(rom,ffta_rom::version(rom).unwrap()),
            menu_profile: MenuProfile::new(rom, ffta_rom::version(rom).unwrap()),
            enabled: std::env::var("FFTA_MOUSE").as_deref() == Ok("1"),
            ..Default::default()
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) {self.cancel();self.enabled=enabled && self.profile.is_some();}
    pub fn enable_replay(&mut self) -> bool {
        self.enabled = self.profile.is_some();
        self.enabled
    }
    pub fn cancel(&mut self) {
        self.shop.cancel();
        self.world_request=None;
        self.request = None;
        self.menu_request = None;
        self.buttons.clear();
        self.button_hold=0;
        self.button_release=0;
        self.facing_key=None;
        self.event = None;
        self.pulse = 0;
        self.release = 0;
    }
    pub fn take_event(&mut self) -> Option<u16> {
        self.event.take()
    }
    pub(crate) fn record_shop_event(&mut self,event:u16){self.event=Some(event);}
    pub(crate) fn queue_shop_key(&mut self,key:u16){
        self.request=None;self.menu_request=None;self.world_request=None;self.shop.cancel();
        self.pulse=0;self.release=0;
        if self.buttons.len()<16 {self.buttons.push_back(key);}
    }
    pub fn interprets(&self, pc: u32) -> bool {
        self.enabled
            && (self.shop.interprets(pc) || (self.request.is_some_and(|r| r.committed.is_none())
                && self.profile.is_some_and(|p| (p.start..p.end).contains(&pc)))
                || (self.menu_request.is_some()
                    && self.menu_profile.is_some_and(|p| (p.start..p.end).contains(&pc)))
                || (self.world_request.is_some()
                    && self.world_profile.is_some_and(|p|(p.start..p.end).contains(&pc)))
                || (self.menu_request.is_some() && self.menu_profile.is_some_and(|p| {
                    let start=if p.cursor==0x08018fc5 {0x08018cac}else{0x08017b68};
                    let setter=(p.cursor&!1)+0x130;
                    (start..start+0x34).contains(&pc) || (setter..setter+12).contains(&pc)
                })))
    }
}
impl Scene {
    pub fn read(mem: &MemMap) -> Option<Self> {
        let state = &mem.ffta_battle_mouse;
        let p = state.profile.filter(|_| state.enabled)?;
        if MenuScene::read(mem).is_some() {
            return None;
        }
        let display=h(&mem.io,0)?;
        // Story text owns A/B even when a suspended field cursor retains its mode.
        if display&0xc00==0xc00 && h(&mem.io,12)?==0x0b08 && h(&mem.io,14)?==0x0a08 {
            return None;
        }
        if display & 0x1007 != 0x1000
            || !((display&0xc00==0xc00 && h(&mem.io, 12)? == 0x4c03 && h(&mem.io, 14)? == 0x4e02)
                || (display&0x300==0x300 && h(&mem.io, 8)? == 0x4c03 && h(&mem.io, 10)? == 0x4e02))
        {
            return None;
        }
        let b = p.base;
        let owner = w(&mem.ewram, b)?;
        offset(owner, 0x30)?;
        let mode = *mem.ewram.get(b + 4)?;
        if !matches!(mode, 1 | 2) {
            return None;
        }
        let width = *mem.ewram.get(0x7f18)?;
        let height = *mem.ewram.get(0x7f19)?;
        if width == 0 || height == 0 || width > 16 || height > 16 {
            return None;
        }
        let table = offset(w(&mem.ewram, 0x7f14)?, width as usize * height as usize * 2)?;
        let min_x = mem.ewram[0x7f1a];
        let min_y = mem.ewram[0x7f1c];
        let bound_w = mem.ewram[0x7f1d];
        let bound_h = mem.ewram[0x7f1f];
        if bound_w == 0
            || bound_h == 0
            || min_x as u16 + bound_w as u16 > width as u16
            || min_y as u16 + bound_h as u16 > height as u16
        {
            return None;
        }
        let selected = (h(&mem.ewram, b + 12)? as u8, h(&mem.ewram, b + 16)? as u8);
        if selected.0 >= width || selected.1 >= height {
            return None;
        }
        let ox = h(&mem.ewram, 0x7f60)? as i16 as i32 - h(&mem.ewram, 0x7f64)? as i16 as i32;
        let oy = h(&mem.ewram, 0x7f62)? as i16 as i32 - h(&mem.ewram, 0x7f66)? as i16 as i32;
        let mut cells = Vec::new();
        for y in min_y..min_y + bound_h {
            for x in min_x..min_x + bound_w {
                let i = table + 2 * (y as usize * width as usize + x as usize);
                let elevation = mem.ewram[i];
                if elevation == 0 || mem.ewram[i + 1] & 8 != 0 {
                    continue;
                }
                let range = range_value(mem, x, y);
                cells.push(Cell {
                    x,
                    y,
                    height: elevation,
                    range,
                    center: [
                        (16 * (x as i32 - y as i32) + ox) as f32,
                        (8 * (x as i32 + y as i32 + 1 - elevation as i32) + oy) as f32,
                    ],
                });
            }
        }
        Some(Self {
            width,
            height,
            selected,
            cells,
            base: b,
        })
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<Cell> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        self.cells
            .iter()
            .filter(|c| (x - c.center[0]).abs() / 16. + (y - c.center[1]).abs() / 8. < 1.)
            .max_by_key(|c| (c.x as u16 + c.y as u16, c.height))
            .copied()
    }
}
/// Exact packed-quarter style query from USA0801D58C / EUR0801E6FC.
pub fn range_value(mem: &MemMap, x: u8, y: u8) -> u8 {
    if x >= 16 || y >= 16 || mem.ewram[0x8168] == 0 || mem.ewram[0x8169] != 3 {
        return 0;
    }
    let i = y as usize * 16 + x as usize;
    let a = mem.ewram[0x8190 + i];
    let b = mem.ewram[0x8290 + i];
    let v = if a != 0 { a } else { b };
    for shift in [6, 4, 2, 0] {
        let n = (v >> shift) & 3;
        if n != 0 {
            return n;
        }
    }
    0
}
pub fn select(mem: &mut MemMap, x: u8, y: u8, confirm: bool) -> bool {
    let Some(scene) = Scene::read(mem) else {
        return false;
    };
    if !scene.cells.iter().any(|c| c.x == x && c.y == y) {
        return false;
    }
    let state = &mut mem.ffta_battle_mouse;
    state.menu_request = None;
    if let Some(r) = state.request.as_mut() {
        if (r.x, r.y) == (x, y) && r.base == scene.base {
            r.confirm |= confirm;
            r.deadline = mem.frames + 120;
            return true;
        }
    }
    state.pulse = 0;
    state.release = 0;
    state.request = Some(Request {
        x,
        y,
        base: scene.base,
        deadline: mem.frames + 120,
        confirm,
        wake_sent: false,
        committed: if scene.selected == (x, y) {
            Some(mem.frames)
        } else {
            None
        },
    });
    if scene.selected != (x, y) {
        state.event = Some(256 + y as u16 * 16 + x as u16);
    }
    true
}
pub fn aim(mem: &mut MemMap, x: f32, y: f32, confirm: bool) -> bool {
    if mem.ffta_dialogue.extra.kind==crate::ffta_dialogue::PanelKind::Notice && mem.ffta_dialogue.extra.active() {
        return if let Some(scene)=MenuScene::read(mem) {
            if let Some(index)=scene.hit(x,y){menu_select(mem,index,confirm)}else{mem.ffta_battle_mouse.cancel();false}
        }else{mem.ffta_battle_mouse.cancel();false};
    }
    if crate::ffta_shop_mouse::aim(mem,x,y,confirm){return true;}
    if world_cursor(mem).is_some() {return world_select(mem,x,y,confirm);}
    if let Some(center)=facing_center(mem) {
        if !confirm && (mem.ffta_battle_mouse.buttons.contains(&0x3fe)
            || (mem.ffta_battle_mouse.button_key==0x3fe && mem.ffta_battle_mouse.button_hold>0)) {return true;}
        let dx=x-center[0];let dy=(y-center[1])*2.;
        if !dx.is_finite() || !dy.is_finite() {return false;}
        let key=if dx.abs()+dy.abs()<6. {None}else{
            let a=dx+dy;let b=dy-dx;
            Some(if a.abs()>=b.abs(){if a>=0. {0x3ef}else{0x3df}}else if b>=0. {0x37f}else{0x3bf})
        };
        if confirm {
            let mut sequence=Vec::new();if let Some(key)=key {sequence.push(key);}sequence.push(0x3fe);
            press_sequence(mem,&sequence);
            mem.ffta_battle_mouse.facing_key=key;
        }else if key.is_some() && mem.ffta_battle_mouse.facing_key!=key {
            press_sequence(mem,&[key.unwrap()]);mem.ffta_battle_mouse.facing_key=key;
        }
        return true;
    }
    if let Some(scene) = MenuScene::read(mem) {
        return if let Some(index) = scene.hit(x,y) { menu_select(mem,index,confirm) }
        else { mem.ffta_battle_mouse.cancel(); false };
    }
    let cell = Scene::read(mem).and_then(|s| s.hit(x, y));
    let Some(c) = cell else {
        mem.ffta_battle_mouse.cancel();
        return false;
    };
    select(mem, c.x, c.y, confirm)
}
pub fn back(mem: &mut MemMap) {
    press(mem,0x3fd);
}
fn world_profile(rom:&[u8],version:Version)->Option<Profile> {
    let (start,x,y)=match version {Version::Europe=>(0x38994,0x38b3a,0x38c0e),Version::Usa=>(0x37428,0x375ce,0x376a2)};
    if rom.get(start..start+12)!=Some(&[0xf0,0xb5,0x16,0x4a,0x10,0x88,0xf0,0x26,6,0x40,0x15,0x49])
        || rom.get(x..x+2)!=Some(&[0xe0,0x80]) || rom.get(y..y+2)!=Some(&[0x20,0x81]) {return None;}
    Some(Profile{base:0,start:0x08000000+start as u32,commit:0x08000000+x as u32,end:0x08000000+y as u32+8})
}
/// World-map cursor: the original object owns world X/Y and vertical scroll.
/// Its screen sprite is a derived result, not the navigation state.
fn world_cursor(mem:&MemMap)->Option<(usize,[u16;2],u16)> {
    let s=&mem.ffta_battle_mouse;
    if !s.enabled || s.world_profile.is_none() || h(&mem.io,0)?&0x0f07!=0x0f00
        || h(&mem.io,8)?!=0xaa03 || h(&mem.io,10)?!=0xac02 {return None;}
    let manager=offset(w(&mem.iwram,0x2810)?,0xa0)?;
    let base=offset(w(&mem.ewram,manager+4)?,0x90)?;
    // 0x20 is also present while hovering a visitable destination. The
    // original movement handler still accepts input in that state.
    if mem.ewram[base]&3!=1 {return None;}
    let x=h(&mem.ewram,base+6)?;let y=h(&mem.ewram,base+8)?;
    let scroll=h(&mem.ewram,base+12)?;
    if !(14..=224).contains(&x) || !(22..=330).contains(&y) || scroll>330 {return None;}
    Some((base,[x,y],scroll))
}
pub fn world_select(mem:&mut MemMap,x:f32,y:f32,confirm:bool)->bool {
    let Some((base,current,scroll))=world_cursor(mem) else{return false;};
    if !x.is_finite() || !y.is_finite() || !(0.0..240.0).contains(&x) || !(0.0..160.0).contains(&y){return false;}
    let x=x.round().clamp(14.,224.)as u8;let y=y.round().clamp(0.,159.)as u8;
    let s=&mut mem.ffta_battle_mouse;
    if let Some(r)=s.world_request.as_mut(){if r.base==base && r.x==x && r.y==y {r.confirm|=confirm;r.deadline=mem.frames+120;return true;}}
    s.cancel();
    let same=current==[x as u16,(y as u16+scroll).clamp(22,330)];
    s.world_request=Some(Request{x,y,base,confirm,deadline:mem.frames+120,wake_sent:false,committed:same.then_some(mem.frames)});
    if !same{s.event=Some(4096+y as u16*240+x as u16);}
    true
}
/// Plain A/B also work in dialogue, forecasts and unsupported menu contexts.
pub fn press(mem:&mut MemMap,key:u16)->bool {press_sequence(mem,&[key])}
fn press_sequence(mem:&mut MemMap,keys:&[u16])->bool {
    if !mem.ffta_battle_mouse.enabled {return false;}
    let state=&mut mem.ffta_battle_mouse;
    state.cancel();state.buttons.extend(keys.iter().copied());true
}
/// Measured Wait prompt: four non-affine arrow pieces, mirrored in X.
pub fn facing_center(mem:&MemMap)->Option<[f32;2]> {
    let s=&mem.ffta_battle_mouse;let p=s.profile.filter(|_|s.enabled)?;
    if *mem.ewram.get(p.base+4)?!=0 {return None;}
    let mut pieces=Vec::new();
    for o in mem.oam.chunks_exact(8) {
        let a=h(o,0)?;let b=h(o,2)?;let c=h(o,4)?;
        if a&0xff00==0 && b&0xee00==0 && matches!(c,0x135|0x139) {
            pieces.push(((b&511)as i32,(a&255)as i32,c,b&0x1000!=0));
        }
    }
    if pieces.len()!=4 {return None;}
    let &(x,y,_,_)=pieces.iter().find(|v|v.2==0x135&&!v.3)?;
    for expected in [(x,y,0x135,false),(x+16,y,0x135,true),(x,y+8,0x139,false),(x+16,y+8,0x139,true)] {
        if !pieces.contains(&expected){return None;}
    }
    Some([(x+12)as f32,(y+8)as f32])
}
pub fn keys(mem: &mut MemMap, physical: u16) -> u16 {
    if physical != RELEASED {
        mem.ffta_battle_mouse.cancel();
        return physical;
    }
    if !mem.ffta_battle_mouse.enabled {
        return physical;
    }
    if let Some(key)=crate::ffta_shop_mouse::keys(mem){return key;}
    {
        let s=&mut mem.ffta_battle_mouse;
        if s.button_hold>0 {s.button_hold-=1;return s.button_key;}
        if s.button_release>0 {s.button_release-=1;return physical;}
        if let Some(key)=s.buttons.pop_front(){s.button_key=key;s.button_hold=3;s.button_release=4;return key;}
    }
    if facing_center(mem).is_some(){return physical;}
    if let Some((base,_,_))=world_cursor(mem) {
        let frame=mem.frames;let s=&mut mem.ffta_battle_mouse;
        if s.pulse>0 {s.pulse-=1;return s.key;}
        if s.release>0 {s.release-=1;return physical;}
        if let Some(r)=s.world_request {
            if r.base!=base || frame>r.deadline {s.cancel();return physical;}
            if let Some(f)=r.committed {
                if frame>=f+2 {s.world_request=None;if r.confirm{s.key=0x3fe;s.pulse=3;s.release=4;return s.key;}}
                return physical;
            }
            return 0x36f; // wake both original X and Y stores
        }
        return physical;
    }
    mem.ffta_battle_mouse.facing_key=None;
    if let Some(scene) = MenuScene::read(mem) {
        return menu_keys(mem,scene,physical);
    }
    if mem.ffta_battle_mouse.menu_request.is_some() {
        mem.ffta_battle_mouse.cancel();
    }
    let Some(scene) = Scene::read(mem) else {
        mem.ffta_battle_mouse.cancel();
        return physical;
    };
    let frame = mem.frames;
    let s = &mut mem.ffta_battle_mouse;
    if s.pulse > 0 {
        s.pulse -= 1;
        return s.key;
    }
    if s.release > 0 {
        s.release -= 1;
        return physical;
    }
    let Some(r) = s.request else {
        return physical;
    };
    if frame > r.deadline || r.base != scene.base {
        s.cancel();
        return physical;
    }
    if r.committed.is_none() {
        if r.wake_sent {
            return physical;
        }
        s.request.as_mut().unwrap().wake_sent = true;
        return 0x3ef;
    }
    if let Some(committed) = r.committed {
        if scene.selected != (r.x, r.y) {
            s.cancel();
            return physical;
        }
        if r.confirm && frame >= committed + 2 {
            s.request = None;
            s.key = 0x3fe;
            s.pulse = 3;
            s.release = 4;
            return s.key;
        }
    }
    physical
}
/// A recorded virtual Right must wake the original cursor routine without
/// being mistaken for a new physical input that cancels its queued destination.
pub fn replay_keys(mem: &mut MemMap) {
    if (!mem.ffta_battle_mouse.shop.pending() && mem.ffta_battle_mouse.request.is_none() && mem.ffta_battle_mouse.menu_request.is_none() && mem.ffta_battle_mouse.world_request.is_none()
        && mem.ffta_battle_mouse.pulse == 0 && mem.ffta_battle_mouse.release == 0
        && mem.ffta_battle_mouse.buttons.is_empty() && mem.ffta_battle_mouse.button_hold==0
        && mem.ffta_battle_mouse.button_release==0) || mem.keys != RELEASED {
        return;
    }
    // Drive both the wake-up pulse and the deferred confirm pulse in native
    // and interpreter replays. The windowed runner calls `keys` every frame;
    // headless replay must mirror that state machine after a mouse event.
    mem.keys = keys(mem, RELEASED);
}
pub fn menu_select(mem: &mut MemMap, target: u8, confirm: bool) -> bool {
    let Some(scene) = MenuScene::read(mem) else { return false; };
    if target >= scene.count { return false; }
    let state = &mut mem.ffta_battle_mouse;
    if let Some(r) = state.menu_request.as_mut() {
        if r.base == scene.base && r.target == target {
            r.confirm |= confirm;
            r.deadline = mem.frames + 120;
            return true;
        }
    }
    state.cancel();
    state.menu_request = Some(MenuRequest { base: scene.base, target, confirm,last_selected:scene.selected,
        context:[0,8,10,12,14].map(|o|h(&mem.io,o).unwrap_or(0)),
        committed: (scene.selected == target).then_some(mem.frames), deadline: mem.frames+120 });
    if scene.selected != target && !scene.horizontal { state.event = Some(512+target as u16); }
    true
}
fn menu_keys(mem: &mut MemMap, scene: MenuScene, physical: u16) -> u16 {
    let frame = mem.frames;
    let state = &mut mem.ffta_battle_mouse;
    state.request = None;
    if state.pulse > 0 { state.pulse -= 1; return state.key; }
    if state.release > 0 { state.release -= 1; return physical; }
    let Some(r) = state.menu_request else { return physical; };
    if r.base != scene.base || frame > r.deadline || r.target >= scene.count {
        state.cancel(); return physical;
    }
    if (scene.binary || scene.polling) && r.committed.is_none() && scene.selected==r.target {
        state.menu_request.as_mut().unwrap().committed=Some(frame);
        state.release=2;
        return physical;
    }
    if scene.polling && r.committed.is_none() && scene.selected!=r.last_selected {
        // These menus consume press edges instead of repeating held input.
        // Release after every observed original step before asking for another.
        state.menu_request.as_mut().unwrap().last_selected=scene.selected;
        state.release=4;
        return physical;
    }
    if let Some(committed) = r.committed {
        if frame < committed+2 { return physical; }
        if scene.selected != r.target { state.cancel(); return physical; }
        state.menu_request = None;
        if r.confirm {
            state.key = 0x3fe; state.pulse = 3; state.release = 4;
            return state.key;
        }
    } else {
        // The menu polls on its own update cadence. Hold until its first
        // verified commit, then the entry hook suppresses any repeated step.
        return if scene.horizontal {if r.target>scene.selected {0x3ef}else{0x3df}}else if scene.polling && scene.selected>r.target {0x3bf}else{0x37f};
    }
    physical
}
impl Machine {
    fn ffta_shop_cursor_entry(&mut self) {
        let s=&self.bus.ffta_battle_mouse;
        let Some(p)=s.menu_profile.filter(|_|s.enabled)else{return;};
        let pc=(p.cursor&!1)+0x130;
        if self.cpu.regs[15]!=pc {return;}
        let Some(request)=s.menu_request.filter(|r|r.committed.is_none())else{return;};
        let Some(scene)=MenuScene::shop_menu(&self.bus).or_else(||MenuScene::shop_inventory(&self.bus))else{return;};
        if scene.base!=request.base || request.target>=scene.count || self.bus.frames>request.deadline
            || self.cpu.regs[0]!=0x02000000+scene.base as u32 {return;}
        let off=(pc-0x08000000)as usize;
        let caller=(self.cpu.regs[14]&!1).checked_sub(0x0800000a).map(|o|o as usize);
        if self.bus.rom.get(off..off+12)!=Some(&[0x09,0x06,0x09,0x0e,0x41,0x74,0x09,0x01,0xc1,0x81,0x70,0x47])
            || caller.and_then(|o|self.bus.rom.get(o..o+6))!=Some(&[0x48,0x46,0x4c,0x30,0x21,0x0e])
            || self.cpu.regs[9].checked_add(0x4c)!=Some(self.cpu.regs[0])
            || self.cpu.regs[6]!=self.cpu.regs[1] || self.cpu.regs[4]>>24!=self.cpu.regs[1] {return;}
        self.cpu.regs[1]=request.target as u32;
        self.cpu.regs[6]=request.target as u32;
        self.cpu.regs[4]=(request.target as u32)<<24;
        self.bus.ffta_battle_mouse.menu_request.as_mut().unwrap().committed=Some(self.bus.frames);
    }
    fn ffta_pub_cursor_entry(&mut self) {
        let s=&self.bus.ffta_battle_mouse;
        let Some(p)=s.menu_profile.filter(|_|s.enabled)else{return;};
        let pc=(p.cursor&!1)+0x130;
        if self.cpu.regs[15]!=pc {return;}
        let Some(request)=s.menu_request.filter(|r|r.committed.is_none())else{return;};
        let Some(scene)=MenuScene::pub_menu(&self.bus)else{return;};
        if scene.base!=request.base || request.target>=scene.count || self.bus.frames>request.deadline
            || self.cpu.regs[0]!=0x02000000+scene.base as u32 {return;}
        let off=(pc-0x08000000)as usize;
        let caller=(self.cpu.regs[14]&!1).checked_sub(0x08000000).map(|v|v as usize);
        // Original setter writes the cursor index and its row offset. Its
        // verified caller subsequently publishes the same index from r6.
        if self.bus.rom.get(off..off+12)!=Some(&[0x09,0x06,0x09,0x0e,0x41,0x74,0x09,0x01,0xc1,0x81,0x70,0x47])
            || caller.and_then(|o|self.bus.rom.get(o..o+8))!=Some(&[0x20,0x1c,0x3c,0x30,0x31,0x16,0x0b,0x22])
            || self.cpu.regs[6]>>24!=self.cpu.regs[1]&255 {return;}
        self.cpu.regs[1]=request.target as u32;
        self.cpu.regs[6]=(self.cpu.regs[6]&0x00ffffff)|((request.target as u32)<<24);
        self.bus.ffta_battle_mouse.menu_request.as_mut().unwrap().committed=Some(self.bus.frames);
    }
    fn ffta_shared_list_entry(&mut self) {
        let s=&self.bus.ffta_battle_mouse;
        let Some(p)=s.menu_profile.filter(|_|s.enabled)else{return;};
        let pc=if p.cursor==0x08018fc5 {0x08018cac}else{0x08017b68};
        if self.cpu.regs[15]!=pc{return;}
        let Some(r)=s.menu_request else{return;};
        let offset=(pc-0x08000000)as usize;
        if self.bus.rom.get(offset..offset+12)!=Some(&[0x10,0xb5,3,0x1c,9,6,9,0x16,0x58,0x8b,9,0x18]){return;}
        let Some(scene)=MenuScene::read(&self.bus)else{return;};
        if !scene.polling || scene.base!=r.base || self.cpu.regs[0]!=0x02000000+scene.base as u32-0x20{return;}
        // The original selector adds a signed delta, clamps it, then its
        // caller updates the model, redraws the row and publishes the cursor.
        self.cpu.regs[1]=if r.committed.is_some(){0}else{(r.target as i32-scene.selected as i32)as u32};
    }
    fn ffta_world_mouse_entry(&mut self) {
        let s=&self.bus.ffta_battle_mouse;
        let Some(p)=s.world_profile.filter(|_|s.enabled)else{return;};
        let pc=self.cpu.regs[15];
        if pc!=p.commit && pc!=p.end-8{return;}
        let Some(r)=s.world_request else{return;};
        let Some((base,_,scroll))=world_cursor(&self.bus)else{return;};
        if r.base!=base || self.cpu.regs[4]!=0x02000000+base as u32 || self.bus.frames>r.deadline{return;}
        self.cpu.regs[0]=if pc==p.commit{r.x as u32}else{(r.y as u32+scroll as u32).clamp(22,330)};
        if pc==p.end-8 && r.committed.is_none() {self.bus.ffta_battle_mouse.world_request.as_mut().unwrap().committed=Some(self.bus.frames);}
    }
    pub fn ffta_battle_mouse_entry(&mut self) {
        if !self.cpu.thumb() {
            return;
        }
        self.ffta_world_mouse_entry();
        crate::ffta_shop_mouse::entry(self);
        self.ffta_shop_cursor_entry();
        self.ffta_pub_cursor_entry();
        self.ffta_shared_list_entry();
        self.ffta_battle_menu_entry();
        let s = &self.bus.ffta_battle_mouse;
        let Some(p) = s.profile.filter(|_| s.enabled) else {
            return;
        };
        if self.cpu.regs[15] != p.commit {
            return;
        }
        let Some(r) = s.request.filter(|r| r.committed.is_none()) else {
            return;
        };
        let Some(scene) = Scene::read(&self.bus) else {
            self.bus.ffta_battle_mouse.cancel();
            return;
        };
        let sp = self.cpu.regs[13];
        let base = if sp >> 24 == 3 {
            w(&self.bus.iwram, (sp & 0x7fff) as usize)
        } else {
            None
        };
        if base != Some(0x02000000 + r.base as u32)
            || scene.base != r.base
            || self.bus.frames > r.deadline
        {
            self.bus.ffta_battle_mouse.cancel();
            return;
        }
        self.cpu.regs[10] = r.x as u32;
        self.cpu.regs[9] = r.y as u32;
        self.bus
            .ffta_battle_mouse
            .request
            .as_mut()
            .unwrap()
            .committed = Some(self.bus.frames);
        if crate::ffta_log::enabled() || std::env::var("FFTA_MOUSE_VERIFY").as_deref() == Ok("1") {
            eprintln!(
                "FFTA_MOUSE_BATTLE frame={} target={},{} pc={:08x} original_cursor_commit=true",
                self.bus.frames, r.x, r.y, p.commit
            );
        }
    }
    fn ffta_battle_menu_entry(&mut self) {
        let state = &self.bus.ffta_battle_mouse;
        let Some(p) = state.menu_profile.filter(|_| state.enabled) else { return; };
        if self.cpu.regs[15] == p.start {
            if let Some(r)=state.menu_request {
                if self.cpu.regs[0] == 0x02000000+r.base as u32 {
                    self.cpu.regs[1] &= !0xc0;
                    if r.committed.is_none() { self.cpu.regs[1] |= 0x80; }
                }
            }
            return;
        }
        if self.cpu.regs[15] != p.commit { return; }
        let Some(r) = state.menu_request.filter(|r| r.committed.is_none()) else { return; };
        let Some(scene) = MenuScene::read(&self.bus) else { self.bus.ffta_battle_mouse.cancel(); return; };
        if scene.base != r.base || self.cpu.regs[5] != 0x02000000+r.base as u32
            || self.cpu.regs[6] != self.cpu.regs[5]+0x58 || r.target >= scene.count
            || self.bus.frames > r.deadline { return; }
        self.cpu.regs[4] = r.target as u32;
        self.bus.ffta_battle_mouse.menu_request.as_mut().unwrap().committed = Some(self.bus.frames);
        if std::env::var("FFTA_MOUSE_VERIFY").as_deref() == Ok("1") {
            eprintln!("FFTA_MOUSE_MENU frame={} base={:08x} count={} target={} pc={:08x} original_commit=true",
                self.bus.frames,0x02000000+r.base as u32,scene.count,r.target,p.commit);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Machine {
        let mut m = Machine::new(vec![0; 192]);
        m.bus.ffta_battle_mouse = State {
            enabled: true,
            profile: Some(Profile {
                base: 0xf3ac,
                start: 0x08024ad8,
                commit: 0x08025132,
                end: 0x0802516e,
            }),
            ..Default::default()
        };
        for (o, v) in [(0, 0x1f40u16), (12, 0x4c03), (14, 0x4e02)] {
            m.bus.io[o..o + 2].copy_from_slice(&v.to_le_bytes());
        }
        m.bus.ewram[0xf3ac..0xf3b0].copy_from_slice(&0x02010000u32.to_le_bytes());
        m.bus.ewram[0xf3b0] = 1;
        m.bus.ewram[0x7f14..0x7f18].copy_from_slice(&0x02030000u32.to_le_bytes());
        m.bus.ewram[0x7f18] = 4;
        m.bus.ewram[0x7f19] = 3;
        m.bus.ewram[0x7f1d] = 4;
        m.bus.ewram[0x7f1f] = 3;
        for i in 0..12 {
            m.bus.ewram[0x30000 + i * 2] = 2;
        }
        m.cpu.cpsr = 0x3f;
        m.cpu.regs[13] = 0x03007000;
        m.bus.iwram[0x7000..0x7004].copy_from_slice(&0x0200f3acu32.to_le_bytes());
        m
    }
    #[test]
    fn dimensions_diamonds_heights_and_invalid_cells() {
        let mut m = fixture();
        let scene = Scene::read(&m.bus).unwrap();
        assert_eq!((scene.width, scene.height, scene.cells.len()), (4, 3, 12));
        for c in scene.cells {
            let hit = Scene::read(&m.bus)
                .unwrap()
                .hit(c.center[0], c.center[1])
                .unwrap();
            assert_eq!((hit.x, hit.y), (c.x, c.y));
        }
        let old = Scene::read(&m.bus).unwrap().cells[6];
        m.bus.ewram[0x30000 + 6 * 2] = 4;
        let high = Scene::read(&m.bus).unwrap().cells[6];
        assert_eq!(high.center[0], old.center[0]);
        assert_eq!(high.center[1], old.center[1] - 16.);
        m.bus.ewram[0x30000 + 6 * 2] = 0;
        assert!(!select(&mut m.bus, 2, 1, false));
        assert!(!select(&mut m.bus, 4, 0, false));
        assert!(!aim(&mut m.bus, f32::NAN, 0., true));
        assert!(!aim(&mut m.bus, 10000., 10000., true));
    }
    #[test]
    fn hover_queues_target_and_only_original_commit_point_changes_registers() {
        let mut m = fixture();
        let before = m.bus.ewram.clone();
        assert!(select(&mut m.bus, 3, 2, false));
        assert_eq!(m.bus.ewram, before);
        assert_eq!(m.bus.ffta_battle_mouse.take_event(), Some(291));
        assert_eq!(keys(&mut m.bus, RELEASED), 0x3ef);
        m.cpu.regs[15] = 0x08025130;
        m.ffta_battle_mouse_entry();
        assert_eq!(m.cpu.regs[10], 0);
        m.cpu.regs[15] = 0x08025132;
        m.ffta_battle_mouse_entry();
        assert_eq!((m.cpu.regs[10], m.cpu.regs[9]), (3, 2));
        assert_eq!(m.bus.ewram, before);
        m.bus.ewram[0xf3b8..0xf3ba].copy_from_slice(&3u16.to_le_bytes());
        m.bus.ewram[0xf3bc..0xf3be].copy_from_slice(&2u16.to_le_bytes());
        for f in 1..10 {
            m.bus.frames = f;
            assert_eq!(keys(&mut m.bus, RELEASED), RELEASED);
        }
    }
    #[test]
    fn click_confirms_once_after_commit_and_physical_input_cancels() {
        let mut m = fixture();
        assert!(select(&mut m.bus, 0, 0, true));
        let mut output = Vec::new();
        for f in 0..12 {
            m.bus.frames = f;
            output.push(keys(&mut m.bus, RELEASED));
        }
        assert_eq!(&output[..2], &[RELEASED; 2]);
        assert_eq!(&output[2..6], &[0x3fe; 4]);
        assert!(output[6..].iter().all(|&k| k == RELEASED));
        assert!(select(&mut m.bus, 3, 2, true));
        assert_eq!(keys(&mut m.bus, 0x3df), 0x3df);
        assert!(m.bus.ffta_battle_mouse.request.is_none());
    }
    #[test]
    fn scene_change_or_wrong_cursor_stack_never_applies_pending_target() {
        let mut m = fixture();
        select(&mut m.bus, 3, 2, false);
        m.cpu.regs[15] = 0x08025132;
        m.bus.iwram[0x7000] ^= 4;
        m.ffta_battle_mouse_entry();
        assert_eq!(m.cpu.regs[10], 0);
        assert!(m.bus.ffta_battle_mouse.request.is_none());
        m.bus.ewram[0xf3b0] = 0;
        assert!(Scene::read(&m.bus).is_none());
        assert!(!select(&mut m.bus, 0, 0, true));
    }
    fn menu_fixture() -> Machine {
        let mut m=fixture();
        m.bus.ffta_battle_mouse.menu_profile=Some(MenuProfile {
            start:0x08028708,commit:0x080287a0,end:0x080287c0,cursor:0x08017e81,
        });
        for (o,v) in [(8,0x4c03u16),(10,0x4e02),(12,0x5c08),(14,0x4a09)] {
            m.bus.io[o..o+2].copy_from_slice(&v.to_le_bytes());
        }
        menu_instance(&mut m,0x2dd70,4,0,84,false);
        m
    }
    fn menu_instance(m:&mut Machine,base:usize,count:u8,selected:u8,y:u16,child:bool) {
        m.bus.ewram[base+9]=count;
        m.bus.ewram[base+0x58..base+0x5c].copy_from_slice(&0x08017e81u32.to_le_bytes());
        m.bus.ewram[base+0x5c..base+0x60].copy_from_slice(&(0x02000000+base as u32+0x38).to_le_bytes());
        for (o,v) in [(base+0x60,160u16),(base+0x62,y),(base+0x66,selected as u16*16)] {
            m.bus.ewram[o..o+2].copy_from_slice(&v.to_le_bytes());
        }
        if child { m.bus.ewram[base+4..base+8].copy_from_slice(&0x0202dd58u32.to_le_bytes()); }
        m.bus.ewram[base+0x68]=7;m.bus.ewram[base+0x69]=selected;
        for (o,v) in [(0,y+selected as u16*16+1),(2,175),(4,0x125)] {
            m.bus.oam[o..o+2].copy_from_slice(&v.to_le_bytes());
        }
    }
    #[test]
    fn shared_layout_scan_matches_queries_and_observes_same_frame_ram_changes() {
        for callback in [0x08017e81u32,0x08018fc5] {
            let mut m=menu_fixture();m.bus.ewram.fill(0);
            m.bus.ffta_battle_mouse.menu_profile.as_mut().unwrap().cursor=callback;
            m.bus.io[12..14].copy_from_slice(&0x0b08u16.to_le_bytes());
            m.bus.io[14..16].copy_from_slice(&0x0a08u16.to_le_bytes());
            let list=0x25000;let cursor=list+0x20;
            m.bus.ewram[cursor..cursor+4].copy_from_slice(&callback.to_le_bytes());
            m.bus.ewram[cursor+4..cursor+8].copy_from_slice(&(0x02000000u32+list as u32).to_le_bytes());
            m.bus.ewram[list+0x13]=1;m.bus.ewram[list+0x15]=10;
            m.bus.ewram[cursor+10..cursor+12].copy_from_slice(&92u16.to_le_bytes());
            for flags in 0..16 {for count in [0,2,3,4,5,12,13] {
                m.bus.ewram[cursor+16]=flags;m.bus.ewram[list+0x16]=count;
                let expected=(MenuScene::site_menu(&m.bus),MenuScene::pub_menu(&m.bus),MenuScene::shop_menu(&m.bus));
                assert_eq!(MenuScene::layout_menus(&m.bus),expected,"callback={callback:x} flags={flags} count={count}");
            }}
        }
    }
    #[test]
    fn bank_window_uses_its_own_bounds_and_survives_cursor_blink_in_both_regions() {
        for europe in [false,true] {
            let mut m=menu_fixture();m.bus.ewram.fill(0);m.bus.oam.fill(0);
            let(callback,handler,updater,stack)=if europe {(0x08018fc5u32,0x5aee4,0x5e7b4,0x85)}else{(0x08017e81u32,0x58d40,0x5c594,0x83)};
            m.bus.ffta_battle_mouse.menu_profile.as_mut().unwrap().cursor=callback;
            m.bus.rom.resize(0x60000,0);
            m.bus.rom[handler..handler+12].copy_from_slice(&[0xf0,0xb5,stack,0xb0,0x0e,0x1c,0,6,5,0x0e,0x31,0x78]);
            m.bus.rom[updater..updater+12].copy_from_slice(&[0xf0,0xb5,0x57,0x46,0x4e,0x46,0x45,0x46,0xe0,0xb4,0x8e,0xb0]);
            for(o,v)in[(0,0x3f40u16),(12,0x0b08),(14,0x0a08)] {m.bus.io[o..o+2].copy_from_slice(&v.to_le_bytes());}
            let base=0x23c4c;
            for(o,v)in[(base,0x08000001+handler as u32),(base+4,0x08000001+updater as u32),(base+0x58,callback),(base+0x5c,0x02000000+base as u32+0x38)] {m.bus.ewram[o..o+4].copy_from_slice(&v.to_le_bytes());}
            m.bus.ewram[base+9]=2;m.bus.ewram[base+0x1e]=6;m.bus.ewram[base+0x68]=15;
            m.bus.ewram[base+0x60..base+0x62].copy_from_slice(&24u16.to_le_bytes());
            m.bus.ewram[base+0x62..base+0x64].copy_from_slice(&92u16.to_le_bytes());
            let scene=MenuScene::read(&m.bus).unwrap();
            assert_eq!((scene.origin,scene.right),([0,92],48));
            assert_eq!(scene.hit(12.,98.),Some(0));assert_eq!(scene.hit(12.,116.),Some(1));
            assert_eq!(scene.hit(60.,116.),None);
            assert!(aim(&mut m.bus,12.,116.,false));
            assert_eq!(m.bus.ffta_battle_mouse.menu_request.unwrap().target,1);
            m.bus.ffta_battle_mouse.set_enabled(false);
            assert!(MenuScene::bank_menu(&m.bus).is_some()); // visual fix also works without mouse
            assert_eq!(MenuScene::layout_menus(&m.bus).0,MenuScene::site_menu(&m.bus));
            let list_pc=if europe{0x5b624}else{0x59454};
            let sig=if europe{[0xf0,0xb5,0x57,0x46,0x4e,0x46,0x45,0x46,0xe0,0xb4,0x86,0xb0]}
                else{[0xf0,0xb5,0x4f,0x46,0x46,0x46,0xc0,0xb4,0x84,0xb0,0x0f,0x1c]};
            m.bus.rom[list_pc..list_pc+12].copy_from_slice(&sig);
            m.bus.ewram[base..base+4].copy_from_slice(&(0x08000001u32+list_pc as u32).to_le_bytes());
            m.bus.ewram[base+0x68]=0;
            assert!(MenuScene::site_menu(&m.bus).is_some());
            assert_eq!(MenuScene::layout_menus(&m.bus).0,MenuScene::site_menu(&m.bus));
            m.bus.rom[list_pc]^=1;
            assert!(MenuScene::site_menu(&m.bus).is_none());
            m.bus.rom[handler]^=1;assert!(MenuScene::bank_menu(&m.bus).is_none());
        }
    }
    #[test]
    fn town_service_parent_uses_owner_geometry_and_rejects_unowned_cursors() {
        for callback in [0x08017e81u32] {for flags in [7,15] {
            let mut m=menu_fixture();m.bus.ewram.fill(0);
            m.bus.rom.resize(0x1000,0);
            m.bus.ffta_battle_mouse.menu_profile.as_mut().unwrap().cursor=callback;
            for(o,v)in[(12,0x0b08u16),(14,0x0a08)]{m.bus.io[o..o+2].copy_from_slice(&v.to_le_bytes());}
            m.bus.rom[0x800..0x802].copy_from_slice(&0xb5f0u16.to_le_bytes());
            m.bus.rom[0x900..0x902].copy_from_slice(&0xb570u16.to_le_bytes());
            let base=0x24000;let cursor=base+0x58;
            for(o,v)in[(base,0x08000801u32),(base+4,0x08000901),(cursor,callback),(cursor+4,0x02000000+base as u32+0x38)]{
                m.bus.ewram[o..o+4].copy_from_slice(&v.to_le_bytes());
            }
            m.bus.ewram[base+9]=3;m.bus.ewram[base+0x1c]=2;m.bus.ewram[base+0x1e]=10;
            m.bus.ewram[cursor+16]=flags;
            m.bus.ewram[cursor+10..cursor+12].copy_from_slice(&60u16.to_le_bytes());
            let scene=MenuScene::read(&m.bus).unwrap();
            assert_eq!((scene.origin,scene.right),([16,60],96));
            assert_eq!(scene.hit(30.,98.),Some(2));
            assert!(aim(&mut m.bus,30.,98.,false));
            assert_eq!(m.bus.ffta_battle_mouse.menu_request.unwrap().target,2);
            assert!(MenuScene::site_menu(&m.bus).is_some());
            m.bus.ewram[cursor+4..cursor+8].fill(0);
            assert!(MenuScene::parent_window(&m.bus).is_none());
        }}
    }
    #[test]
    fn usa_pub_root_detaches_from_dialogue_without_guest_buffer_changes() {
        use crate::{mem,ppu,ffta_dialogue::{PanelKind,LAYER_WIDTH,LAYER_INSET}};
        mem::set_extra(22);mem::set_extra_y(0);
        let mut m=menu_fixture();m.bus.ewram.fill(0);
        m.bus.ffta_movement.enabled=true;
        m.bus.ffta_bg_edge=crate::ppu::FftaBgEdge::Transparent;
        for(o,v)in[(12,0x0b08u16),(14,0x0a08)]{m.bus.io[o..o+2].copy_from_slice(&v.to_le_bytes());}
        let list=0x25000;let cursor=list+0x20;
        for(o,v)in[(cursor,0x08017e81u32),(cursor+4,0x02000000+list as u32)]{m.bus.ewram[o..o+4].copy_from_slice(&v.to_le_bytes());}
        m.bus.ewram[list+0x13]=1;m.bus.ewram[list+0x15]=10;m.bus.ewram[list+0x16]=4;
        m.bus.ewram[cursor+10..cursor+12].copy_from_slice(&92u16.to_le_bytes());m.bus.ewram[cursor+16]=15;
        m.bus.oam.fill(0);for o in m.bus.oam.chunks_exact_mut(8){o[..2].copy_from_slice(&0x200u16.to_le_bytes());}
        for(o,v)in[(0,93u16),(2,87),(4,0x127)]{m.bus.oam[o..o+2].copy_from_slice(&v.to_le_bytes());}
        m.bus.vram.fill(0);m.bus.vram[0x8020..0x8040].fill(0x11);
        m.bus.vram[0x124e0..0x12500].fill(0x11);
        m.bus.palette[2..4].copy_from_slice(&31u16.to_le_bytes());
        m.bus.palette[0x202..0x204].copy_from_slice(&0x3e0u16.to_le_bytes());
        for (cols,rows) in [(0..12,10..20),(12..30,2..4)] {for y in rows {for x in cols.clone(){
            let o=0x5000+(y*32+x)*2;m.bus.vram[o..o+2].copy_from_slice(&1u16.to_le_bytes());
        }}}
        for y in 0..160 {ppu::render_scanline(&mut m.bus,y);}
        let hardware=m.bus.framebuffer.clone();let oam=m.bus.oam.clone();let ram=m.bus.ewram.clone();
        m.bus.ffta_dialogue.separate=true;
        for y in 0..160 {ppu::render_scanline(&mut m.bus,y);}
        let s=&m.bus.ffta_dialogue;assert!(s.town_layout);assert_eq!(s.extra.kind,PanelKind::Actions);
        let at=|x,y|y*LAYER_WIDTH+x+LAYER_INSET as usize;
        assert_eq!(s.extra.pixels[at(8,96)].resolve(0),31);
        assert!(!s.pixels[at(8,96)].visible());
        assert_eq!(s.extra.pixels[at(87,93)].resolve(0),0x3e0);
        assert_eq!(s.pixels[at(160,16)].resolve(0),31);
        assert!(!s.extra.pixels[at(160,16)].visible());
        assert_eq!(m.bus.framebuffer,hardware);assert_eq!(m.bus.oam,oam);assert_eq!(m.bus.ewram,ram);
        mem::set_extra(0);mem::set_extra_y(0);
    }
    #[test]
    fn town_list_keeps_mouse_ownership_when_arrow_blinks() {
        let mut m=menu_fixture();
        m.bus.io[12..14].copy_from_slice(&0x0b08u16.to_le_bytes());
        m.bus.io[14..16].copy_from_slice(&0x0a08u16.to_le_bytes());
        m.bus.ewram.fill(0);m.bus.oam.fill(0);
        let list=0x25000;let cursor=list+0x20;
        m.bus.ewram[cursor..cursor+4].copy_from_slice(&0x08017e81u32.to_le_bytes());
        m.bus.ewram[cursor+4..cursor+8].copy_from_slice(&(0x02000000u32+list as u32).to_le_bytes());
        m.bus.ewram[list+0x13]=7;m.bus.ewram[list+0x15]=16;m.bus.ewram[list+0x16]=8;
        m.bus.ewram[cursor+0xa..cursor+0xc].copy_from_slice(&20u16.to_le_bytes());
        m.bus.ewram[cursor+0x10]=3;m.bus.ewram[cursor+0x11]=2;
        let scene=MenuScene::read(&m.bus).unwrap();
        assert_eq!((scene.count,scene.selected,scene.origin,scene.right),(8,2,[56,20],200));
        assert!(MenuScene::pub_menu(&m.bus).is_none());
        assert_eq!(scene.hit(100.,133.),Some(7));
        assert!(aim(&mut m.bus,100.,133.,false));
        assert_eq!(m.bus.ffta_battle_mouse.menu_request.unwrap().target,7);
        // Pub MENU: Rumors/Missions/Quit Mission/Leave, unlike the LIST submenu.
        m.bus.ffta_battle_mouse.cancel();
        m.bus.ewram[list+0x13]=1;m.bus.ewram[list+0x15]=9;m.bus.ewram[list+0x16]=4;
        m.bus.ewram[cursor+0xa..cursor+0xc].copy_from_slice(&92u16.to_le_bytes());
        m.bus.ewram[cursor+0x10]=15;m.bus.ewram[cursor+0x11]=0;
        let menu=MenuScene::read(&m.bus).unwrap();
        assert_eq!((menu.count,menu.origin,menu.right),(4,[8,92],96));
        assert!(MenuScene::pub_menu(&m.bus).is_some());
        // Labels determine the original window width. Do not pin English's
        // width: translated labels must retain the same root-panel ownership.
        for width in 9..=25 {
            m.bus.ewram[list+0x15]=width;
            for flags in [7,15] {
            m.bus.ewram[cursor+0x10]=flags;
            let root=MenuScene::pub_menu(&m.bus).unwrap();
            assert_eq!(root.right,(8+(width as i32+2)*8).min(240));
            }
        }
        m.bus.ewram[list+0x15]=9;
        assert!(aim(&mut m.bus,40.,145.,false));
        assert_eq!(m.bus.ffta_battle_mouse.menu_request.unwrap().target,3);
        // Monster Bank uses the same active root-window style, but a different
        // placement and row count from Pub/Shop. Keep list selection direct.
        m.bus.ffta_battle_mouse.cancel();
        m.bus.ewram[list+0x13]=2;m.bus.ewram[list+0x15]=10;m.bus.ewram[list+0x16]=5;
        m.bus.ewram[cursor+0xa..cursor+0xc].copy_from_slice(&36u16.to_le_bytes());
        m.bus.ewram[cursor+0x10]=7;m.bus.ewram[cursor+0x11]=0;
        let site=MenuScene::read(&m.bus).unwrap();
        assert_eq!((site.count,site.origin),(5,[16,36]));
        assert!(MenuScene::site_menu(&m.bus).is_some());
        assert!(MenuScene::pub_menu(&m.bus).is_none());
        assert!(aim(&mut m.bus,40.,108.,false));
        assert_eq!(m.bus.ffta_battle_mouse.menu_request.unwrap().target,4);
        m.bus.io[12..14].copy_from_slice(&0x5c08u16.to_le_bytes());
        assert!(MenuScene::town_menu(&m.bus).is_none());
    }
    #[test]
    fn shop_root_and_inventory_keep_direct_selection_and_quantity_ownership() {
        for callback in [0x08017e81u32,0x08018fc5] {
            let mut m=menu_fixture();m.bus.ewram.fill(0);
            m.bus.ffta_battle_mouse.menu_profile.as_mut().unwrap().cursor=callback;
            m.bus.io[12..14].copy_from_slice(&0x0b08u16.to_le_bytes());
            m.bus.io[14..16].copy_from_slice(&0x0a08u16.to_le_bytes());
            let list=0x25000;let cursor=list+0x20;
            m.bus.ewram[cursor..cursor+4].copy_from_slice(&callback.to_le_bytes());
            m.bus.ewram[cursor+4..cursor+8].copy_from_slice(&(0x02000000u32+list as u32).to_le_bytes());
            m.bus.ewram[list+0x13]=1;m.bus.ewram[list+0x15]=7;m.bus.ewram[list+0x16]=3;
            m.bus.ewram[cursor+10..cursor+12].copy_from_slice(&92u16.to_le_bytes());
            m.bus.ewram[cursor+16]=15;
            assert!(MenuScene::shop_menu(&m.bus).is_some());
            assert!(MenuScene::pub_menu(&m.bus).is_none());
            let pc=(callback&!1)+0x130;let off=(pc-0x08000000)as usize;
            m.bus.rom.resize(0x72000,0);
            m.bus.rom[off..off+12].copy_from_slice(&[0x09,0x06,0x09,0x0e,0x41,0x74,0x09,0x01,0xc1,0x81,0x70,0x47]);
            m.bus.rom[0x2b360..0x2b366].copy_from_slice(&[0x48,0x46,0x4c,0x30,0x21,0x0e]);
            assert!(menu_select(&mut m.bus,2,false));
            m.cpu.regs[0]=0x02000000+cursor as u32;m.cpu.regs[1]=1;
            m.cpu.regs[4]=0x01000000;m.cpu.regs[6]=1;
            m.cpu.regs[9]=m.cpu.regs[0]-0x4c;m.cpu.regs[14]=0x0802b36b;m.cpu.regs[15]=pc;
            let before=m.bus.ewram.clone();m.ffta_battle_mouse_entry();
            assert_eq!((m.cpu.regs[1],m.cpu.regs[4],m.cpu.regs[6]),(2,0x02000000,2));
            assert_eq!(m.bus.ewram,before);
            for _ in 0..5 {m.step();}
            assert_eq!(m.bus.ewram[cursor+17],2);
            m.bus.ffta_battle_mouse.cancel();
            let inventory=if callback==0x08018fc5 {0x71030}else{0x6eb48};
            m.bus.rom[inventory..inventory+12].copy_from_slice(&[0xf0,0xb5,0x87,0xb0,6,0x1c,0x11,0x4f,0x39,0x68,0x11,0x4a]);
            m.bus.ewram[cursor..cursor+4].copy_from_slice(&(0x08000001+inventory as u32).to_le_bytes());
            m.bus.ewram[list+0x13]=2;m.bus.ewram[list+0x15]=24;m.bus.ewram[list+0x16]=4;
            m.bus.ewram[list+0x18..list+0x1a].copy_from_slice(&3u16.to_le_bytes());
            m.bus.ewram[cursor+10..cursor+12].copy_from_slice(&20u16.to_le_bytes());
            m.bus.ewram[cursor+16]=3;m.bus.ewram[cursor+17]=0;
            let items=MenuScene::read(&m.bus).unwrap();
            assert_eq!((items.count,items.origin,items.right),(3,[16,20],224));
            assert_eq!(items.hit(100.,54.),Some(2));
            m.bus.io[..2].copy_from_slice(&0x5f40u16.to_le_bytes());
            assert!(MenuScene::read(&m.bus).is_none()); // quantity owns A/B
            m.bus.io[..2].copy_from_slice(&0x1f40u16.to_le_bytes());
            m.bus.ewram[list+0x18]=5;assert_eq!(MenuScene::read(&m.bus).unwrap().count,4);
            m.bus.ewram[list+0x1a]=1;m.bus.ewram[cursor+17]=3;
            assert_eq!(MenuScene::read(&m.bus).unwrap().selected,3);
            m.bus.ewram[list+0x1a]=5;assert!(MenuScene::read(&m.bus).is_none());
        }
    }
    #[test]
    fn pub_direct_commit_updates_cursor_and_callers_model_for_both_profiles() {
        for callback in [0x08017e81u32,0x08018fc5] {
            let mut m=menu_fixture();m.bus.ewram.fill(0);
            m.bus.ffta_battle_mouse.menu_profile.as_mut().unwrap().cursor=callback;
            m.bus.io[12..14].copy_from_slice(&0x0b08u16.to_le_bytes());
            m.bus.io[14..16].copy_from_slice(&0x0a08u16.to_le_bytes());
            let list=0x25000;let cursor=list+0x20;
            m.bus.ewram[cursor..cursor+4].copy_from_slice(&callback.to_le_bytes());
            m.bus.ewram[cursor+4..cursor+8].copy_from_slice(&(0x02000000u32+list as u32).to_le_bytes());
            m.bus.ewram[list+0x13]=1;m.bus.ewram[list+0x15]=12;m.bus.ewram[list+0x16]=4;
            m.bus.ewram[cursor+10..cursor+12].copy_from_slice(&92u16.to_le_bytes());
            m.bus.ewram[cursor+16]=15;
            let pc=(callback&!1)+0x130;let off=(pc-0x08000000)as usize;
            m.bus.rom.resize(0x62000,0);
            m.bus.rom[off..off+12].copy_from_slice(&[0x09,0x06,0x09,0x0e,0x41,0x74,0x09,0x01,0xc1,0x81,0x70,0x47]);
            m.bus.rom[0x61c96..0x61c9e].copy_from_slice(&[0x20,0x1c,0x3c,0x30,0x31,0x16,0x0b,0x22]);
            assert!(menu_select(&mut m.bus,2,false));
            m.cpu.regs[0]=0x02000000+cursor as u32;m.cpu.regs[1]=1;
            m.cpu.regs[6]=0x01000000;m.cpu.regs[14]=0x08061c97;m.cpu.regs[15]=pc;
            assert!(m.bus.ffta_battle_mouse.interprets(pc));
            let before=m.bus.ewram.clone();
            m.ffta_battle_mouse_entry();
            assert_eq!((m.cpu.regs[1],m.cpu.regs[6]),(2,0x02000000));
            assert_eq!(m.bus.ewram,before); // guest instructions perform the stores
            for _ in 0..5 {m.step();}
            assert_eq!(m.bus.ewram[cursor+17],2);
            assert_eq!(h(&m.bus.ewram,cursor+14),Some(32));
            m.bus.ffta_battle_mouse.cancel();m.bus.ewram[cursor+17]=0;
            assert!(menu_select(&mut m.bus,2,false));
            m.bus.rom[off]^=1;m.cpu.regs[15]=pc;m.cpu.regs[1]=1;m.cpu.regs[6]=0x01000000;
            m.ffta_battle_mouse_entry();assert_eq!(m.cpu.regs[1],1);
            assert!(m.bus.ffta_battle_mouse.menu_request.unwrap().committed.is_none());
        }
    }
    #[test]
    fn menu_uses_live_count_and_leaf_cursor_not_option_names() {
        let mut m=menu_fixture();
        let menu=MenuScene::read(&m.bus).unwrap();
        assert_eq!((menu.count,menu.selected,menu.origin),(4,0,[160,84]));
        for row in 0..4 { assert_eq!(menu.hit(195.,85.+row as f32*16.),Some(row)); }
        assert_eq!(menu.hit(195.,149.),None);
        assert_eq!(menu.hit(f32::NAN,90.),None);
        menu_instance(&mut m,0x2dd70,4,3,84,false);
        menu_instance(&mut m,0x2def4,1,0,132,true);
        let child=MenuScene::read(&m.bus).unwrap();
        assert_eq!((child.base,child.count),(0x2def4,1));
        assert!(!menu_select(&mut m.bus,1,true));
        m.bus.ewram[0x2def4+0x58]^=1;
        assert_eq!(MenuScene::read(&m.bus).unwrap().base,0x2dd70);
        m.bus.oam[4]=0x20;
        assert!(MenuScene::read(&m.bus).is_none());
    }
    #[test]
    fn menu_jump_is_original_register_commit_and_click_confirms_once() {
        let mut m=menu_fixture();
        let before=m.bus.ewram.clone();
        assert!(menu_select(&mut m.bus,3,true));
        assert_eq!(m.bus.ffta_battle_mouse.take_event(),Some(515));
        assert_eq!(keys(&mut m.bus,RELEASED),0x37f);
        m.cpu.regs[15]=0x080287a0;m.cpu.regs[5]=0x0202dd70;m.cpu.regs[6]=0x0202ddc8;
        m.cpu.regs[4]=1;m.ffta_battle_mouse_entry();
        assert_eq!(m.cpu.regs[4],3);
        assert_eq!(m.bus.ewram,before);
        menu_instance(&mut m,0x2dd70,4,3,84,false);
        let mut pressed=0;
        for frame in 1..12 {m.bus.frames=frame;pressed+=usize::from(keys(&mut m.bus,RELEASED)==0x3fe);}
        assert_eq!(pressed,4);
        assert!(menu_select(&mut m.bus,1,true));
        assert_eq!(keys(&mut m.bus,0x3bf),0x3bf);
        assert!(m.bus.ffta_battle_mouse.menu_request.is_none());
    }
    #[test]
    fn replay_drives_deferred_confirm_and_releases_the_whole_pulse() {
        let mut m=fixture();
        assert!(select(&mut m.bus,0,0,true));
        let mut keys_out=Vec::new();
        for f in 0..12 { m.bus.frames=f;m.bus.keys=RELEASED;replay_keys(&mut m.bus);keys_out.push(m.bus.keys); }
        assert_eq!(&keys_out[2..6],&[0x3fe;4]);
        assert!(keys_out[6..].iter().all(|&v|v==RELEASED));
    }
    #[test]
    fn notice_choices_follow_repeated_bullets_and_confirm_after_selection() {
        for second in [65,89,113] {
            let mut m=menu_fixture();
            m.bus.ffta_dialogue.extra.kind=crate::ffta_dialogue::PanelKind::Notice;
            m.bus.ffta_dialogue.extra.bounds=Some([16,16,224,64]);
            m.bus.oam.fill(0);for o in m.bus.oam.chunks_exact_mut(8){o[..2].copy_from_slice(&0x200u16.to_le_bytes());}
            for(o,v)in[(0,44u16),(2,27),(4,0x125)]{m.bus.oam[o..o+2].copy_from_slice(&v.to_le_bytes());}
            m.bus.vram.fill(0);
            // Synthetic glyph pixels, deliberately unaligned to tile boundaries.
            let mut dot=|x:i32,y:i32| {
                let tile=(y/8*32+x/8)as u16+1;
                let o=0xe000+((y/8*32+x/8)*2)as usize;
                m.bus.vram[o..o+2].copy_from_slice(&tile.to_le_bytes());
                let o=0x8000+tile as usize*32+(y%8)as usize*4+(x%8)as usize/2;
                m.bus.vram[o]|=6<<(4*(x%2));
            };
            for x in [27,second] {for yy in 1..7 {for xx in 1..7 {
                if xx==1||xx==6||yy==1||yy==6 {dot(x+xx,44+yy);}
            }}}
            for y in 44..52 {for x in second+12..second+21 {dot(x,y);}}
            let scene=MenuScene::read(&m.bus).unwrap();
            assert_eq!(scene.hit(40.,47.),Some(0));
            assert_eq!(scene.hit(second as f32+15.,47.),Some(1));
            assert_eq!(scene.hit(40.,70.),None);
            assert!(aim(&mut m.bus,second as f32+15.,47.,true));
            assert_eq!(keys(&mut m.bus,RELEASED),0x3ef);
            m.bus.oam[2..4].copy_from_slice(&(second as u16).to_le_bytes());
            assert_eq!(keys(&mut m.bus,RELEASED),RELEASED);
            let mut confirms=0;
            for f in 1..16 {m.bus.frames=f;confirms+=usize::from(keys(&mut m.bus,RELEASED)==0x3fe);}
            assert_eq!(confirms,4);
            assert_eq!(keys(&mut m.bus,RELEASED),RELEASED);
            // Informational notices without a real choice cursor stay passive.
            m.bus.oam[..2].copy_from_slice(&0x200u16.to_le_bytes());
            assert!(MenuScene::read(&m.bus).is_none());
        }
    }
    #[test]
    fn binary_confirmation_waits_for_original_selection_then_confirms_once() {
        let mut m=menu_fixture();
        m.bus.rom.resize(0x29358,0);
        m.bus.rom[0x29350..0x29358].copy_from_slice(&[0xf0,0xb5,4,0x1c,0x26,0x1c,0x58,0x36]);
        let b=0x2dd70;
        m.bus.ewram[b+8]=10;m.bus.ewram[b+9]=0;
        m.bus.ewram[b+0x5c..b+0x60].fill(0);
        for(o,v)in[(b+0x60,74u16),(b+0x62,107),(b+0x84,80),(b+0x86,104),(b+0x8c,88),(b+0x8e,120),(0x40000,0)] {
            if o<0x40000 {m.bus.ewram[o..o+2].copy_from_slice(&v.to_le_bytes());}
        }
        m.bus.oam[0..2].copy_from_slice(&108u16.to_le_bytes());
        m.bus.oam[2..4].copy_from_slice(&89u16.to_le_bytes());
        let scene=MenuScene::read(&m.bus).unwrap();assert!(scene.binary);
        assert_eq!(scene.hit(110.,110.),Some(0));assert_eq!(scene.hit(110.,126.),Some(1));
        assert!(menu_select(&mut m.bus,1,true));
        assert_eq!(keys(&mut m.bus,RELEASED),0x37f);
        // The original handler changes cursor coordinates and selection;
        // OAM can still contain its previous frame during publication.
        m.bus.ewram[b+0x69]=1;
        m.bus.ewram[b+0x60..b+0x62].copy_from_slice(&82u16.to_le_bytes());
        m.bus.ewram[b+0x62..b+0x64].copy_from_slice(&123u16.to_le_bytes());
        let mut count=0;
        for f in 1..15 {m.bus.frames=f;count+=usize::from(keys(&mut m.bus,RELEASED)==0x3fe);}
        assert_eq!(count,4);
    }
    #[test]
    fn world_window_list_uses_live_visible_count_and_original_navigation() {
        let mut m=menu_fixture();
        m.bus.rom.resize(0x38034,0);
        m.bus.rom[0x38028..0x38034].copy_from_slice(&[0xf0,0xb5,0x87,0xb0,0x84,0x46,0x0e,0x1c,0x34,0x1c,0x31,0x7c]);
        m.bus.iwram[0x2810..0x2814].copy_from_slice(&0x0200f3a4u32.to_le_bytes());
        let list=0xfd70;let cursor=list+0x20;let win=0x9e0+4*0x4c;
        m.bus.ewram[list-0x48]=4;
        m.bus.ewram[list+0x16]=4;
        m.bus.ewram[list+0x18..list+0x1a].copy_from_slice(&3u16.to_le_bytes());
        m.bus.ewram[cursor..cursor+4].copy_from_slice(&0x08038029u32.to_le_bytes());
        m.bus.ewram[cursor+4..cursor+8].copy_from_slice(&0x0200fd70u32.to_le_bytes());
        m.bus.ewram[cursor+0xa..cursor+0xc].copy_from_slice(&44u16.to_le_bytes());
        m.bus.iwram[win]=2;m.bus.iwram[win+0x34]=4;m.bus.iwram[win+0x35]=2;
        m.bus.iwram[win+0x36]=22;m.bus.iwram[win+0x49]=1;
        let scene=MenuScene::read(&m.bus).unwrap();
        assert!(scene.polling);assert_eq!((scene.count,scene.origin,scene.right),(3,[32,56],208));
        assert_eq!(scene.hit(70.,79.),Some(1));
        assert!(menu_select(&mut m.bus,1,true));
        assert_eq!(keys(&mut m.bus,RELEASED),0x37f);
        m.bus.ewram[cursor+0x11]=1;
        let mut presses=0;
        for f in 1..16 {m.bus.frames=f;presses+=usize::from(keys(&mut m.bus,RELEASED)==0x3fe);}
        assert_eq!(presses,4);
        m.bus.ewram[list+0x18]=5;
        assert!(!MenuScene::read(&m.bus).is_some_and(|s|s.polling));
    }
    #[test]
    fn world_target_changes_original_registers_and_defers_confirm() {
        let mut m=fixture();
        let p=Profile{base:0,start:0x08038994,commit:0x08038b3a,end:0x08038c16};
        m.bus.ffta_battle_mouse.world_profile=Some(p);
        m.bus.io[8..10].copy_from_slice(&0xaa03u16.to_le_bytes());
        m.bus.io[10..12].copy_from_slice(&0xac02u16.to_le_bytes());
        m.bus.iwram[0x2810..0x2814].copy_from_slice(&0x0200f3a4u32.to_le_bytes());
        m.bus.ewram[0xf3a8..0xf3ac].copy_from_slice(&0x02002c10u32.to_le_bytes());
        m.bus.ewram[0x2c10]=0x61; // visitable destination must retain mouse input
        for(o,v)in[(0x2c16,128u16),(0x2c18,240),(0x2c1c,156)]{m.bus.ewram[o..o+2].copy_from_slice(&v.to_le_bytes());}
        assert!(world_select(&mut m.bus,190.,90.,true));
        assert_eq!(keys(&mut m.bus,RELEASED),0x36f);
        let before=m.bus.ewram.clone();
        m.cpu.regs[4]=0x02002c10;m.cpu.regs[15]=p.commit;m.ffta_battle_mouse_entry();
        assert_eq!(m.cpu.regs[0],190);assert_eq!(m.bus.ewram,before);
        m.cpu.regs[15]=p.end-8;m.ffta_battle_mouse_entry();
        assert_eq!(m.cpu.regs[0],246);assert_eq!(m.bus.ewram,before);
        m.bus.frames=1;m.cpu.regs[15]=p.commit;m.cpu.regs[0]=191;m.ffta_battle_mouse_entry();
        assert_eq!(m.cpu.regs[0],190); // pending guest key release must not drift by one pixel
        let mut presses=0;
        for f in 1..15{m.bus.frames=f;presses+=usize::from(keys(&mut m.bus,RELEASED)==0x3fe);}
        assert_eq!(presses,4);
        m.bus.ewram[0x2c10]=0;
        assert!(!world_select(&mut m.bus,100.,90.,true));
    }
    #[test]
    fn shared_text_list_uses_live_visible_rows_with_wide_arrow() {
        let mut m=menu_fixture();
        m.bus.ewram[0x2dd70+9]=0;
        let cursor=0x25f08;let list=cursor-0x20;
        m.bus.ewram[cursor..cursor+4].copy_from_slice(&0x08017e81u32.to_le_bytes());
        m.bus.ewram[cursor+4..cursor+8].copy_from_slice(&(0x02000000+list as u32).to_le_bytes());
        m.bus.ewram[cursor+8..cursor+10].copy_from_slice(&40u16.to_le_bytes());
        m.bus.ewram[cursor+10..cursor+12].copy_from_slice(&20u16.to_le_bytes());
        m.bus.ewram[cursor+16]=3;
        m.bus.ewram[list+0x13]=7;m.bus.ewram[list+0x15]=16;m.bus.ewram[list+0x16]=8;
        for(o,v)in[(0,0x4014u16),(2,40),(4,0x120)]{m.bus.oam[o..o+2].copy_from_slice(&v.to_le_bytes());}
        let scene=MenuScene::read(&m.bus).unwrap();
        assert_eq!((scene.count,scene.origin,scene.right),(8,[56,20],200));
        assert!(scene.polling);
        assert_eq!(scene.hit(80.,72.),Some(3));
        m.bus.oam[2..4].copy_from_slice(&32u16.to_le_bytes());
        assert_eq!(MenuScene::read(&m.bus).unwrap().count,8);
        m.bus.oam[2..4].copy_from_slice(&31u16.to_le_bytes());
        assert!(MenuScene::read(&m.bus).is_none());
        m.bus.oam[2..4].copy_from_slice(&32u16.to_le_bytes());
        assert!(menu_select(&mut m.bus,3,false));
        assert_eq!(keys(&mut m.bus,RELEASED),0x37f);
        m.bus.rom.resize(0x17b74,0);
        m.bus.rom[0x17b68..0x17b74].copy_from_slice(&[0x10,0xb5,3,0x1c,9,6,9,0x16,0x58,0x8b,9,0x18]);
        m.cpu.regs[15]=0x08017b68;m.cpu.regs[0]=0x02000000+list as u32;m.cpu.regs[1]=1;
        let before=m.bus.ewram.clone();m.ffta_battle_mouse_entry();
        assert_eq!(m.cpu.regs[1],3);assert_eq!(m.bus.ewram,before);
        m.bus.ewram[cursor+17]=1;
        m.bus.ewram[cursor+14..cursor+16].copy_from_slice(&16u16.to_le_bytes());
        assert_eq!(keys(&mut m.bus,RELEASED),RELEASED);
        for _ in 0..4 {assert_eq!(keys(&mut m.bus,RELEASED),RELEASED);}
        assert_eq!(keys(&mut m.bus,RELEASED),0x37f);
        m.bus.ewram[cursor+17]=3;
        m.bus.ewram[cursor+14..cursor+16].copy_from_slice(&48u16.to_le_bytes());
        assert_eq!(keys(&mut m.bus,RELEASED),RELEASED);
        m.bus.io[1]=0;
        assert!(MenuScene::read(&m.bus).is_none());
    }
    #[test]
    fn plain_clicks_work_without_a_field_or_menu_and_physical_input_wins() {
        let mut m=fixture();m.bus.ewram[0xf3b0]=0;
        assert!(press(&mut m.bus,0x3fe));
        let out:Vec<_>=(0..12).map(|_|keys(&mut m.bus,RELEASED)).collect();
        assert_eq!(&out[..4],&[0x3fe;4]);
        assert!(out[4..].iter().all(|&v|v==RELEASED));
        back(&mut m.bus);assert_eq!(keys(&mut m.bus,RELEASED),0x3fd);
        assert_eq!(keys(&mut m.bus,0x3bf),0x3bf);
        assert_eq!(keys(&mut m.bus,RELEASED),RELEASED);
    }
    #[test]
    fn free_unit_selection_does_not_require_unused_ui_backgrounds() {
        let mut m=fixture();
        for(o,v)in[(0,0x1340u16),(8,0x4c03),(10,0x4e02),(12,0x0b08),(14,0x0a08)] {
            m.bus.io[o..o+2].copy_from_slice(&v.to_le_bytes());
        }
        assert!(Scene::read(&m.bus).is_some());
        m.bus.io[..2].copy_from_slice(&0x1f40u16.to_le_bytes());
        assert!(Scene::read(&m.bus).is_none());
        press(&mut m.bus,0x3fe);assert_eq!(keys(&mut m.bus,RELEASED),0x3fe);
    }
    #[test]
    fn facing_arrows_turn_first_then_confirm_once_and_do_not_repeat_on_hover() {
        let mut m=fixture();m.bus.ewram[0xf3b0]=0;
        for(i,(x,y,t,flip))in[(107u16,49u16,0x135u16,0u16),(123,49,0x135,0x1000),(107,57,0x139,0),(123,57,0x139,0x1000)].into_iter().enumerate(){
            for(o,v)in[(i*8,y),(i*8+2,x|flip),(i*8+4,t)]{m.bus.oam[o..o+2].copy_from_slice(&v.to_le_bytes());}
        }
        assert_eq!(facing_center(&m.bus),Some([119.,57.]));
        assert!(aim(&mut m.bus,151.,73.,false));
        let out:Vec<_>=(0..10).map(|_|keys(&mut m.bus,RELEASED)).collect();
        assert_eq!(out.iter().filter(|&&k|k==0x3ef).count(),4);
        aim(&mut m.bus,151.,73.,false);assert_eq!(keys(&mut m.bus,RELEASED),RELEASED);
        aim(&mut m.bus,151.,73.,true);
        let out:Vec<_>=(0..20).map(|_|keys(&mut m.bus,RELEASED)).collect();
        assert_eq!(out.iter().filter(|&&k|k==0x3ef).count(),4);
        assert_eq!(out.iter().filter(|&&k|k==0x3fe).count(),4);
        assert!(out.iter().position(|&k|k==0x3ef)<out.iter().position(|&k|k==0x3fe));
    }
}
/// Optional compact diagnostics; no guest writes or timing changes.
pub fn verification(mem: &MemMap) {
    if std::env::var("FFTA_MOUSE_VERIFY").as_deref() != Ok("1") {
        return;
    }
    let from = std::env::var("FFTA_MOUSE_VERIFY_FROM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12550);
    if mem.frames < from || mem.frames % 10 != 0 {
        return;
    }
    let base = mem.ffta_battle_mouse.profile.map_or(0xf3ac, |p| p.base);
    println!(
        "FFTA_MOUSE_STATE frame={} mode={} selected={},{} scene={:?}",
        mem.frames,
        mem.ewram[base + 4],
        h(&mem.ewram, base + 12).unwrap_or(65535),
        h(&mem.ewram, base + 16).unwrap_or(65535),
        Scene::read(mem).map(|s| (s.width, s.height, s.cells.len()))
    );
}
