//! App window: a native egui window in its own process, so closing it frees all its memory.

mod host;
mod link;
mod pages;
mod process;
mod render;
mod schema;
mod tree;

pub use link::{Status, ToOverlay, ToWindow, WindowProcess};
pub use process::run;
#[cfg(test)]
pub use render::feed_sample;
