//! Config section and row construction for the plugin workbench.

mod rows;
mod sections;

pub(crate) use self::{rows::*, sections::*};

pub mod api {
    pub use super::rows::config_path;
}
