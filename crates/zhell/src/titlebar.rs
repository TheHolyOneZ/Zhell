use zhell_core::config::Buttons;
use zhell_core::layout::Rect;
use zhell_render::{CellMetrics, Renderer};

use crate::draw::{TabInfo, label};
use crate::theme::{RADIUS, RADIUS_SMALL, Rgba, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Tab(usize),

    Port(usize, u16),
    CloseTab(usize),
    NewTab,
    Minimize,
    Maximize,
    Close,

    Drag,
}

pub struct Layout {
    pub height: f32,
    pub logo: Rect,
    pub tabs: Vec<Rect>,
    pub new_tab: Rect,

    pub chips: Vec<(usize, u16, Rect)>,

    pub controls: Vec<(Hit, Rect)>,
    width: f32,
}

pub fn height(m: CellMetrics, scale: f32) -> f32 {
    (m.height + 16.0 * scale).max(34.0 * scale).round()
}

pub fn layout(width: f32, tabs_info: &[TabInfo], m: CellMetrics, scale: f32, buttons: Buttons) -> Layout {
    let tab_count = tabs_info.len();
    let h = height(m, scale);
    let pad = (8.0 * scale).round();
    let logo_size = (20.0 * scale).round();
    let logo = Rect { x: pad + 2.0 * scale, y: ((h - logo_size) / 2.0).round(), w: logo_size, h: logo_size };

    let btn_w = (match buttons {
        Buttons::Dots => 20.0,
        _ => 30.0,
    } * scale)
        .round();
    let mut controls_v = Vec::new();
    let mut right = width - if buttons == Buttons::Dots { 8.0 * scale } else { 2.0 * scale };
    if buttons != Buttons::None {
        for hit in [Hit::Close, Hit::Maximize, Hit::Minimize] {
            right -= btn_w;
            controls_v.push((hit, Rect { x: right, y: 0.0, w: btn_w, h }));
        }
        controls_v.reverse();
    }

    let drag_space = (48.0 * scale).round();
    let new_w = (h - 10.0 * scale).round();
    let tabs_x = logo.x + logo.w + pad;
    let tabs_space = (right - drag_space - new_w - pad - tabs_x).max(0.0);
    let tab_w = if tab_count == 0 { 0.0 } else { (tabs_space / tab_count as f32).min(m.width * 26.0 + 24.0 * scale).floor() };
    let inset = (5.0 * scale).round();
    let tabs: Vec<Rect> =
        (0..tab_count).map(|i| Rect { x: tabs_x + i as f32 * tab_w, y: inset, w: tab_w - 2.0 * scale, h: h - inset * 2.0 }).collect();
    let after = tabs_x + tab_count as f32 * tab_w;
    let new_tab = Rect { x: after + 2.0 * scale, y: inset, w: new_w, h: h - inset * 2.0 };

    let mut chips = Vec::new();
    for (i, (info, t)) in tabs_info.iter().zip(&tabs).enumerate() {
        let close_w = t.h + 4.0 * scale;
        let mut x = t.x + t.w - close_w;
        for port in info.ports.iter().rev() {
            let w = (port.to_string().len() as f32 + 2.0) * m.width;
            x -= w + 4.0 * scale;

            if x < t.x + m.width * 6.0 {
                break;
            }
            chips.push((i, *port, Rect { x, y: t.y + 4.0 * scale, w, h: t.h - 8.0 * scale }));
        }
    }
    Layout { height: h, logo, tabs, new_tab, chips, controls: controls_v, width }
}

impl Layout {
    pub fn tab_close(&self, i: usize, scale: f32) -> Option<Rect> {
        let t = self.tabs.get(i)?;
        let s = (t.h - 8.0 * scale).round();
        (t.w > s * 4.0).then(|| Rect { x: t.x + t.w - s - 4.0 * scale, y: t.y + (t.h - s) / 2.0, w: s, h: s })
    }

    pub fn hit(&self, x: f32, y: f32, scale: f32) -> Option<Hit> {
        if y < 0.0 || y >= self.height || x < 0.0 || x >= self.width {
            return None;
        }
        if let Some((hit, _)) = self.controls.iter().find(|(_, r)| r.contains(x, y)) {
            return Some(*hit);
        }
        if let Some((tab, port, _)) = self.chips.iter().find(|(_, _, r)| r.contains(x, y)) {
            return Some(Hit::Port(*tab, *port));
        }
        for i in 0..self.tabs.len() {
            if self.tab_close(i, scale).is_some_and(|r| r.contains(x, y)) {
                return Some(Hit::CloseTab(i));
            }
            if self.tabs[i].contains(x, y) {
                return Some(Hit::Tab(i));
            }
        }
        if self.new_tab.contains(x, y) {
            return Some(Hit::NewTab);
        }
        Some(Hit::Drag)
    }
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, 1.0]
}

pub struct State {
    pub buttons: Buttons,
    pub hover: Option<Hit>,
    pub focused: bool,
    pub maximized: bool,

    pub background: usize,

    pub opacity: f32,

    pub sharing: bool,

    pub active: Option<Rect>,
}

const LOGO_KEY: u64 = 0x5A00_0000_0000_0000;

pub fn draw(r: &mut Renderer, theme: &Theme, l: &Layout, tabs: &[TabInfo], st: &State, scale: f32) {
    let m = r.cell_metrics();
    let fg = theme.foreground;
    let bg = theme.background;
    let bar = mix(bg, fg, 0.035);
    r.rect(0.0, 0.0, l.width, l.height, [bar[0], bar[1], bar[2], st.opacity]);
    r.rect(0.0, l.height - scale.round().max(1.0), l.width, scale.round().max(1.0), mix(bg, fg, 0.09));
    let dim = |t: f32| mix(bar, fg, t);
    let text_alpha = if st.focused { 1.0 } else { 0.6 };

    let lg = l.logo;
    let size = lg.w.round().max(1.0) as u32;
    let px = icon_pixels(size);
    let s = size as f32;
    r.image(LOGO_KEY | size as u64, size, size, &px, [0.0, 0.0, s, s], [lg.x.round(), lg.y.round(), s, s]);

    let radius = RADIUS * scale;
    let active = st.active.or_else(|| tabs.iter().zip(&l.tabs).find(|(t, _)| t.active).map(|(_, r)| *r));
    for (i, (tab, rect)) in tabs.iter().zip(&l.tabs).enumerate() {
        let hovered = st.hover == Some(Hit::Tab(i)) || st.hover == Some(Hit::CloseTab(i));
        if !tab.active && hovered {
            r.rounded_rect(rect.x, rect.y, rect.w, rect.h, radius, dim(0.06));
        }
    }
    if let Some(a) = active {
        r.rounded_rect(a.x, a.y, a.w, a.h, radius, mix(bg, fg, 0.12));
        let ab = (2.0 * scale).round();
        r.rounded_rect(a.x + radius, a.y + a.h - ab, (a.w - radius * 2.0).max(0.0), ab, ab / 2.0, theme.accent);
    }
    for (i, (tab, rect)) in tabs.iter().zip(&l.tabs).enumerate() {
        let hovered = st.hover == Some(Hit::Tab(i)) || st.hover == Some(Hit::CloseTab(i));
        let ty = (rect.y + (rect.h - m.height) / 2.0).round();
        let close = l.tab_close(i, scale);
        let first_chip = l.chips.iter().filter(|(t, _, _)| *t == i).map(|(_, _, r)| r.x).fold(f32::MAX, f32::min);
        let text_end = close.map_or(rect.x + rect.w - 10.0 * scale, |c| c.x - 4.0 * scale).min(first_chip - 4.0 * scale);
        let text_w = text_end - rect.x - 10.0 * scale;
        let number = if i < 9 { format!("{} ", i + 1) } else { String::new() };
        let c_num = dim(0.35);
        let c_txt = if tab.active { fg } else { dim(0.6) };
        let x0 = rect.x + 10.0 * scale;
        let after = if tab.recording {
            let d = (7.0 * scale).round();
            r.rounded_rect(x0, (rect.y + (rect.h - d) / 2.0).round(), d, d, d / 2.0, theme.palette[1]);
            x0 + d + m.width * 0.6
        } else {
            label(r, &number, x0, ty, text_w, [c_num[0], c_num[1], c_num[2], text_alpha], false)
        };
        label(r, &tab.label, after, ty, x0 + text_w - after, [c_txt[0], c_txt[1], c_txt[2], text_alpha], tab.active);
        if let Some(c) = close
            && (tab.active || hovered)
        {
            if st.hover == Some(Hit::CloseTab(i)) {
                r.rounded_rect(c.x, c.y, c.w, c.h, RADIUS_SMALL * scale, dim(0.16));
            }
            cross(r, c, (c.w * 0.32).round(), scale, dim(0.55));
        }
    }

    for (tab, port, c) in &l.chips {
        let hovered = st.hover == Some(Hit::Port(*tab, *port));
        let fill = if hovered { mix(bg, theme.palette[2], 0.35) } else { mix(bg, theme.palette[2], 0.18) };
        r.rounded_rect(c.x, c.y, c.w, c.h, RADIUS_SMALL * scale, fill);
        let ty = (c.y + (c.h - m.height) / 2.0).round();
        let d = (5.0 * scale).round();
        r.rounded_rect(c.x + m.width * 0.6 - d / 2.0, (c.y + (c.h - d) / 2.0).round(), d, d, d / 2.0, theme.palette[2]);
        label(r, &port.to_string(), c.x + m.width * 1.3, ty, c.w, fg, false);
    }

    let n = l.new_tab;
    if st.hover == Some(Hit::NewTab) {
        r.rounded_rect(n.x, n.y, n.w, n.h, radius, dim(0.08));
    }
    let arm = (n.h * 0.22).round();
    let t = scale.round().max(1.0) * 1.5;
    let (cx, cy) = (n.x + n.w / 2.0, n.y + n.h / 2.0);
    r.rect((cx - arm).round(), (cy - t / 2.0).round(), arm * 2.0, t, dim(0.55));
    r.rect((cx - t / 2.0).round(), (cy - arm).round(), t, arm * 2.0, dim(0.55));

    let mut right = l.controls.first().map_or(l.width, |(_, r)| r.x) - 10.0 * scale;
    let ty = ((l.height - m.height) / 2.0).round();
    if st.background > 0 {
        let text = format!("● {} bg", st.background);
        let w = text.chars().count() as f32 * m.width;
        right -= w;
        label(r, &text, right, ty, w + m.width, dim(0.5), false);
        right -= 12.0 * scale;
    }
    if st.sharing {
        let text = "◉ sharing";
        let w = (text.chars().count() as f32 + 1.0) * m.width;
        right -= w;
        let chip_h = m.height + 6.0 * scale;
        r.rounded_rect(right, ((l.height - chip_h) / 2.0).round(), w, chip_h, RADIUS_SMALL * scale, [0.86, 0.22, 0.27, 0.9]);
        label(r, text, right + m.width / 2.0, ty, w, [1.0, 1.0, 1.0, 1.0], true);
    }

    let any_hovered = l.controls.iter().any(|(h, _)| st.hover == Some(*h));
    for (hit, rect) in &l.controls {
        let hovered = st.hover == Some(*hit);
        if st.buttons == Buttons::Dots {
            let d = (11.0 * scale).round();
            let (x, y) = ((rect.x + (rect.w - d) / 2.0).round(), (rect.y + (rect.h - d) / 2.0).round());
            let color = match hit {
                Hit::Close => [0.93, 0.37, 0.35, 1.0],
                Hit::Maximize => [0.38, 0.79, 0.39, 1.0],
                _ => [0.96, 0.75, 0.31, 1.0],
            };
            let c = if any_hovered { color } else { dim(0.22) };
            r.rounded_rect(x, y, d, d, d / 2.0, c);
            if hovered {
                let k = (d * 0.22).round().max(2.0);
                let ink = [0.0, 0.0, 0.0, 0.55];
                let (cx, cy) = (x + d / 2.0, y + d / 2.0);
                match hit {
                    Hit::Close => cross(r, Rect { x, y, w: d, h: d }, k, scale * 0.8, ink),
                    Hit::Minimize => r.rect((cx - k).round(), (cy - scale / 2.0).round(), k * 2.0, scale.round().max(1.0), ink),
                    _ => {
                        r.rect((cx - k).round(), (cy - scale / 2.0).round(), k * 2.0, scale.round().max(1.0), ink);
                        r.rect((cx - scale / 2.0).round(), (cy - k).round(), scale.round().max(1.0), k * 2.0, ink);
                    }
                }
            }
            continue;
        }

        let s = (22.0 * scale).round().min(rect.h - 8.0 * scale);
        let b = Rect { x: (rect.x + (rect.w - s) / 2.0).round(), y: (rect.y + (rect.h - s) / 2.0).round(), w: s, h: s };
        if hovered {
            let c = if *hit == Hit::Close { [0.86, 0.22, 0.27, 1.0] } else { dim(0.10) };
            r.rounded_rect(b.x, b.y, b.w, b.h, RADIUS_SMALL * scale, c);
        }
        let icon = if hovered && *hit == Hit::Close {
            [1.0, 1.0, 1.0, 1.0]
        } else if hovered {
            dim(0.85)
        } else {
            dim(0.45)
        };
        let line = scale.round().max(1.0);
        let k = (4.5 * scale).round();
        let (cx, cy) = ((b.x + b.w / 2.0).round(), (b.y + b.h / 2.0).round());
        match hit {
            Hit::Minimize => r.rect(cx - k, cy, k * 2.0, line, icon),
            Hit::Maximize => {
                let o = |r: &mut Renderer, x: f32, y: f32, w: f32| {
                    r.rect(x, y, w, line, icon);
                    r.rect(x, y + w - line, w, line, icon);
                    r.rect(x, y, line, w, icon);
                    r.rect(x + w - line, y, line, w, icon);
                };
                if st.maximized {
                    let w = k * 2.0 - 2.0 * scale;
                    o(r, cx - k + 2.0 * scale, cy - k, w);
                    r.rect(cx - k, cy - k + 2.0 * scale, w, w, if hovered { dim(0.10) } else { bar });
                    o(r, cx - k, cy - k + 2.0 * scale, w);
                } else {
                    o(r, cx - k, cy - k, k * 2.0);
                }
            }
            Hit::Close => cross(r, b, k, scale, icon),
            _ => {}
        }
    }
}

fn cross(r: &mut Renderer, area: Rect, k: f32, scale: f32, color: Rgba) {
    let (cx, cy) = (area.x + area.w / 2.0, area.y + area.h / 2.0);
    let dot = (1.4 * scale).max(1.2);
    let steps = (k * 2.0 / (dot * 0.5)).ceil() as i32;
    for i in 0..=steps {
        let t = -k + 2.0 * k * i as f32 / steps as f32;
        r.rounded_rect(cx + t - dot / 2.0, cy + t - dot / 2.0, dot, dot, dot / 2.0, color);
        r.rounded_rect(cx + t - dot / 2.0, cy - t - dot / 2.0, dot, dot, dot / 2.0, color);
    }
}

fn icon_pixels(size: u32) -> std::sync::Arc<Vec<u8>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static SOURCE: OnceLock<Option<image::RgbaImage>> = OnceLock::new();
    static SCALED: OnceLock<Mutex<HashMap<u32, Arc<Vec<u8>>>>> = OnceLock::new();
    let cache = SCALED.get_or_init(Default::default);
    if let Some(px) = cache.lock().ok().and_then(|c| c.get(&size).cloned()) {
        return px;
    }
    let source = SOURCE.get_or_init(|| {
        image::load_from_memory_with_format(include_bytes!("../../../assets/icons/zhell-256.png"), image::ImageFormat::Png)
            .map(|i| i.into_rgba8())
            .map_err(|e| log::warn!("app icon: {e}"))
            .ok()
    });
    let px = Arc::new(match source {
        Some(img) if img.width() == size => img.as_raw().clone(),
        Some(img) => image::imageops::resize(img, size, size, image::imageops::FilterType::Lanczos3).into_raw(),
        None => vec![0; (size * size * 4) as usize],
    });
    if let Ok(mut c) = cache.lock() {
        c.insert(size, px.clone());
    }
    px
}

pub fn window_icon() -> Option<winit::window::Icon> {
    winit::window::Icon::from_rgba(icon_pixels(256).to_vec(), 256, 256).ok()
}
