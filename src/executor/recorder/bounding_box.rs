//! Bounding box for tracking affected region in recordings

/// Bounding box for tracking affected region
#[derive(Debug, Clone)]
pub struct BoundingBox {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl BoundingBox {
    pub fn new() -> Self {
        Self {
            min: [i32::MAX, i32::MAX, i32::MAX],
            max: [i32::MIN, i32::MIN, i32::MIN],
        }
    }

    /// Expand the bounding box to include a position
    pub fn expand(&mut self, pos: [i32; 3]) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(pos[i]);
            self.max[i] = self.max[i].max(pos[i]);
        }
    }

    /// Check if the bounding box has any valid points
    pub fn is_valid(&self) -> bool {
        self.min[0] <= self.max[0] && self.min[1] <= self.max[1] && self.min[2] <= self.max[2]
    }

    /// Get cleanup region with padding
    pub fn to_cleanup_region(&self, padding: i32) -> [[i32; 3]; 2] {
        [
            [self.min[0] - padding, self.min[1] - padding, self.min[2] - padding],
            [self.max[0] + padding, self.max[1] + padding, self.max[2] + padding],
        ]
    }
}

impl Default for BoundingBox {
    fn default() -> Self {
        Self::new()
    }
}
