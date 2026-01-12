use super::model::TextEntity;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplacementRule {
    #[serde(rename = "type")]
    rule_type: String,
    pattern: String,
    replacement: String,
}

#[derive(Debug, Clone)]
pub struct TextReplacer {
    rules: Vec<ReplacementRule>,
    compiled_regexes: Vec<Option<Regex>>,
}

impl TextReplacer {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            compiled_regexes: Vec::new(),
        }
    }

    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        let path = path.as_ref();
        if !path.exists() {
            tracing::warn!("替换规则文件不存在: {:?}", path);
            self.rules.clear();
            self.compiled_regexes.clear();
            return Ok(());
        }

        let content = std::fs::read_to_string(path)?;
        let rules: Vec<ReplacementRule> = serde_json::from_str(&content)?;

        let mut new_rules = Vec::new();
        let mut new_regexes = Vec::new();

        for rule in rules {
            if rule.pattern.is_empty() {
                continue;
            }

            let compiled = if rule.rule_type == "regex" {
                match Regex::new(&rule.pattern) {
                    Ok(re) => Some(re),
                    Err(err) => {
                        tracing::warn!(
                            "跳过无效正则替换规则 pattern=`{}` error={}",
                            rule.pattern,
                            err
                        );
                        continue;
                    }
                }
            } else {
                None
            };

            new_rules.push(rule);
            new_regexes.push(compiled);
        }

        self.rules = new_rules;
        self.compiled_regexes = new_regexes;

        tracing::info!("替换规则已加载: count={}", self.rules.len());

        Ok(())
    }

    pub fn process_with_entities(
        &self,
        text: &str,
        entities: Vec<TextEntity>,
    ) -> (String, Vec<TextEntity>) {
        if self.rules.is_empty() {
            return (text.to_string(), entities);
        }

        let mut current_text = text.to_string();
        let mut current_entities = entities;

        use common::utf16::*;

        for (i, rule) in self.rules.iter().enumerate() {
            let compiled = self.compiled_regexes.get(i).and_then(|c| c.as_ref());

            let max_iterations = if rule.rule_type == "regex" { 100 } else { 1000 };
            let mut iterations = 0;

            loop {
                if iterations >= max_iterations {
                    tracing::warn!(
                        "替换规则达到最大迭代次数: pattern=`{}` iterations={}",
                        rule.pattern,
                        iterations
                    );
                    break;
                }

                let (match_start, match_end) = if rule.rule_type == "regex" {
                    if let Some(regex) = compiled {
                        if let Some(m) = regex.find(&current_text) {
                            (m.start(), m.end())
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                } else if let Some(pos) = current_text.find(&rule.pattern) {
                    (pos, pos + rule.pattern.len())
                } else {
                    break;
                };

                let replacement = if rule.rule_type == "regex" {
                    if let Some(regex) = compiled {
                        if let Some(caps) = regex.captures(&current_text[match_start..match_end]) {
                            let mut result = rule.replacement.clone();
                            for (j, cap) in caps.iter().enumerate().skip(1) {
                                if let Some(m) = cap {
                                    result = result.replace(&format!("${}", j), m.as_str());
                                }
                            }
                            result
                        } else {
                            rule.replacement.clone()
                        }
                    } else {
                        rule.replacement.clone()
                    }
                } else {
                    rule.replacement.clone()
                };

                let old_utf16_len = utf16_len(&current_text[match_start..match_end]);
                let new_utf16_len = utf16_len(&replacement);
                let utf16_diff = new_utf16_len as i32 - old_utf16_len as i32;

                let prefix_utf16_len = utf16_offset(&current_text, match_start);
                let match_start_utf16 = prefix_utf16_len as i32;
                let match_end_utf16 = match_start_utf16 + old_utf16_len as i32;

                current_entities = current_entities
                    .into_iter()
                    .filter(|ent| {
                        let ent_start = ent.offset;
                        let ent_end = ent.offset + ent.length;

                        if ent_end <= match_start_utf16 {
                            return true;
                        }
                        if ent_start >= match_end_utf16 {
                            return true;
                        }
                        if ent_start <= match_start_utf16 && ent_end >= match_end_utf16 {
                            return true;
                        }
                        false
                    })
                    .map(|mut ent| {
                        let ent_start = ent.offset;
                        let ent_end = ent.offset + ent.length;

                        if ent_end <= match_start_utf16 {
                        } else if ent_start >= match_end_utf16 {
                            ent.offset += utf16_diff;
                        } else if ent_start <= match_start_utf16 && ent_end >= match_end_utf16 {
                            ent.length += utf16_diff;
                        }
                        ent
                    })
                    .collect();

                current_text.replace_range(match_start..match_end, &replacement);
                iterations += 1;

                if rule.rule_type != "regex" && rule.replacement.contains(&rule.pattern) {
                    break;
                }
            }
        }

        (current_text, current_entities)
    }
}

impl Default for TextReplacer {
    fn default() -> Self {
        Self::new()
    }
}
