use crate::utf16::utf16_slice;
use regex::Regex;

pub fn clean_text(text: &str) -> String {
    let re = Regex::new(r"[^\w\u4e00-\u9fa5]+").unwrap();
    re.replace_all(text, "").to_lowercase()
}

pub fn truncate_text(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text.to_string();
    }
    let mut result = String::new();
    for c in text.chars().take(max_len.saturating_sub(3)) {
        result.push(c);
    }
    result.push_str("...");
    result
}

pub fn find_last_paragraph_boundary(text: &str, max_utf16: usize) -> (usize, bool) {
    let mut last_newline_utf16 = 0;
    let mut found_newline = false;

    let mut current_utf16 = 0;
    let chars: Vec<_> = text.char_indices().collect();

    for (_byte_idx, ch) in chars.iter() {
        current_utf16 += ch.len_utf16();

        if current_utf16 > max_utf16 {
            break;
        }

        if *ch == '\n' {
            last_newline_utf16 = current_utf16;
            found_newline = true;
        }
    }

    (last_newline_utf16, found_newline)
}

pub fn truncate_by_paragraph(text: &str, max_utf16: usize) -> String {
    let (boundary, is_newline) = find_last_paragraph_boundary(text, max_utf16);

    if is_newline && boundary > 0 {
        let truncated = utf16_slice(text, 0, boundary);
        truncated.trim_end_matches('\n').to_string()
    } else if max_utf16 > 0 {
        let truncated = utf16_slice(text, 0, max_utf16.saturating_sub(3));
        format!("{}...", truncated)
    } else {
        String::new()
    }
}
