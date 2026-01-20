//! Tests for the recorder module

use super::bounding_box::BoundingBox;
use super::state::RecorderState;

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
