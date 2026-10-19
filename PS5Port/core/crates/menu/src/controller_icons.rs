//! Own vector controller symbols, matching the HTML reference's silhouettes.
use super::canvas::Canvas;
const EDGE: [u8; 3] = [26, 18, 9];
const WHITE: [u8; 3] = [255, 255, 255];
const GREY: [u8; 3] = [75, 79, 88];
pub(super) fn supported(v: &str) -> bool {
    matches!(
        v,
        "A" | "B"
            | "X"
            | "Y"
            | "CROSS"
            | "CIRCLE"
            | "SQUARE"
            | "TRIANGLE"
            | "LB"
            | "RB"
            | "LT"
            | "RT"
            | "L"
            | "R"
            | "ZL"
            | "ZR"
            | "L1"
            | "R1"
            | "L2"
            | "R2"
            | "BACK"
            | "START"
            | "GUIDE"
            | "MINUS"
            | "PLUS"
            | "HOME"
            | "SHARE"
            | "OPTIONS"
            | "PS"
            | "L3"
            | "R3"
            | "LS CLICK"
            | "RS CLICK"
            | "DPAD UP"
            | "DPAD DOWN"
            | "DPAD LEFT"
            | "DPAD RIGHT"
            | "LS UP"
            | "LS DOWN"
            | "LS LEFT"
            | "LS RIGHT"
            | "RS UP"
            | "RS DOWN"
            | "RS LEFT"
            | "RS RIGHT"
    ) || v.starts_with("BUTTON ")
}
pub(super) fn draw(c: &mut Canvas, v: &str, x: f32, y: f32, size: f32) {
    let s = size / 32.;
    let cx = x + 16. * s;
    let cy = y + 16. * s;
    let mut label = None;
    match v {
        "A" | "B" | "X" | "Y" => {
            let color = match v {
                "A" => [76, 175, 80],
                "B" => [224, 69, 58],
                "X" => [47, 111, 208],
                _ => [242, 193, 46],
            };
            c.circle(cx, cy, 13. * s, color, EDGE);
            c.text(
                x + 10. * s,
                y + 7. * s,
                v,
                if v == "Y" { EDGE } else { WHITE },
                20. * s,
                false,
                22. * s,
            );
        }
        "CROSS" | "CIRCLE" | "SQUARE" | "TRIANGLE" => {
            let color = match v {
                "CROSS" => [91, 145, 221],
                "CIRCLE" => [246, 48, 73],
                "SQUARE" => [225, 119, 191],
                _ => [12, 211, 151],
            };
            // Give the PlayStation symbols the look of finished controller
            // buttons: a graphite keycap, thin metallic rim, and crisp
            // colored glyphs. All geometry stays vector and scales cleanly.
            let disk = tiny_skia::PathBuilder::from_circle(cx, cy + 0.5 * s, 14. * s).unwrap();
            c.path(disk.clone(), [4, 5, 8], 0.);
            c.path(disk, [78, 80, 87], 0.9 * s);
            c.path(
                tiny_skia::PathBuilder::from_circle(cx, cy, 11.8 * s).unwrap(),
                [39, 41, 47],
                0.65 * s,
            );
            let weight = 2.5 * s;
            match v {
                "CROSS" => {
                    for (x1, y1, x2, y2) in [(8.2, 8.2, 23.8, 23.8), (23.8, 8.2, 8.2, 23.8)] {
                        c.line(
                            x + x1 * s,
                            y + y1 * s,
                            x + x2 * s,
                            y + y2 * s,
                            EDGE,
                            weight + s,
                        );
                        c.line(
                            x + x1 * s,
                            y + y1 * s,
                            x + x2 * s,
                            y + y2 * s,
                            color,
                            weight,
                        );
                    }
                }
                "CIRCLE" => {
                    let ring = tiny_skia::PathBuilder::from_circle(cx, cy, 8.8 * s).unwrap();
                    c.path(ring.clone(), EDGE, 4.0 * s);
                    c.path(ring, color, 2.7 * s);
                }
                "SQUARE" => {
                    let mut p = tiny_skia::PathBuilder::new();
                    p.move_to(x + 8. * s, y + 8. * s);
                    p.line_to(x + 24. * s, y + 8. * s);
                    p.line_to(x + 24. * s, y + 24. * s);
                    p.line_to(x + 8. * s, y + 24. * s);
                    p.close();
                    let path = p.finish().unwrap();
                    c.path(path.clone(), EDGE, 4.0 * s);
                    c.path(path, color, 2.7 * s);
                }
                _ => {
                    let mut p = tiny_skia::PathBuilder::new();
                    p.move_to(cx, y + 7. * s);
                    p.line_to(x + 24.5 * s, y + 23.5 * s);
                    p.line_to(x + 7.5 * s, y + 23.5 * s);
                    p.close();
                    let path = p.finish().unwrap();
                    c.path(path.clone(), EDGE, 4.0 * s);
                    c.path(path, color, 2.7 * s);
                }
            }
        }
        "LB" | "RB" | "L" | "R" | "ZL" | "ZR" | "L1" | "R1" | "L2" | "R2" => {
            if matches!(v, "L2" | "R2") {
                // Analog triggers are drawn as a curved, inset pull lever so
                // they read differently from the shoulder buttons above.
                let flip = v == "R2";
                let tx = |n: f32| x + if flip { 32. - n } else { n } * s;
                let mut p = tiny_skia::PathBuilder::new();
                p.move_to(tx(4.), y + 5. * s);
                p.line_to(tx(27.), y + 5. * s);
                p.line_to(tx(25.), y + 20. * s);
                p.quad_to(tx(23.), y + 28. * s, tx(15.), y + 28. * s);
                p.line_to(tx(9.), y + 25. * s);
                p.close();
                let path = p.finish().unwrap();
                c.path(path.clone(), [66, 70, 79], 0.);
                c.path(path, EDGE, 1.5 * s);
                c.line(
                    tx(8.),
                    y + 11. * s,
                    tx(24.),
                    y + 11. * s,
                    [170, 176, 187],
                    s,
                );
            } else {
                c.rect(
                    x - 4. * s,
                    y + 8. * s,
                    40. * s,
                    18. * s,
                    8. * s,
                    GREY,
                    [45, 48, 55],
                    EDGE,
                );
            }
            label = Some((v, 9., 11., 14.));
        }
        "LT" | "RT" => {
            let mut p = tiny_skia::PathBuilder::new();
            let flip = v == "RT";
            let tx = |n: f32| x + if flip { 32. - n } else { n } * s;
            p.move_to(tx(3.), y + 27. * s);
            p.line_to(tx(3.), y + 13. * s);
            p.quad_to(tx(3.), y + 3. * s, tx(15.), y + 3. * s);
            p.line_to(tx(29.), y + 3. * s);
            p.line_to(tx(29.), y + 28. * s);
            p.close();
            let path = p.finish().unwrap();
            c.path(path.clone(), [58, 61, 69], 0.);
            c.path(path, EDGE, 1.5 * s);
            label = Some((v, 9., 11., 14.));
        }
        "BACK" | "START" | "GUIDE" | "MINUS" | "PLUS" | "HOME" | "SHARE" | "OPTIONS" | "PS" => {
            c.circle(
                cx,
                cy,
                13. * s,
                if v == "GUIDE" { [46, 139, 62] } else { GREY },
                EDGE,
            );
            if matches!(v, "START" | "PLUS" | "OPTIONS") {
                for yy in [11., 16., 21.] {
                    c.line(
                        x + 10. * s,
                        y + yy * s,
                        x + 22. * s,
                        y + yy * s,
                        WHITE,
                        2. * s,
                    );
                }
            } else if matches!(v, "BACK" | "MINUS" | "SHARE") {
                for (xx, yy) in [(9., 10.), (13., 14.)] {
                    c.rect(
                        x + xx * s,
                        y + yy * s,
                        10. * s,
                        8. * s,
                        s,
                        GREY,
                        GREY,
                        WHITE,
                    );
                }
            } else if matches!(v, "HOME" | "PS") {
                label = Some((if v == "HOME" { "HOME" } else { "PS" }, 7., 11., 12.));
            } else {
                c.line(
                    x + 10. * s,
                    y + 10. * s,
                    x + 22. * s,
                    y + 22. * s,
                    WHITE,
                    2.5 * s,
                );
                c.line(
                    x + 22. * s,
                    y + 10. * s,
                    x + 10. * s,
                    y + 22. * s,
                    WHITE,
                    2.5 * s,
                );
            }
        }
        v if v.starts_with("DPAD ") => {
            let mut p = tiny_skia::PathBuilder::new();
            for (i, (xx, yy)) in [
                (12., 3.),
                (20., 3.),
                (20., 12.),
                (29., 12.),
                (29., 20.),
                (20., 20.),
                (20., 29.),
                (12., 29.),
                (12., 20.),
                (3., 20.),
                (3., 12.),
                (12., 12.),
            ]
            .iter()
            .enumerate()
            {
                if i == 0 {
                    p.move_to(x + xx * s, y + yy * s);
                } else {
                    p.line_to(x + xx * s, y + yy * s);
                }
            }
            p.close();
            let path = p.finish().unwrap();
            c.path(path.clone(), GREY, 0.);
            let (a, b, w, h) = match v {
                "DPAD UP" => (12., 3., 8., 9.),
                "DPAD DOWN" => (12., 20., 8., 9.),
                "DPAD LEFT" => (3., 12., 9., 8.),
                _ => (20., 12., 9., 8.),
            };
            c.rect(
                x + a * s,
                y + b * s,
                w * s,
                h * s,
                0.,
                [255, 184, 31],
                [255, 184, 31],
                [255, 184, 31],
            );
            c.path(path, EDGE, 1.5 * s);
        }
        v if v == "L3" || v == "R3" || v.starts_with("LS ") || v.starts_with("RS ") => {
            c.circle(cx, cy, 13. * s, [43, 46, 53], EDGE);
            c.circle(cx, cy, 9. * s, [91, 96, 107], [170, 176, 187]);
            label = Some((
                if v.starts_with("LS") || v == "L3" {
                    "L3"
                } else {
                    "R3"
                },
                9.,
                10.,
                14.,
            ));
            if !v.ends_with("CLICK") && v != "L3" && v != "R3" {
                let (a, b) = if v.ends_with("UP") {
                    (16., 2.)
                } else if v.ends_with("DOWN") {
                    (16., 30.)
                } else if v.ends_with("LEFT") {
                    (2., 16.)
                } else {
                    (30., 16.)
                };
                c.circle(x + a * s, y + b * s, 3. * s, [255, 184, 31], EDGE);
            }
        }
        value if value.starts_with("BUTTON ") => {
            // DirectInput exposes only ordinal button numbers. Keep those
            // visibly generic instead of drawing a numbered circle that
            // looks like a face-button icon.
            c.rect(
                x,
                y + 5. * s,
                32. * s,
                22. * s,
                5. * s,
                [237, 220, 184],
                [221, 199, 155],
                [184, 146, 65],
            );
            c.text(
                x + 3. * s,
                y + 11. * s,
                &format!("BTN {}", value.strip_prefix("BUTTON ").unwrap_or("?")),
                [35, 24, 16],
                10. * s,
                true,
                27. * s,
            );
        }
        _ => {
            c.circle(cx, cy, 13. * s, GREY, EDGE);
            label = Some((v.strip_prefix("BUTTON ").unwrap_or("?"), 9., 10., 14.));
        }
    }
    if let Some((t, xx, yy, fs)) = label {
        c.text(x + xx * s, y + yy * s, t, WHITE, fs * s, false, 28. * s);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_xinput_controls_have_icons() {
        for value in [
            "A",
            "B",
            "X",
            "Y",
            "CROSS",
            "CIRCLE",
            "SQUARE",
            "TRIANGLE",
            "LB",
            "RB",
            "LT",
            "RT",
            "L",
            "R",
            "ZL",
            "ZR",
            "L1",
            "R1",
            "L2",
            "R2",
            "BACK",
            "START",
            "GUIDE",
            "MINUS",
            "PLUS",
            "HOME",
            "SHARE",
            "OPTIONS",
            "PS",
            "L3",
            "R3",
            "LS CLICK",
            "RS CLICK",
            "DPAD UP",
            "DPAD DOWN",
            "DPAD LEFT",
            "DPAD RIGHT",
            "LS UP",
            "LS DOWN",
            "LS LEFT",
            "LS RIGHT",
            "RS UP",
            "RS DOWN",
            "RS LEFT",
            "RS RIGHT",
        ] {
            assert!(supported(value));
            let pixels = vec![[0, 0, 0, 255]; 64 * 64];
            let mut c = Canvas::new(&pixels, 64, 64);
            draw(&mut c, value, 16., 16., 32.);
            assert!(
                c.pix.data().chunks_exact(4).any(|p| p[..3] != [0, 0, 0]),
                "{value}"
            );
        }
    }
}
