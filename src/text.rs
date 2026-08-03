//! Small text-fitting helpers shared by the TUI table and the CLI.

/// Densest author description that fits `width`. Candidates from most
/// to least verbose — the full "A, B, and C" list, then "A, B, +N"
/// prefixes with a count, then "A et al." — and the first that fits
/// wins; a truncated surname is the last resort.
pub fn fit_authors(author: &str, width: usize) -> String {
    let surnames: Vec<String> = author
        .split(" and ")
        .map(|a| {
            a.trim()
                .split(',')
                .next()
                .unwrap_or("")
                .trim_matches(['{', '}'])
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect();
    let n = surnames.len();
    if n == 0 {
        return String::new();
    }
    let mut candidates: Vec<String> = vec![match n {
        1 => surnames[0].clone(),
        2 => format!("{} and {}", surnames[0], surnames[1]),
        _ => format!("{}, and {}", surnames[..n - 1].join(", "), surnames[n - 1]),
    }];
    for k in (1..n).rev() {
        candidates.push(format!("{}, +{}", surnames[..k].join(", "), n - k));
    }
    if n > 1 {
        candidates.push(format!("{} et al.", surnames[0]));
    }
    for c in &candidates {
        if c.chars().count() <= width {
            return c.clone();
        }
    }
    let mut s: String = surnames[0].chars().take(width.saturating_sub(1)).collect();
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn fit_authors_candidates() {
        let a3 = "{Quist}, J. and {Clyburn}, M. and {Fearing}, S.";
        assert_eq!(super::fit_authors(a3, 40), "Quist, Clyburn, and Fearing");
        assert_eq!(super::fit_authors(a3, 20), "Quist, Clyburn, +1");
        assert_eq!(super::fit_authors(a3, 14), "Quist, +2");
        assert_eq!(super::fit_authors(a3, 9), "Quist, +2");
        assert_eq!(super::fit_authors("{Quist}, J.", 20), "Quist");
        assert_eq!(
            super::fit_authors("{Quist}, J. and {Blomqvist}, A.", 30),
            "Quist and Blomqvist"
        );
        let many = (0..13)
            .map(|i| format!("{{A{i}}}, X."))
            .collect::<Vec<_>>()
            .join(" and ");
        assert_eq!(super::fit_authors(&many, 14), "A0, A1, +11");
        assert_eq!(super::fit_authors(&many, 10), "A0, +12");
        assert_eq!(super::fit_authors("{Verylongsurname}, Q. and {B}, C.", 8), "Verylon…");
    }
}
