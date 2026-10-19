//! Invisible spatial/event layer, independent of game, renderer and input backend.
//! Coordinates are logical UI/world pixels after the caller's inverse transform.
pub const CELL_SIZE: f32 = 8.;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventId { pub context: u16, pub option: u16 }
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    /// None is a modal blocker: consume this area without hitting a lower layer.
    pub event: Option<EventId>,
    pub bounds: [f32; 4],
    pub layer: i16,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerUpdate {
    pub cell: Option<[usize; 2]>,
    pub hovered: Option<EventId>,
    pub entered: Option<EventId>,
    pub left: Option<EventId>,
    pub clicked: Option<EventId>,
}
#[derive(Default)]
pub struct InteractionGrid {
    bounds: Option<[f32; 4]>,
    columns: usize,
    rows: usize,
    cells: Vec<Vec<usize>>,
    regions: Vec<Region>,
    hovered: Option<EventId>,
    revision: u64,
}
fn valid(b: [f32; 4]) -> bool {
    b.iter().all(|v| v.is_finite()) && b[0] < b[2] && b[1] < b[3]
}
fn contains(b: [f32; 4], x: f32, y: f32) -> bool {
    x >= b[0] && x < b[2] && y >= b[1] && y < b[3]
}
impl InteractionGrid {
    /// Republish only when viewport, targets or layer ordering changes.
    pub fn update(&mut self, bounds: [f32; 4], regions: &[Region]) -> bool {
        if self.bounds == Some(bounds) && self.regions == regions { return false; }
        if !valid(bounds) { self.clear(); return false; }
        let columns = ((bounds[2] - bounds[0]) / CELL_SIZE).ceil() as usize;
        let rows = ((bounds[3] - bounds[1]) / CELL_SIZE).ceil() as usize;
        let Some(count) = columns.checked_mul(rows).filter(|&n| n <= 262144) else {
            self.clear(); return false;
        };
        self.bounds = Some(bounds); self.columns = columns; self.rows = rows;
        self.cells.resize_with(count, Vec::new);
        for cell in &mut self.cells { cell.clear(); }
        self.regions.clear(); self.regions.extend_from_slice(regions);
        self.hovered = None; self.revision += 1;
        let mut order: Vec<_> = (0..regions.len()).collect();
        order.sort_by_key(|&i| (regions[i].layer, i));
        for index in order {
            let b = regions[index].bounds;
            if !valid(b) { continue; }
            let clipped = [b[0].max(bounds[0]), b[1].max(bounds[1]), b[2].min(bounds[2]), b[3].min(bounds[3])];
            if !valid(clipped) { continue; }
            let left = ((clipped[0] - bounds[0]) / CELL_SIZE).floor() as usize;
            let top = ((clipped[1] - bounds[1]) / CELL_SIZE).floor() as usize;
            let right = (((clipped[2] - bounds[0]) / CELL_SIZE).ceil() as usize).min(columns);
            let bottom = (((clipped[3] - bounds[1]) / CELL_SIZE).ceil() as usize).min(rows);
            for y in top..bottom { for x in left..right { self.cells[y * columns + x].push(index); } }
        }
        true
    }
    pub fn clear(&mut self) {
        self.bounds = None; self.cells.clear(); self.regions.clear(); self.hovered = None;
        self.columns = 0; self.rows = 0;
    }
    pub fn reset_pointer(&mut self) { self.hovered = None; }
    pub fn revision(&self) -> u64 { self.revision }
    pub fn cell_count(&self) -> usize { self.cells.len() }
    pub fn cell_at(&self, x: f32, y: f32) -> Option<[usize; 2]> {
        let b = self.bounds?;
        if !x.is_finite() || !y.is_finite() || !contains(b, x, y) { return None; }
        let col = ((x - b[0]) / CELL_SIZE).floor() as usize;
        let row = ((y - b[1]) / CELL_SIZE).floor() as usize;
        (col < self.columns && row < self.rows).then_some([col, row])
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<EventId> {
        let [col, row] = self.cell_at(x, y)?;
        for &i in self.cells[row * self.columns + col].iter().rev() {
            let r = &self.regions[i];
            // Exact bounds keep tiny gaps and cell-sharing options distinct.
            if contains(r.bounds, x, y) { return r.event; }
        }
        None
    }
    pub fn pointer(&mut self, x: f32, y: f32, click: bool) -> PointerUpdate {
        let hovered = self.hit(x, y);
        let changed = hovered != self.hovered;
        let update = PointerUpdate { cell: self.cell_at(x, y), hovered,
            entered: if changed { hovered } else { None },
            left: if changed { self.hovered } else { None },
            clicked: if click { hovered } else { None } };
        self.hovered = hovered; update
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn region(option:u16,bounds:[f32;4],layer:i16)->Region {
        Region{event:Some(EventId{context:1,option}),bounds,layer}
    }
    #[test]
    fn sharing_a_cell_does_not_merge_targets_or_fill_gaps() {
        let mut grid=InteractionGrid::default();
        grid.update([0.,0.,240.,160.],&[region(1,[1.,1.,3.,7.],0),region(2,[5.,1.,7.,7.],0)]);
        assert_eq!(grid.cell_at(2.,2.),grid.cell_at(6.,2.));
        assert_eq!(grid.hit(2.,2.).unwrap().option,1);assert_eq!(grid.hit(6.,2.).unwrap().option,2);
        assert_eq!(grid.hit(4.,2.),None);assert_eq!(grid.hit(3.,2.),None);
        assert_eq!(grid.hit(f32::NAN,2.),None);assert_eq!(grid.hit(240.,2.),None);
    }
    #[test]
    fn hover_fires_on_target_change_not_on_cell_change() {
        let mut grid=InteractionGrid::default();let id=EventId{context:1,option:2};
        let regions=[region(2,[1.,1.,40.,40.],0)];
        grid.update([0.,0.,240.,160.],&regions);
        let first=grid.pointer(2.,2.,false);assert_eq!(first.entered,Some(id));
        let next=grid.pointer(25.,25.,false);assert_ne!(first.cell,next.cell);assert_eq!(next.entered,None);
        assert!(!grid.update([0.,0.,240.,160.],&regions));assert_eq!(grid.revision(),1);
        assert_eq!(grid.pointer(25.,25.,true).clicked,Some(id));
        let leave=grid.pointer(80.,80.,false);assert_eq!(leave.left,Some(id));assert_eq!(leave.hovered,None);
    }
    #[test]
    fn topmost_layer_and_modal_blockers_win_and_old_targets_expire() {
        let mut grid=InteractionGrid::default();
        grid.update([0.,0.,240.,160.],&[region(1,[0.,0.,100.,100.],0),region(2,[10.,10.,30.,30.],2),Region{event:None,bounds:[15.,15.,20.,20.],layer:3}]);
        assert_eq!(grid.hit(12.,12.).unwrap().option,2);assert_eq!(grid.hit(16.,16.),None);
        grid.update([0.,0.,240.,160.],&[region(3,[150.,0.,200.,100.],0)]);
        assert_eq!(grid.hit(12.,12.),None);assert_eq!(grid.hit(160.,12.).unwrap().option,3);
        grid.clear();assert_eq!(grid.hit(160.,12.),None);
    }
    #[test]
    fn negative_zoom_margins_and_resizing_are_supported() {
        let mut grid=InteractionGrid::default();
        grid.update([-448.,-80.,688.,240.],&[region(4,[84.,107.,160.,125.],0)]);
        assert_eq!(grid.cell_count(),142*40);
        assert_eq!(grid.hit(120.,115.).unwrap().option,4);assert_eq!(grid.hit(-400.,0.),None);
        grid.update([-164.,0.,404.,160.],&[region(4,[84.,107.,160.,125.],0)]);
        assert_eq!(grid.cell_count(),71*20);assert_eq!(grid.hit(120.,115.).unwrap().option,4);
        grid.update([0.,0.,f32::INFINITY,160.],&[]);assert_eq!(grid.cell_count(),0);
    }
    #[test]
    fn grid_matches_exact_layered_reference_across_the_screen() {
        let mut regions=Vec::new();
        for i in 0..64 {let x=(i*37%236)as f32;let y=(i*23%156)as f32;
            regions.push(region(i,[x+0.25,y+0.5,(x+19.).min(240.),(y+13.).min(160.)],(i%4)as i16));}
        let mut grid=InteractionGrid::default();grid.update([0.,0.,240.,160.],&regions);
        for y in 0..320 {for x in 0..480 {
            let (x,y)=(x as f32*0.5,y as f32*0.5);
            let expected=regions.iter().enumerate().filter(|(_,r)|contains(r.bounds,x,y)).max_by_key(|(i,r)|(r.layer,*i)).and_then(|(_,r)|r.event);
            assert_eq!(grid.hit(x,y),expected,"x={x} y={y}");
        }}
    }
}
