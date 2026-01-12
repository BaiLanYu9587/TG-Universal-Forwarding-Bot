use crate::model::{EntityType, TextEntity};
use common::text::truncate_by_paragraph;
use common::utf16::{char_from_utf16_offset, utf16_len};

pub fn trim_long_entities_if_needed(
    text: &str,
    entities: &[TextEntity],
    limit: usize,
    header_utf16_len: usize,
    footer_utf16_len: usize,
) -> (String, Vec<TextEntity>) {
    let current_len = utf16_len(text);
    if current_len <= limit {
        return (text.to_string(), entities.to_vec());
    }

    let target_entities: Vec<_> = entities
        .iter()
        .filter(|e| {
            matches!(
                e.entity_type,
                EntityType::Code | EntityType::Pre | EntityType::Blockquote
            )
        })
        .cloned()
        .collect();

    if target_entities.is_empty() {
        let available = limit
            .saturating_sub(header_utf16_len)
            .saturating_sub(footer_utf16_len)
            .saturating_sub(3);

        if available > 0 {
            let truncated = truncate_by_paragraph(text, available);
            let truncated_len = utf16_len(&truncated) as i32;
            let filtered_entities: Vec<TextEntity> = entities
                .iter()
                .filter(|ent| ent.offset + ent.length <= truncated_len)
                .cloned()
                .collect();
            return (truncated, filtered_entities);
        } else {
            let truncated = truncate_by_paragraph(text, limit.saturating_sub(3));
            let truncated_len = utf16_len(&truncated) as i32;
            let filtered_entities: Vec<TextEntity> = entities
                .iter()
                .filter(|ent| ent.offset + ent.length <= truncated_len)
                .cloned()
                .collect();
            return (truncated, filtered_entities);
        }
    }

    let mut ranges: Vec<(i32, i32)> = target_entities
        .iter()
        .filter_map(|entity| {
            if entity.length <= 0 {
                return None;
            }
            let start = entity.offset.max(0);
            let end = entity.offset.saturating_add(entity.length);
            if end <= start {
                None
            } else {
                Some((start, end))
            }
        })
        .collect();

    ranges.sort_by_key(|(start, _)| *start);
    let mut merged_ranges: Vec<(i32, i32)> = Vec::new();
    for (start, end) in ranges {
        if let Some((_, last_end)) = merged_ranges.last_mut() {
            if start <= *last_end {
                if end > *last_end {
                    *last_end = end;
                }
                continue;
            }
        }
        merged_ranges.push((start, end));
    }

    let mut result = text.to_string();
    for (start, end) in merged_ranges.iter().rev() {
        let utf16_start = (*start).max(0) as usize;
        let utf16_end = (*end).max(0) as usize;

        let byte_start = match char_from_utf16_offset(&result, utf16_start) {
            Ok(pos) => pos,
            Err(_) => continue,
        };
        let byte_end = match char_from_utf16_offset(&result, utf16_end) {
            Ok(pos) => pos,
            Err(_) => continue,
        };

        if byte_start < result.len() && byte_end <= result.len() && byte_start < byte_end {
            result.replace_range(byte_start..byte_end, "");
        }
    }

    let mut remaining_entities: Vec<TextEntity> = entities
        .iter()
        .filter(|e| {
            !matches!(
                e.entity_type,
                EntityType::Code | EntityType::Pre | EntityType::Blockquote
            )
        })
        .cloned()
        .collect();

    if !merged_ranges.is_empty() {
        remaining_entities.retain(|entity| {
            let ent_start = entity.offset;
            let ent_end = entity.offset + entity.length;
            !merged_ranges
                .iter()
                .any(|(start, end)| ent_start < *end && ent_end > *start)
        });

        for entity in remaining_entities.iter_mut() {
            let mut shift = 0i32;
            for (start, end) in &merged_ranges {
                if *end <= entity.offset {
                    shift = shift.saturating_add(end - start);
                } else {
                    break;
                }
            }
            if shift > 0 {
                entity.offset = entity.offset.saturating_sub(shift);
            }
        }
    }

    let available = limit
        .saturating_sub(header_utf16_len)
        .saturating_sub(footer_utf16_len)
        .saturating_sub(3);

    if utf16_len(&result) > limit {
        if available > 0 {
            let truncated = truncate_by_paragraph(&result, available);
            let truncated_len = utf16_len(&truncated) as i32;
            remaining_entities.retain(|ent| ent.offset + ent.length <= truncated_len);
            (truncated, remaining_entities)
        } else {
            let truncated = truncate_by_paragraph(&result, limit.saturating_sub(3));
            let truncated_len = utf16_len(&truncated) as i32;
            remaining_entities.retain(|ent| ent.offset + ent.length <= truncated_len);
            (truncated, remaining_entities)
        }
    } else {
        (result, remaining_entities)
    }
}

pub fn should_trim_long_entities(text: &str) -> bool {
    utf16_len(text) > 1024
}
