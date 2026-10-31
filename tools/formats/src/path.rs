//! Content paths are case-insensitive and use forward slashes.

/// Normalizes a virtual path: lowercase, `/` separators, no leading `./` or `/`,
/// no duplicate separators.
pub fn normalize(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for part in path.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(&part.to_ascii_lowercase());
    }
    out
}

/// Splits a normalized path into (directory, file name).
pub fn split(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    }
}

/// Matches a file name against a wildcard pattern supporting `*` and `?`.
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    let p = pattern.as_bytes();
    let n = name.as_bytes();
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            pi += 1;
            mark = ni;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert_eq!(normalize("Models\\Props//Barrel01.MDL"), "models/props/barrel01.mdl");
        assert_eq!(normalize("./maps/test_map.bsp"), "maps/test_map.bsp");
        assert_eq!(normalize("/lua/"), "lua");
    }

    #[test]
    fn wildcards() {
        assert!(wildcard_match("*.bsp", "test_map.bsp"));
        assert!(wildcard_match("test_*", "test_flat.bsp"));
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("barrel0?.mdl", "barrel01.mdl"));
        assert!(!wildcard_match("*.mdl", "barrel01.vvd"));
    }
}
