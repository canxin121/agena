use super::*;

pub(super) fn resolve(p: &str, family: F, input: &ResourceInput) -> Result<Option<Route>> {
    let a = input.action;
    let resource = input.resource.as_deref().unwrap_or("");
    let id = || required(input.id.as_deref(), "id");
    let item = || required(input.item_id.as_deref(), "item_id");
    let sub = || required(input.sub_item_id.as_deref(), "sub_item_id");
    let mut route = Route {
        method: Method::GET,
        path: String::new(),
        model: false,
        multipart: false,
        binary: false,
    };
    let (method, path) = match (p, family) {
        ("chatgpt", F::Tasks) if a == A::Compact => {
            route.model = true;
            (Method::POST, "responses/compact".into())
        }
        ("gemini", F::Knowledge) if a == A::Upload => (
            Method::POST,
            format!(
                "fileSearchStores/{}:uploadToFileSearchStore",
                segment(
                    input
                        .id
                        .as_deref()
                        .ok_or_else(|| invalid("id is required"))?,
                    "fileSearchStores/"
                )?
            ),
        ),
        ("gemini", F::Embeddings) if a == A::CreateBatch => {
            route.model = true;
            (Method::POST, "models/{model}:batchEmbedContents".into())
        }
        ("gemini", F::Batches)
            if (resource == "embeddings" && a == A::Create) || a == A::Update =>
        {
            if !matches!(resource, "" | "generate_content" | "embeddings") {
                return Err(invalid(
                    "batch resource must be generate_content or embeddings",
                ));
            }
            route.model = true;
            if a == A::Create {
                (Method::POST, "models/{model}:asyncBatchEmbedContent".into())
            } else {
                (
                    Method::PATCH,
                    format!(
                        "batches/{}:{}",
                        segment(
                            input
                                .id
                                .as_deref()
                                .ok_or_else(|| invalid("id is required"))?,
                            "batches/"
                        )?,
                        if resource == "embeddings" {
                            "updateEmbedContentBatch"
                        } else {
                            "updateGenerateContentBatch"
                        }
                    ),
                )
            }
        }
        ("chatgpt", F::Platform)
            if resource == "audio/voices"
                && a == A::Create
                && input
                    .parameters
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    == Some("prompt") =>
        {
            (Method::POST, "audio/voices".into())
        }
        ("chatgpt", F::Platform) if resource == "audio/voice_consents" => match a {
            A::Create => {
                route.multipart = true;
                (Method::POST, resource.into())
            }
            A::List => (Method::GET, resource.into()),
            A::Get => (Method::GET, format!("{resource}/{}", id()?)),
            A::Update => (Method::POST, format!("{resource}/{}", id()?)),
            A::Delete => (Method::DELETE, format!("{resource}/{}", id()?)),
            _ => return Err(unsupported(family, a)),
        },
        ("chatgpt", F::Platform) if resource == "fine_tuning/jobs/checkpoints" => {
            if a != A::List {
                return Err(unsupported(family, a));
            }
            (
                Method::GET,
                format!("fine_tuning/jobs/{}/checkpoints", id()?),
            )
        }
        ("chatgpt", F::Platform) if resource == "fine_tuning/checkpoints/permissions" => match a {
            A::Create => (
                Method::POST,
                format!(
                    "fine_tuning/checkpoints/{}/permissions",
                    checkpoint(input.id.as_deref())?
                ),
            ),
            A::List => (
                Method::GET,
                format!(
                    "fine_tuning/checkpoints/{}/permissions",
                    checkpoint(input.id.as_deref())?
                ),
            ),
            A::Delete => (
                Method::DELETE,
                format!(
                    "fine_tuning/checkpoints/{}/permissions/{}",
                    checkpoint(input.id.as_deref())?,
                    item()?
                ),
            ),
            _ => return Err(unsupported(family, a)),
        },
        ("chatgpt", F::Platform) if resource == "fine_tuning/alpha/graders" => {
            let method = match a {
                A::Run => "run",
                A::Validate => "validate",
                _ => return Err(unsupported(family, a)),
            };
            (Method::POST, format!("{resource}/{method}"))
        }
        ("chatgpt", F::Platform) if resource == "evals/runs/output_items" => {
            let base = format!("evals/{}/runs/{}/output_items", id()?, item()?);
            match a {
                A::List => (Method::GET, base),
                A::Get => (Method::GET, format!("{base}/{}", sub()?)),
                _ => return Err(unsupported(family, a)),
            }
        }
        ("chatgpt", F::Platform) if resource == "evals" && a == A::DeleteRun => {
            (Method::DELETE, format!("evals/{}/runs/{}", id()?, item()?))
        }
        ("chatgpt", F::Agents)
            if matches!(
                resource,
                "sessions/artifacts"
                    | "sessions/turns"
                    | "sessions/subagents"
                    | "sessions/traces"
                    | "sessions/subagents/turns"
            ) =>
        {
            let base = if resource == "sessions/subagents/turns" {
                format!("agents/sessions/{}/subagents/{}/turns", id()?, item()?)
            } else {
                format!(
                    "agents/sessions/{}/{}",
                    id()?,
                    resource.strip_prefix("sessions/").unwrap()
                )
            };
            let child = || {
                if resource == "sessions/subagents/turns" {
                    sub()
                } else {
                    item()
                }
            };
            match a {
                A::List => (Method::GET, base),
                A::Get if resource != "sessions/traces" => {
                    (Method::GET, format!("{base}/{}", child()?))
                }
                A::Download if resource == "sessions/artifacts" => {
                    route.binary = true;
                    (Method::GET, format!("{base}/{}/content", child()?))
                }
                A::Delete if resource == "sessions/artifacts" => {
                    (Method::DELETE, format!("{base}/{}", child()?))
                }
                A::Items
                    if matches!(
                        resource,
                        "sessions/turns" | "sessions/subagents" | "sessions/subagents/turns"
                    ) =>
                {
                    (Method::GET, format!("{base}/{}/items", child()?))
                }
                _ => return Err(unsupported(family, a)),
            }
        }
        ("claude", F::Agents) if resource == "sessions/resources" => {
            let base = format!("sessions/{}/resources", id()?);
            match a {
                A::Create => (Method::POST, base),
                A::List => (Method::GET, base),
                A::Get => (Method::GET, format!("{base}/{}", item()?)),
                A::Update => (Method::POST, format!("{base}/{}", item()?)),
                A::Delete => (Method::DELETE, format!("{base}/{}", item()?)),
                _ => return Err(unsupported(family, a)),
            }
        }
        ("claude", F::Agents) if resource == "sessions/threads" => {
            let base = format!("sessions/{}/threads", id()?);
            match a {
                A::List => (Method::GET, base),
                A::Get => (Method::GET, format!("{base}/{}", item()?)),
                A::Archive => (Method::POST, format!("{base}/{}/archive", item()?)),
                A::Events => (Method::GET, format!("{base}/{}/events", item()?)),
                A::StreamEvents => (Method::GET, format!("{base}/{}/stream", item()?)),
                _ => return Err(unsupported(family, a)),
            }
        }
        ("claude", F::Agents) if resource == "deployment_runs" => match a {
            A::List => (Method::GET, resource.into()),
            A::Get => (Method::GET, format!("{resource}/{}", id()?)),
            _ => return Err(unsupported(family, a)),
        },
        ("claude", F::Agents) if resource == "sessions" && a == A::StreamEvents => {
            (Method::GET, format!("sessions/{}/events/stream", id()?))
        }
        _ => return Ok(None),
    };
    route.method = method;
    route.path = path;
    Ok(Some(route))
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    segment(
        value.ok_or_else(|| invalid(format!("{name} is required for this resource")))?,
        "",
    )
}

fn checkpoint(value: Option<&str>) -> Result<String> {
    let value = value.ok_or_else(|| invalid("id is required for the checkpoint"))?;
    if value.is_empty()
        || value.len() > 512
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b':' | b'-' | b'_' | b'.'))
    {
        return Err(invalid("invalid checkpoint model ID"));
    }
    Ok(url::form_urlencoded::byte_serialize(value.as_bytes()).collect())
}
