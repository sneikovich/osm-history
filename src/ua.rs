//! Coarse User-Agent classification. Only the family is kept; the raw string is never stored.

/// Returns `(os, browser)`. Order matters: many UAs mention several engines.
pub fn parse(ua: &str) -> (Option<&'static str>, Option<&'static str>) {
    (os(ua), browser(ua))
}

fn os(ua: &str) -> Option<&'static str> {
    const RULES: &[(&str, &str)] = &[
        ("Windows", "Windows"),
        ("Android", "Android"),
        ("iPhone", "iOS"),
        ("iPad", "iOS"),
        ("CrOS", "ChromeOS"),
        ("Mac OS X", "macOS"),
        ("Macintosh", "macOS"),
        ("OpenBSD", "OpenBSD"),
        ("FreeBSD", "FreeBSD"),
        ("Linux", "Linux"),
    ];
    first_match(ua, RULES)
}

fn browser(ua: &str) -> Option<&'static str> {
    const RULES: &[(&str, &str)] = &[
        ("Edg/", "Edge"),
        ("OPR/", "Opera"),
        ("Firefox/", "Firefox"),
        ("FxiOS/", "Firefox"),
        ("Chrome/", "Chrome"),
        ("CriOS/", "Chrome"),
        ("Safari/", "Safari"),
        ("curl/", "curl"),
    ];
    first_match(ua, RULES)
}

fn first_match(ua: &str, rules: &[(&str, &'static str)]) -> Option<&'static str> {
    rules
        .iter()
        .find(|(needle, _)| ua.contains(needle))
        .map(|&(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn common_agents() {
        let cases = [
            (
                "Mozilla/5.0 (X11; Linux x86_64; rv:143.0) Gecko/20100101 Firefox/143.0",
                (Some("Linux"), Some("Firefox")),
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0",
                (Some("Windows"), Some("Edge")),
            ),
            (
                "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Mobile Safari/537.36",
                (Some("Android"), Some("Chrome")),
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
                (Some("iOS"), Some("Safari")),
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
                (Some("macOS"), Some("Chrome")),
            ),
            ("curl/8.15.0", (None, Some("curl"))),
            ("", (None, None)),
        ];
        for (ua, expected) in cases {
            assert_eq!(parse(ua), expected, "{ua}");
        }
    }
}
