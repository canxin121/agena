//! System-test fixtures that drive the real Agena product from ordinary Rust tests.
//!
//! A case owns its server process, isolated files, HTTP clients and scripted
//! model. Model decisions are supplied by the test; provider parsing, tools,
//! permissions, plugins and persistence run through the production application.
//! All configured model responses are local and deterministic. This crate does
//! not contain a test runner or invoke Cargo to build a product executable.

pub mod case;
pub mod cli;
pub mod clock;
pub mod config;
pub mod fault;
pub mod fixtures;
pub mod git;
pub mod http;
pub mod model;
pub mod native;
pub mod process;
pub mod terminal;
pub mod tool;
pub mod user;

/// Result returned by a test case or fixture operation.
pub type TestResult<T = ()> = anyhow::Result<T>;

pub use agena_failure::{Failure, ModelFeedbackKind};
pub use case::{StartupFailure, TestAgena, TestAgenaBuilder, TestCase};
pub use cli::CliOutput;
pub use clock::{ClockDriver, SchedulerObservation, TestClock};
pub use config::{
    CasePaths, ConfigFile, ConfigSnapshot, EditOptions, LoadedConfigSnapshot, ReloadHandle,
    SettingsDriver, SettingsLayer,
};
pub use fault::{FaultDriver, FaultEffect, FaultState, FaultSyscall};
pub use fixtures::{FixtureWebSocketUpgrade, HttpFixture};
pub use git::{FsDriver, GitDriver, WorkbenchDriver};
pub use http::{TestHttpClient, TestHttpResponse};
pub use model::{
    FunctionCall, ModelDriver, ModelEvent, ModelHandle, ModelOptions, ModelProtocol, ModelRequest,
    ModelRoute, Reply, RequestMatch, ScriptedModel, StepHandle, Usage,
};
pub use native::NativeProtocol;
pub use process::{ProcessExit, ShutdownKind};
pub use terminal::{TerminalDriver, TerminalHandle};
pub use tool::{ToolResult, ToolRun};
pub use user::{PendingInteraction, RunHandle, RunResult, SessionOptions, UserDriver, UserSession};
