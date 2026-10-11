//! Block-related utilities for parsing, normalization, and matching

use flint_core::test_spec::Block;
use rustc_hash::FxHashMap;

/// Compare a requested placement with the state observed by Azalea.
pub fn placement_matches(actual: &Block, expected: &Block) -> bool {
    let actual_id = actual.id.strip_prefix("minecraft:").unwrap_or(&actual.id);
    let expected_id = expected
        .id
        .strip_prefix("minecraft:")
        .unwrap_or(&expected.id);
    actual_id == expected_id && properties_match(actual, expected)
}

/// Check that all expected block properties match an actual Azalea block state.
pub fn properties_match(actual: &Block, expected: &Block) -> bool {
    expected.properties.iter().all(|(name, expected_value)| {
        actual
            .properties
            .get(name)
            .is_some_and(|actual_value| actual_value.eq_ignore_ascii_case(expected_value))
    })
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
            nbt: None,
        };
    }

    Block {
        id: block_str.to_string(),
        properties: FxHashMap::default(),
        nbt: None,
    }
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
    fn placement_accepts_default_minecraft_namespace() {
        let actual = make_block("minecraft:copper_chain[axis=y,waterlogged=false]");
        let expected = make_block("copper_chain[axis=y]");
        assert!(placement_matches(&actual, &expected));
        assert!(!placement_matches(
            &actual,
            &make_block("other:copper_chain")
        ));
        assert!(!placement_matches(
            &actual,
            &make_block("copper_chain[axis=x]")
        ));
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
