use std::path::{Path, PathBuf};

use zhell_core::config::{BackgroundConfig, Fit, expand_tilde};

pub struct Loaded {
    pub generation: u64,
    pub image: Option<(u32, u32, Vec<u8>)>,
    pub shader: Option<String>,

    pub errors: Vec<String>,
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loaded").field("generation", &self.generation).field("errors", &self.errors).finish_non_exhaustive()
    }
}

pub fn resolve(p: &str, config_dir: Option<&Path>) -> PathBuf {
    let p = PathBuf::from(expand_tilde(p.trim()));
    match config_dir {
        Some(dir) if p.is_relative() => dir.join(p),
        _ => p,
    }
}

pub fn shader_file(cfg: &BackgroundConfig, config_dir: Option<&Path>) -> Option<PathBuf> {
    let s = cfg.shader.trim();
    (!s.is_empty() && !zhell_render::BUILTIN_SHADERS.iter().any(|(n, _)| *n == s)).then(|| resolve(s, config_dir))
}

pub fn load(cfg: &BackgroundConfig, config_dir: Option<&Path>, max_side: u32, generation: u64) -> Loaded {
    let mut errors = Vec::new();
    let image = (!cfg.image.trim().is_empty()).then(|| resolve(&cfg.image, config_dir)).and_then(|path| {
        let img = image::ImageReader::open(&path)
            .map_err(|e| e.to_string())
            .and_then(|r| r.with_guessed_format().map_err(|e| e.to_string()))
            .and_then(|r| r.decode().map_err(|e| e.to_string()));
        match img {
            Ok(img) => {
                let img = if img.width().max(img.height()) > max_side {
                    img.resize(max_side, max_side, image::imageops::FilterType::Triangle)
                } else {
                    img
                };
                let rgba = img.into_rgba8();
                Some((rgba.width(), rgba.height(), rgba.into_raw()))
            }
            Err(e) => {
                errors.push(format!("Background image {}: {e}", path.display()));
                None
            }
        }
    });
    let shader = match cfg.shader.trim() {
        "" => None,
        name => match zhell_render::BUILTIN_SHADERS.iter().find(|(n, _)| *n == name) {
            Some((_, src)) => Some((*src).to_owned()),
            None => {
                let path = resolve(name, config_dir);
                match std::fs::read_to_string(&path) {
                    Ok(src) => Some(src),
                    Err(e) => {
                        errors.push(format!("Background shader {}: {e}", path.display()));
                        None
                    }
                }
            }
        },
    };
    Loaded { generation, image, shader, errors }
}

pub fn fit(f: Fit) -> zhell_render::Fit {
    match f {
        Fit::Cover => zhell_render::Fit::Cover,
        Fit::Contain => zhell_render::Fit::Contain,
        Fit::Stretch => zhell_render::Fit::Stretch,
        Fit::Tile => zhell_render::Fit::Tile,
        Fit::Center => zhell_render::Fit::Center,
    }
}

pub fn short_shader_error(message: &str, name: &str) -> String {
    let reason = message.lines().find_map(|l| l.trim().strip_prefix("error:")).map(str::trim).unwrap_or(message.trim());
    let line = message.lines().find_map(|l| {
        let rest = l.split("wgsl:").nth(1)?;
        rest.split(':').next()?.trim().parse::<u32>().ok()
    });
    match line {
        Some(n) => format!("{name} line {n}: {reason}"),
        None => format!("{name}: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_errors_are_one_line() {
        let src = "fn background(px: vec2<f32>, uv: vec2<f32>) -> vec4<f32> {\n  return nope;\n}\n";
        let e = zhell_render::validate_shader(src).unwrap_err();
        let s = short_shader_error(&e, "my.wgsl");
        assert!(s.starts_with("my.wgsl line 2: ") && !s.contains('\n'), "{s}");
    }

    #[test]
    fn builtin_names_are_not_files() {
        let cfg = BackgroundConfig { shader: "aurora".into(), ..Default::default() };
        assert_eq!(shader_file(&cfg, None), None);
        let cfg = BackgroundConfig { shader: "fx.wgsl".into(), ..Default::default() };
        assert_eq!(shader_file(&cfg, Some(Path::new("/c"))), Some(PathBuf::from("/c/fx.wgsl")));
    }
}
