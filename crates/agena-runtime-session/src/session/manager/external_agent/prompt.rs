use std::collections::BTreeSet;

use agena_domain::{ContentId, ContentKind, ExternalAgentCommand, external_agent_command_input};
use agena_provider::CompletionInputAttachmentSource;
use agena_storage::kernel::ReadLease;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

use super::*;
use crate::session::kernel::{media, run_read_budget};

pub(super) struct TurnInput<'a> {
    entries: Vec<&'a PromptEntry>,
    structured: BTreeSet<ContentId>,
    resume_anchor: Option<ExternalAgentHistoryAnchor>,
    continuity: Option<agena_storage::kernel::ExternalAgentRoundContinuity>,
    message: EntryId,
    document: &'a ComposerDocument,
}

#[derive(Clone, Copy, Default)]
pub(super) struct PromptCapabilities {
    pub(super) image: bool,
    pub(super) audio: bool,
    pub(super) embedded_context: bool,
    pub(super) skill: bool,
    pub(super) skill_inline: bool,
}

impl<'a> TurnInput<'a> {
    pub(super) fn new(
        facts: &'a PromptView,
        execution: &ExecutionView,
        continuity: Option<agena_storage::kernel::ExternalAgentRoundContinuity>,
    ) -> Result<Self, AppError> {
        let message_index = facts
            .entries
            .iter()
            .position(|entry| entry.entry.id == execution.message_id);
        let Some(EntryPayload::User {
            document,
            author,
            origin,
        }) = message_index.map(|index| &facts.entries[index].entry.payload)
        else {
            return Err(AppError::Config(
                "Send a user message to start an external agent turn.".into(),
            ));
        };
        require_direct_user(author.as_ref(), origin.as_ref())?;
        validate_document(document)?;
        let resume_anchor = if let Some(continuity) = &continuity {
            continuity.resume_anchor.clone()
        } else {
            message_index.and_then(|index| {
                // Failure notices attached after this same input are outside the
                // native context. Preserve the original cut when retrying it, and
                // skip any earlier failed/cancelled user messages that never
                // committed a native round.
                facts.entries[index + 1..]
                    .iter()
                    .all(|entry| {
                        entry.entry.message_id == execution.message_id
                            && matches!(entry.entry.payload, EntryPayload::Notice { .. })
                    })
                    .then(|| {
                        let mut failed_users = BTreeSet::new();
                        let tail = facts.entries[..index].iter().rev().find_map(|entry| {
                            if matches!(entry.entry.payload, EntryPayload::Notice { .. })
                                && failed_users.contains(&entry.entry.message_id)
                            {
                                return None;
                            }
                            if matches!(entry.entry.payload, EntryPayload::User { .. })
                                && matches!(
                                    entry.entry.execution_phase,
                                    ExecutionPhase::Failed | ExecutionPhase::Cancelled
                                )
                            {
                                failed_users.insert(entry.entry.id);
                                return None;
                            }
                            Some(entry.entry.id)
                        });
                        ExternalAgentHistoryAnchor { tail }
                    })
            })
        };
        Ok(Self {
            entries: facts
                .entries
                .iter()
                .filter(|entry| !matches!(entry.entry.payload, EntryPayload::Notice { .. }))
                .collect(),
            structured: facts
                .sources
                .iter()
                .filter(|stream| stream.kind == ContentKind::Structured)
                .map(|stream| stream.id)
                .collect(),
            message: execution.message_id,
            document,
            resume_anchor,
            continuity,
        })
    }

    pub(super) fn can_resume_anchor(&self, anchor: &ExternalAgentHistoryAnchor) -> bool {
        self.resume_anchor.as_ref() == Some(anchor)
    }

    pub(super) fn can_resume(&self, prior: &ExternalAgentResume) -> bool {
        if let Some(continuity) = &self.continuity {
            return continuity.can_resume_previous;
        }
        let Some((current, history)) = self.entries.split_last() else {
            return false;
        };
        if current.entry.id != self.message || prior.message == self.message {
            return false;
        }
        let Some(position) = history
            .iter()
            .position(|entry| entry.entry.id == prior.message)
        else {
            return false;
        };
        // Match the old input and require that everything appended since it
        // came from the successful round we own. A branch starts a new native session.
        let (input, output) = history.split_at(position + 1);
        input[position].entry.execution_phase == ExecutionPhase::Succeeded
            && output
                .iter()
                .all(|entry| follows_native_round(entry, prior.round))
            && signature(input, prior.incarnation.map(|_| &self.structured), None)
                .is_ok_and(|signature| signature == prior.input)
    }

    pub(super) fn can_resume_pending(&self, prior: &ExternalAgentPendingTurn) -> bool {
        if let Some(continuity) = &self.continuity {
            return continuity.can_resume_pending;
        }
        let Some((current, history)) = self.entries.split_last() else {
            return false;
        };
        if current.entry.id != self.message {
            return false;
        }
        let input = if prior.message == self.message {
            self.entries.as_slice()
        } else {
            let Some(position) = history
                .iter()
                .position(|entry| entry.entry.id == prior.message)
            else {
                return false;
            };
            let (input, output) = history.split_at(position + 1);
            if !output
                .iter()
                .all(|entry| follows_native_round(entry, prior.round))
            {
                return false;
            }
            input
        };
        signature(input, Some(&self.structured), Some(prior.message))
            .is_ok_and(|signature| signature == prior.input)
    }

    pub(super) fn pending_turn(
        &self,
        round: RoundId,
        delivery: ExternalAgentPromptDelivery,
    ) -> Result<ExternalAgentPendingTurn, AppError> {
        let input = if let Some(continuity) = &self.continuity {
            continuity.pending_signature
        } else {
            signature(&self.entries, Some(&self.structured), Some(self.message))?
        };
        Ok(ExternalAgentPendingTurn {
            message: self.message,
            round,
            input,
            delivery,
        })
    }

    pub(super) fn resume_state(
        &self,
        session: SessionId,
        round: RoundId,
        link: ExternalAgentLink,
    ) -> Result<ExternalAgentResume, AppError> {
        let input = if let Some(continuity) = &self.continuity {
            if link.incarnation.is_some() {
                continuity.resume_signature_structured
            } else {
                continuity.resume_signature_unstructured
            }
        } else {
            signature(
                &self.entries,
                link.incarnation.map(|_| &self.structured),
                None,
            )?
        };
        Ok(ExternalAgentResume {
            session,
            cwd: link.cwd,
            round,
            native_id: link.native_id,
            incarnation: link.incarnation,
            input,
            message: self.message,
        })
    }

    pub(super) async fn blocks_with_capabilities(
        &self,
        reader: &KernelReader,
        session: SessionId,
        model: &ModelRef,
        agent: ExternalAgent,
        cwd: &std::path::Path,
        capabilities: PromptCapabilities,
    ) -> Result<(Vec<Value>, Vec<ReadLease>), AppError> {
        let mut blocks = Vec::new();
        let mut media_leases = Vec::new();
        let mut text = String::new();
        for node in &self.document.0 {
            let activity = match node {
                ComposerNode::Text { text: chunk } => {
                    text.push_str(chunk);
                    continue;
                }
                ComposerNode::Activity { activity } => activity,
            };
            if !text.is_empty() {
                blocks.push(text_block(std::mem::take(&mut text)));
            }
            let block = match &activity.payload {
                ActivityPayload::Resource(resource)
                    if resource.delivery == ResourceDelivery::ModelInput =>
                {
                    let direct_type = match resource.kind {
                        ResourceKind::Image if capabilities.image => Some("image"),
                        ResourceKind::Audio if capabilities.audio => Some("audio"),
                        ResourceKind::Image => {
                            return Err(AppError::Config(format!(
                                "{} does not support image input over ACP. Send the image as a project file reference.",
                                agent.label(),
                            )));
                        }
                        ResourceKind::Audio => {
                            return Err(AppError::Config(format!(
                                "{} does not support audio input over ACP. Send the audio as a project file reference.",
                                agent.label(),
                            )));
                        }
                        _ if capabilities.embedded_context => None,
                        _ => {
                            return Err(AppError::Config(format!(
                                "{} does not support embedded ACP resources. Send a project file reference.",
                                agent.label(),
                            )));
                        }
                    };
                    let hydrated = media::hydrate(
                        reader,
                        session,
                        self.message,
                        resource,
                        model,
                        run_read_budget(),
                    )
                    .await
                    .map_err(kernel_error)?;
                    let data = match hydrated.attachment.source {
                        CompletionInputAttachmentSource::ProviderData { data, .. } => data,
                        CompletionInputAttachmentSource::Unavailable { reason } => {
                            return Err(AppError::Config(format!(
                                "The attachment is unavailable because {reason}."
                            )));
                        }
                        _ => {
                            return Err(AppError::Internal(
                                "expected stored ACP attachment bytes".into(),
                            ));
                        }
                    };
                    media_leases.extend(hydrated.lease);
                    if let Some(kind) = direct_type {
                        json!({"type":kind,"mimeType":hydrated.attachment.mime,"data":data})
                    } else {
                        let uri = format!("agena:attachment/{}", activity.id);
                        embedded_resource(uri, &hydrated.attachment.mime, data)
                    }
                }
                ActivityPayload::Resource(resource) => {
                    let uri = match &resource.reference {
                        ResourceReference::WorkspacePath { path } => {
                            url::Url::from_file_path(cwd.join(path))
                                .map_err(|_| {
                                    AppError::Config(
                                        "The project file reference has no valid URI.".into(),
                                    )
                                })?
                                .to_string()
                        }
                        ResourceReference::Url { url } => url::Url::parse(url)
                            .map_err(|_| {
                                AppError::Config(
                                    "The attachment reference has no valid URI.".into(),
                                )
                            })?
                            .to_string(),
                        _ => unreachable!("external agent document was validated"),
                    };
                    let mut block = json!({"type":"resource_link","name":resource.name,"uri":uri});
                    if let Some(mime) = &resource.media_type {
                        block["mimeType"] = json!(mime);
                    }
                    if let Some(size) = resource.size_bytes {
                        block["size"] = json!(size);
                    }
                    block
                }
                ActivityPayload::SkillReference(skill) => {
                    if let Some(owner) = &skill.external_agent {
                        if owner != &agent || !capabilities.skill {
                            return Err(AppError::Config(
                                "The current ACP agent does not accept this native Skill reference.".into(),
                            ));
                        }
                        json!({"type":"skill", "name":skill.name})
                    } else {
                        text_block(skill.model_context_text())
                    }
                }
                ActivityPayload::TextArtifact(artifact) => text_block(artifact.text.clone()),
                _ => unreachable!("external agent document was validated"),
            };
            blocks.push(block);
        }
        if !text.is_empty() {
            blocks.push(text_block(text));
        }
        Ok((blocks, media_leases))
    }

    pub(super) fn has_prior_turns(&self) -> bool {
        if let Some(continuity) = &self.continuity {
            return continuity.has_prior_turns;
        }
        self.entries.iter().any(|entry| {
            entry.entry.id != self.message
                && matches!(entry.entry.payload, EntryPayload::User { .. })
        })
    }
}

fn embedded_resource(uri: String, mime: &str, data: String) -> Value {
    if mime.starts_with("text/")
        && let Ok(bytes) = STANDARD.decode(&data)
        && let Ok(text) = String::from_utf8(bytes)
    {
        json!({"type":"resource","resource":{"uri":uri,"mimeType":mime,"text":text}})
    } else {
        json!({"type":"resource","resource":{"uri":uri,"mimeType":mime,"blob":data}})
    }
}

pub(super) fn route_command(
    blocks: &mut Vec<Value>,
    commands: &[ExternalAgentCommand],
    capabilities: PromptCapabilities,
) -> Result<bool, AppError> {
    let find_skill = |name: &str| {
        commands
            .iter()
            .find(|entry| entry.skill && entry.name.eq_ignore_ascii_case(name))
            .map(|entry| entry.name.as_str())
    };
    let mut routed_slash_command = false;
    if let Some(first) = blocks.first_mut().filter(|block| block["type"] == "text")
        && let Some(input) = first["text"]
            .as_str()
            .and_then(external_agent_command_input)
    {
        let (name, args) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
        if capabilities.skill
            && let Some(canonical) = find_skill(name)
        {
            let args = args.trim_start().to_owned();
            *first = json!({"type": "skill", "name": canonical});
            if !args.is_empty() {
                blocks.insert(1, text_block(args));
            }
        } else {
            *first = text_block(format!("/{input}"));
            routed_slash_command = true;
        }
    }
    for (index, block) in blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| block["type"] == "skill")
    {
        if !capabilities.skill || (!capabilities.skill_inline && index != 0) {
            return Err(AppError::Config(
                "This ACP agent accepts a single native Skill at the start of a prompt.".into(),
            ));
        }
        if !block["name"]
            .as_str()
            .is_some_and(|name| find_skill(name).is_some())
        {
            return Err(AppError::Config(
                "A native Skill in the prompt is no longer advertised by the current agent.".into(),
            ));
        }
    }
    Ok(routed_slash_command)
}

fn text_block(text: String) -> Value {
    json!({"type": "text", "text": text})
}

fn follows_native_round(entry: &PromptEntry, round: RoundId) -> bool {
    entry.round_id == Some(round)
        || (matches!(
            entry.entry.execution_phase,
            ExecutionPhase::Failed | ExecutionPhase::Cancelled
        ) && matches!(entry.entry.payload, EntryPayload::User { .. }))
}

fn signature(
    entries: &[&PromptEntry],
    structured: Option<&BTreeSet<ContentId>>,
    retrying: Option<EntryId>,
) -> Result<[u8; 32], AppError> {
    let mut hash = Sha256::new();
    for entry in entries {
        // Completing a turn increments its user entry revision even though
        // the actual document is unchanged. Include the document and attempt
        // generation instead, which also catches edits and retries.
        let revision = match entry.entry.payload {
            EntryPayload::User { .. } => None,
            // The native conversation is authoritative for these documents.
            // A late replacement keeps the same entry and content identity.
            EntryPayload::Text { content_id } | EntryPayload::Reasoning { content_id }
                if structured.is_some_and(|sources| sources.contains(&content_id)) =>
            {
                None
            }
            _ => Some(entry.entry.revision),
        };
        let generation = (retrying != Some(entry.entry.id)).then_some(entry.entry.generation);
        let bytes = serde_json::to_vec(&(
            entry.entry.id,
            generation,
            entry.round_id,
            revision,
            &entry.entry.payload,
        ))?;
        hash.update(bytes.len().to_le_bytes());
        hash.update(bytes);
    }
    Ok(hash.finalize().into())
}
