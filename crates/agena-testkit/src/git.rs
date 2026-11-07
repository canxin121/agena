//! Drive real Git and filesystem workbench HTTP APIs inside a case-owned root.
//! This is a protocol convenience layer, not a replacement for production handlers.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use reqwest::{Method, RequestBuilder};
use serde::Serialize;

use crate::{TestCase, TestHttpResponse, config::owned_path};

/// Workbench HTTP operations scoped to a directory owned by this test case.
#[derive(Clone)]
pub struct WorkbenchDriver {
    case: TestCase,
    directory: PathBuf,
    family: &'static str,
}

pub type GitDriver = WorkbenchDriver;
pub type FsDriver = WorkbenchDriver;

impl TestCase {
    /// Start with the real Server's default workspace. For additional repos,
    /// use `.git().in_directory("workspace/my-repo")`.
    pub fn git(&self) -> GitDriver {
        WorkbenchDriver {
            case: self.clone(),
            directory: self.workspace().to_path_buf(),
            family: "git",
        }
    }

    /// Test the actual `/workbench/fs` endpoints with the same scoped API.
    /// Other filesystem endpoints without a directory context remain accessible
    /// through the raw `case.http()` client.
    pub fn fs_workbench(&self) -> FsDriver {
        WorkbenchDriver {
            case: self.clone(),
            directory: self.workspace().to_path_buf(),
            family: "fs",
        }
    }
}

impl WorkbenchDriver {
    /// Select a directory relative to the case root, rejecting traversal,
    /// absolute paths and existing symlink ancestors. The Server still owns
    /// repo discovery, path validation and the actual operations.
    pub fn in_directory(mut self, relative: impl AsRef<Path>) -> Result<Self> {
        self.directory = owned_path(&self.case.paths().root, relative.as_ref())?;
        Ok(self)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Construct the exact authenticated HTTP request, including its directory.
    /// Unwrapped query parameters and malformed bodies can be supplied to
    /// exercise negative paths without adding a new testkit method.
    pub async fn request(&self, method: Method, action: &str) -> Result<RequestBuilder> {
        ensure!(
            !action.is_empty()
                && action.split('/').all(|segment| {
                    !segment.is_empty()
                        && segment
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
                }),
            "workbench action must be a relative route path without traversal"
        );
        let directory = self
            .directory
            .to_str()
            .context("workbench fixture directory is not UTF-8")?;
        Ok(self
            .case
            .http()
            .await
            .request(
                method,
                &format!("/api/v1/workbench/{}/{action}", self.family),
            )?
            .query(&[("directory", directory)]))
    }

    pub async fn get(&self, action: &str) -> Result<TestHttpResponse> {
        self.get_with_query(action, &[]).await
    }

    pub async fn get_with_query(
        &self,
        action: &str,
        query: &[(&str, &str)],
    ) -> Result<TestHttpResponse> {
        let http = self.case.http().await;
        http.execute(self.request(Method::GET, action).await?.query(query))
            .await
    }

    pub async fn post(&self, action: &str, value: &impl Serialize) -> Result<TestHttpResponse> {
        let http = self.case.http().await;
        http.execute(self.request(Method::POST, action).await?.json(value))
            .await
    }
}
