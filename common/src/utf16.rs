use anyhow::Result;

pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

// byte_offset 为 UTF-8 字节偏移
pub fn utf16_offset(text: &str, byte_offset: usize) -> usize {
    let mut count = 0;
    for (i, c) in text.char_indices() {
        if i >= byte_offset {
            break;
        }
        count += c.len_utf16();
    }
    count
}

pub fn char_from_utf16_offset(text: &str, utf16_offset: usize) -> Result<usize> {
    let mut current = 0;
    for (i, c) in text.char_indices() {
        if current == utf16_offset {
            return Ok(i);
        }
        let next = current + c.len_utf16();
        if next > utf16_offset {
            return Ok(i);
        }
        current = next;
    }
    Ok(text.len())
}

pub fn utf16_slice(text: &str, utf16_start: usize, utf16_end: usize) -> String {
    let char_start = match char_from_utf16_offset(text, utf16_start) {
        Ok(pos) => pos,
        Err(_) => return String::new(),
    };
    let char_end = match char_from_utf16_offset(text, utf16_end) {
        Ok(pos) => pos,
        Err(_) => text.len(),
    };
    text[char_start..char_end].to_string()
}
