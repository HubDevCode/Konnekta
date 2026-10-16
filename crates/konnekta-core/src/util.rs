use std::path::{Path, PathBuf};

/// Make a file name received from the network safe to use locally: strips
/// directories, control characters and reserved names. Never returns an empty
/// string or something that escapes the target directory.
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut out: String = base
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c => c,
        })
        .collect();
    out = out.trim().trim_matches('.').to_string();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3",
    ];
    let stem = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    if out.is_empty() || RESERVED.contains(&stem.as_str()) {
        out = format!("file_{out}");
    }
    if out.len() > 200 {
        let ext = Path::new(&out)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", &e[..e.len().min(16)]))
            .unwrap_or_default();
        let mut cut = 200 - ext.len();
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out = format!("{}{}", &out[..cut], ext);
    }
    out
}

/// Pick a path inside `dir` that does not exist yet: `name`, `name (1)`, ...
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = p.extension().and_then(|s| s.to_str());
    for i in 1.. {
        let n = match ext {
            Some(e) => format!("{stem} ({i}).{e}"),
            None => format!("{stem} ({i})"),
        };
        let c = dir.join(n);
        if !c.exists() {
            return c;
        }
    }
    unreachable!()
}

/// Truncate a string for logging without leaking full private content.
pub fn redact(s: &str) -> String {
    format!("<{} chars redacted>", s.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(sanitize_filename(".."), "file_");
        assert_eq!(sanitize_filename("con.txt"), "file_con.txt");
        assert_eq!(sanitize_filename("a\u{0}b\nc.png"), "abc.png");
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        let long = format!("{}.jpg", "a".repeat(500));
        let s = sanitize_filename(&long);
        assert!(s.len() <= 200 && s.ends_with(".jpg"));
    }

    #[test]
    fn unique() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), b"").unwrap();
        assert_eq!(unique_path(d.path(), "a.txt"), d.path().join("a (1).txt"));
        assert_eq!(unique_path(d.path(), "b.txt"), d.path().join("b.txt"));
    }
}
