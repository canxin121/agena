use super::*;
use agena_tui_session::session_hub::{HubPagination, SessionHubPresentation};
use std::collections::BTreeMap;

fn loaded_page() -> HubState {
    let item = |id| SessionHubItem {
        session_id: id,
        workspace_id: 1,
        pinned: true,
        favorite: false,
        title: format!("session-{id}"),
        label: format!("session-{id}"),
        status: "Idle".into(),
        updated: String::new(),
        detail: String::new(),
        is_new_session: id == 0,
    };
    let target = HubPageTarget::DirectorySection(1, SessionHubSectionKind::Pinned);
    let mut state = HubState::new();
    state.presentation = SessionHubPresentation::empty();
    state
        .presentation
        .set_directories(vec![SessionHubDirectory {
            workspace_id: 1,
            path: "/project".into(),
            new_session: item(0),
            sections: vec![SessionHubSection::new(
                SessionHubSectionKind::Pinned,
                (41..=60).map(item).collect(),
            )],
            items: (41..=60).map(item).collect(),
            expanded: true,
            visible_limit: 20,
        }]);
    state.presentation.set_pagination(BTreeMap::from([
        (
            target,
            HubPagination {
                page: 2,
                has_more: true,
                total: Some(100),
            },
        ),
        (HubPageTarget::Directory(1), HubPagination::default()),
    ]));
    state.pages.insert(target, 2);
    state.presentation.select_page_target(target, true);
    state
}

#[test]
fn resizing_keeps_selected_session_in_its_bucket_and_new_page() {
    let mut state = loaded_page();
    let target = HubPageTarget::DirectorySection(1, SessionHubSectionKind::Pinned);
    // Page 2, item 19 => absolute offset 59; rebasing only the old page
    // start (40) would request page 1 and lose the selected session.
    assert_eq!(
        state.presentation.selected_page_position(),
        Some((target, 2, 19))
    );
    assert!(state.resize_page_size(25));
    assert_eq!(state.pages[&target], 2);
    assert_eq!(state.presentation.selected_item().unwrap().session_id, 60);
    assert!(state.resize_page_size(10));
    assert_eq!(state.pages[&target], 5);
    // No response arrived yet: still anchor to loaded 20-item pages.
    assert!(state.resize_page_size(30));
    assert_eq!(state.pages[&target], 1);
    assert!(!state.resize_page_size(30));
}

#[test]
fn resizing_during_page_navigation_preserves_requested_direction() {
    let mut state = loaded_page();
    let target = HubPageTarget::DirectorySection(1, SessionHubSectionKind::Pinned);
    state.pages.insert(target, 3);
    state.pending_page = Some((target, false));
    assert!(state.resize_page_size(10));
    assert_eq!(state.pages[&target], 6);
    assert_eq!(state.pending_page, Some((target, false)));

    let mut state = loaded_page();
    state.pages.insert(target, 1);
    state.pending_page = Some((target, true));
    assert!(state.resize_page_size(10));
    assert_eq!(state.pages[&target], 3);
    assert_eq!(state.pending_page, Some((target, true)));
}

#[test]
fn resizing_a_new_search_does_not_reuse_the_old_catalog_page() {
    let mut state = loaded_page();
    state.pages.clear();
    state.query = "session".into();
    assert!(state.resize_page_size(25));
    assert!(state.pages.is_empty());
    // Clearing a search also resets pagination, even if it returns to the
    // last loaded query while a search response is still in flight.
    state.query.clear();
    state.search_changed_at = Some(std::time::Instant::now());
    assert!(state.resize_page_size(10));
    assert!(state.pages.is_empty());
}
