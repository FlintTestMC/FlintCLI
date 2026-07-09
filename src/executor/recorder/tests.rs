//! Tests for the recorder module

use super::bounding_box::BoundingBox;
use super::state::RecorderState;
use flint_core::test_spec::ActionType;

#[test]
fn test_bounding_box() {
    let mut bb = BoundingBox::new();
    assert!(!bb.is_valid());

    bb.expand([0, 0, 0]);
    assert!(bb.is_valid());
    assert_eq!(bb.min, [0, 0, 0]);
    assert_eq!(bb.max, [0, 0, 0]);

    bb.expand([5, 10, -3]);
    assert_eq!(bb.min, [0, 0, -3]);
    assert_eq!(bb.max, [5, 10, 0]);
}

#[test]
fn test_local_position() {
    let mut recorder = RecorderState::new("test", std::path::Path::new("/tmp"));
    recorder.set_origin([100, 64, 200]);

    assert_eq!(recorder.to_local([100, 64, 200]), [0, 0, 0]);
    assert_eq!(recorder.to_local([105, 65, 198]), [5, 1, -2]);
}

#[test]
fn test_record_use_emits_tp_then_interact() {
    let mut recorder = RecorderState::new("test", std::path::Path::new("/tmp"));
    recorder.set_origin([100, 64, 200]);
    recorder.record_use(
        [101.5, 64.0, 198.25],
        Some([90.0, 15.0]),
        Some("minecraft:bone_meal".to_string()),
    );

    let spec = recorder.generate_test_spec();
    assert_eq!(spec.timeline.len(), 2);

    match &spec.timeline[0].action_type {
        ActionType::Tp {
            entity_alias,
            pos,
            rot,
        } => {
            assert_eq!(entity_alias, "player");
            assert_eq!(*pos, [1.5, 0.0, -1.75]);
            assert_eq!(*rot, Some([90.0, 15.0]));
        }
        other => panic!("expected tp, got {other:?}"),
    }

    match &spec.timeline[1].action_type {
        ActionType::Interact { item } => {
            assert_eq!(item.as_deref(), Some("minecraft:bone_meal"));
        }
        other => panic!("expected interact, got {other:?}"),
    }
}
