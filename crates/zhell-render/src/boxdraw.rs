type Arms = [u8; 4];

const BOX_TABLE: [&str; 128] = [
    "0101", "0202", "1010", "2020", "D", "D", "D", "D", "D", "D", "D", "D",
    "0110", "0210", "0120", "0220", "0011", "0012", "0021", "0022",
    "1100", "1200", "2100", "2200", "1001", "1002", "2001", "2002",
    "1110", "1210", "2110", "1120", "2120", "2210", "1220", "2220",
    "1011", "1012", "2011", "1021", "2021", "2012", "1022", "2022",
    "0111", "0112", "0211", "0212", "0121", "0122", "0221", "0222",
    "1101", "1102", "1201", "1202", "2101", "2102", "2201", "2202",
    "1111", "1112", "1211", "1212", "2111", "1121", "2121", "2112",
    "2211", "1122", "1221", "2212", "1222", "2122", "2221", "2222",
    "D", "D", "D", "D",
    "0303", "3030", "0310", "0130", "0330", "0013", "0031", "0033",
    "1300", "3100", "3300", "1003", "3001", "3003", "1310", "3130",
    "3330", "1013", "3031", "3033", "0313", "0131", "0333", "1303",
    "3101", "3303", "1313", "3131", "3333",
    "A", "A", "A", "A",
    "X", "X", "X",
    "0001", "1000", "0100", "0010", "0002", "2000", "0200", "0020",
    "0102", "1020", "0201", "2010",
];

pub fn is_builtin(ch: char) -> bool {
    matches!(ch as u32, 0x2500..=0x259F | 0xE0B0..=0xE0B6)
}

struct Canvas {
    w: usize,
    h: usize,
    a: Vec<u8>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Self { w, h, a: vec![0; w * h] }
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, alpha: u8) {
        let cx = |v: f32| (v.round().max(0.0) as usize).min(self.w);
        let cy = |v: f32| (v.round().max(0.0) as usize).min(self.h);
        let (x0, x1, y0, y1) = (cx(x0), cx(x1), cy(y0), cy(y1));
        for y in y0..y1 {
            for x in x0..x1 {
                let p = &mut self.a[y * self.w + x];
                *p = (*p).max(alpha);
            }
        }
    }

    fn sdf(&mut self, f: impl Fn(f32, f32) -> f32) {
        for y in 0..self.h {
            for x in 0..self.w {
                let d = f(x as f32 + 0.5, y as f32 + 0.5);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                let p = &mut self.a[y * self.w + x];
                *p = (*p).max((cov * 255.0).round() as u8);
            }
        }
    }
}

pub fn rasterise(ch: char, w: u32, h: u32) -> Option<Vec<u8>> {
    if !is_builtin(ch) || w == 0 || h == 0 {
        return None;
    }
    let mut c = Canvas::new(w as usize, h as usize);
    let (wf, hf) = (w as f32, h as f32);

    let light = (hf / 12.0).round().max(1.0);
    let cp = ch as u32;

    match cp {
        0x2500..=0x257F => {
            let spec = BOX_TABLE[(cp - 0x2500) as usize];
            match spec {
                "D" => dashes(&mut c, cp, light),
                "A" => arc(&mut c, cp, light),
                "X" => diagonal(&mut c, cp, light),
                _ => {
                    let b = spec.as_bytes();
                    lines(&mut c, [b[0] - b'0', b[1] - b'0', b[2] - b'0', b[3] - b'0'], light);
                }
            }
        }
        0x2580..=0x259F => blocks(&mut c, cp, wf, hf),
        0xE0B0..=0xE0B6 => powerline(&mut c, cp, light),
        _ => return None,
    }
    Some(c.a)
}

fn thickness(weight: u8, light: f32) -> f32 {
    match weight {
        1 => light,
        2 => light * 2.0,
        3 => light * 3.0,
        _ => 0.0,
    }
}

fn lines(c: &mut Canvas, arms: Arms, light: f32) {
    let (w, h) = (c.w as f32, c.h as f32);
    let [up, right, down, left] = arms;

    let cx = ((w - light) / 2.0).floor() + light / 2.0;
    let cy = ((h - light) / 2.0).floor() + light / 2.0;
    let gap = light;
    let any_double = arms.contains(&3);

    let vert_half = thickness(up, light).max(thickness(down, light)) / 2.0;
    let horiz_half = thickness(left, light).max(thickness(right, light)) / 2.0;

    for (dir, weight) in arms.iter().enumerate() {
        let weight = *weight;
        if weight == 0 {
            continue;
        }
        let horizontal = dir == 1 || dir == 3;
        if weight == 3 {
            for side in [-1.0f32, 1.0] {
                let off = side * gap;
                let (perp_a, perp_b) = if horizontal { (up, down) } else { (left, right) };
                let perp_on_side = if side < 0.0 { perp_a } else { perp_b };
                let reach = if perp_on_side == 3 {
                    -gap
                } else if perp_on_side != 0 {
                    0.0
                } else {
                    gap + light / 2.0
                };
                let t = light / 2.0;
                match dir {
                    1 => c.rect(cx - reach, cy + off - t, w, cy + off + t, 255),
                    3 => c.rect(0.0, cy + off - t, cx + reach, cy + off + t, 255),
                    0 => c.rect(cx + off - t, 0.0, cx + off + t, cy + reach, 255),
                    _ => c.rect(cx + off - t, cy - reach, cx + off + t, h, 255),
                }
            }
        } else {
            let t = thickness(weight, light) / 2.0;
            let ext_h = if any_double { gap + light / 2.0 } else { vert_half.max(t) };
            let ext_v = if any_double { gap + light / 2.0 } else { horiz_half.max(t) };
            match dir {
                1 => c.rect(cx - ext_h, cy - t, w, cy + t, 255),
                3 => c.rect(0.0, cy - t, cx + ext_h, cy + t, 255),
                0 => c.rect(cx - t, 0.0, cx + t, cy + ext_v, 255),
                _ => c.rect(cx - t, cy - ext_v, cx + t, h, 255),
            }
        }
    }
}

fn dashes(c: &mut Canvas, cp: u32, light: f32) {
    let (n, heavy, vertical) = match cp {
        0x2504 => (3, false, false),
        0x2505 => (3, true, false),
        0x2506 => (3, false, true),
        0x2507 => (3, true, true),
        0x2508 => (4, false, false),
        0x2509 => (4, true, false),
        0x250A => (4, false, true),
        0x250B => (4, true, true),
        0x254C => (2, false, false),
        0x254D => (2, true, false),
        0x254E => (2, false, true),
        _ => (2, true, true),
    };
    let t = if heavy { light * 2.0 } else { light };
    let (w, h) = (c.w as f32, c.h as f32);
    let len = if vertical { h } else { w };
    let seg = len / n as f32;

    let gap = (seg * 0.35).round().max(1.0);
    let cx = ((w - t) / 2.0).floor();
    let cy = ((h - t) / 2.0).floor();
    for i in 0..n {
        let a = (i as f32 * seg + gap / 2.0).floor();
        let b = a + (seg - gap).round().max(1.0);
        if vertical {
            c.rect(cx, a, cx + t, b, 255);
        } else {
            c.rect(a, cy, b, cy + t, 255);
        }
    }
}

fn arc(c: &mut Canvas, cp: u32, light: f32) {
    let (w, h) = (c.w as f32, c.h as f32);
    let cx = ((w - light) / 2.0).floor() + light / 2.0;
    let cy = ((h - light) / 2.0).floor() + light / 2.0;
    let r = cx.min(cy);

    let (sx, sy) = match cp {
        0x256D => (1.0, 1.0),
        0x256E => (-1.0, 1.0),
        0x256F => (-1.0, -1.0),
        _ => (1.0, -1.0),
    };
    let (ox, oy) = (cx + sx * r, cy + sy * r);
    let half = light / 2.0;
    c.sdf(|x, y| {
        if (x - ox) * sx > 0.0 || (y - oy) * sy > 0.0 {
            return f32::MAX;
        }
        let d = ((x - ox).powi(2) + (y - oy).powi(2)).sqrt();
        (d - r).abs() - half
    });

    if sx > 0.0 {
        c.rect(ox, cy - half, w, cy + half, 255);
    } else {
        c.rect(0.0, cy - half, ox, cy + half, 255);
    }
    if sy > 0.0 {
        c.rect(cx - half, oy, cx + half, h, 255);
    } else {
        c.rect(cx - half, 0.0, cx + half, oy, 255);
    }
}

fn diagonal(c: &mut Canvas, cp: u32, light: f32) {
    let (w, h) = (c.w as f32, c.h as f32);
    let half = light / 2.0;
    let len = (w * w + h * h).sqrt();

    let rising = move |x: f32, y: f32| ((h * x + w * y - w * h) / len).abs() - half;
    let falling = move |x: f32, y: f32| ((h * x - w * y) / len).abs() - half;
    match cp {
        0x2571 => c.sdf(rising),
        0x2572 => c.sdf(falling),
        _ => c.sdf(|x, y| rising(x, y).min(falling(x, y))),
    }
}

fn blocks(c: &mut Canvas, cp: u32, w: f32, h: f32) {
    let eighth_h = |n: f32| h * n / 8.0;
    let eighth_w = |n: f32| w * n / 8.0;
    match cp {
        0x2580 => c.rect(0.0, 0.0, w, eighth_h(4.0), 255),
        0x2581..=0x2588 => {
            let n = (cp - 0x2580) as f32;
            c.rect(0.0, h - eighth_h(n), w, h, 255);
        }
        0x2589..=0x258F => {
            let n = (0x2590 - cp) as f32;
            c.rect(0.0, 0.0, eighth_w(n), h, 255);
        }
        0x2590 => c.rect(eighth_w(4.0), 0.0, w, h, 255),
        0x2591 => c.rect(0.0, 0.0, w, h, 64),
        0x2592 => c.rect(0.0, 0.0, w, h, 128),
        0x2593 => c.rect(0.0, 0.0, w, h, 192),
        0x2594 => c.rect(0.0, 0.0, w, eighth_h(1.0), 255),
        0x2595 => c.rect(w - eighth_w(1.0), 0.0, w, h, 255),
        _ => {
            let q: u8 = match cp {
                0x2596 => 0b0010,
                0x2597 => 0b0001,
                0x2598 => 0b1000,
                0x2599 => 0b1011,
                0x259A => 0b1001,
                0x259B => 0b1110,
                0x259C => 0b1101,
                0x259D => 0b0100,
                0x259E => 0b0110,
                _ => 0b0111,
            };
            let (mx, my) = ((w / 2.0).round(), (h / 2.0).round());
            if q & 0b1000 != 0 {
                c.rect(0.0, 0.0, mx, my, 255);
            }
            if q & 0b0100 != 0 {
                c.rect(mx, 0.0, w, my, 255);
            }
            if q & 0b0010 != 0 {
                c.rect(0.0, my, mx, h, 255);
            }
            if q & 0b0001 != 0 {
                c.rect(mx, my, w, h, 255);
            }
        }
    }
}

fn powerline(c: &mut Canvas, cp: u32, light: f32) {
    let (w, h) = (c.w as f32, c.h as f32);
    let mid = h / 2.0;

    let slope_len = (w * w + mid * mid).sqrt();
    let upper = move |x: f32, y: f32| (mid * x - w * y) / slope_len;
    let lower = move |x: f32, y: f32| (mid * x + w * (y - h)) / slope_len;
    let mirror = |x: f32| w - x;
    match cp {
        0xE0B0 => c.sdf(|x, y| upper(x, y).max(lower(x, y))),
        0xE0B2 => c.sdf(|x, y| upper(mirror(x), y).max(lower(mirror(x), y))),
        0xE0B1 => c.sdf(|x, y| upper(x, y).abs().min(lower(x, y).abs()) - light / 2.0),
        0xE0B3 => c.sdf(|x, y| {
            let x = mirror(x);
            upper(x, y).abs().min(lower(x, y).abs()) - light / 2.0
        }),
        0xE0B4 => c.sdf(|x, y| ((x).powi(2) / (w * w) + (y - mid).powi(2) / (mid * mid)).sqrt() * w - w),
        0xE0B6 => c.sdf(|x, y| ((mirror(x)).powi(2) / (w * w) + (y - mid).powi(2) / (mid * mid)).sqrt() * w - w),
        _ => {
            c.sdf(|x, y| {
                let d = ((x).powi(2) / (w * w) + (y - mid).powi(2) / (mid * mid)).sqrt() * w - w;
                d.abs() - light / 2.0
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(a: &[u8], w: usize, y: usize) -> &[u8] {
        &a[y * w..(y + 1) * w]
    }

    fn col(a: &[u8], w: usize, h: usize, x: usize) -> Vec<u8> {
        (0..h).map(|y| a[y * w + x]).collect()
    }

    #[test]
    fn horizontal_line_spans_full_width() {
        let (w, h) = (9, 19);
        let a = rasterise('─', w, h).unwrap();
        let lit: Vec<usize> = (0..h as usize).filter(|&y| row(&a, 9, y).iter().all(|&p| p == 255)).collect();
        assert!(!lit.is_empty(), "no full-width row");
        assert!(lit.len() <= 2);

        assert_eq!(a.iter().filter(|&&p| p == 255).count(), lit.len() * w as usize);
    }

    #[test]
    fn vertical_line_spans_full_height() {
        let (w, h) = (9, 19);
        let a = rasterise('│', w, h).unwrap();
        assert!((0..w as usize).any(|x| col(&a, 9, 19, x).iter().all(|&p| p == 255)));
    }

    #[test]
    fn cross_lines_meet_at_same_pixels_as_neighbours() {
        let (w, h) = (10, 21);
        let a = rasterise('─', w, h).unwrap();
        let b = rasterise('┼', w, h).unwrap();
        assert_eq!(row(&a, 10, 0), row(&b, 10, 0).iter().map(|_| 0).collect::<Vec<_>>().as_slice());
        let edge_a = col(&a, 10, 21, 0);
        let edge_b = col(&b, 10, 21, 0);
        assert_eq!(edge_a, edge_b);
    }

    #[test]
    fn blocks_cover_expected_area() {
        let a = rasterise('█', 8, 16).unwrap();
        assert!(a.iter().all(|&p| p == 255));
        let lower = rasterise('▄', 8, 16).unwrap();
        assert_eq!(lower.iter().filter(|&&p| p == 255).count(), 8 * 8);
        assert!(row(&lower, 8, 15).iter().all(|&p| p == 255));
    }

    #[test]
    fn every_builtin_renders_something() {
        for cp in (0x2500..=0x259F).chain(0xE0B0..=0xE0B6) {
            let ch = char::from_u32(cp).unwrap();
            let a = rasterise(ch, 10, 21).unwrap();
            assert!(a.iter().any(|&p| p > 0), "U+{cp:04X} is blank");
        }
    }

    #[test]
    fn dashes_have_gaps_in_small_cells() {
        let a = rasterise('┄', 9, 19).unwrap();
        let line = (0..19).map(|y| row(&a, 9, y)).find(|r| r.iter().any(|&p| p > 0)).unwrap();
        assert!(line.iter().filter(|&&p| p == 0).count() >= 3, "{line:?}");
    }

    #[test]
    fn non_builtin_is_none() {
        assert!(rasterise('a', 10, 20).is_none());
    }
}
