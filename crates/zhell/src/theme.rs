use zhell_proto::{Color, named};

pub type Rgba = [f32; 4];

pub const RADIUS: f32 = 5.0;

pub const RADIUS_SMALL: f32 = 4.0;

pub struct Theme {
    pub foreground: Rgba,
    pub background: Rgba,
    pub cursor: Rgba,

    pub accent: Rgba,

    pub selection: Rgba,

    pub palette: [Rgba; 256],
}

const fn hex(v: u32) -> Rgba {
    [
        ((v >> 16) & 0xff) as f32 / 255.0,
        ((v >> 8) & 0xff) as f32 / 255.0,
        (v & 0xff) as f32 / 255.0,
        1.0,
    ]
}

impl Default for Theme {
    fn default() -> Self {
        let ansi = [
            0x1c1b26, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
            0x414868, 0xff8ba0, 0xb4e88a, 0xf5c88a, 0x99b8ff, 0xcfb4ff, 0xa4ddff, 0xe6e9f8,
        ];
        let mut palette = [[0.0; 4]; 256];
        for (i, c) in ansi.iter().enumerate() {
            palette[i] = hex(*c);
        }
        let level = |v: usize| if v == 0 { 0.0 } else { (55.0 + v as f32 * 40.0) / 255.0 };
        for i in 0..216 {
            palette[16 + i] = [level(i / 36), level((i / 6) % 6), level(i % 6), 1.0];
        }
        for i in 0..24 {
            let v = (8.0 + i as f32 * 10.0) / 255.0;
            palette[232 + i] = [v, v, v, 1.0];
        }
        Self {
            foreground: hex(0xdcdcf0),
            background: hex(0x0f0e17),
            cursor: hex(0x7c3aed),
            accent: hex(0x7c3aed),
            selection: [0.486, 0.227, 0.929, 0.40],
            palette,
        }
    }
}

impl Theme {
    pub fn from_spec(spec: &zhell_core::config::ThemeSpec) -> Self {
        let rgb = |c: [u8; 3]| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0];
        let mut t = Self::default();
        for (dst, src) in t.palette.iter_mut().zip(spec.ansi) {
            *dst = rgb(src);
        }
        t.foreground = rgb(spec.foreground);
        t.background = rgb(spec.background);
        t.cursor = rgb(spec.cursor);
        t.accent = rgb(spec.accent);
        let s = rgb(spec.selection);
        t.selection = [s[0], s[1], s[2], 0.55];
        t
    }

    pub fn resolve(&self, c: Color) -> Rgba {
        match c {
            Color::Rgb(r, g, b) => [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0],
            Color::Indexed(i) => self.palette[i as usize],
            Color::Named(n) => match n {
                0..=15 => self.palette[n as usize],
                named::FOREGROUND => self.foreground,
                named::BACKGROUND => self.background,
                named::CURSOR => self.cursor,

                259..=266 => dim(self.palette[(n - 259) as usize]),

                267 => self.foreground,
                268 => dim(self.foreground),
                _ => self.foreground,
            },
        }
    }
}

pub fn dim(c: Rgba) -> Rgba {
    [c[0] * 0.66, c[1] * 0.66, c[2] * 0.66, c[3]]
}
