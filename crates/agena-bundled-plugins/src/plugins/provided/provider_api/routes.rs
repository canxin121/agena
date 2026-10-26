use super::{Action as A, Family as F, ResourceInput, Result, Route, invalid, segment};
use reqwest::Method;
mod extensions;

pub(super) fn resolve(provider: &str, family: F, input: &ResourceInput) -> Result<Route> {
    if let Some(route) = extensions::resolve(provider, family, input)? {
        return Ok(route);
    }
    let id = |prefix: &str| {
        segment(
            input
                .id
                .as_deref()
                .ok_or_else(|| invalid("id is required for this action"))?,
            prefix,
        )
    };
    let item = |prefix: &str| {
        segment(
            input
                .item_id
                .as_deref()
                .ok_or_else(|| invalid("item_id is required for this action"))?,
            prefix,
        )
    };
    let model = || {
        // The model path is filled by the caller using the active connection.
        // A marker avoids accepting any caller-provided endpoint.
        Ok::<_, agena_plugin_sdk::PluginError>("{model}".to_owned())
    };
    let a = input.action;
    let p = provider;
    let mut route = Route {
        method: Method::GET,
        path: String::new(),
        model: false,
        multipart: false,
        binary: false,
    };
    let (method, path) = match family {
        F::Files => match a {
            A::List => (Method::GET, "files".into()),
            A::Get | A::Wait => (Method::GET, format!("files/{}", id("files/")?)),
            A::Delete => (Method::DELETE, format!("files/{}", id("files/")?)),
            A::Download => {
                route.binary = true;
                (
                    Method::GET,
                    format!(
                        "files/{}{}",
                        id("files/")?,
                        if p == "gemini" {
                            ":download"
                        } else {
                            "/content"
                        }
                    ),
                )
            }
            A::Upload if p != "gemini" => {
                route.multipart = true;
                (Method::POST, "files".into())
            }
            _ => return Err(unsupported(family, a)),
        },
        F::Knowledge if p == "chatgpt" => match a {
            A::Create => (Method::POST, "vector_stores".into()),
            A::List => (Method::GET, "vector_stores".into()),
            A::Get | A::Wait => (
                Method::GET,
                format!("vector_stores/{}", id("vector_stores/")?),
            ),
            A::Update => (
                Method::POST,
                format!("vector_stores/{}", id("vector_stores/")?),
            ),
            A::Delete => (
                Method::DELETE,
                format!("vector_stores/{}", id("vector_stores/")?),
            ),
            A::Search => (
                Method::POST,
                format!("vector_stores/{}/search", id("vector_stores/")?),
            ),
            A::AddFile => (
                Method::POST,
                format!("vector_stores/{}/files", id("vector_stores/")?),
            ),
            A::ListFiles => (
                Method::GET,
                format!("vector_stores/{}/files", id("vector_stores/")?),
            ),
            A::GetFile => (
                Method::GET,
                format!(
                    "vector_stores/{}/files/{}",
                    id("vector_stores/")?,
                    item("files/")?
                ),
            ),
            A::UpdateFile => (
                Method::POST,
                format!(
                    "vector_stores/{}/files/{}",
                    id("vector_stores/")?,
                    item("files/")?
                ),
            ),
            A::DeleteFile => (
                Method::DELETE,
                format!(
                    "vector_stores/{}/files/{}",
                    id("vector_stores/")?,
                    item("files/")?
                ),
            ),
            A::DownloadFile => (
                Method::GET,
                format!(
                    "vector_stores/{}/files/{}/content",
                    id("vector_stores/")?,
                    item("files/")?
                ),
            ),
            A::CreateBatch => (
                Method::POST,
                format!("vector_stores/{}/file_batches", id("vector_stores/")?),
            ),
            A::GetBatch => (
                Method::GET,
                format!(
                    "vector_stores/{}/file_batches/{}",
                    id("vector_stores/")?,
                    item("")?
                ),
            ),
            A::CancelBatch => (
                Method::POST,
                format!(
                    "vector_stores/{}/file_batches/{}/cancel",
                    id("vector_stores/")?,
                    item("")?
                ),
            ),
            A::ListBatchFiles => (
                Method::GET,
                format!(
                    "vector_stores/{}/file_batches/{}/files",
                    id("vector_stores/")?,
                    item("")?
                ),
            ),
            _ => return Err(unsupported(family, a)),
        },
        F::Knowledge if p == "gemini" => match a {
            A::Create => (Method::POST, "fileSearchStores".into()),
            A::List => (Method::GET, "fileSearchStores".into()),
            A::Get => (
                Method::GET,
                format!("fileSearchStores/{}", id("fileSearchStores/")?),
            ),
            A::Delete => (
                Method::DELETE,
                format!("fileSearchStores/{}", id("fileSearchStores/")?),
            ),
            A::ImportFile => (
                Method::POST,
                format!("fileSearchStores/{}:importFile", id("fileSearchStores/")?),
            ),
            A::ListFiles => (
                Method::GET,
                format!("fileSearchStores/{}/documents", id("fileSearchStores/")?),
            ),
            A::GetFile => (
                Method::GET,
                format!(
                    "fileSearchStores/{}/documents/{}",
                    id("fileSearchStores/")?,
                    item("documents/")?
                ),
            ),
            A::DeleteFile => (
                Method::DELETE,
                format!(
                    "fileSearchStores/{}/documents/{}",
                    id("fileSearchStores/")?,
                    item("documents/")?
                ),
            ),
            _ => return Err(unsupported(family, a)),
        },
        F::Containers if p == "chatgpt" => match a {
            A::Create => (Method::POST, "containers".into()),
            A::List => (Method::GET, "containers".into()),
            A::Get => (Method::GET, format!("containers/{}", id("")?)),
            A::Delete => (Method::DELETE, format!("containers/{}", id("")?)),
            A::AddFile | A::Upload => {
                route.multipart = input.path.is_some();
                (Method::POST, format!("containers/{}/files", id("")?))
            }
            A::ListFiles => (Method::GET, format!("containers/{}/files", id("")?)),
            A::GetFile => (
                Method::GET,
                format!("containers/{}/files/{}", id("")?, item("")?),
            ),
            A::DeleteFile => (
                Method::DELETE,
                format!("containers/{}/files/{}", id("")?, item("")?),
            ),
            A::DownloadFile => {
                route.binary = true;
                (
                    Method::GET,
                    format!("containers/{}/files/{}/content", id("")?, item("")?),
                )
            }
            _ => return Err(unsupported(family, a)),
        },
        F::Tasks => {
            let collection = if p == "chatgpt" {
                "responses"
            } else if p == "gemini" {
                "interactions"
            } else {
                return Err(unsupported(family, a));
            };
            match a {
                A::Create => {
                    route.model = true;
                    (Method::POST, collection.into())
                }
                A::Get | A::Wait => (Method::GET, format!("{collection}/{}", id("")?)),
                A::Delete => (Method::DELETE, format!("{collection}/{}", id("")?)),
                A::Cancel => (Method::POST, format!("{collection}/{}/cancel", id("")?)),
                A::Items if p == "chatgpt" => {
                    (Method::GET, format!("responses/{}/input_items", id("")?))
                }
                _ => return Err(unsupported(family, a)),
            }
        }
        F::Batches => {
            let collection = if p == "claude" {
                "messages/batches"
            } else {
                "batches"
            };
            match a {
                A::Create if p == "gemini" => {
                    route.model = true;
                    (
                        Method::POST,
                        format!("models/{}:batchGenerateContent", model()?),
                    )
                }
                A::Create => (Method::POST, collection.into()),
                A::List => (Method::GET, collection.into()),
                A::Get | A::Wait => (Method::GET, format!("{collection}/{}", id("batches/")?)),
                A::Cancel => (
                    Method::POST,
                    format!(
                        "{collection}/{}{}",
                        id("batches/")?,
                        if p == "gemini" { ":cancel" } else { "/cancel" }
                    ),
                ),
                A::Delete if p != "chatgpt" => {
                    (Method::DELETE, format!("{collection}/{}", id("batches/")?))
                }
                A::Results if p == "claude" => {
                    (Method::GET, format!("{collection}/{}/results", id("")?))
                }
                _ => return Err(unsupported(family, a)),
            }
        }
        F::Skills if p == "claude" => match a {
            A::Create => {
                route.multipart = true;
                (Method::POST, "skills".into())
            }
            A::List => (Method::GET, "skills".into()),
            A::Get => (Method::GET, format!("skills/{}", id("")?)),
            A::Delete => (Method::DELETE, format!("skills/{}", id("")?)),
            A::CreateVersion => {
                route.multipart = true;
                (Method::POST, format!("skills/{}/versions", id("")?))
            }
            A::ListVersions => (Method::GET, format!("skills/{}/versions", id("")?)),
            A::GetVersion => (
                Method::GET,
                format!("skills/{}/versions/{}", id("")?, item("")?),
            ),
            A::DeleteVersion => (
                Method::DELETE,
                format!("skills/{}/versions/{}", id("")?, item("")?),
            ),
            _ => return Err(unsupported(family, a)),
        },
        F::Caches if p == "gemini" => match a {
            A::Create => {
                route.model = true;
                (Method::POST, "cachedContents".into())
            }
            A::List => (Method::GET, "cachedContents".into()),
            A::Get => (
                Method::GET,
                format!("cachedContents/{}", id("cachedContents/")?),
            ),
            A::Update => (
                Method::PATCH,
                format!("cachedContents/{}", id("cachedContents/")?),
            ),
            A::Delete => (
                Method::DELETE,
                format!("cachedContents/{}", id("cachedContents/")?),
            ),
            _ => return Err(unsupported(family, a)),
        },
        F::Operations if p == "gemini" => match a {
            A::Get | A::Wait => (
                Method::GET,
                google_operation_name(
                    input
                        .id
                        .as_deref()
                        .ok_or_else(|| invalid("id is required"))?,
                )?,
            ),
            _ => return Err(unsupported(family, a)),
        },
        F::Inference => {
            require_create(a, family)?;
            route.model = true;
            (
                Method::POST,
                match p {
                    "chatgpt" => "responses",
                    "claude" => "messages",
                    _ => "interactions",
                }
                .into(),
            )
        }
        F::Images if p == "chatgpt" => {
            require_create(a, family)?;
            route.model = true;
            (Method::POST, "images/generations".into())
        }
        F::Transcription if p == "chatgpt" => {
            require_create(a, family)?;
            route.model = true;
            route.multipart = true;
            (
                Method::POST,
                if input.resource.as_deref() == Some("translations") {
                    "audio/translations"
                } else {
                    "audio/transcriptions"
                }
                .into(),
            )
        }
        F::Speech if p == "chatgpt" => {
            require_create(a, family)?;
            route.model = true;
            route.binary = true;
            (Method::POST, "audio/speech".into())
        }
        F::Speech | F::Video | F::Music if p == "gemini" => {
            require_create(a, family)?;
            route.model = true;
            (Method::POST, "interactions".into())
        }
        F::Embeddings if p == "chatgpt" => {
            require_create(a, family)?;
            route.model = true;
            (Method::POST, "embeddings".into())
        }
        F::Embeddings if p == "gemini" => {
            require_create(a, family)?;
            route.model = true;
            (Method::POST, format!("models/{}:embedContent", model()?))
        }
        F::Moderation if p == "chatgpt" => {
            require_create(a, family)?;
            route.model = true;
            (Method::POST, "moderations".into())
        }
        F::Tokens => {
            require_create(a, family)?;
            route.model = true;
            (
                Method::POST,
                match p {
                    "chatgpt" => "responses/input_tokens".into(),
                    "claude" => "messages/count_tokens".into(),
                    _ => format!("models/{}:countTokens", model()?),
                },
            )
        }
        F::Agents => {
            let collection = input.resource.as_deref().unwrap_or(if p == "chatgpt" {
                "sessions"
            } else {
                "agents"
            });
            let allowed = if p == "chatgpt" {
                &["agents", "sessions", "environments"][..]
            } else if p == "claude" {
                &["agents", "sessions", "environments", "deployments"][..]
            } else {
                &["agents"][..]
            };
            if !allowed.contains(&collection) {
                return Err(invalid("unsupported managed-agent collection"));
            }
            let base = if p == "chatgpt" {
                if collection == "agents" {
                    "agents".into()
                } else {
                    format!("agents/{collection}")
                }
            } else {
                collection.into()
            };
            route.model = matches!(p, "chatgpt" | "claude")
                && collection == "agents"
                && (a == A::Create || (a == A::Update && input.model.is_some()));
            managed(&base, p, collection, a, &id, &item)?
        }
        F::Platform if p == "chatgpt" => {
            let collection=input.resource.as_deref().ok_or_else(||invalid("resource is required: conversations, fine_tuning/jobs, evals, uploads or audio/voices"))?;
            if ![
                "conversations",
                "fine_tuning/jobs",
                "evals",
                "uploads",
                "audio/voices",
            ]
            .contains(&collection)
            {
                return Err(invalid("unsupported platform collection"));
            }
            route.model = collection == "fine_tuning/jobs" && a == A::Create;
            platform(collection, a, &id, &item, &mut route)?
        }
        _ => return Err(unsupported(family, a)),
    };
    route.method = method;
    route.path = path;
    Ok(route)
}

fn require_create(action: A, family: F) -> Result<()> {
    if action == A::Create {
        Ok(())
    } else {
        Err(unsupported(family, action))
    }
}
fn unsupported(family: F, action: A) -> agena_plugin_sdk::PluginError {
    invalid(format!(
        "unsupported {family:?} action {action:?}; consult tools_help for supported actions"
    ))
}

fn managed(
    base: &str,
    provider: &str,
    collection: &str,
    action: A,
    id: &impl Fn(&str) -> Result<String>,
    item: &impl Fn(&str) -> Result<String>,
) -> Result<(Method, String)> {
    let allowed: &[A] = match (provider, collection) {
        ("chatgpt", "agents") => &[A::Create, A::List, A::Get, A::Update, A::Delete],
        ("chatgpt", "sessions") => &[
            A::Create,
            A::List,
            A::Get,
            A::Wait,
            A::Update,
            A::Delete,
            A::Events,
            A::CreateEvent,
            A::Items,
        ],
        ("chatgpt", "environments") => &[A::Create, A::List, A::Get, A::AddFile, A::ListFiles],
        ("claude", "agents") => &[
            A::Create,
            A::List,
            A::Get,
            A::Update,
            A::Archive,
            A::ListVersions,
            A::GetVersion,
        ],
        ("claude", "sessions") => &[
            A::Create,
            A::List,
            A::Get,
            A::Wait,
            A::Update,
            A::Delete,
            A::Archive,
            A::Events,
            A::CreateEvent,
        ],
        ("claude", "environments") => {
            &[A::Create, A::List, A::Get, A::Update, A::Delete, A::Archive]
        }
        ("claude", "deployments") => &[
            A::Create,
            A::List,
            A::Get,
            A::Update,
            A::Archive,
            A::Run,
            A::Pause,
            A::Unpause,
        ],
        ("gemini", "agents") => &[A::Create, A::List, A::Get, A::Delete],
        _ => &[],
    };
    if !allowed.contains(&action) {
        return Err(invalid(
            "unsupported action for this managed-agent resource",
        ));
    }
    Ok(match action {
        A::List => (Method::GET, base.into()),
        A::Create => (Method::POST, base.into()),
        A::Get | A::Wait => (Method::GET, format!("{base}/{}", id("")?)),
        A::Update => (Method::POST, format!("{base}/{}", id("")?)),
        A::Delete => (Method::DELETE, format!("{base}/{}", id("")?)),
        A::Archive | A::Run | A::Pause | A::Unpause => {
            let suffix = match action {
                A::Archive => "archive",
                A::Run => "run",
                A::Pause => "pause",
                _ => "unpause",
            };
            (Method::POST, format!("{base}/{}/{suffix}", id("")?))
        }
        A::Events | A::CreateEvent => (
            if action == A::Events {
                Method::GET
            } else {
                Method::POST
            },
            format!("{base}/{}/events", id("")?),
        ),
        A::Items => (Method::GET, format!("{base}/{}/items", id("")?)),
        A::ListVersions => (Method::GET, format!("{base}/{}/versions", id("")?)),
        A::GetVersion => (
            Method::GET,
            format!("{base}/{}/versions/{}", id("")?, item("")?),
        ),
        A::ListFiles | A::AddFile => (
            if action == A::AddFile {
                Method::POST
            } else {
                Method::GET
            },
            format!("{base}/{}/files", id("")?),
        ),
        _ => return Err(invalid("unsupported managed-agent action")),
    })
}

fn platform(
    base: &str,
    action: A,
    id: &impl Fn(&str) -> Result<String>,
    item: &impl Fn(&str) -> Result<String>,
    route: &mut Route,
) -> Result<(Method, String)> {
    let allowed: &[A] = match base {
        "conversations" => &[
            A::Create,
            A::Get,
            A::Update,
            A::Delete,
            A::Items,
            A::AddItems,
            A::GetItem,
            A::DeleteItem,
        ],
        "fine_tuning/jobs" => &[
            A::Create,
            A::List,
            A::Get,
            A::Wait,
            A::Cancel,
            A::Events,
            A::Pause,
            A::Unpause,
        ],
        "evals" => &[
            A::Create,
            A::List,
            A::Get,
            A::Update,
            A::Delete,
            A::CreateRun,
            A::ListRuns,
            A::GetRun,
            A::CancelRun,
        ],
        "uploads" => &[A::Create, A::AddPart, A::Complete, A::Cancel],
        "audio/voices" => &[A::Create, A::Delete],
        _ => &[],
    };
    if !allowed.contains(&action) {
        return Err(invalid("unsupported action for this platform resource"));
    }
    Ok(match action {
        A::List => (Method::GET, base.into()),
        A::Create => {
            route.multipart = base == "audio/voices";
            (Method::POST, base.into())
        }
        A::Get | A::Wait => (Method::GET, format!("{base}/{}", id("")?)),
        A::Update => (Method::POST, format!("{base}/{}", id("")?)),
        A::Delete => (Method::DELETE, format!("{base}/{}", id("")?)),
        A::Cancel | A::Complete | A::Pause | A::Unpause => {
            let suffix = match action {
                A::Cancel => "cancel",
                A::Complete => "complete",
                A::Pause => "pause",
                _ => "resume",
            };
            (Method::POST, format!("{base}/{}/{suffix}", id("")?))
        }
        A::AddPart => {
            route.multipart = true;
            (Method::POST, format!("uploads/{}/parts", id("")?))
        }
        A::Events => (Method::GET, format!("{base}/{}/events", id("")?)),
        A::Items | A::AddItems => (
            if action == A::Items {
                Method::GET
            } else {
                Method::POST
            },
            format!("{base}/{}/items", id("")?),
        ),
        A::GetItem | A::DeleteItem => (
            if action == A::GetItem {
                Method::GET
            } else {
                Method::DELETE
            },
            format!("{base}/{}/items/{}", id("")?, item("")?),
        ),
        A::ListRuns | A::CreateRun => (
            if action == A::ListRuns {
                Method::GET
            } else {
                Method::POST
            },
            format!("evals/{}/runs", id("")?),
        ),
        A::GetRun => (Method::GET, format!("evals/{}/runs/{}", id("")?, item("")?)),
        A::CancelRun => (
            Method::POST,
            format!("evals/{}/runs/{}/cancel", id("")?, item("")?),
        ),
        _ => return Err(invalid("unsupported platform action")),
    })
}

fn google_operation_name(name: &str) -> Result<String> {
    let segments: Vec<_> = name.split('/').collect();
    match segments.as_slice() {
        ["operations", operation] => Ok(format!("operations/{}", segment(operation, "")?)),
        ["fileSearchStores", store, "operations", operation] => Ok(format!(
            "fileSearchStores/{}/operations/{}",
            segment(store, "")?,
            segment(operation, "")?
        )),
        ["fileSearchStores", store, "upload", "operations", operation] => Ok(format!(
            "fileSearchStores/{}/upload/operations/{}",
            segment(store, "")?,
            segment(operation, "")?
        )),
        ["files", file, "operations", operation] => Ok(format!(
            "files/{}/operations/{}",
            segment(file, "")?,
            segment(operation, "")?
        )),
        _ => Err(invalid(
            "expected operations/ID or fileSearchStores/ID/operations/ID",
        )),
    }
}
