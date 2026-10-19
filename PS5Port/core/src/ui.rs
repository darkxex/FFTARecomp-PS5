//! The UI layer of the PC build (`crates/screen/src/ui_layer.rs`, its CPU part, unchanged except that the
//! letterbox of the picture is given instead of derived from a window size).
#![allow(dead_code)]

/// Scale and top-left corner of the picture on screen, as `letterbox_f` returns them.
pub type Lb = (f32, f32, f32);

// Native UI is sampled once in physical window coordinates, after world zoom.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub rect: [f32; 4],
    pub clip: [f32; 4],
}
impl Layout {
    /// A complete modal stays horizontally centered and grows down from its
    /// original top margin, independently of the world's camera and zoom.
    pub fn centered_panel(window:Lb,base_width:f32,size:f32,bounds:[i32;4])->Self {
        let(s,x,y)=window;
        let k=s*size.clamp(0.5,2.);
        Self{rect:[(x+base_width*s*0.5-(120.+(bounds[0]+bounds[2])as f32*0.5)*k).round(),
            (y+bounds[1]as f32*(s-k)).round(),480.*k,160.*k],clip:[x,y,base_width*s,160.*s]}
    }
    /// Center the complete host UI canvas inside the game viewport.
    pub fn centered(window:Lb,base_width:f32,size:f32)->Self {
        let(s,x,y)=window;
        let k=s*size.clamp(0.5,2.);
        let left=x+base_width*s*0.5-240.*k;
        Self{rect:[left.round(),(y+80.*(s-k)).round(),480.*k,160.*k],clip:[x,y,base_width*s,160.*s]}
    }
    pub fn new(
        window: Lb,
        base_width: f32,
        right: bool,
        size: f32,
        offset: [f32; 2],
    ) -> Self {
        Self::new_with_anchor(window, base_width, right, size, offset, 0.)
    }
    pub fn new_with_anchor(
        window: Lb,
        base_width: f32,
        right: bool,
        size: f32,
        offset: [f32; 2],
        anchor_y: f32,
    ) -> Self {
        let (s, x, y) = window;
        let k = s * size.clamp(0.5, 2.);
        let left = if right {
            x + base_width * s - 240. * k
        } else {
            x
        };
        Self {
            rect: [
                (left - 120. * k + offset[0] * s).round(),
                (y + anchor_y.clamp(0., 160.) * (s - k) + offset[1] * s).round(),
                480. * k,
                160. * k,
            ],
            clip: [x, y, base_width * s, 160. * s],
        }
    }
    /// A compact native panel grows inward from its own content corner.
    pub fn at_corner(
        window: Lb,
        base_width: f32,
        right: bool,
        bottom: bool,
        size: f32,
        length: f32,
        bounds: [i32; 4],
    ) -> Self {
        let (s, x, y) = window;
        let k = s * size.clamp(0.5, 2.);
        // Length changes native backing geometry before upload, never texture scale.
        let _ = length;
        let horizontal = k;
        let left = if right {
            x + base_width * s - bounds[2] as f32 * horizontal
        } else {
            x - bounds[0] as f32 * horizontal
        };
        let top = if bottom {
            y + 160. * s - bounds[3] as f32 * k
        } else {
            y - bounds[1] as f32 * k
        };
        Self {
            rect: [(left - 120. * horizontal).round(), top.round(), 480. * horizontal, 160. * k],
            clip: [x, y, base_width * s, 160. * s],
        }
    }
    pub fn source(&self, x: f32, y: f32) -> Option<usize> {
        if x < self.clip[0]
            || y < self.clip[1]
            || x >= self.clip[0] + self.clip[2]
            || y >= self.clip[1] + self.clip[3]
        {
            return None;
        }
        let u = (x - self.rect[0]) / self.rect[2];
        let v = (y - self.rect[1]) / self.rect[3];
        ((0. ..1.).contains(&u) && (0. ..1.).contains(&v))
            .then(|| (v * 160.).floor() as usize * 480 + (u * 480.).floor() as usize)
    }
}
#[derive(Clone, Copy)]
pub struct Frame<'a> {
    pub pixels: &'a [[u8; 4]],
    pub raw_world: &'a [u16],
    pub world_dims: (u32, u32),
    pub lut: &'a [[u8; 4]],
    pub layout: Layout,
    pub world_view: [f32; 4],
}
impl Frame<'_> {
    pub fn sample(&self, x: f32, y: f32) -> Option<[u8; 4]> {
        let data = *self.pixels.get(self.layout.source(x, y)?)?;
        if data[2] == 0 && data[3] == 16 {
            return None;
        }
        let c = self.layout.clip;
        let wx = (self.world_view[0] + (x - c[0]) / c[2] * self.world_view[2])
            .floor()
            .clamp(0., self.world_dims.0 as f32 - 1.) as usize;
        let wy = (self.world_view[1] + (y - c[1]) / c[3] * self.world_view[3])
            .floor()
            .clamp(0., self.world_dims.1 as f32 - 1.) as usize;
        let world = *self.raw_world.get(wy * self.world_dims.0 as usize + wx)?;
        let front = u16::from_le_bytes([data[0], data[1]]);
        let mut color = 0;
        for shift in [0, 5, 10] {
            let lane = if shift == 5 { 2 } else { 1 };
            color |= ((((((front >> shift) & 31) * lane * data[2] as u16
                + ((world >> shift) & 31) * lane * data[3] as u16
                + 8)
                >> 4)
                .min(if shift == 5 { 63 } else { 31 }))
                / lane)
                << shift;
        }
        Some(self.lut[color as usize])
    }
}
