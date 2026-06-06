//! Strongly-typed hook input / output structures. Plugin authors write against
//! these; the host serializes them as JSON-RPC params.

pub mod agent;
pub mod auth;
pub mod chat;
pub mod command;
pub mod config;
pub mod event;
pub mod notification;
pub mod permission;
pub mod prompt;
pub mod provider;
pub mod session;
pub mod shell;
pub mod tool;

pub use agent::*;
pub use auth::*;
pub use chat::*;
pub use command::*;
pub use config::*;
pub use event::*;
pub use notification::*;
pub use permission::*;
pub use prompt::*;
pub use provider::*;
pub use session::*;
pub use shell::*;
pub use tool::*;

pub trait IntoHookOutput<T> {
    fn into_hook_output(self) -> crate::Result<T>;
}

impl<T> IntoHookOutput<Option<T>> for Option<T> {
    fn into_hook_output(self) -> crate::Result<Option<T>> {
        Ok(self)
    }
}

impl<T> IntoHookOutput<Option<T>> for T {
    fn into_hook_output(self) -> crate::Result<Option<T>> {
        Ok(Some(self))
    }
}

impl<T, E> IntoHookOutput<Option<T>> for std::result::Result<Option<T>, E>
where
    E: Into<crate::PluginError>,
{
    fn into_hook_output(self) -> crate::Result<Option<T>> {
        self.map_err(Into::into)
    }
}

impl<T, E> IntoHookOutput<Option<T>> for std::result::Result<T, E>
where
    E: Into<crate::PluginError>,
{
    fn into_hook_output(self) -> crate::Result<Option<T>> {
        self.map(Some).map_err(Into::into)
    }
}

impl IntoHookOutput<()> for () {
    fn into_hook_output(self) -> crate::Result<()> {
        Ok(())
    }
}

impl<E> IntoHookOutput<()> for std::result::Result<(), E>
where
    E: Into<crate::PluginError>,
{
    fn into_hook_output(self) -> crate::Result<()> {
        self.map_err(Into::into)
    }
}
