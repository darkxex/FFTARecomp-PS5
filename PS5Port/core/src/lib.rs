//! C ABI over `gba-core` for the PS5 app.
//!
//! The PS5 cannot compile or JIT the recompiled blocks at run time, so the
//! machine runs on the interpreter alone (the same tier the other ports fall
//! back to). Video leaves as XRGB8888, audio as 48 kHz stereo i16. The pause
//! menu of the PC build (`crates/menu`) is drawn here, over the last frame.
use gba_core::{mem, Machine};
use input_config::AvConfig;
use screen::{ColorLut, ColorSettings, ResponseMode, ScreenKind, Temporal};
use std::os::raw::{c_char, c_int};

mod mixer;
mod ui;

const MAX_STEPS: u64 = 5_000_000;
/// Each zoom-out step multiplies the view by 2^(1/6); six steps reach x2.
const ZOOM_STEP: f32 = 1.122_462;
const MAX_ZOOM_LEVEL: u32 = 6;
const OUT_RATE: u32 = 48_000;
const MENU_W: usize = 1280;
const MENU_H: usize = 720;

pub struct FftaCore {
    machine: Box<Machine>,
    lut: ColorLut,
    video_key: String,
    temporal_pixels: usize,
    temporal: Temporal,
    padded: Vec<u16>,
    rgba: Vec<[u8; 4]>,
    pixels: Vec<u32>,
    ui_pixels: Vec<[u8; 4]>,
    panel_pixels: Vec<[u8; 4]>,
    ui_out: [Vec<[u8; 4]>; 2],
    view: [usize; 4], // x, y, w, h inside the rendered frame
    zoom_level: u32,
    audio: Vec<i16>,
    mixer: mixer::Mixer,
    av: AvConfig,
    menu: menu::Menu,
    menu_open: bool,
    menu_canvas: Vec<[u8; 4]>,
    config_path: Option<String>,
}

/// The colour table of the menu's Screen, Darken and Brightness (PC defaults: the "raw" screen).
fn build_lut(av: &AvConfig) -> ColorLut {
    let screen = ScreenKind::by_name(&av.screen).unwrap_or(ScreenKind::Raw);
    ColorLut::build_with_brightness(
        &ColorSettings {
            screen,
            darken: AvConfig::knob(&av.screen_darken).map(f64::from).unwrap_or(f64::NAN),
            ..ColorSettings::default()
        },
        av.video_brightness as f64,
    )
}

/// The temporal response of the menu's Response row.
fn build_temporal(av: &AvConfig, pixels: usize) -> Temporal {
    let screen = ScreenKind::by_name(&av.screen).unwrap_or(ScreenKind::Raw);
    Temporal::new(
        pixels,
        ResponseMode::by_name(&av.response).unwrap_or(ResponseMode::Smart),
        0,
        AvConfig::knob(&av.response_keep).unwrap_or_else(|| screen::blend::default_rho(screen)),
    )
}

/// Everything the video settings of the menu change; the tables are rebuilt when this does.
fn video_key(av: &AvConfig) -> String {
    format!("{}|{}|{}|{}|{}", av.screen, av.screen_darken, av.video_brightness, av.response, av.response_keep)
}

/// What the PC shows of the rendered frame (`display_frame` of the play loop): when the zoom needs rows the
/// scene has not rendered, only the 160 new rows count and the rest is black, never stale objects.
fn display_frame<'a>(bus: &'a gba_core::mem::MemMap, padding: &'a mut Vec<u16>) -> &'a [u16] {
    let (width, height, ey) = (mem::render_width(), mem::render_height(), mem::extra_y());
    if ey == 0 || gba_core::ppu::ffta_zoom_rows_ready(bus) {
        return &bus.wide_framebuffer[..width * height];
    }
    padding.resize(width * height, 0);
    let start = width * ey;
    let end = start + width * 160;
    padding[..start].fill(0);
    padding[end..].fill(0);
    padding[start..end].copy_from_slice(&bus.wide_framebuffer[..width * 160]);
    padding
}

/// Extra columns per side for the aspect ratio of the menu ("auto" is 16:9, the screen's).
fn aspect_extra(name: &str) -> usize {
    match name.trim() {
        "3:2" => 0,
        "16:10" => 8,
        "20:9" => 58,
        "21:9" => 66,
        "32:9" => 164,
        _ => 22,
    }
}

impl FftaCore {
    fn sync_view(&mut self) {
        let base_extra = aspect_extra(&self.av.video_aspect);
        let z = if self.zoom_level == 0 { 1.0 } else { ZOOM_STEP.powi(self.zoom_level as i32) };
        let vw = (240 + 2 * base_extra) as f32 * z;
        let vh = 160.0 * z;
        let ex = (((vw - 240.0) / 2.0).ceil().max(0.0) as usize).min(mem::MAX_EXTRA);
        let ey = (((vh - 160.0) / 2.0).ceil().max(0.0) as usize).min(mem::MAX_EXTRA_Y);
        mem::set_extra(ex);
        mem::set_extra_y(ey);
        let (rw, rh) = (mem::render_width() as f32, mem::render_height() as f32);
        let (vw, vh) = (vw.min(rw), vh.min(rh));
        self.view = [((rw - vw) / 2.0) as usize, ((rh - vh) / 2.0) as usize, vw as usize, vh as usize];
        let d = &mut self.machine.bus.ffta_dialogue;
        d.view_zoom = z;
        d.view_width = vw;
        d.view_height = vh;
        d.style.scale = self.av.bubble_size;
        d.style.length = self.av.bubble_length;
    }

    fn run_one_frame(&mut self, keys: u16) {
        let m = &mut *self.machine;
        m.bus.keys = keys & 0x3ff;
        m.run_frame(MAX_STEPS);
        // the screen's temporal response advances once per emulated frame, as in the PC build
        let frame = display_frame(&self.machine.bus, &mut self.padded);
        if self.temporal_pixels != frame.len() {
            self.temporal_pixels = frame.len();
            self.temporal = build_temporal(&self.av, frame.len());
        }
        self.temporal.push(frame, &self.lut);
    }

    fn convert(&mut self) {
        let n = mem::render_width() * mem::render_height();
        self.rgba.resize(n, [0, 0, 0, 255]);
        self.temporal.compose(&self.lut, &mut self.rgba);
        self.pixels.resize(n, 0xff00_0000);
        for (o, p) in self.pixels.iter_mut().zip(&self.rgba) {
            *o = 0xff00_0000 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
        }
    }

    /// What the PC does after each emulated frame: pack the UI layer and the extra panel for presentation.
    fn build_ui_pixels(&mut self) {
        use gba_core::ffta_dialogue as d;
        let state = &self.machine.bus.ffta_dialogue;
        if let Some((pixels, _)) = state.layer() {
            self.ui_pixels.resize(pixels.len(), [0; 4]);
            if state.action_layout {
                d::pack_panel(pixels, &state.backing, state.style.length, false, &mut self.ui_pixels);
            } else if state.town_layout || state.shop_layout {
                d::pack_panel(pixels, &[], 1., true, &mut self.ui_pixels);
            } else {
                for (out, pixel) in self.ui_pixels.iter_mut().zip(pixels) {
                    *out = pixel.packed();
                }
            }
        } else {
            self.ui_pixels.clear();
        }
        let extra = &state.extra;
        if state.separate && extra.active() {
            self.panel_pixels.resize(extra.pixels.len(), [0; 4]);
            let b = d::panel_bounds(&extra.pixels, &extra.backing).unwrap_or([0; 4]);
            let length = if state.town_layout || extra.kind == d::PanelKind::Notice { 1. } else { state.style.length };
            d::pack_panel(&extra.pixels, &extra.backing, length, state.town_layout || b[0] + b[2] >= 240, &mut self.panel_pixels);
        } else {
            self.panel_pixels.clear();
        }
    }

    /// `ui_layout` of the PC's play loop: where layer `index` (0 the UI, 1 the extra panel) goes on screen.
    fn ui_layout(&self, lb: ui::Lb, index: usize) -> Option<ui::Layout> {
        use gba_core::ffta_dialogue as d;
        let state = &self.machine.bus.ffta_dialogue;
        let base_w = (240 + 2 * aspect_extra(&self.av.video_aspect)) as f32;
        if index == 1 {
            if !state.separate || !state.extra.active() {
                return None;
            }
            if state.extra.kind == d::PanelKind::Notice {
                return Some(ui::Layout::centered_panel(lb, base_w, state.style.scale, state.extra.bounds?));
            }
            let b = if state.extra.kind == d::PanelKind::Actions {
                d::panel_bounds(&state.extra.pixels, &state.extra.backing)?
            } else {
                d::pixel_bounds(&state.extra.pixels)?
            };
            return Some(ui::Layout::at_corner(
                lb,
                base_w,
                state.town_layout || b[0] + b[2] >= 240,
                b[1] + b[3] >= 160,
                state.style.scale,
                if state.town_layout { 1. } else { state.style.length },
                b,
            ));
        }
        let (_, right) = state.layer()?;
        if state.action_layout {
            let b = d::panel_bounds(&state.pixels, &state.backing)?;
            Some(ui::Layout::at_corner(lb, base_w, false, true, state.style.scale, state.style.length, b))
        } else if state.shop_layout || state.site_layout {
            Some(ui::Layout::centered(lb, base_w, state.style.scale))
        } else if state.town_layout || state.shop_layout {
            Some(ui::Layout::new_with_anchor(lb, base_w, true, state.style.scale, [0., 0.], 0.))
        } else {
            Some(ui::Layout::new_with_anchor(
                lb,
                base_w,
                right,
                state.style.scale,
                [state.style.offset_x, state.style.offset_y],
                state.vertical_anchor(),
            ))
        }
    }

    /// The layer as the PC's `Frame::sample` draws it, one value per UI texel (480x160): translucent
    /// parts are mixed with the world under them, the rest is opaque or clear (alpha 0).
    fn compose_layer(&mut self, layout: &ui::Layout, index: usize) {
        use gba_core::ffta_dialogue as d;
        let state = &self.machine.bus.ffta_dialogue;
        let pixels = if index == 0 { &self.ui_pixels } else { &self.panel_pixels };
        let world: &[u16] = if index == 1 && state.extra.kind == d::PanelKind::Stats {
            &state.extra.background
        } else {
            &self.machine.bus.wide_framebuffer
        };
        let (ww, wh) = (mem::render_width(), mem::render_height());
        let out = &mut self.ui_out[index];
        out.resize(480 * 160, [0; 4]);
        let c = layout.clip;
        let view = [self.view[0] as f32, self.view[1] as f32, self.view[2] as f32, self.view[3] as f32];
        for v in 0..160usize {
            for u in 0..480usize {
                let data = pixels.get(v * 480 + u).copied().unwrap_or([0, 0, 0, 16]);
                if data[2] == 0 && data[3] == 16 {
                    out[v * 480 + u] = [0; 4];
                    continue;
                }
                let x = layout.rect[0] + (u as f32 + 0.5) / 480. * layout.rect[2];
                let y = layout.rect[1] + (v as f32 + 0.5) / 160. * layout.rect[3];
                let wx = (view[0] + (x - c[0]) / c[2] * view[2]).floor().clamp(0., ww as f32 - 1.) as usize;
                let wy = (view[1] + (y - c[1]) / c[3] * view[3]).floor().clamp(0., wh as f32 - 1.) as usize;
                let under = world.get(wy * ww + wx).copied().unwrap_or(0);
                let front = u16::from_le_bytes([data[0], data[1]]);
                let mut color = 0u16;
                for shift in [0, 5, 10] {
                    let lane = if shift == 5 { 2 } else { 1 };
                    color |= (((((front >> shift) & 31) * lane * data[2] as u16 + ((under >> shift) & 31) * lane * data[3] as u16 + 8) >> 4)
                        .min(if shift == 5 { 63 } else { 31 }))
                        / lane
                        << shift;
                }
                let t = self.lut.table[color as usize];
                out[v * 480 + u] = [t[0], t[1], t[2], 255];
            }
        }
    }

    fn save_config(&self) {
        let Some(path) = &self.config_path else { return };
        let a = &self.av;
        let text = format!(
            "audio_volume={}\nff_speed={}\nff_audio={}\nvideo_brightness={}\nscaling={}\nvideo_aspect={}\nbubble_size={}\nbubble_length={}\nscreen={}\nscreen_darken={}\nresponse={}\ngrid={}\naudio_enhanced={}\nshow_fps={}\n",
            a.audio_volume, a.ff_speed, a.ff_audio, a.video_brightness, a.scaling, a.video_aspect, a.bubble_size, a.bubble_length, a.screen, a.screen_darken, a.response, a.grid, a.audio_enhanced, a.show_fps
        );
        let tmp = format!("{path}.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }

    fn load_config(&mut self, path: &str) {
        self.config_path = Some(path.to_string());
        let Ok(text) = std::fs::read_to_string(path) else { return };
        let a = &mut self.av;
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "audio_volume" => a.audio_volume = v.parse().unwrap_or(a.audio_volume).clamp(0.0, 1.0),
                "ff_speed" => a.ff_speed = v.parse().ok().filter(|s| [2, 4, 8].contains(s)).unwrap_or(a.ff_speed),
                "ff_audio" => a.ff_audio = if ["silent", "fast", "normal"].contains(&v) { v.into() } else { a.ff_audio.clone() },
                "video_brightness" => a.video_brightness = v.parse().unwrap_or(a.video_brightness).clamp(0.0, 2.0),
                "scaling" => a.scaling = if ["off", "nearest", "sharp", "epx"].contains(&v) { v.into() } else { "nearest".into() },
                "audio_enhanced" => a.audio_enhanced = v == "true",
                "show_fps" => a.show_fps = v == "true",
                "grid" => a.grid = if v == "auto" || v.parse::<f32>().is_ok() { v.into() } else { a.grid.clone() },
                "video_aspect" => a.video_aspect = v.to_string(),
                "bubble_size" => a.bubble_size = v.parse().unwrap_or(a.bubble_size),
                "bubble_length" => a.bubble_length = v.parse().unwrap_or(a.bubble_length),
                "screen" => a.screen = ScreenKind::by_name(v).map_or(a.screen.clone(), |k| k.name().to_string()),
                "screen_darken" => a.screen_darken = if v == "auto" || v.parse::<f32>().is_ok() { v.into() } else { a.screen_darken.clone() },
                "response" => a.response = ResponseMode::by_name(v).map_or(a.response.clone(), |m| m.name().to_string()),
                _ => {}
            }
        }
    }
}

/// Creates the machine from the cartridge image. Returns null when the file is not
/// one of the supported Final Fantasy Tactics Advance dumps (USA or Europe).
#[no_mangle]
pub extern "C" fn ffta_core_create(rom: *const u8, len: usize) -> *mut FftaCore {
    if rom.is_null() || len < 0x100 || len > 32 * 1024 * 1024 {
        return std::ptr::null_mut();
    }
    let rom = unsafe { std::slice::from_raw_parts(rom, len) }.to_vec();
    if gba_core::ffta_rom::version(&rom).is_none() {
        return std::ptr::null_mut();
    }
    let av = AvConfig::default();
    let mut core = Box::new(FftaCore {
        machine: Box::new(Machine::new(rom)),
        lut: build_lut(&av),
        video_key: video_key(&av),
        temporal_pixels: 0,
        temporal: build_temporal(&av, 1),
        padded: Vec::new(),
        rgba: Vec::new(),
        pixels: Vec::new(),
        ui_pixels: Vec::new(),
        panel_pixels: Vec::new(),
        ui_out: [Vec::new(), Vec::new()],
        view: [0, 0, 240, 160],
        zoom_level: 0,
        audio: Vec::with_capacity(4096),
        mixer: mixer::Mixer::new(OUT_RATE as f64, av.audio_enhanced, 1.0 - av.audio_volume),
        av,
        menu: menu::Menu::new(true, false),
        menu_open: false,
        menu_canvas: vec![[0; 4]; MENU_W * MENU_H],
        config_path: None,
    });
    // the audio taps and the engine HLE of the PC build (Enhanced Audio)
    core.machine.bus.tap_channels = true;
    let _ = mixer::arm_audio_hle(&mut core.machine);
    // the PC build draws dialogues, menus and panels as a layer of their own
    core.machine.bus.ffta_dialogue.separate = true;
    core.sync_view();
    Box::into_raw(core)
}

#[no_mangle]
pub extern "C" fn ffta_core_destroy(core: *mut FftaCore) {
    if !core.is_null() {
        drop(unsafe { Box::from_raw(core) });
        mem::set_extra(0);
        mem::set_extra_y(0);
    }
}

/// Where the menu's settings are kept; reads them if the file exists.
#[no_mangle]
pub extern "C" fn ffta_core_set_config(core: *mut FftaCore, path: *const c_char) {
    let c = unsafe { &mut *core };
    if let Ok(p) = unsafe { std::ffi::CStr::from_ptr(path) }.to_str() {
        c.load_config(p);
        c.sync_view();
    }
}

/// Zoom out by `steps` (negative zooms in); x1 is the floor.
#[no_mangle]
pub extern "C" fn ffta_core_zoom(core: *mut FftaCore, steps: c_int) -> c_int {
    let c = unsafe { &mut *core };
    c.zoom_level = (c.zoom_level as i32 + steps).clamp(0, MAX_ZOOM_LEVEL as i32) as u32;
    c.zoom_level as c_int
}

/// Fast-forward multiplier chosen in the menu (2, 4 or 8).
#[no_mangle]
pub extern "C" fn ffta_core_ff_speed(core: *mut FftaCore) -> c_int {
    unsafe { &*core }.av.ff_speed as c_int
}

/// 1 when the menu asks for the frame counter.
#[no_mangle]
pub extern "C" fn ffta_core_show_fps(core: *mut FftaCore) -> c_int {
    unsafe { &*core }.av.show_fps as c_int
}

/// The scaler of the menu: 0 off (exact area average), 1 nearest, 2 sharp bilinear, 3 EPX.
#[no_mangle]
pub extern "C" fn ffta_core_scaling(core: *mut FftaCore) -> c_int {
    match unsafe { &*core }.av.scaling.as_str() {
        "off" => 0,
        "sharp" => 2,
        "epx" => 3,
        _ => 1,
    }
}

/// The pixel grid's strength, 0..1 ("auto" is the screen model's default: none for raw).
#[no_mangle]
pub extern "C" fn ffta_core_grid(core: *mut FftaCore) -> f32 {
    let av = &unsafe { &*core }.av;
    let kind = ScreenKind::by_name(&av.screen).unwrap_or(ScreenKind::Raw);
    AvConfig::knob(&av.grid).unwrap_or_else(|| kind.default_grid_strength()).clamp(0.0, 1.0)
}

/// Emulates `frames` (1..=8) GBA frames with `keys` as KEYINPUT (bit set =
/// released, as the hardware reads it). Audio of all of them is shaped by
/// the fast-forward audio mode and the volume of the menu.
#[no_mangle]
pub extern "C" fn ffta_core_run(core: *mut FftaCore, keys: u16, frames: c_int) {
    let c = unsafe { &mut *core };
    let frames = frames.clamp(1, 8) as usize;
    c.mixer.set_fast_forward(frames > 1, &c.av.ff_audio.clone(), c.av.ff_speed);
    c.mixer.enhanced_on = c.av.audio_enhanced;
    c.mixer.vol_cut = 1.0 - c.av.audio_volume.clamp(0.0, 1.0);
    c.audio.clear();
    c.sync_view();
    let key = video_key(&c.av);
    if key != c.video_key {
        c.lut = build_lut(&c.av);
        c.video_key = key;
        c.temporal_pixels = 0; // the response is rebuilt with the next frame
    }
    for _ in 0..frames {
        c.run_one_frame(keys);
    }
    c.convert();
    c.build_ui_pixels();
    c.mixer.feed(&mut c.machine.bus);
    c.mixer.render(&mut c.audio);
}

/// The picture as XRGB8888, cropped to what the zoom shows. `pitch` is in bytes.
#[no_mangle]
pub extern "C" fn ffta_core_video(core: *mut FftaCore, w: *mut c_int, h: *mut c_int, pitch: *mut usize) -> *const u32 {
    let c = unsafe { &*core };
    let rw = mem::render_width();
    let [x, y, vw, vh] = c.view;
    unsafe {
        *w = vw as c_int;
        *h = vh as c_int;
        *pitch = rw * 4;
    }
    if c.pixels.len() < rw * (y + vh) {
        return std::ptr::null();
    }
    unsafe { c.pixels.as_ptr().add(y * rw + x) }
}

/// Stereo i16 frames produced by the last `ffta_core_run` (48 kHz).
#[no_mangle]
pub extern "C" fn ffta_core_audio(core: *mut FftaCore, frames: *mut usize) -> *const i16 {
    let c = unsafe { &*core };
    unsafe { *frames = c.audio.len() / 2 };
    c.audio.as_ptr()
}

/// The battery save, or null when the game has none yet.
#[no_mangle]
pub extern "C" fn ffta_core_save(core: *mut FftaCore, len: *mut usize) -> *const u8 {
    let c = unsafe { &*core };
    match c.machine.bus.save_data() {
        Some(d) => {
            unsafe { *len = d.len() };
            d.as_ptr()
        }
        None => {
            unsafe { *len = 0 };
            std::ptr::null()
        }
    }
}

#[no_mangle]
pub extern "C" fn ffta_core_load_save(core: *mut FftaCore, data: *const u8, len: usize) {
    if data.is_null() {
        return;
    }
    let c = unsafe { &mut *core };
    c.machine.bus.load_save_data(unsafe { std::slice::from_raw_parts(data, len) });
}

/* ---- the pause menu ------------------------------------------------------------ */

/// Opens (1) or closes (0) the menu; it always opens on its first page.
#[no_mangle]
pub extern "C" fn ffta_core_menu_set_open(core: *mut FftaCore, open: c_int) {
    let c = unsafe { &mut *core };
    if open != 0 && !c.menu_open {
        c.menu.reset();
    }
    c.menu_open = open != 0;
}

#[no_mangle]
pub extern "C" fn ffta_core_menu_is_open(core: *mut FftaCore) -> c_int {
    unsafe { &*core }.menu_open as c_int
}

/// One navigation press: 0 up, 1 down, 2 left, 3 right, 4 confirm, 5 cancel.
/// Returns 0 nothing, 1 resume (the menu closed), 2 exit the game.
#[no_mangle]
pub extern "C" fn ffta_core_menu_input(core: *mut FftaCore, input: c_int) -> c_int {
    let c = unsafe { &mut *core };
    let ev = match input {
        0 => menu::MenuInput::Up,
        1 => menu::MenuInput::Down,
        2 => menu::MenuInput::Left,
        3 => menu::MenuInput::Right,
        4 => menu::MenuInput::Confirm,
        _ => menu::MenuInput::Cancel,
    };
    match c.menu.handle(ev, &mut c.av) {
        Some(menu::MenuAction::Resume) => {
            c.menu_open = false;
            1
        }
        Some(menu::MenuAction::Quit) => 2,
        Some(menu::MenuAction::Changed(_)) => {
            c.save_config();
            0
        }
        _ => 0,
    }
}

/// The menu over the last picture, 1280x720 RGBA (the result is valid until the next call).
#[no_mangle]
pub extern "C" fn ffta_core_menu_draw(core: *mut FftaCore, w: *mut c_int, h: *mut c_int) -> *const u8 {
    let c = unsafe { &mut *core };
    let rw = mem::render_width();
    let [vx, vy, vw, vh] = c.view;
    // the game picture, as large as fits and centred, under the menu's dimming
    let (dw, dh) = if vw * MENU_H > vh * MENU_W { (MENU_W, (MENU_W * vh / vw).max(1)) } else { ((MENU_H * vw / vh).max(1), MENU_H) };
    let (ox, oy) = ((MENU_W - dw) / 2, (MENU_H - dh) / 2);
    for px in c.menu_canvas.iter_mut() {
        *px = [0, 0, 0, 255];
    }
    if c.pixels.len() >= rw * (vy + vh) {
        for y in 0..dh {
            let sy = vy + y * vh / dh;
            for x in 0..dw {
                let p = c.pixels[sy * rw + vx + x * vw / dw];
                c.menu_canvas[(oy + y) * MENU_W + ox + x] = [(p >> 16) as u8, (p >> 8) as u8, p as u8, 255];
            }
        }
    }
    let keys: [String; 10] = Default::default();
    c.menu.draw(&c.av, &keys, "", "", "", "", &mut c.menu_canvas, MENU_W, MENU_H);
    unsafe {
        *w = MENU_W as c_int;
        *h = MENU_H as c_int;
    }
    c.menu_canvas.as_ptr() as *const u8
}

/// One layer of the UI for the screen: a 480x160 RGBA picture (alpha 0 where there is none), the rectangle it
/// is drawn in and the area it is clipped to, in output pixels.
#[repr(C)]
pub struct UiLayerOut {
    pub rgba: *const u8,
    pub rect: [f32; 4],
    pub clip: [f32; 4],
}

/// The layers of dialogues, menus and panels for the picture drawn at (x, y, w, h) on screen.
/// Fills `out` (room for 2) and returns how many.
#[no_mangle]
pub extern "C" fn ffta_core_ui_layers(core: *mut FftaCore, x: f32, y: f32, w: f32, out: *mut UiLayerOut) -> c_int {
    let c = unsafe { &mut *core };
    if c.menu_open {
        return 0;
    }
    let base_w = (240 + 2 * aspect_extra(&c.av.video_aspect)) as f32;
    let lb: ui::Lb = (w / base_w, x, y);
    let mut n = 0;
    for index in 0..2 {
        let Some(layout) = c.ui_layout(lb, index) else { continue };
        c.compose_layer(&layout, index);
        unsafe {
            *out.add(n) = UiLayerOut { rgba: c.ui_out[index].as_ptr() as *const u8, rect: layout.rect, clip: layout.clip };
        }
        n += 1;
    }
    n as c_int
}
