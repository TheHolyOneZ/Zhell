use std::path::Path;

pub fn prefers_dark() -> bool {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".config")));
    let Some(config) = config else { return true };
    if let Some(dark) = kde(&config.join("kdeglobals")) {
        return dark;
    }
    if let Some(dark) = gtk(&config.join("gtk-4.0/settings.ini")).or_else(|| gtk(&config.join("gtk-3.0/settings.ini"))) {
        return dark;
    }
    true
}

fn kde(path: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_window = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_window = line == "[Colors:Window]";
        } else if in_window && let Some(v) = line.strip_prefix("BackgroundNormal=") {
            let c: Vec<f32> = v.split(',').filter_map(|n| n.trim().parse().ok()).collect();
            if c.len() >= 3 {
                return Some(luminance(c[0], c[1], c[2]) < 0.5);
            }
        }
    }
    None
}

fn gtk(path: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines().map(str::trim) {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "gtk-application-prefer-dark-theme" => return Some(matches!(v.trim(), "1" | "true")),
            "gtk-theme-name" if v.trim().to_lowercase().contains("dark") => return Some(true),
            _ => {}
        }
    }
    None
}

fn luminance(r: f32, g: f32, b: f32) -> f32 {
    (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_kde_and_gtk() {
        let dir = std::env::temp_dir().join(format!("zhell-appearance-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let k = dir.join("kdeglobals");
        std::fs::write(&k, "[Colors:View]\nBackgroundNormal=255,255,255\n[Colors:Window]\nBackgroundNormal=40,40,40\n").unwrap();
        assert_eq!(kde(&k), Some(true));
        std::fs::write(&k, "[Colors:Window]\nBackgroundNormal=239,240,241\n").unwrap();
        assert_eq!(kde(&k), Some(false));
        let g = dir.join("settings.ini");
        std::fs::write(&g, "[Settings]\ngtk-theme-name=Adwaita\ngtk-application-prefer-dark-theme=0\n").unwrap();
        assert_eq!(gtk(&g), Some(false));
        let _ = std::fs::remove_dir_all(dir);
    }
}
