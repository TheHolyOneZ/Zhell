use std::collections::HashMap;

use crate::boxdraw;

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping, Style, SwashCache, SwashContent, Weight,
};

const MAX_WORDS: usize = 50_000;

pub const ATLAS_SIZE: u32 = 2048;
const PAD: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct GlyphStyle {
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct AtlasGlyph {
    pub offset: [f32; 2],
    pub size: [f32; 2],
    pub uv: [f32; 2],
    pub color: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontConfig {
    pub family: Option<String>,
    pub size_px: f32,
    pub line_height: f32,
}

pub struct GlyphAtlas {
    fonts: FontSystem,

    full_pending: bool,
    swash: SwashCache,
    buffer: Buffer,

    font: FontConfig,

    requested: FontConfig,
    pub metrics: CellMetrics,

    advance: f32,
    map: HashMap<(String, GlyphStyle), Option<AtlasGlyph>>,

    words: HashMap<(String, GlyphStyle), Option<Vec<AtlasGlyph>>>,

    glyphs: HashMap<CacheKey, Option<AtlasGlyph>>,

    pub uploads: Vec<(u32, u32, u32, u32, Vec<u8>)>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,

    pub overflowed: bool,
}

struct Preload {
    full: Option<std::thread::JoinHandle<FontSystem>>,
    quick: Option<std::thread::JoinHandle<Option<FontSystem>>>,
}

static PRELOAD: std::sync::Mutex<Preload> = std::sync::Mutex::new(Preload { full: None, quick: None });

pub fn preload_fonts(family: Option<String>, cache: Option<std::path::PathBuf>, on_full: Option<Box<dyn FnOnce() + Send>>) {
    let Ok(mut slot) = PRELOAD.lock() else { return };
    if slot.full.is_some() {
        return;
    }
    let key = family.clone().unwrap_or_else(|| "<monospace>".into());
    if let Some(path) = cache.clone() {
        let key = key.clone();
        slot.quick = std::thread::Builder::new().name("fonts-quick".into()).spawn(move || quick_fonts(&path, &key)).ok();
    }
    slot.full = std::thread::Builder::new()
        .name("fonts".into())
        .spawn(move || {
            let mut fonts = crate::fontscan::font_system();
            let font = FontConfig { family, size_px: 16.0, line_height: 1.0 };
            for bold in [false, true] {
                fonts.get_font_matches(&attrs(&font, GlyphStyle { bold, italic: false }));
            }
            if let Some(path) = cache {
                remember_family(&fonts, &font, &key, &path);
            }
            if let Some(f) = on_full {
                f();
            }
            fonts
        })
        .ok();
}

fn quick_fonts(cache: &std::path::Path, key: &str) -> Option<FontSystem> {
    let text = std::fs::read_to_string(cache).ok()?;
    let mut lines = text.lines();
    if lines.next()? != key {
        return None;
    }
    let family = lines.next()?.to_owned();
    let mut db = cosmic_text::fontdb::Database::new();
    for path in lines {
        db.load_font_file(path).ok()?;
    }
    if db.is_empty() {
        return None;
    }
    db.set_monospace_family(&family);
    let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".into());
    Some(FontSystem::new_with_locale_and_db(locale, db))
}

fn remember_family(fonts: &FontSystem, font: &FontConfig, key: &str, cache: &std::path::Path) {
    let db = fonts.db();
    let a = attrs(font, GlyphStyle::default());
    let query = cosmic_text::fontdb::Query { families: &[a.family], ..Default::default() };
    let Some(family) = db.query(&query).and_then(|id| db.face(id)).and_then(|f| f.families.first()).map(|(n, _)| n.clone()) else { return };
    let mut paths: Vec<String> = db
        .faces()
        .filter(|f| f.families.iter().any(|(n, _)| *n == family))
        .filter_map(|f| match &f.source {
            cosmic_text::fontdb::Source::File(p) => Some(p.display().to_string()),
            _ => None,
        })
        .collect();
    paths.sort();
    paths.dedup();
    if paths.is_empty() || paths.len() > 64 {
        return;
    }
    let text = format!("{key}\n{family}\n{}\n", paths.join("\n"));
    if std::fs::read_to_string(cache).ok().as_deref() != Some(text.as_str()) {
        if let Some(dir) = cache.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(cache, text);
    }
}

fn font_system() -> (FontSystem, bool) {
    let Ok(mut slot) = PRELOAD.lock() else { return (crate::fontscan::font_system(), false) };
    let full_done = slot.full.as_ref().is_none_or(|h| h.is_finished());
    if !full_done
        && let Some(quick) = slot.quick.take().and_then(|h| h.join().ok()).flatten()
    {
        return (quick, true);
    }
    slot.quick = None;
    let full = slot.full.take().and_then(|h| h.join().ok());
    (full.unwrap_or_else(crate::fontscan::font_system), false)
}

impl GlyphAtlas {
    pub fn new(font: FontConfig) -> Self {
        let t = std::time::Instant::now();
        let (fonts, quick) = font_system();
        log::debug!("atlas: {} fonts ready after waiting {} ms", if quick { "quick-start" } else { "all" }, t.elapsed().as_millis());

        let buffer = Buffer::new_empty(Metrics::new(font.size_px, font.size_px * font.line_height));
        let mut atlas = Self {
            fonts,
            full_pending: quick,
            swash: SwashCache::new(),
            buffer,
            font: font.clone(),
            requested: font.clone(),
            metrics: CellMetrics { width: 1.0, height: 1.0, baseline: 1.0 },
            advance: 1.0,
            map: HashMap::new(),
            words: HashMap::new(),
            glyphs: HashMap::new(),
            uploads: Vec::new(),
            shelf_x: 0,
            shelf_y: 0,
            shelf_h: 0,
            overflowed: false,
        };
        atlas.set_font(font);
        log::debug!("atlas: ready at {} ms", t.elapsed().as_millis());
        atlas
    }

    pub fn upgrade_fonts(&mut self) -> bool {
        if !self.full_pending {
            return false;
        }
        let handle = {
            let Ok(mut slot) = PRELOAD.lock() else { return false };
            if !slot.full.as_ref().is_some_and(|h| h.is_finished()) {
                return false;
            }
            slot.full.take()
        };
        self.full_pending = false;
        let Some(fonts) = handle.and_then(|h| h.join().ok()) else { return false };
        self.fonts = fonts;
        self.set_font(self.requested.clone());
        true
    }

    pub fn set_font(&mut self, font: FontConfig) {
        let line_h = (font.size_px * font.line_height).round().max(1.0);
        self.buffer.set_metrics(Metrics::new(font.size_px, line_h));
        self.buffer.set_size(None, None);
        self.requested = font.clone();
        let mut font = font;
        if let Some(name) = font.family.as_deref()
            && !crate::fontscan::has_family(self.fonts.db(), name)
        {
            log::warn!("font family {name:?} isn't installed; using the default monospace font");
            font.family = None;
        }
        self.font = font;
        self.buffer.set_text("M", &attrs(&self.font, GlyphStyle::default()), Shaping::Advanced, None);
        self.buffer.shape_until_scroll(&mut self.fonts, false);
        let (adv, baseline) = self
            .buffer
            .layout_runs()
            .next()
            .and_then(|r| r.glyphs.first().map(|g| (g.w, r.line_y)))
            .unwrap_or((self.font.size_px * 0.6, line_h * 0.8));
        self.metrics = CellMetrics { width: adv.round().max(1.0), height: line_h, baseline: baseline.round() };
        self.advance = adv.max(1.0);
        self.clear();
    }
}

fn attrs(font: &FontConfig, style: GlyphStyle) -> Attrs<'_> {
        let family = match &font.family {
            Some(name) => Family::Name(name),
            None => Family::Monospace,
        };
        let mut a = Attrs::new().family(family);
        if style.bold {
            a = a.weight(Weight::BOLD);
        }
        if style.italic {
            a = a.style(Style::Italic);
        }
        a
}

impl GlyphAtlas {
    pub fn clear(&mut self) {
        self.map.clear();
        self.words.clear();
        self.glyphs.clear();
        self.uploads.clear();
        self.shelf_x = 0;
        self.shelf_y = 0;
        self.shelf_h = 0;
    }

    pub fn get(&mut self, text: &str, style: GlyphStyle) -> Option<AtlasGlyph> {
        if let Some(g) = self.map.get(&(text.to_owned(), style)) {
            return *g;
        }
        let g = self.rasterise(text, style);
        self.map.insert((text.to_owned(), style), g);
        g
    }

    pub fn get_word(&mut self, word: &str, style: GlyphStyle) -> Option<Vec<AtlasGlyph>> {
        if let Some(w) = self.words.get(&(word.to_owned(), style)) {
            return w.clone();
        }
        if self.words.len() > MAX_WORDS {
            self.words.clear();
        }
        let shaped = self.shape_word(word, style);
        self.words.insert((word.to_owned(), style), shaped.clone());
        shaped
    }

    fn shape_word(&mut self, word: &str, style: GlyphStyle) -> Option<Vec<AtlasGlyph>> {
        let attrs = attrs(&self.font, style);
        self.buffer.set_text(word, &attrs, Shaping::Advanced, None);
        self.buffer.shape_until_scroll(&mut self.fonts, false);
        let cell_w = self.metrics.width;
        let adv = self.advance;
        let baseline = self.metrics.baseline;

        let starts: Vec<usize> = word.char_indices().map(|(b, _)| b).collect();
        let mut placed = Vec::new();
        let mut phys = Vec::new();
        for run in self.buffer.layout_runs() {
            if run.line_i > 0 {
                return None;
            }
            for g in run.glyphs {
                let cell = starts.iter().position(|&b| b == g.start)?;

                let deviation = g.x - cell as f32 * adv;
                if deviation.abs() > 1.0 {
                    return None;
                }

                let mut at_origin = g.clone();
                at_origin.x = deviation;
                phys.push((cell, at_origin.physical((0.0, 0.0), 1.0)));
            }
        }
        for (cell, p) in phys {
            let g = match self.glyphs.get(&p.cache_key) {
                Some(g) => *g,
                None => {
                    let g = self.rasterise_glyph(p.cache_key);
                    self.glyphs.insert(p.cache_key, g);
                    g
                }
            };
            if let Some(mut g) = g {
                g.offset[0] += cell as f32 * cell_w + p.x as f32;
                g.offset[1] += baseline + p.y as f32;
                placed.push(g);
            }
        }
        Some(placed)
    }

    fn rasterise_glyph(&mut self, key: CacheKey) -> Option<AtlasGlyph> {
        let img = self.swash.get_image_uncached(&mut self.fonts, key)?;
        let (w, h) = (img.placement.width, img.placement.height);
        if w == 0 || h == 0 {
            return None;
        }
        let rgba: Vec<u8> = match img.content {
            SwashContent::Mask => img.data.iter().flat_map(|&a| [255, 255, 255, a]).collect(),
            SwashContent::Color => img.data.clone(),
            SwashContent::SubpixelMask => img
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [255, 255, 255, ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3) as u8])
                .collect(),
        };
        let (ax, ay) = self.allocate(w, h)?;
        self.uploads.push((ax, ay, w, h, rgba));
        Some(AtlasGlyph {
            offset: [img.placement.left as f32, -img.placement.top as f32],
            size: [w as f32, h as f32],
            uv: [ax as f32, ay as f32],
            color: img.content == SwashContent::Color,
        })
    }

    fn rasterise(&mut self, text: &str, style: GlyphStyle) -> Option<AtlasGlyph> {
        let mut chars = text.chars();
        if let (Some(ch), None) = (chars.next(), chars.next())
            && boxdraw::is_builtin(ch)
        {
            return self.rasterise_builtin(ch);
        }
        let attrs = attrs(&self.font, style);
        self.buffer.set_text(text, &attrs, Shaping::Advanced, None);
        self.buffer.shape_until_scroll(&mut self.fonts, false);

        let mut parts = Vec::new();
        let baseline = self.metrics.baseline;
        for run in self.buffer.layout_runs() {
            for glyph in run.glyphs {
                let phys = glyph.physical((0.0, 0.0), 1.0);
                parts.push(phys);
            }
        }
        let mut images = Vec::new();
        for phys in parts {
            if let Some(img) = self.swash.get_image_uncached(&mut self.fonts, phys.cache_key)
                && img.placement.width > 0
                && img.placement.height > 0
            {
                images.push((phys.x + img.placement.left, phys.y - img.placement.top, img));
            }
        }
        if images.is_empty() {
            return None;
        }
        let min_x = images.iter().map(|i| i.0).min()?;
        let min_y = images.iter().map(|i| i.1).min()?;
        let max_x = images.iter().map(|i| i.0 + i.2.placement.width as i32).max()?;
        let max_y = images.iter().map(|i| i.1 + i.2.placement.height as i32).max()?;
        let (w, h) = ((max_x - min_x) as u32, (max_y - min_y) as u32);
        let color = images.iter().any(|i| i.2.content == SwashContent::Color);

        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for (x, y, img) in &images {
            let (iw, ih) = (img.placement.width as usize, img.placement.height as usize);
            let (ox, oy) = ((x - min_x) as usize, (y - min_y) as usize);
            for row in 0..ih {
                for col in 0..iw {
                    let dst = ((oy + row) * w as usize + ox + col) * 4;
                    let px = match img.content {
                        SwashContent::Mask => {
                            let a = img.data[row * iw + col];
                            [255, 255, 255, a]
                        }
                        SwashContent::Color => {
                            let s = (row * iw + col) * 4;
                            [img.data[s], img.data[s + 1], img.data[s + 2], img.data[s + 3]]
                        }
                        SwashContent::SubpixelMask => {
                            let s = (row * iw + col) * 4;
                            let a = img.data[s..s + 3].iter().map(|&v| v as u32).sum::<u32>() / 3;
                            [255, 255, 255, a as u8]
                        }
                    };

                    for k in 0..4 {
                        rgba[dst + k] = rgba[dst + k].max(px[k]);
                    }
                }
            }
        }

        let (ax, ay) = self.allocate(w, h)?;
        self.uploads.push((ax, ay, w, h, rgba));
        Some(AtlasGlyph {
            offset: [min_x as f32, baseline + min_y as f32],
            size: [w as f32, h as f32],
            uv: [ax as f32, ay as f32],
            color,
        })
    }

    fn rasterise_builtin(&mut self, ch: char) -> Option<AtlasGlyph> {
        let (w, h) = (self.metrics.width as u32, self.metrics.height as u32);
        let mask = boxdraw::rasterise(ch, w, h)?;
        let rgba = mask.iter().flat_map(|&a| [255, 255, 255, a]).collect();
        let (ax, ay) = self.allocate(w, h)?;
        self.uploads.push((ax, ay, w, h, rgba));
        Some(AtlasGlyph { offset: [0.0, 0.0], size: [w as f32, h as f32], uv: [ax as f32, ay as f32], color: false })
    }

    fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w + PAD > ATLAS_SIZE || h + PAD > ATLAS_SIZE {
            return None;
        }
        if self.shelf_x + w + PAD > ATLAS_SIZE {
            self.shelf_y += self.shelf_h;
            self.shelf_x = 0;
            self.shelf_h = 0;
        }
        if self.shelf_y + h + PAD > ATLAS_SIZE {
            log::debug!("glyph atlas full; clearing");
            self.clear();
            self.overflowed = true;
        }
        let pos = (self.shelf_x, self.shelf_y);
        self.shelf_x += w + PAD;
        self.shelf_h = self.shelf_h.max(h + PAD);
        Some(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atlas() -> GlyphAtlas {
        GlyphAtlas::new(FontConfig { family: None, size_px: 16.0, line_height: 1.2 })
    }

    #[test]
    fn metrics_are_sane() {
        let a = atlas();
        let m = a.metrics;
        assert!(m.width >= 6.0 && m.width <= 14.0, "{m:?}");
        assert_eq!(m.height, 19.0);
        assert!(m.baseline > 8.0 && m.baseline < m.height, "{m:?}");
    }

    #[test]
    fn words_line_up_with_cells() {
        let mut a = atlas();
        let w = a.get_word("hello", GlyphStyle::default()).expect("monospace word");
        assert_eq!(w.len(), 5);
        let cw = a.metrics.width;
        for (i, g) in w.iter().enumerate() {
            assert!(g.offset[0] >= i as f32 * cw - 2.0 && g.offset[0] < (i + 1) as f32 * cw, "{i}: {g:?}");
        }

        let uploads = a.uploads.len();
        a.get_word("hello", GlyphStyle::default());
        assert_eq!(a.uploads.len(), uploads);
    }

    #[test]
    fn rasterises_and_caches() {
        let mut a = atlas();
        let g = a.get("A", GlyphStyle::default()).expect("glyph");
        assert!(g.size[0] > 0.0 && g.size[1] > 0.0);
        assert!(!g.color);
        assert_eq!(a.uploads.len(), 1);
        a.get("A", GlyphStyle::default());
        assert_eq!(a.uploads.len(), 1, "second lookup must hit the cache");
        assert!(a.get(" ", GlyphStyle::default()).is_none());
    }
}
