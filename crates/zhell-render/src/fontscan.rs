#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
use std::collections::HashSet;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
use std::path::{Path, PathBuf};

use cosmic_text::{FontSystem, fontdb};

const EMBEDDED: [&[u8]; 4] = [
    include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../fonts/JetBrainsMono-BoldItalic.ttf"),
];

pub const FALLBACK_FAMILY: &str = "JetBrains Mono";

const MONOSPACE: &[&str] = if cfg!(windows) {
    &["Cascadia Mono", "Cascadia Code", "Consolas", "Lucida Console"]
} else if cfg!(target_os = "macos") {
    &["SF Mono", "Menlo", "Monaco"]
} else {
    &["Noto Sans Mono", "DejaVu Sans Mono", "Ubuntu Mono", "Liberation Mono", "Noto Mono", "Adwaita Mono", "Source Code Pro", "Fira Mono"]
};

pub fn font_system() -> FontSystem {
    let t = std::time::Instant::now();
    #[allow(unused_mut)]
    let mut preferred: Vec<String> = Vec::new();
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
    let mut db = match parallel() {
        Some((db, prefs)) => {
            preferred = prefs;
            db
        }
        None => system_db(),
    };
    #[cfg(not(all(unix, not(any(target_os = "macos", target_os = "android")))))]
    let mut db = system_db();
    for font in EMBEDDED {
        db.load_font_source(fontdb::Source::Binary(std::sync::Arc::new(font)));
    }
    let mono = default_monospace(&db, &preferred);
    db.set_monospace_family(mono.clone());

    db.set_sans_serif_family("Open Sans");
    db.set_serif_family("DejaVu Serif");
    log::debug!("fonts: {} faces in {} ms, monospace = {mono}", db.len(), t.elapsed().as_millis());
    let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".into());
    FontSystem::new_with_locale_and_db(locale, db)
}

fn system_db() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    db
}

pub fn has_family(db: &fontdb::Database, family: &str) -> bool {
    db.faces().any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(family)))
}

fn default_monospace(db: &fontdb::Database, preferred: &[String]) -> String {
    let mono = |name: &str| db.faces().any(|f| f.monospaced && f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)));
    preferred
        .iter()
        .map(String::as_str)
        .chain(MONOSPACE.iter().copied())
        .find(|n| mono(n))
        .unwrap_or(FALLBACK_FAMILY)
        .to_owned()
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
fn parallel() -> Option<(fontdb::Database, Vec<String>)> {
    let t = std::time::Instant::now();
    let (dirs, preferred) = fontconfig();
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for dir in dirs {
        collect(&dir, &mut files, &mut seen, 0);
    }
    if files.is_empty() {
        return None;
    }
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
    let chunk = files.len().div_ceil(threads);
    let parts: Vec<Vec<fontdb::FaceInfo>> = std::thread::scope(|s| {
        let handles: Vec<_> = files
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    let mut db = fontdb::Database::new();
                    for f in part {
                        if let Err(e) = db.load_font_file(f) {
                            log::debug!("font {}: {e}", f.display());
                        }
                    }
                    db.faces().cloned().collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let mut db = fontdb::Database::new();
    for face in parts.into_iter().flatten() {
        db.push_face_info(face);
    }
    log::debug!("fonts: {} faces from {} files in {} ms ({threads} threads)", db.len(), files.len(), t.elapsed().as_millis());
    Some((db, preferred))
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
fn fontconfig() -> (Vec<PathBuf>, Vec<String>) {
    let home = std::env::var("HOME").ok();
    let mut fc = fontconfig_parser::FontConfig::default();
    if let Ok(file) = std::env::var("FONTCONFIG_FILE") {
        let _ = fc.merge_config(Path::new(&file));
    } else {
        let config_home = std::env::var("XDG_CONFIG_HOME").ok().map(PathBuf::from).or_else(|| home.as_ref().map(|h| Path::new(h).join(".config")));
        let read_global = config_home.is_none_or(|p| fc.merge_config(&p.join("fontconfig/fonts.conf")).is_err());
        if read_global {
            let _ = fc.merge_config(Path::new("/etc/fonts/local.conf"));
        }
        let _ = fc.merge_config(Path::new("/etc/fonts/fonts.conf"));
    }

    let monospace: Vec<String> = std::process::Command::new("fc-match")
        .args(["-f", "%{family}", "monospace"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|f| f.split(',').next().map(|f| f.trim().to_owned()))
        .filter(|f| !f.is_empty())
        .into_iter()
        .collect();
    let mut dirs: Vec<PathBuf> = fc
        .dirs
        .into_iter()
        .filter_map(|d| match d.path.strip_prefix("~") {
            Ok(rest) => home.as_ref().map(|h| Path::new(h).join(rest)),
            Err(_) => Some(d.path),
        })
        .collect();
    if dirs.is_empty() {
        dirs = vec!["/usr/share/fonts".into(), "/usr/local/share/fonts".into()];
        if let Some(h) = &home {
            dirs.push(Path::new(h).join(".fonts"));
            dirs.push(Path::new(h).join(".local/share/fonts"));
        }
    }
    (dirs, monospace)
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
fn collect(dir: &Path, out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, depth: u8) {
    if depth > 16 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();

        let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen.insert(real.clone()) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&real) else { continue };
        if meta.is_dir() {
            collect(&real, out, seen, depth + 1);
        } else if matches!(real.extension().and_then(|e| e.to_str()), Some("ttf" | "ttc" | "TTF" | "TTC" | "otf" | "otc" | "OTF" | "OTC")) {
            out.push(real);
        }
    }
}
