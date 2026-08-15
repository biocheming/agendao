pub mod color {
    pub fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                // CSI sequence: ESC [
                if chars.peek() == Some(&'[') {
                    chars.next(); // consume '['
                                  // Consume parameter bytes (0x30-0x3F) and intermediate bytes (0x20-0x2F)
                                  // until final byte (0x40-0x7E)
                    loop {
                        match chars.next() {
                            Some(c) if ('\x40'..='\x7e').contains(&c) => break,
                            Some(_) => continue,
                            None => break,
                        }
                    }
                    continue;
                }
                // OSC sequence: ESC ]
                if chars.peek() == Some(&']') {
                    chars.next();
                    // Consume until ST (ESC \ or BEL \x07)
                    loop {
                        match chars.next() {
                            Some('\x07') => break,
                            Some('\x1b') if chars.peek() == Some(&'\\') => {
                                chars.next();
                                break;
                            }
                            Some(_) => continue,
                            None => break,
                        }
                    }
                    continue;
                }
                // Simple two-byte escape: ESC + single char
                if chars.peek().is_some() {
                    chars.next();
                }
                continue;
            }
            out.push(ch);
        }
        out
    }
}

pub mod format {
    pub fn truncate_chars(value: &str, limit: usize) -> String {
        if value.chars().count() <= limit {
            return value.to_string();
        }
        let mut truncated = value
            .chars()
            .take(limit.saturating_sub(24))
            .collect::<String>();
        truncated.push_str("\n...[truncated]...");
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color() {
        let input = "\x1b[32mhello\x1b[0m";
        assert_eq!(color::strip_ansi(input), "hello");
    }
}
