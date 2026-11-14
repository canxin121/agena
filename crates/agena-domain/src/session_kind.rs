//! Which kind of conversation a capability may serve.
//!
//! A **project session** owns a working directory and runs project tooling.
//! A **Bot session** belongs to a registered Bot and manages projects and
//! ordinary conversations through its own tools. Bot ownership applies across
//! local and external entrances; a channel's private workspace does not turn
//! its Bot conversation into a project conversation.
//!
//! [`SessionKind`] is that distinction, and [`ApplicableSessions`] is the set a
//! capability declares. The vocabulary lives in `agena-domain` because every
//! layer needs to name it: the storage projection records it on a session, the
//! plugin contracts declare it on commands and tools, and the runtime executor
//! consults it before advertising or running anything.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The two conversation shapes a capability can be scoped to.
///
/// Deliberately only two. Chat-channel sessions and subagents resolve to one
/// of these when they are created, so a plugin author never has to reason
/// about an adapter they do not ship.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    /// A conversation bound to a project workspace: it has a working
    /// directory and its tools act on that project.
    #[default]
    Project,
    /// A Bot-owned conversation, local or external, whose tools manage
    /// projects and ordinary conversations.
    Bot,
}

impl SessionKind {
    pub const ALL: [Self; 2] = [Self::Project, Self::Bot];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Bot => "bot",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "project" => Some(Self::Project),
            "bot" => Some(Self::Bot),
            _ => None,
        }
    }

    /// Resolve capability applicability from conversation ownership and scope.
    ///
    /// Bot ownership takes precedence over the private workspace allocated to
    /// a channel. Unowned conversations retain their workspace-based kind.
    pub const fn from_scope(workspace_id: Option<i64>, bot_owned: bool) -> Self {
        if bot_owned {
            Self::Bot
        } else {
            Self::from_workspace(workspace_id)
        }
    }

    /// Resolve a context that has no separate Bot ownership information.
    pub const fn from_workspace(workspace_id: Option<i64>) -> Self {
        match workspace_id {
            Some(_) => Self::Project,
            None => Self::Bot,
        }
    }
}

/// The session kinds a command or tool is declared for.
///
/// The default is "every kind" so that existing declarations keep working and
/// a new plugin stays usable without declaring anything.
///
/// On the wire it accepts one kind name (`"bot"`), a list of kinds
/// (`["project", "bot"]`), or the explicit `"all"`. An unknown name is a
/// rejected declaration rather than one that silently never applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema)]
#[schemars(with = "SessionSurfaceWire")]
pub struct ApplicableSessions(SessionSurfaceSet);

/// The wire shape of [`ApplicableSessions`]: a kind name, a list of kind
/// names, or `"all"`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
enum SessionSurfaceWire {
    All(AllKinds),
    One(SessionKind),
    Many(Vec<SessionKind>),
}

/// The `"all"` spelling, accepted next to a kind name.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AllKinds {
    All,
}

impl Serialize for ApplicableSessions {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.is_all() {
            return serializer.serialize_str("all");
        }
        let kinds = self.kinds().collect::<Vec<_>>();
        match kinds.as_slice() {
            [only] => serializer.serialize_str(only.as_str()),
            _ => kinds.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ApplicableSessions {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match SessionSurfaceWire::deserialize(deserializer)? {
            SessionSurfaceWire::All(AllKinds::All) => Self::all(),
            SessionSurfaceWire::One(kind) => Self::only(kind),
            SessionSurfaceWire::Many(kinds) => Self::from_kinds(&kinds),
        })
    }
}

/// A bit set over [`SessionKind`], kept private so every construction path
/// performs the union/intersection checks callers rely on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SessionSurfaceSet(u8);

const PROJECT_BIT: u8 = 1 << 0;
const BOT_BIT: u8 = 1 << 1;
const ALL_BITS: u8 = PROJECT_BIT | BOT_BIT;

impl SessionSurfaceSet {
    const fn bit(self, kind: SessionKind) -> u8 {
        match kind {
            SessionKind::Project => PROJECT_BIT,
            SessionKind::Bot => BOT_BIT,
        }
    }

    const fn contains(self, kind: SessionKind) -> bool {
        self.0 & self.bit(kind) != 0
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    const fn complement(self) -> Self {
        Self(ALL_BITS & !self.0)
    }
}

impl Default for ApplicableSessions {
    fn default() -> Self {
        Self::all()
    }
}

impl ApplicableSessions {
    /// Every session kind. This is what an undeclared capability means.
    pub const fn all() -> Self {
        Self(SessionSurfaceSet(ALL_BITS))
    }

    /// No session kind at all. Only useful as an intermediate value; a
    /// declaration with this set is rejected by validation.
    pub const fn none() -> Self {
        Self(SessionSurfaceSet(0))
    }

    pub const fn project() -> Self {
        Self(SessionSurfaceSet(PROJECT_BIT))
    }

    pub const fn bot() -> Self {
        Self(SessionSurfaceSet(BOT_BIT))
    }

    /// A set naming exactly `kind`.
    pub const fn only(kind: SessionKind) -> Self {
        Self::from_kinds(&[kind])
    }

    /// A set naming each kind given. Duplicates collapse.
    pub const fn from_kinds(kinds: &[SessionKind]) -> Self {
        let mut bits = 0u8;
        let mut index = 0;
        while index < kinds.len() {
            bits |= match kinds[index] {
                SessionKind::Project => PROJECT_BIT,
                SessionKind::Bot => BOT_BIT,
            };
            index += 1;
        }
        Self(SessionSurfaceSet(bits))
    }

    pub const fn contains(self, kind: SessionKind) -> bool {
        self.0.contains(kind)
    }

    /// True when a declaration names no session kind. Such a declaration is
    /// invalid rather than "applies nowhere": refusing it at validation keeps
    /// a typo from turning into a silently invisible command.
    pub const fn is_empty(self) -> bool {
        self.0.is_empty()
    }

    /// True when the set covers every kind, i.e. it says nothing.
    pub const fn is_all(&self) -> bool {
        (self.0).0 == ALL_BITS
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0.union(other.0))
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0.intersect(other.0))
    }

    /// True when the two sets have at least one kind in common. Two
    /// declarations of one slash may coexist only when this is false: a
    /// client asks for one command by name, so an ambiguous overlap would make
    /// which implementation runs depend on catalog order.
    pub const fn overlaps(self, other: Self) -> bool {
        !self.0.intersect(other.0).is_empty()
    }

    /// The kinds this set does not name.
    pub const fn complement(self) -> Self {
        Self(self.0.complement())
    }

    /// The kinds named by this set but not by `other`.
    pub const fn remove(self, other: Self) -> Self {
        self.intersection(other.complement())
    }

    pub fn kinds(self) -> impl Iterator<Item = SessionKind> {
        SessionKind::ALL
            .into_iter()
            .filter(move |kind| self.contains(*kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_applies_everywhere() {
        assert_eq!(ApplicableSessions::default(), ApplicableSessions::all());
        assert!(ApplicableSessions::default().is_all());
        assert!(!ApplicableSessions::default().is_empty());
    }

    #[test]
    fn a_declared_kind_is_the_only_one_that_matches() {
        let project = ApplicableSessions::project();
        assert!(project.contains(SessionKind::Project));
        assert!(!project.contains(SessionKind::Bot));
        assert!(!project.is_all());
        assert!(!project.is_empty());
        assert_eq!(
            project.kinds().collect::<Vec<_>>(),
            vec![SessionKind::Project]
        );
    }

    #[test]
    fn overlap_decides_whether_two_declarations_can_share_a_slash() {
        let project = ApplicableSessions::project();
        let bot = ApplicableSessions::bot();
        assert!(!project.overlaps(bot));
        assert!(!bot.overlaps(project));
        assert!(project.overlaps(ApplicableSessions::all()));
        assert!(ApplicableSessions::all().overlaps(bot));
        // Two declarations that both apply everywhere cannot share a name.
        assert!(ApplicableSessions::all().overlaps(ApplicableSessions::all()));
    }

    #[test]
    fn union_remove_and_intersection_compose() {
        let all = ApplicableSessions::all();
        let project = ApplicableSessions::project();
        let bot = ApplicableSessions::bot();
        assert_eq!(project.union(bot), all);
        assert_eq!(project.intersection(bot), ApplicableSessions::none());
        assert_eq!(all.remove(bot), project);
        assert_eq!(project.remove(project), ApplicableSessions::none());
    }

    #[test]
    fn workspace_is_the_discriminator() {
        assert_eq!(SessionKind::from_workspace(Some(9)), SessionKind::Project);
        assert_eq!(SessionKind::from_workspace(None), SessionKind::Bot);
        assert_eq!(SessionKind::default(), SessionKind::Project);
    }

    #[test]
    fn bot_ownership_takes_precedence_over_a_channel_private_workspace() {
        assert_eq!(SessionKind::from_scope(Some(9), true), SessionKind::Bot);
        assert_eq!(SessionKind::from_scope(None, true), SessionKind::Bot);
        assert_eq!(
            SessionKind::from_scope(Some(9), false),
            SessionKind::Project
        );
        assert_eq!(SessionKind::from_scope(None, false), SessionKind::Bot);
    }

    #[test]
    fn wire_spelling_is_the_kind_name() {
        assert_eq!(
            serde_json::to_string(&ApplicableSessions::bot()).unwrap(),
            "\"bot\""
        );
        assert_eq!(
            serde_json::from_str::<ApplicableSessions>("\"project\"").unwrap(),
            ApplicableSessions::project()
        );
        assert_eq!(
            serde_json::from_str::<ApplicableSessions>("\"all\"").unwrap(),
            ApplicableSessions::all()
        );
        assert_eq!(
            serde_json::from_str::<ApplicableSessions>("[\"project\",\"bot\"]").unwrap(),
            ApplicableSessions::all()
        );
        assert_eq!(
            serde_json::to_string(&ApplicableSessions::all()).unwrap(),
            "\"all\""
        );
        assert!(serde_json::from_str::<ApplicableSessions>("\"channel\"").is_err());
    }
}
