pub fn score(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    let t: Vec<char> = text.chars().collect();
    let lower: Vec<char> = t.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let mut positions = Vec::with_capacity(q.len());
    let mut score = 0;
    let mut ti = 0;
    let mut prev: Option<usize> = None;
    for qc in &q {
        let next_any = (ti..lower.len()).find(|&i| lower[i] == *qc)?;
        let word_start = |i: usize| i == 0 || !t[i - 1].is_alphanumeric() || (t[i].is_uppercase() && t[i - 1].is_lowercase());
        let pick = if prev.is_some_and(|p| p + 1 == next_any) {
            next_any
        } else {
            (next_any..lower.len()).find(|&i| lower[i] == *qc && word_start(i)).unwrap_or(next_any)
        };
        score += 1;
        if word_start(pick) {
            score += 8;
        }
        if prev.is_some_and(|p| p + 1 == pick) {
            score += 5;
        }
        if let Some(p) = prev {
            score -= ((pick - p - 1) as i32).min(5);
        }
        positions.push(pick);
        prev = Some(pick);
        ti = pick + 1;
    }

    score -= (t.len() as i32) / 16;
    Some((score, positions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_in_order_case_insensitive() {
        assert!(score("spr", "Split pane right").is_some());
        assert!(score("xyz", "Split pane right").is_none());
        assert_eq!(score("", "anything").unwrap().0, 0);
    }

    #[test]
    fn word_starts_beat_scattered_matches() {
        let a = score("sp", "Split pane right").unwrap().0;
        let b = score("sp", "Previous tab").map_or(i32::MIN, |s| s.0);
        assert!(a > b);
        let (s1, _) = score("nt", "New tab").unwrap();
        let (s2, _) = score("nt", "Font: reset size").unwrap();
        assert!(s1 > s2, "{s1} vs {s2}");
    }

    #[test]
    fn positions_point_at_matched_chars() {
        let (_, pos) = score("nt", "New tab").unwrap();
        assert_eq!(pos, vec![0, 4]);
    }
}
