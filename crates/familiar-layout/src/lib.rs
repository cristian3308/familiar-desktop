//! Platform-independent geometry used by the shell surfaces.
//!
//! Everything here is pure math so it can be unit-tested on any OS and
//! called from the UI thread on every pointer move without allocating more
//! than the output vector.

pub mod dock;
pub mod easing;
pub mod grid;
pub mod shake;

pub use dock::{DockMetrics, DockSlot, SlotKind, layout_dock};
pub use grid::WidgetGrid;
pub use shake::ShakeDetector;
