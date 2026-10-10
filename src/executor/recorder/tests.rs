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
fn test_removal_can_establish_origin_and_clears_snapshot() {
    let mut recorder = RecorderState::new("test", std::path::Path::new("/tmp"));
    recorder
        .snapshot
        .insert([10, 64, 20], "minecraft:stone".to_string());

    recorder.record_remove([10, 64, 20]);

    assert_eq!(recorder.origin, Some([10, 64, 20]));
    assert!(!recorder.snapshot.contains_key(&[10, 64, 20]));
    assert_eq!(recorder.timeline[0].actions.len(), 1);
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

#[test]
fn test_record_use_without_item_saves_a_loadable_spec() {
    let temp_dir = std::env::temp_dir().join(format!(
        "flintmc-recorder-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&temp_dir).unwrap();
    let mut recorder = RecorderState::new("active_hand", &temp_dir);
    recorder.record_use([1.5, 64.0, 2.5], None, None);

    let path = recorder.save().unwrap();
    let json = std::fs::read_to_string(&path).unwrap();

    assert!(!json.contains("\"item\": null"));
    assert!(!json.contains("\"rot\": null"));
    flint_core::test_spec::TestSpec::from_file(&path, false).unwrap();
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(temp_dir).unwrap();
}
