//! Block-related utilities for parsing, normalization, and matching

use flint_core::test_spec::Block;
use rustc_hash::FxHashMap;

/// Check that all expected block properties match an actual Azalea block state.
pub fn properties_match(actual: &Block, expected: &Block) -> bool {
    expected.properties.iter().all(|(name, expected_value)| {
        actual
            .properties
            .get(name)
            .is_some_and(|actual_value| actual_value.eq_ignore_ascii_case(expected_value))
    })
}

/// Extract block ID and properties from Azalea debug string
/// Input: "BlockState(id: 6795, OakFence { east: false, ... })"
/// Output: "minecraft:oak_fence[east=false,west=false]"
pub fn extract_block_id(debug_str: &str) -> String {
    let s = debug_str.trim();
    let (name, properties) = block_state_parts(s);
    let block_id = normalize_block_id(name);
    format_properties(block_id, properties)
}

fn block_state_parts(value: &str) -> (&str, Option<&str>) {
    if let Some(after_id) = value
        .strip_prefix("BlockState(id:")
        .and_then(|rest| rest.split_once(',').map(|(_, value)| value.trim()))
    {
        return split_name_and_properties(after_id);
    }
    if let Some(inner) = value
        .strip_prefix("BlockState")
        .and_then(|rest| rest.split_once('{').map(|(_, value)| value))
    {
        let end = inner.find([',', '}']).unwrap_or(inner.len());
        return (inner[..end].trim(), None);
    }
    let name = value.split([',', '{', ' ', '}']).next().unwrap_or(value);
    (name, None)
}

fn split_name_and_properties(value: &str) -> (&str, Option<&str>) {
    let Some(brace_start) = value.find('{') else {
        let end = value.find(')').unwrap_or(value.len());
        return (value[..end].trim(), None);
    };
    let end = value.rfind('}').unwrap_or(value.len());
    (
        value[..brace_start].trim(),
        Some(&value[brace_start + 1..end]),
    )
}

fn normalize_block_id(name: &str) -> String {
    let mut normalized = String::new();
    for (index, character) in name.chars().enumerate() {
        if character.is_uppercase() && index > 0 {
            normalized.push('_');
        }
        normalized.push(character.to_ascii_lowercase());
    }
    if normalized.contains(':') {
        normalized
    } else {
        format!("minecraft:{normalized}")
    }
}

fn format_properties(block_id: String, properties: Option<&str>) -> String {
    let mut pairs: Vec<_> = properties
        .into_iter()
        .flat_map(|properties| properties.split(','))
        .filter_map(|property| property.trim().split_once(':'))
        .map(|(key, value)| {
            let key = match key.trim() {
                "kind" => "type",
                key => key,
            };
            format!("{}={}", key.to_lowercase(), value.trim().to_lowercase())
        })
        .collect();
    if pairs.is_empty() {
        return block_id;
    }
    pairs.sort();
    format!("{block_id}[{}]", pairs.join(","))
}

/// Create a Block from a block ID string (potentially with properties)
/// Input: "minecraft:oak_fence[east=true,west=false]"
pub fn make_block(block_str: &str) -> Block {
    let base = Block {
        id: block_str.to_string(),
        properties: FxHashMap::default(),
        nbt: None,
    };
    // Check for properties: "minecraft:oak_fence[east=true,west=false]"
    if let Some(open_bracket) = block_str.find('[')
        && let Some(close_bracket) = block_str.find(']')
    {
        let id = block_str[..open_bracket].to_string();
        let props_str = &block_str[open_bracket + 1..close_bracket];

        let mut properties = FxHashMap::default();
        for pair in props_str.split(',') {
            if let Some((k, v)) = pair.split_once('=') {
                properties.insert(
                    k.trim().to_string(),
                    v.strip_prefix('_').unwrap_or(v).trim().to_string(),
                );
            }
        }

        return Block {
            id,
            properties,
            ..base
        };
    }

    base
}

/// Normalize block name for comparison (remove minecraft: prefix and underscores)
#[allow(dead_code)]
pub fn normalize_block_name(name: &str) -> String {
    name.trim_start_matches("minecraft:")
        .to_lowercase()
        .replace('_', "")
}

/// Check if actual block matches expected block name
#[allow(dead_code)]
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
    fn test_extract_block_id_normalizes_generated_kind_property() {
        let input = "BlockState(id: 1, OakSlab { kind: Double, waterlogged: false })";
        let result = extract_block_id(input);
        assert_eq!(result, "minecraft:oak_slab[type=double,waterlogged=false]");
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
        assert_eq!(block.properties.get("east"), Some(&"true".to_string()));
    }

    #[test]
    fn test_block_matches() {
        assert!(block_matches("OakFence", "minecraft:oak_fence"));
        assert!(block_matches("minecraft:oak_fence", "oak_fence"));
        assert!(!block_matches("SpruceFence", "oak_fence"));
    }

    #[test]
    fn properties_match_requires_exact_property_names() {
        let actual = make_block("minecraft:oak_slab[type=bottom,waterlogged=false]");
        let expected = make_block("minecraft:oak_slab[type=bottom,waterlogged=false]");
        assert!(properties_match(&actual, &expected));
    }
}
