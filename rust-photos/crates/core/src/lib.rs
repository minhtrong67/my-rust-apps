//! Rust Photos core: everything that is not drawing widgets.
//! The old Tauri app split this between Rust commands and TypeScript; here it is one typed Rust library.
pub mod decode;
pub mod edit;
pub mod exif_util;
pub mod files;
pub mod i18n;
pub mod library;
pub mod store;
pub mod theme;
pub mod thumbs;

pub use library::Entry;
