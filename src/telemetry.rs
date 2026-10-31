/// Terminal-safe bounded text. Never pass response bodies or provider error strings here.
pub fn display(s: &str) -> String {
    display_limit(s, 4096)
}
/// Bounded plain-text views may display longer saved descriptions without logging them.
pub fn display_limit(s: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for x in chars.by_ref() {
                        if ('@'..='~').contains(&x) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(x) = chars.next() {
                        if x == '\u{7}' || (x == '\u{1b}' && chars.next() == Some('\\')) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !c.is_control() || c == '\n' || c == '\t' {
            out.push(c);
        }
        if out.len() >= limit {
            break;
        }
    }
    out
}
/// URLs in events expose neither query credentials nor fragments.
pub fn identity(s: &str) -> String {
    if let Ok(mut u) = url::Url::parse(s) {
        u.set_query(None);
        u.set_fragment(None);
        let _ = u.set_username("");
        let _ = u.set_password(None);
        display(u.as_str())
    } else {
        display(s)
    }
}
