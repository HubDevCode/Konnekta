use std::path::{Path, PathBuf};

/// Make a file name received from the network safe to use locally: strips
/// directories, control characters and reserved names. Never returns an empty
/// string or something that escapes the target directory.
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut out: String = base
        .chars()
        // Control and text-direction characters (e.g. "invoice\u{202E}fdp.exe").
        .filter(|c| !c.is_control() && !matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c => c,
        })
        .collect();
    out = out.trim().trim_matches('.').to_string();
    // Never hidden, never mistaken for a command-line option.
    if out.starts_with('-') {
        out = format!("_{out}");
    }
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

/// Free space check before accepting a file: at least 1 GiB must stay free
/// (5% of the disk on disks smaller than 20 GiB).
pub fn enough_space(dir: &Path, incoming: u64) -> bool {
    let (Ok(free), Ok(total)) = (fs2::available_space(dir), fs2::total_space(dir)) else {
        return true; // cannot tell: do not block
    };
    let reserve = if total < 20 << 30 {
        total / 20
    } else {
        1 << 30
    };
    free.saturating_sub(incoming) >= reserve
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
        assert_eq!(
            sanitize_filename("fattura\u{202E}fdp.exe"),
            "fatturafdp.exe"
        );
        assert_eq!(sanitize_filename("-rf"), "_-rf");
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
