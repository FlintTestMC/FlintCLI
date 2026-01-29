//! Block-related utilities for parsing, normalization, and matching

use flint_core::test_spec::Block;
use std::collections::HashMap;

/// Extract block ID and properties from Azalea debug string
/// Input: "BlockState(id: 6795, OakFence { east: false, ... })"
/// Output: "minecraft:oak_fence[east=false,west=false]"
pub fn extract_block_id(debug_str: &str) -> String {
    let s = debug_str.trim();

    // 1. Extract Name and Properties part
    let (name_part, props_part) = if s.starts_with("BlockState(id:") {
        if let Some(comma_pos) = s.find(',') {
            let after_id = s[comma_pos + 1..].trim(); // "OakFence { ... })"
            // Check if it has properties
            if let Some(brace_start) = after_id.find('{') {
                let name = after_id[..brace_start].trim();
                let props_end = after_id.rfind('}').unwrap_or(after_id.len());
                let props = &after_id[brace_start + 1..props_end];
                (name, Some(props))
            } else {
                // No properties: "Stone)"
                let end = after_id.find(')').unwrap_or(after_id.len());
                (after_id[..end].trim(), None)
            }
        } else {
            ("air", None)
        }
    } else if s.starts_with("BlockState") {
        // Fallback for "BlockState { stone, properties: {...} }"
        if let Some(inner_start) = s.find('{') {
            let inner = &s[inner_start + 1..];
            let end = inner.find(|c| c == ',' || c == '}').unwrap_or(inner.len());
            (inner[..end].trim(), None)
        } else {
            ("air", None)
        }
    } else {
        // Raw string?
        (
            s.split(|c| c == ',' || c == '{' || c == ' ' || c == '}')
                .next()
                .unwrap_or(s),
            None,
        )
    };

    // 2. Normalize Name (PascalCase -> snake_case)
    let mut snake = String::new();
    for (i, c) in name_part.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                snake.push('_');
            }
            snake.push(c.to_ascii_lowercase());
        } else {
            snake.push(c);
        }
    }
    let block_id = if snake.contains(':') {
        snake
    } else {
        format!("minecraft:{}", snake)
    };

    // 3. Format Properties
    if let Some(props_str) = props_part {
        // "east: false, north: false"
        let mut pairs = Vec::new();
        for part in props_str.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some((k, v)) = part.split_once(':') {
                pairs.push(format!(
                    "{}={}",
                    k.trim().to_lowercase(),
                    v.trim().to_lowercase()
                ));
            }
        }
        if !pairs.is_empty() {
            // Sort for deterministic output
            pairs.sort();
            return format!("{}[{}]", block_id, pairs.join(","));
        }
    }

    block_id
}

/// Create a Block from a block ID string (potentially with properties)
/// Input: "minecraft:oak_fence[east=true,west=false]"
pub fn make_block(block_str: &str) -> Block {
    // Check for properties: "minecraft:oak_fence[east=true,west=false]"
    if let Some(open_bracket) = block_str.find('[')
        && let Some(close_bracket) = block_str.find(']')
    {
        let id = block_str[..open_bracket].to_string();
        let props_str = &block_str[open_bracket + 1..close_bracket];

        let mut properties = HashMap::new();
        for pair in props_str.split(',') {
            if let Some((k, v)) = pair.split_once('=') {
                properties.insert(
                    k.trim().to_string(),
                    serde_json::Value::String(v.trim().to_string()),
                );
            }
        }

        return Block { id, properties };
    }

    Block {
        id: block_str.to_string(),
        properties: HashMap::new(),
    }
}

/// Normalize block name for comparison (remove minecraft: prefix and underscores)
pub fn normalize_block_name(name: &str) -> String {
    name.trim_start_matches("minecraft:")
        .to_lowercase()
        .replace('_', "")
}

/// Check if actual block matches expected block name
pub fn block_matches(actual: &str, expected: &str) -> bool {
    let actual_lower = actual.to_lowercase();
    let expected_normalized = normalize_block_name(expected);
    actual_lower.contains(&expected_normalized)
        || actual_lower.replace('_', "").contains(&expected_normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_block_id_simple() {
        let input = "BlockState(id: 1, Stone)";
        assert_eq!(extract_block_id(input), "minecraft:stone");
    }

    #[test]
    fn test_extract_block_id_with_properties() {
        let input = "BlockState(id: 6795, OakFence { east: false, north: true })";
        let result = extract_block_id(input);
        assert!(result.starts_with("minecraft:oak_fence["));
        assert!(result.contains("east=false"));
        assert!(result.contains("north=true"));
    }

    #[test]
    fn test_make_block_simple() {
        let block = make_block("minecraft:stone");
        assert_eq!(block.id, "minecraft:stone");
        assert!(block.properties.is_empty());
    }

    #[test]
    fn test_make_block_with_properties() {
        let block = make_block("minecraft:oak_fence[east=true,west=false]");
        assert_eq!(block.id, "minecraft:oak_fence");
        assert_eq!(
            block.properties.get("east"),
            Some(&serde_json::Value::String("true".to_string()))
        );
    }

    #[test]
    fn test_block_matches() {
        assert!(block_matches("OakFence", "minecraft:oak_fence"));
        assert!(block_matches("minecraft:oak_fence", "oak_fence"));
        assert!(!block_matches("SpruceFence", "oak_fence"));
    }
}
