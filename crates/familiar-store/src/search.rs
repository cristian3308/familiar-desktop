//! In-memory launcher search.
//!
//! Scoring: a subsequence fuzzy match (prefix and word-start hits weigh
//! most), plus a small boost from launch frequency and recency.

use crate::AppEntry;

#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub index: usize,
    pub score: i64,
}

/// Returns `None` when `query` is not a subsequence of `name`.
pub fn fuzzy_score(query: &str, name: &str) -> Option<i64> {
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let n: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    let mut score = 0i64;
    let mut qi = 0;
    let mut prev_match: Option<usize> = None;
    for (i, c) in n.iter().enumerate() {
        if qi == q.len() {
            break;
        }
        if *c != q[qi] {
            continue;
        }
        let word_start = i == 0 || !n[i - 1].is_alphanumeric();
        score += 10;
        if i == 0 {
            score += 40;
        } else if word_start {
            score += 25;
        }
        if prev_match == Some(i.wrapping_sub(1)) {
            score += 15;
        }
        prev_match = Some(i);
        qi += 1;
    }
    if qi < q.len() {
        return None;
    }
    // Shorter names win ties ("Code" over "Code Helper").
    Some(score - n.len() as i64 / 4)
}

pub struct AppSearch {
    apps: Vec<AppEntry>,
}

impl AppSearch {
    pub fn new(apps: Vec<AppEntry>) -> Self {
        Self { apps }
    }

    pub fn apps(&self) -> &[AppEntry] {
        &self.apps
    }

    pub fn get(&self, index: usize) -> Option<&AppEntry> {
        self.apps.get(index)
    }

    pub fn note_launch(&mut self, launch_target: &str, now_unix: i64) {
        if let Some(a) = self.apps.iter_mut().find(|a| a.launch_target == launch_target) {
            a.launch_count += 1;
            a.last_launched = Some(now_unix);
        }
    }

    /// Best `limit` matches. An empty query lists the most used apps.
    pub fn search(&self, query: &str, limit: usize, now_unix: i64) -> Vec<SearchHit> {
        let mut hits: Vec<SearchHit> = self
            .apps
            .iter()
            .enumerate()
            .filter_map(|(index, a)| {
                let base = fuzzy_score(query, &a.name)?;
                Some(SearchHit { index, score: base * 10 + usage_boost(a, now_unix) })
            })
            .collect();
        hits.sort_unstable_by(|a, b| {
            b.score.cmp(&a.score).then_with(|| self.apps[a.index].name.cmp(&self.apps[b.index].name))
        });
        hits.truncate(limit);
        hits
    }
}

fn usage_boost(a: &AppEntry, now_unix: i64) -> i64 {
    let freq = ((a.launch_count as f64 + 1.0).ln() * 40.0) as i64;
    let recency = match a.last_launched {
        Some(t) if now_unix - t < 3_600 => 60,
        Some(t) if now_unix - t < 86_400 => 30,
        Some(t) if now_unix - t < 7 * 86_400 => 10,
        _ => 0,
    };
    freq + recency
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps(names: &[&str]) -> AppSearch {
        AppSearch::new(
            names
                .iter()
                .map(|n| AppEntry { name: (*n).into(), launch_target: n.to_lowercase(), ..Default::default() })
                .collect(),
        )
    }

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_score("vsc", "Visual Studio Code").is_some());
        assert!(fuzzy_score("xyz", "Visual Studio Code").is_none());
        assert_eq!(fuzzy_score("", "Anything"), Some(0));
    }

    #[test]
    fn prefix_and_word_starts_rank_first() {
        let s = apps(&["Notepad", "OneNote", "Snipping Tool", "Notion"]);
        let hits = s.search("no", 10, 0);
        let names: Vec<_> = hits.iter().map(|h| s.get(h.index).unwrap().name.as_str()).collect();
        let mut top = names[..2].to_vec();
        top.sort();
        assert_eq!(top, ["Notepad", "Notion"]);
        assert!(names.contains(&"OneNote"));
    }

    #[test]
    fn usage_breaks_ties() {
        let mut s = apps(&["Spotify", "Steam"]);
        s.note_launch("steam", 1000);
        let hits = s.search("s", 10, 1000);
        assert_eq!(s.get(hits[0].index).unwrap().name, "Steam");
    }

    #[test]
    fn large_index_is_fast() {
        let names: Vec<String> = (0..5000).map(|i| format!("Application Number {i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let s = apps(&refs);
        let start = std::time::Instant::now();
        let hits = s.search("app 42", 8, 0);
        let elapsed = start.elapsed();
        assert!(!hits.is_empty());
        // Generous bound so debug builds on slow CI pass; release is < 2 ms.
        assert!(elapsed.as_millis() < 200, "{elapsed:?}");
    }
}
