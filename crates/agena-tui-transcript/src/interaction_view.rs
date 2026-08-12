//! Live interaction content for pending user-input parts rendered inline in
//! the transcript.
//!
//! A pending interaction part (plan review or ask-user) renders as an
//! expandable Activity. When expanded and still awaiting a decision, the app
//! attaches an inline document — the same flat plan + decision layout the
//! overlay used — so the part itself is the interaction surface
//! ("everything is a part"). This module exposes the small projections the
//! renderer and the app share: correlating a part back to its `request_id`,
//! and carrying the pre-rendered inline document rows.

use crate::{
    RequestPartResource, TranscriptActivityContent, TranscriptEntryPart, TranscriptPartContent,
};

/// The `request_id` of a pending user-input interaction part, or `None` when
/// the part is not an interaction or has already been answered. Only pending
/// parts are interactive in the transcript, so the key router and the inline
/// renderer agree on this boundary.
pub fn interaction_request_id_for_part<'a>(
    part: &'a TranscriptEntryPart<'a>,
) -> Option<&'a str> {
    match &part.content {
        TranscriptPartContent::Activity(TranscriptActivityContent::Request(request)) => {
            match request.as_ref() {
                RequestPartResource::UserInput { request, reply } => {
                    reply.is_none().then_some(request.request_id.as_str())
                }
            }
        }
        _ => None,
    }
}
