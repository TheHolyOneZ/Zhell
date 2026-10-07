use std::collections::HashMap;

use base64::Engine as _;

pub const TAG_PREFIX: &str = "zhell:img/";

const MAX_BYTES_PER_PANE: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Copy, Debug)]
struct Placement {
    cols: Option<u16>,
    rows: Option<u16>,
    move_cursor: bool,
}

#[derive(Default)]
pub struct Effect {
    pub inject: String,

    pub reply: Option<String>,

    pub placed: Option<u32>,
}

#[derive(Default)]
pub struct Images {
    pub images: HashMap<u32, Image>,

    kitty_ids: HashMap<u32, u32>,
    next_id: u32,

    pending: Option<(HashMap<String, String>, Vec<u8>)>,
    order: Vec<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub cell_w: u16,
    pub cell_h: u16,
    pub cols: u16,
}

fn parse_keys(s: &str) -> HashMap<String, String> {
    s.split(',')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

impl Images {
    fn add(&mut self, img: Image) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        self.images.insert(id, img);
        self.order.push(id);

        let mut total: usize = self.images.values().map(|i| i.rgba.len()).sum();
        while total > MAX_BYTES_PER_PANE && self.order.len() > 1 {
            let old = self.order.remove(0);
            if let Some(i) = self.images.remove(&old) {
                total -= i.rgba.len();
            }
        }
        id
    }

    fn place(&mut self, rgba: Vec<u8>, w: u32, h: u32, at: Placement, g: Geometry) -> (u32, String) {
        let Placement { cols, rows, move_cursor } = at;
        let cw = g.cell_w.max(1) as u32;
        let ch = g.cell_h.max(1) as u32;

        let mut c = cols.map_or(w.div_ceil(cw), u32::from).max(1);
        let mut r = rows.map_or(h.div_ceil(ch), u32::from).max(1);
        if c > g.cols as u32 {
            let scale = g.cols as f32 / c as f32;
            c = g.cols as u32;
            if rows.is_none() {
                r = ((r as f32 * scale).ceil() as u32).max(1);
            }
        }
        r = r.min(500);
        let (dw, dh) = (c * cw, r * ch);

        let (tw, th) = if cols.is_some() && rows.is_some() {
            (dw, dh)
        } else {
            let s = (dw as f32 / w as f32).min(dh as f32 / h as f32).min(1.0);
            (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1))
        };
        let pixels = downscale(&rgba, w, h, tw, th);
        let id = self.add(Image { width: tw, height: th, rgba: pixels, cols: c as u16, rows: r as u16 });

        let mut inject = String::new();
        for row in 0..r {
            inject.push_str(&format!("\x1b]8;id=zi{id}r{row};{TAG_PREFIX}{id}/{row}\x1b\\"));
            inject.push_str(&" ".repeat(c as usize));
            inject.push_str("\x1b]8;;\x1b\\");
            if row + 1 < r {
                inject.push_str(&format!("\r\n\x1b[{}C", 0));
            }
        }
        if !move_cursor {
            inject.push_str(&format!("\x1b[{r}A"));
        }

        (id, inject.replace("\x1b[0C", ""))
    }

    pub fn kitty(&mut self, payload: &[u8], g: Geometry) -> Effect {
        let text = String::from_utf8_lossy(payload);
        let (control, data) = text.split_once(';').unwrap_or((&text, ""));
        let mut keys = parse_keys(control);
        let mut data = data.as_bytes().to_vec();

        if let Some((first, mut acc)) = self.pending.take() {
            acc.extend_from_slice(&data);
            if keys.get("m").map(String::as_str) == Some("1") {
                self.pending = Some((first, acc));
                return Effect::default();
            }
            keys = first;
            data = acc;
        } else if keys.get("m").map(String::as_str) == Some("1") {
            self.pending = Some((keys, data));
            return Effect::default();
        }

        let get = |k: &str| keys.get(k).map(String::as_str);
        let num = |k: &str| get(k).and_then(|v| v.parse::<u32>().ok());
        let action = get("a").unwrap_or("t");
        let quiet = num("q").unwrap_or(0);
        let kid = num("i");
        let respond = |ok: Result<(), String>| -> Option<String> {
            let id = kid?;
            match ok {
                Ok(()) if quiet == 0 => Some(format!("\x1b_Gi={id};OK\x1b\\")),
                Err(e) if quiet < 2 => Some(format!("\x1b_Gi={id};{e}\x1b\\")),
                _ => None,
            }
        };

        match action {
            "q" => Effect { reply: respond(decode(&keys, &data).map(drop)), ..Default::default() },
            "d" => {
                match (get("d").unwrap_or("a"), kid) {
                    ("i" | "I", Some(k)) => {
                        if let Some(id) = self.kitty_ids.remove(&k) {
                            self.images.remove(&id);
                        }
                    }
                    ("a" | "A", _) => {
                        self.images.clear();
                        self.kitty_ids.clear();
                    }
                    _ => {}
                }
                Effect::default()
            }
            "t" | "T" | "p" => {
                let decoded = if action == "p" {
                    match kid.and_then(|k| self.kitty_ids.get(&k)).and_then(|id| self.images.get(id)) {
                        Some(i) => Ok((i.rgba.clone(), i.width, i.height)),
                        None => Err("ENOENT:image not found".to_owned()),
                    }
                } else {
                    decode(&keys, &data)
                };
                let (rgba, w, h) = match decoded {
                    Ok(v) => v,
                    Err(e) => return Effect { reply: respond(Err(e)), ..Default::default() },
                };
                if action == "t" {
                    let id = self.add(Image { width: w, height: h, rgba, cols: 0, rows: 0 });
                    if let Some(k) = kid {
                        self.kitty_ids.insert(k, id);
                    }
                    return Effect { reply: respond(Ok(())), ..Default::default() };
                }
                let cols = num("c").map(|v| v as u16).filter(|v| *v > 0);
                let rows = num("r").map(|v| v as u16).filter(|v| *v > 0);
                let move_cursor = num("C").unwrap_or(0) == 0;
                let (id, inject) = self.place(rgba, w, h, Placement { cols, rows, move_cursor }, g);
                if let Some(k) = kid {
                    self.kitty_ids.insert(k, id);
                }
                Effect { inject, reply: respond(Ok(())), placed: Some(id) }
            }
            _ => Effect::default(),
        }
    }

    pub fn sixel(&mut self, payload: &[u8], g: Geometry) -> Effect {
        let Some((rgba, w, h)) = decode_sixel(payload) else { return Effect::default() };
        let (id, mut inject) = self.place(rgba, w, h, Placement { cols: None, rows: None, move_cursor: true }, g);

        inject.push_str("\r\n");
        Effect { inject, reply: None, placed: Some(id) }
    }
}

fn decode(keys: &HashMap<String, String>, data: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let get = |k: &str| keys.get(k).map(String::as_str);
    let num = |k: &str| get(k).and_then(|v| v.parse::<u32>().ok());
    let raw = base64::engine::general_purpose::STANDARD
        .decode(data.iter().copied().filter(|c| !c.is_ascii_whitespace()).collect::<Vec<u8>>())
        .map_err(|_| "EINVAL:bad base64".to_owned())?;
    let mut bytes = match get("t").unwrap_or("d") {
        "d" => raw,
        "f" | "t" => {
            let path = String::from_utf8(raw).map_err(|_| "EINVAL:bad path".to_owned())?;

            let p = std::path::Path::new(&path);
            if !p.is_file() {
                return Err("EBADF:not a file".into());
            }
            let b = std::fs::read(p).map_err(|e| format!("EBADF:{e}"))?;
            if get("t") == Some("t") && path.contains("tty-graphics-protocol") {
                let _ = std::fs::remove_file(p);
            }
            b
        }
        _ => return Err("EINVAL:unsupported transmission medium".into()),
    };
    if get("o") == Some("z") {
        bytes = miniz_oxide::inflate::decompress_to_vec_zlib(&bytes).map_err(|_| "EINVAL:bad zlib data".to_owned())?;
    }
    match num("f").unwrap_or(32) {
        100 => decode_png(&bytes),
        f @ (24 | 32) => {
            let (w, h) = (num("s").unwrap_or(0), num("v").unwrap_or(0));
            let bpp = (f / 8) as usize;
            if w == 0 || h == 0 || bytes.len() < w as usize * h as usize * bpp {
                return Err("EINVAL:size mismatch".into());
            }
            let rgba = if f == 32 {
                bytes[..w as usize * h as usize * 4].to_vec()
            } else {
                bytes.as_chunks::<3>().0.iter().take(w as usize * h as usize).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
            };
            Ok((rgba, w, h))
        }
        _ => Err("EINVAL:unsupported format".into()),
    }
}

fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("EINVAL:{e}"))?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w as u64 * h as u64 > 64_000_000 {
        return Err("EFBIG:image too large".into());
    }
    let mut buf = vec![0; reader.output_buffer_size().ok_or("EINVAL:png size")?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("EINVAL:{e}"))?;
    let buf = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf.to_vec(),
        png::ColorType::Rgb => buf.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("EINVAL:indexed png".into()),
    };
    Ok((rgba, w, h))
}

fn downscale(src: &[u8], w: u32, h: u32, tw: u32, th: u32) -> Vec<u8> {
    if tw >= w && th >= h {
        return src.to_vec();
    }
    let mut out = vec![0u8; (tw * th * 4) as usize];
    for y in 0..th {
        let y0 = (y as u64 * h as u64 / th as u64) as u32;
        let y1 = (((y + 1) as u64 * h as u64 / th as u64) as u32).max(y0 + 1).min(h);
        for x in 0..tw {
            let x0 = (x as u64 * w as u64 / tw as u64) as u32;
            let x1 = (((x + 1) as u64 * w as u64 / tw as u64) as u32).max(x0 + 1).min(w);
            let mut acc = [0u64; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * w + sx) * 4) as usize;
                    for k in 0..4 {
                        acc[k] += src[i + k] as u64;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            let o = ((y * tw + x) * 4) as usize;
            for k in 0..4 {
                out[o + k] = (acc[k] / n) as u8;
            }
        }
    }
    out
}

pub fn decode_sixel(payload: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let q = payload.iter().position(|&c| c == b'q')?;
    let params: Vec<u32> = std::str::from_utf8(&payload[..q]).ok()?.split(';').map(|p| p.parse().unwrap_or(0)).collect();

    let transparent_bg = params.get(1) == Some(&1);
    let data = &payload[q + 1..];

    let mut palette: Vec<[u8; 4]> = default_palette();
    let mut color = 0usize;
    let (mut x, mut y) = (0u32, 0u32);
    let mut w = 0u32;

    let mut pixels: HashMap<(u32, u32), [u8; 4]> = HashMap::new();
    let mut i = 0;
    let read_num = |i: &mut usize| -> u32 {
        let mut n = 0u32;
        while *i < data.len() && data[*i].is_ascii_digit() {
            n = n.saturating_mul(10).saturating_add((data[*i] - b'0') as u32);
            *i += 1;
        }
        n
    };
    let put = |pixels: &mut HashMap<(u32, u32), [u8; 4]>, x: u32, y: u32, bits: u8, c: [u8; 4], count: u32| {
        for dx in 0..count {
            for b in 0..6 {
                if bits & (1 << b) != 0 {
                    pixels.insert((x + dx, y + b), c);
                }
            }
        }
    };
    let mut raster: Option<(u32, u32)> = None;
    while i < data.len() {
        let c = data[i];
        match c {
            b'"' => {
                i += 1;
                let mut vals = Vec::new();
                loop {
                    vals.push(read_num(&mut i));
                    if i < data.len() && data[i] == b';' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                if vals.len() >= 4 {
                    raster = Some((vals[2], vals[3]));
                }
            }
            b'#' => {
                i += 1;
                let n = read_num(&mut i) as usize;
                if i < data.len() && data[i] == b';' {
                    i += 1;
                    let mut v = Vec::new();
                    loop {
                        v.push(read_num(&mut i));
                        if i < data.len() && data[i] == b';' {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    if v.len() >= 4 {
                        let rgb = if v[0] == 2 {
                            [pct(v[1]), pct(v[2]), pct(v[3])]
                        } else {
                            hls_to_rgb(v[1], v[2], v[3])
                        };
                        if n >= palette.len() {
                            palette.resize(n + 1, [0, 0, 0, 255]);
                        }
                        palette[n] = [rgb[0], rgb[1], rgb[2], 255];
                    }
                }
                color = n.min(palette.len().saturating_sub(1).max(n));
                if color >= palette.len() {
                    palette.resize(color + 1, [0, 0, 0, 255]);
                }
            }
            b'!' => {
                i += 1;
                let count = read_num(&mut i).max(1);
                if i < data.len() && (b'?'..=b'~').contains(&data[i]) {
                    put(&mut pixels, x, y, data[i] - b'?', palette[color], count);
                    x += count;
                    w = w.max(x);
                    i += 1;
                }
            }
            b'$' => {
                x = 0;
                i += 1;
            }
            b'-' => {
                x = 0;
                y += 6;
                i += 1;
            }
            b'?'..=b'~' => {
                put(&mut pixels, x, y, c - b'?', palette[color], 1);
                x += 1;
                w = w.max(x);
                i += 1;
            }
            _ => i += 1,
        }
        if w > 10_000 || y > 10_000 {
            return None;
        }
    }
    let h = pixels.keys().map(|(_, py)| py + 1).max().unwrap_or(0).max(raster.map_or(0, |r| r.1));
    let w = w.max(raster.map_or(0, |r| r.0));
    if w == 0 || h == 0 {
        return None;
    }
    let bg = if transparent_bg { [0, 0, 0, 0] } else { [0, 0, 0, 255] };
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for py in 0..h {
        for px in 0..w {
            rgba.extend_from_slice(pixels.get(&(px, py)).unwrap_or(&bg));
        }
    }
    Some((rgba, w, h))
}

fn pct(v: u32) -> u8 {
    ((v.min(100) * 255 + 50) / 100) as u8
}

fn hls_to_rgb(h: u32, l: u32, s: u32) -> [u8; 3] {
    let h = ((h + 240) % 360) as f32 / 360.0;
    let (l, s) = (l.min(100) as f32 / 100.0, s.min(100) as f32 / 100.0);
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return [v, v, v];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |t: f32| {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round() as u8
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}

fn default_palette() -> Vec<[u8; 4]> {
    let c = [
        (0, 0, 0), (20, 20, 80), (80, 13, 13), (20, 80, 20), (80, 20, 80), (20, 80, 80), (80, 80, 20), (53, 53, 53),
        (26, 26, 26), (33, 33, 60), (60, 26, 26), (33, 60, 33), (60, 33, 60), (33, 60, 60), (60, 60, 33), (80, 80, 80),
    ];
    let mut p: Vec<[u8; 4]> = c.iter().map(|&(r, g, b)| [pct(r), pct(g), pct(b), 255]).collect();
    p.resize(256, [0, 0, 0, 255]);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: Geometry = Geometry { cell_w: 10, cell_h: 20, cols: 80 };

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&vec![200u8; (w * h * 4) as usize]).unwrap();
        }
        out
    }

    #[test]
    fn kitty_png_places_tagged_rows_and_replies() {
        let mut imgs = Images::default();
        let b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes(30, 50));
        let e = imgs.kitty(format!("a=T,f=100,i=7;{b64}").as_bytes(), G);
        let id = e.placed.unwrap();
        let img = &imgs.images[&id];
        assert_eq!((img.cols, img.rows), (3, 3));
        assert_eq!(e.inject.matches(TAG_PREFIX).count(), 3);
        assert_eq!(e.reply.as_deref(), Some("\x1b_Gi=7;OK\x1b\\"));
    }

    #[test]
    fn kitty_chunks_and_raw_rgb() {
        let mut imgs = Images::default();
        let raw = vec![255u8; 4 * 2 * 3];
        let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
        let (a, b) = b64.split_at(8);
        assert!(imgs.kitty(format!("a=T,f=24,s=4,v=2,m=1;{a}").as_bytes(), G).placed.is_none());
        let e = imgs.kitty(format!("m=0;{b}").as_bytes(), G);
        assert!(e.placed.is_some());
    }

    #[test]
    fn kitty_query_and_errors() {
        let mut imgs = Images::default();
        let ok = imgs.kitty(b"a=q,i=31,s=1,v=1,f=24,t=d;AAAA", G);
        assert_eq!(ok.reply.as_deref(), Some("\x1b_Gi=31;OK\x1b\\"));
        assert!(imgs.images.is_empty(), "queries store nothing");
        let bad = imgs.kitty(b"a=T,i=2,f=100;bm90IGEgcG5n", G);
        assert!(bad.reply.unwrap().starts_with("\x1b_Gi=2;EINVAL"));
    }

    #[test]
    fn wide_images_fit_the_terminal() {
        let mut imgs = Images::default();
        let b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes(2000, 100));
        let id = imgs.kitty(format!("a=T,f=100;{b64}").as_bytes(), G).placed.unwrap();
        let img = &imgs.images[&id];
        assert_eq!(img.cols, 80);
        assert!(img.width <= 800 && img.height <= img.rows as u32 * 20);
    }

    #[test]
    fn sixel_decodes() {
        let s = b"0;0q\"1;1;4;6#1;2;100;0;0#1~~#2;2;0;100;0#2~~";
        let (rgba, w, h) = decode_sixel(s).unwrap();
        assert_eq!((w, h), (4, 6));
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba[8..12], &[0, 255, 0, 255]);
    }
}
