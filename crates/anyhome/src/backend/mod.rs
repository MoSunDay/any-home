//! Filesystem backends. `realfs` is the on-disk layer; the dispatcher in
//! [`crate::dispatcher`] adds the magic-path DB layer on top.

pub mod realfs;

pub use realfs::{RealFs, ROOT_ID};
