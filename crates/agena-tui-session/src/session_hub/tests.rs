use super::*;
use ratatui::{Terminal, backend::TestBackend};

pub(super) fn item(workspace_id: i64, session_id: i64) -> SessionHubItem {
    SessionHubItem {
        session_id,
        workspace_id,
        pinned: false,
        favorite: false,
        title: format!("session-{session_id}"),
        label: format!("session-{session_id}"),
        status: "Idle".to_owned(),
        updated: "10-25 17:00".to_owned(),
        detail: String::new(),
        is_new_session: false,
    }
}

fn directory(id: i64) -> SessionHubDirectory {
    let mut new_session = item(id, 0);
    new_session.label = "New Session".to_owned();
    new_session.is_new_session = true;
    SessionHubDirectory {
        workspace_id: id,
        path: format!("/projects/project-{id}"),
        new_session,
        sections: Vec::new(),
        items: vec![item(id, id * 100)],
        expanded: true,
        visible_limit: 20,
    }
}

fn marked_directory(id: i64) -> SessionHubDirectory {
    let mut directory = directory(id);
    let mut pinned = item(id, id * 100 + 1);
    pinned.pinned = true;
    let mut favorite = item(id, id * 100 + 2);
    favorite.favorite = true;
    directory.sections = vec![
        SessionHubSection::new(SessionHubSectionKind::Pinned, vec![pinned]),
        SessionHubSection::new(SessionHubSectionKind::Favorites, vec![favorite]),
    ];
    directory
}

fn scoped_ids(hub: &SessionHubPresentation, target: HubPageTarget) -> Vec<i64> {
    hub.rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| match row {
            HubRow::Item(item) if hub.row_page_target(index) == target && !item.is_new_session => {
                Some(item.session_id)
            }
            _ => None,
        })
        .collect()
}

fn select_item(hub: &mut SessionHubPresentation, target: HubPageTarget, id: i64) {
    let index = hub
        .rows
        .iter()
        .enumerate()
        .find_map(|(index, row)| {
            (matches!(row, HubRow::Item(item) if item.session_id == id)
                && hub.row_page_target(index) == target)
                .then_some(index)
        })
        .expect("session exists in requested bucket");
    hub.select_row(index);
}

#[test]
fn current_project_capacity_uses_the_rendered_table_height() {
    for (height, expected) in [(24, 11), (36, 23), (60, 47)] {
        let area = Rect::new(0, 0, 100, height);
        assert_eq!(
            current_project_page_size(area, false, false, &I18n::english()),
            expected
        );
        assert_eq!(
            current_project_page_size(area, true, true, &I18n::english()),
            expected
        );
    }
    for height in 0..=16 {
        assert!(
            current_project_page_size(Rect::new(0, 0, 80, height), false, false, &I18n::english())
                >= 1
        );
    }
}

#[test]
fn terminal_sized_server_page_is_not_truncated_by_the_old_local_limit() {
    let area = Rect::new(0, 0, 100, 60);
    let capacity = current_project_page_size(area, false, false, &I18n::english());
    let mut project = directory(1);
    project.items = (1..=capacity as i64).map(|id| item(1, id)).collect();
    let mut hub = SessionHubPresentation::empty();
    hub.set_directories(vec![project]);
    let target = HubPageTarget::Directory(1);
    hub.set_pagination(BTreeMap::from([(
        target,
        HubPagination {
            page: 0,
            has_more: true,
            total: Some(200),
        },
    )]));
    hub.select_new_session(1);
    let regions = hub_row_regions(
        hub_layout(area, false, hub.view(), false, &I18n::english())[2],
        &hub,
    );
    let displayed = regions
        .rows
        .iter()
        .filter(
            |(index, _)| matches!(&hub.rows[*index], HubRow::Item(item) if !item.is_new_session),
        )
        .count();
    assert_eq!(displayed, capacity);
    assert_eq!(scoped_ids(&hub, target).len(), capacity);
    assert!(
        regions
            .rows
            .iter()
            .any(|(index, _)| matches!(hub.rows[*index], HubRow::Page { forward: true, .. }))
    );
    // The next button fits too; only the unused previous-button slot is spare.
    assert_eq!(regions.rows.len() + 1, hub.directory_height.get());
}

#[test]
fn three_views_cycle_in_both_directions() {
    let mut view = HubView::default();
    assert_eq!(view, HubView::CurrentProject);
    for expected in [
        HubView::Projects,
        HubView::AllSessions,
        HubView::CurrentProject,
    ] {
        view = view.next(1);
        assert_eq!(view, expected);
    }
    for expected in [
        HubView::AllSessions,
        HubView::Projects,
        HubView::CurrentProject,
    ] {
        view = view.next(-1);
        assert_eq!(view, expected);
    }
}

#[test]
fn project_navigation_skips_buckets_expands_destinations_and_wraps() {
    let mut hub = SessionHubPresentation::empty();
    let mut second = marked_directory(2);
    second.expanded = false;
    hub.set_catalog(
        vec![marked_directory(1), second, marked_directory(3)],
        vec![],
    );
    hub.select_new_session(1);
    assert_eq!(hub.move_selection_project(1), None);
    assert_eq!(hub.selected_workspace(), Some(1));
    hub.set_view(HubView::Projects);
    assert_eq!(hub.move_selection_project(1), Some(2));
    assert!(hub.selected_item().unwrap().is_new_session);
    assert_eq!(hub.selected_workspace(), Some(2));
    assert!(hub.expanded_directories().contains(&2));
    assert!(matches!(
        hub.rows[hub.directory_offset.get()],
        HubRow::Directory {
            workspace_id: 2,
            ..
        }
    ));
    assert_eq!(hub.move_selection_project(1), None);
    assert_eq!(hub.selected_workspace(), Some(3));
    hub.move_selection_project(1);
    assert_eq!(hub.selected_workspace(), Some(1));
    hub.move_selection_project(-1);
    assert_eq!(hub.selected_workspace(), Some(3));
    hub.set_query("session");
    select_item(&mut hub, HubPageTarget::Directory(3), 300);
    hub.move_selection_project(-1);
    assert_eq!(hub.selected_item().unwrap().session_id, 201);
}

#[test]
fn view_tabs_use_matching_mouse_cells_including_wide_translations() {
    for locale in ["en-US", "zh-CN"] {
        let i18n = I18n::resolve(Some(locale), None);
        for width in [50, 80, 120, 220] {
            let area = Rect::new(3, 2, width, 32);
            for has_error in [false, true] {
                let tabs = hub_view_tabs(
                    hub_layout(area, has_error, HubView::CurrentProject, false, &i18n)[0],
                    &i18n,
                );
                assert_eq!(tabs.len(), 3);
                for (view, rect, _) in tabs {
                    for column in rect.x..rect.right() {
                        assert_eq!(
                            hub_view_hit_test(area, has_error, column, rect.y, &i18n),
                            Some(view)
                        );
                        assert_eq!(
                            hub_hit_test(
                                area,
                                &SessionHubPresentation::empty(),
                                has_error,
                                false,
                                column,
                                rect.y,
                                &i18n
                            ),
                            None
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn flat_sessions_use_all_available_space_without_project_rows() {
    for width in [50, 80, 120, 220] {
        let mut hub = SessionHubPresentation::empty();
        hub.set_view(HubView::AllSessions);
        hub.set_sections(vec![SessionHubSection::new(
            SessionHubSectionKind::All,
            (1..=30).map(|id| item(id % 2, id)).collect(),
        )]);
        let area = Rect::new(0, 0, width, 32);
        let content = hub_layout(area, false, hub.view(), false, &I18n::english())[2];
        let regions = hub_row_regions(content, &hub);
        assert!(regions.divider.is_none());
        assert_eq!(regions.headers.len(), 1);
        assert!(
            regions
                .rows
                .iter()
                .all(|(_, rect)| rect.width == content.width)
        );
        assert!(!hub.rows.iter().any(|row| matches!(
            row,
            HubRow::Directory { .. } | HubRow::DirectoryHeader { .. }
        )));
        hub.move_selection_end();
        let regions = hub_row_regions(content, &hub);
        assert!(
            regions
                .rows
                .iter()
                .any(|(index, _)| *index == hub.selection())
        );
    }
}

#[test]
fn all_sessions_shows_global_mark_buckets_with_independent_pagination() {
    let mut hub = SessionHubPresentation::empty();
    hub.set_view(HubView::AllSessions);
    let mut old_pin = item(2, 200);
    old_pin.pinned = true;
    let mut old_favorite = item(3, 300);
    old_favorite.favorite = true;
    hub.set_sections(vec![
        SessionHubSection::new(SessionHubSectionKind::Pinned, vec![old_pin]),
        SessionHubSection::new(SessionHubSectionKind::Favorites, vec![old_favorite]),
        SessionHubSection::new(SessionHubSectionKind::All, vec![item(1, 100)]),
    ]);
    let pins = HubPageTarget::Section(SessionHubSectionKind::Pinned);
    let favorites = HubPageTarget::Section(SessionHubSectionKind::Favorites);
    let all = HubPageTarget::Section(SessionHubSectionKind::All);
    hub.set_pagination(BTreeMap::from([
        (
            pins,
            HubPagination {
                page: 1,
                has_more: false,
                total: Some(21),
            },
        ),
        (
            favorites,
            HubPagination {
                page: 0,
                has_more: true,
                total: Some(22),
            },
        ),
        (
            all,
            HubPagination {
                page: 2,
                has_more: true,
                total: Some(100),
            },
        ),
    ]));
    for (target, id, previous, next) in [
        (pins, 200, Some((pins, 0)), None),
        (favorites, 300, None, Some((favorites, 1))),
        (all, 100, Some((all, 1)), Some((all, 3))),
    ] {
        assert_eq!(scoped_ids(&hub, target), vec![id]);
        select_item(&mut hub, target, id);
        assert_eq!(hub.page_request(false), previous);
        assert_eq!(hub.page_request(true), next);
    }
    assert!(!hub.rows.iter().any(|row| matches!(
        row,
        HubRow::Directory { .. } | HubRow::DirectoryHeader { .. }
    )));
    // Global mark pages must never appear in project views.
    hub.set_view(HubView::Projects);
    assert!(!hub.rows.iter().any(|row| matches!(
        row,
        HubRow::Header(SessionHubSectionKind::Pinned | SessionHubSectionKind::Favorites)
    )));
}

#[test]
fn all_sessions_marks_update_duplicates_and_keep_pending_rows_stable() {
    let mut hub = SessionHubPresentation::empty();
    hub.set_view(HubView::AllSessions);
    let pins = HubPageTarget::Section(SessionHubSectionKind::Pinned);
    let favorites = HubPageTarget::Section(SessionHubSectionKind::Favorites);
    let all = HubPageTarget::Section(SessionHubSectionKind::All);
    hub.set_sections(vec![
        SessionHubSection::new(SessionHubSectionKind::Pinned, vec![]),
        SessionHubSection::new(SessionHubSectionKind::Favorites, vec![]),
        SessionHubSection::new(SessionHubSectionKind::All, vec![item(2, 200)]),
    ]);
    hub.set_pagination(BTreeMap::from([
        (
            pins,
            HubPagination {
                total: Some(0),
                ..Default::default()
            },
        ),
        (
            favorites,
            HubPagination {
                total: Some(0),
                ..Default::default()
            },
        ),
    ]));
    assert!(
        hub.rows
            .contains(&HubRow::Header(SessionHubSectionKind::Pinned))
    );
    assert!(
        hub.rows
            .contains(&HubRow::Header(SessionHubSectionKind::Favorites))
    );
    hub.set_item_pinned(200, true);
    hub.set_item_favorite(200, true);
    hub.set_item_pinned(200, true);
    assert_eq!(hub.pagination[&pins].total, Some(1));
    assert_eq!(hub.pagination[&favorites].total, Some(1));
    assert_eq!(scoped_ids(&hub, pins), vec![200]);
    assert_eq!(scoped_ids(&hub, favorites), vec![200]);
    select_item(&mut hub, pins, 200);
    let identities = hub.rows.iter().map(HubRow::identity).collect::<Vec<_>>();
    let selection = hub.selection();
    for index in 0..30 {
        hub.set_pending_marks(200, Some(index % 2 == 0), Some(index % 3 == 0));
        assert_eq!(
            hub.rows.iter().map(HubRow::identity).collect::<Vec<_>>(),
            identities
        );
        assert_eq!(hub.selection(), selection);
        assert_eq!(hub.pagination[&pins].total, Some(1));
        for row in hub.rows() {
            if let HubRow::Item(item) = row {
                assert_eq!(item.pinned, index % 2 == 0);
                assert_eq!(item.favorite, index % 3 == 0);
            }
        }
    }
    // Reconcile pending markers before applying authoritative membership.
    hub.set_pending_marks(200, Some(true), Some(true));
    hub.set_item_pinned(200, false);
    assert_eq!(hub.selected_item().unwrap().session_id, 200);
    assert!(scoped_ids(&hub, pins).is_empty());
    assert_eq!(scoped_ids(&hub, favorites), vec![200]);
    hub.set_item_favorite(200, false);
    assert_eq!(hub.selected_page_target(), all);
    assert_eq!(hub.pagination[&pins].total, Some(0));
    assert_eq!(hub.pagination[&favorites].total, Some(0));
}

#[test]
fn pending_mark_bursts_keep_rows_counts_and_selection_stable() {
    let mut hub = SessionHubPresentation::empty();
    let mut project = marked_directory(1);
    project.items.push(project.sections[0].items[0].clone());
    hub.set_catalog(
        vec![project, marked_directory(2)],
        vec![SessionHubSection::new(
            SessionHubSectionKind::Recent,
            vec![item(1, 101)],
        )],
    );
    let target = HubPageTarget::DirectorySection(1, SessionHubSectionKind::Pinned);
    hub.set_pagination(BTreeMap::from([(
        target,
        HubPagination {
            total: Some(1),
            ..Default::default()
        },
    )]));
    select_item(&mut hub, target, 101);
    let identities = hub.rows.iter().map(HubRow::identity).collect::<Vec<_>>();
    let selection = hub.selection;
    hub.directory_offset.set(2);
    for index in 0..50 {
        hub.set_pending_marks(101, Some(index % 2 == 1), Some(index % 3 == 1));
        assert_eq!(
            hub.rows.iter().map(HubRow::identity).collect::<Vec<_>>(),
            identities
        );
        assert_eq!(hub.selection, selection);
        assert_eq!(hub.directory_offset.get(), 2);
        assert_eq!(hub.pagination[&target].total, Some(1));
        for row in hub.rows() {
            if let HubRow::Item(item) = row
                && item.session_id == 101
            {
                assert_eq!(item.pinned, index % 2 == 1);
                assert_eq!(item.favorite, index % 3 == 1);
            }
        }
    }
    // Rebuilding for navigation/filtering must retain the same pending marks.
    hub.set_query("session-101");
    assert!(hub.selected_item().unwrap().pinned);
    assert!(hub.selected_item().unwrap().favorite);
}

#[test]
fn committed_unpin_keeps_the_session_selected_instead_of_a_bucket_neighbor() {
    let mut hub = SessionHubPresentation::empty();
    let mut project = marked_directory(1);
    project.items.push(project.sections[0].items[0].clone());
    let mut neighbor = item(1, 103);
    neighbor.pinned = true;
    project.sections[0].items.push(neighbor);
    hub.set_catalog(vec![project.clone()], vec![]);
    select_item(
        &mut hub,
        HubPageTarget::DirectorySection(1, SessionHubSectionKind::Pinned),
        101,
    );
    hub.set_pending_marks(101, Some(false), None);
    project.sections[0].items.remove(0);
    project
        .items
        .iter_mut()
        .find(|item| item.session_id == 101)
        .unwrap()
        .pinned = false;
    hub.set_catalog(vec![project], vec![]);
    assert_eq!(hub.selected_item().unwrap().session_id, 101);
    assert_eq!(hub.selected_page_target(), HubPageTarget::Directory(1));
    assert!(!hub.selected_item().unwrap().pinned);
}

#[test]
fn opening_focuses_project_create_and_catalog_refresh_preserves_it() {
    let mut hub = SessionHubPresentation::empty();
    hub.set_view(HubView::Projects);
    hub.set_directories(vec![directory(2)]);
    hub.select_new_session(2);
    assert!(
        hub.selected_item()
            .is_some_and(|item| item.is_new_session && item.workspace_id == 2)
    );
    hub.set_catalog(vec![directory(1), marked_directory(2)], vec![]);
    assert!(
        hub.selected_item()
            .is_some_and(|item| item.is_new_session && item.workspace_id == 2)
    );
    assert_eq!(hub.rows.first(), Some(&HubRow::NewProject));
    assert!(matches!(hub.rows.get(1), Some(HubRow::Directory { .. })));
    assert_eq!(
        hub.rows
            .iter()
            .filter(|row| matches!(row, HubRow::Item(item) if item.is_new_session))
            .count(),
        2
    );

    // Loading and live refresh must not steal focus once the user has moved.
    select_item(
        &mut hub,
        HubPageTarget::DirectorySection(2, SessionHubSectionKind::Favorites),
        202,
    );
    hub.set_catalog(vec![marked_directory(2), directory(1)], vec![]);
    assert_eq!(hub.selected_item().unwrap().session_id, 202);
    assert_eq!(
        hub.selected_page_target(),
        HubPageTarget::DirectorySection(2, SessionHubSectionKind::Favorites)
    );
}

#[test]
fn new_project_action_is_first_selectable_and_survives_catalog_and_search_updates() {
    let mut hub = SessionHubPresentation::empty();
    hub.set_view(HubView::Projects);
    assert_eq!(hub.rows(), &[HubRow::NewProject]);
    hub.set_catalog(vec![directory(1), directory(2)], vec![]);
    hub.select_new_session(1);
    hub.move_selection(-2);
    assert_eq!(hub.selected_row(), Some(&HubRow::NewProject));
    assert_eq!(hub.selected_workspace(), None);
    assert_eq!(hub.selected_page_position(), None);
    assert_eq!(hub.selected_page_target(), HubPageTarget::Workspaces);
    hub.set_catalog(vec![directory(2), directory(1)], vec![]);
    assert_eq!(hub.selected_row(), Some(&HubRow::NewProject));
    hub.set_query("no matching projects");
    assert_eq!(hub.rows(), &[HubRow::NewProject]);
    hub.set_query("");
    assert_eq!(hub.selected_row(), Some(&HubRow::NewProject));
    for view in [HubView::CurrentProject, HubView::AllSessions] {
        hub.set_view(view);
        assert!(!hub.rows().contains(&HubRow::NewProject));
    }
}

#[test]
fn marked_buckets_are_scoped_and_can_include_older_sessions() {
    let mut hub = SessionHubPresentation::empty();
    hub.set_catalog(vec![marked_directory(1), marked_directory(2)], vec![]);
    for id in [1, 2] {
        assert_eq!(
            scoped_ids(
                &hub,
                HubPageTarget::DirectorySection(id, SessionHubSectionKind::Pinned)
            ),
            vec![id * 100 + 1]
        );
        assert_eq!(
            scoped_ids(
                &hub,
                HubPageTarget::DirectorySection(id, SessionHubSectionKind::Favorites)
            ),
            vec![id * 100 + 2]
        );
        assert_eq!(
            scoped_ids(&hub, HubPageTarget::Directory(id)),
            vec![id * 100]
        );
    }
    assert!(!hub.rows.iter().any(|row| matches!(
        row,
        HubRow::Header(
            SessionHubSectionKind::Pinned
                | SessionHubSectionKind::Favorites
                | SessionHubSectionKind::New
        )
    )));
}

#[test]
fn pin_and_favorite_updates_are_independent_and_stay_in_the_owning_project() {
    let mut hub = SessionHubPresentation::empty();
    hub.set_catalog(
        vec![directory(1), marked_directory(2)],
        vec![SessionHubSection::new(
            SessionHubSectionKind::Recent,
            vec![item(1, 100)],
        )],
    );
    let pins = HubPageTarget::DirectorySection(1, SessionHubSectionKind::Pinned);
    let favorites = HubPageTarget::DirectorySection(1, SessionHubSectionKind::Favorites);
    hub.set_pagination(BTreeMap::from([
        (
            pins,
            HubPagination {
                total: Some(0),
                ..Default::default()
            },
        ),
        (
            favorites,
            HubPagination {
                total: Some(0),
                ..Default::default()
            },
        ),
    ]));
    hub.set_item_pinned(100, true);
    hub.set_item_favorite(100, true);
    hub.set_item_pinned(100, true); // Duplicate authoritative notifications are idempotent.
    assert_eq!(scoped_ids(&hub, pins), vec![100]);
    assert_eq!(scoped_ids(&hub, favorites), vec![100]);
    assert_eq!(hub.pagination[&pins].total, Some(1));
    assert_eq!(hub.pagination[&favorites].total, Some(1));
    for row in hub.rows() {
        if let HubRow::Item(item) = row
            && item.session_id == 100
        {
            assert!(item.pinned && item.favorite);
        }
    }
    hub.set_item_pinned(100, false);
    assert!(scoped_ids(&hub, pins).is_empty());
    assert_eq!(scoped_ids(&hub, favorites), vec![100]);
    assert_eq!(hub.pagination[&pins].total, Some(0));
    assert_eq!(
        scoped_ids(
            &hub,
            HubPageTarget::DirectorySection(2, SessionHubSectionKind::Pinned)
        ),
        vec![201]
    );
    hub.set_item_favorite(100, false);
    assert!(scoped_ids(&hub, favorites).is_empty());
}

#[test]
fn project_bucket_pagination_has_distinct_identity_and_selection() {
    let mut hub = SessionHubPresentation::empty();
    let pins1 = HubPageTarget::DirectorySection(1, SessionHubSectionKind::Pinned);
    let pins2 = HubPageTarget::DirectorySection(2, SessionHubSectionKind::Pinned);
    let favorites1 = HubPageTarget::DirectorySection(1, SessionHubSectionKind::Favorites);
    hub.set_catalog(vec![marked_directory(1), marked_directory(2)], vec![]);
    hub.set_pagination(
        [pins1, pins2, favorites1]
            .into_iter()
            .map(|target| {
                (
                    target,
                    HubPagination {
                        page: 1,
                        has_more: true,
                        total: Some(45),
                    },
                )
            })
            .collect(),
    );
    select_item(&mut hub, pins1, 101);
    assert_eq!(hub.page_request(true), Some((pins1, 2)));
    assert_eq!(hub.page_request(false), Some((pins1, 0)));
    let next = hub
        .rows
        .iter()
        .position(
            |row| matches!(row, HubRow::Page { target, forward: true, .. } if *target == pins2),
        )
        .unwrap();
    hub.select_row(next);
    assert_eq!(hub.selected_workspace(), Some(2));
    hub.set_catalog(vec![marked_directory(1), marked_directory(2)], vec![]);
    assert!(
        matches!(hub.selected_row(), Some(HubRow::Page { target, forward: true, .. }) if *target == pins2)
    );
    hub.select_page_target(favorites1, false);
    assert_eq!(hub.selected_item().unwrap().session_id, 102);
    assert_eq!(hub.selected_page_target(), favorites1);
}

#[test]
fn updating_another_page_does_not_insert_into_the_current_page() {
    let mut hub = SessionHubPresentation::empty();
    let target = HubPageTarget::DirectorySection(1, SessionHubSectionKind::Pinned);
    hub.set_catalog(vec![marked_directory(1)], vec![]);
    hub.set_pagination(BTreeMap::from([(
        target,
        HubPagination {
            page: 1,
            total: Some(21),
            ..Default::default()
        },
    )]));
    hub.set_item_pinned(100, true);
    assert_eq!(scoped_ids(&hub, target), vec![101]);
    assert_eq!(hub.pagination[&target].total, Some(22));
}

#[test]
fn collapsing_and_refreshing_a_project_preserves_expansion_and_skips_headers() {
    let mut hub = SessionHubPresentation::empty();
    hub.set_catalog(vec![marked_directory(1), marked_directory(2)], vec![]);
    hub.toggle_directory(1, Some(false));
    hub.set_catalog(vec![marked_directory(1), marked_directory(2)], vec![]);
    assert!(!hub.expanded_directories().contains(&1));
    assert!(
        scoped_ids(
            &hub,
            HubPageTarget::DirectorySection(1, SessionHubSectionKind::Pinned)
        )
        .is_empty()
    );
    hub.move_selection_home();
    for _ in 0..hub.rows.len() * 2 {
        hub.move_selection_section(1);
        assert!(hub.selected_row().unwrap().identity().is_some());
    }
    hub.move_selection_home();
    for _ in 0..hub.rows.len() {
        hub.move_selection(1);
        assert!(hub.selected_row().unwrap().identity().is_some());
    }
}

#[test]
fn search_finds_project_bucket_items_and_hides_create_actions() {
    let mut hub = SessionHubPresentation::empty();
    hub.set_catalog(vec![marked_directory(1), marked_directory(2)], vec![]);
    hub.set_query("session-202");
    assert!(
        !hub.rows
            .iter()
            .any(|row| matches!(row, HubRow::Item(item) if item.is_new_session))
    );
    assert_eq!(
        scoped_ids(
            &hub,
            HubPageTarget::DirectorySection(2, SessionHubSectionKind::Favorites)
        ),
        vec![202]
    );
    assert!(!hub.rows.iter().any(|row| matches!(
        row,
        HubRow::Directory {
            workspace_id: 1,
            ..
        }
    )));
    hub.set_query("");
    assert_eq!(hub.expanded_directories(), BTreeSet::from([1, 2]));
    hub.select_new_session(1);
    assert!(hub.selected_item().unwrap().is_new_session);
}

#[test]
fn project_headers_render_in_the_tree_and_mouse_matches_both_viewports() {
    for (width, height) in [(80, 32), (140, 32), (220, 32), (50, 12)] {
        let mut hub = SessionHubPresentation::empty();
        hub.set_catalog(
            vec![marked_directory(1), marked_directory(2)],
            vec![SessionHubSection::new(
                SessionHubSectionKind::Running,
                vec![item(2, 299)],
            )],
        );
        let i18n = I18n::english();
        let area = Rect::new(0, 0, width, height);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        // Scroll through every selectable row, exercising independent offsets.
        for index in hub.item_indices() {
            hub.select_row(index);
            terminal
                .draw(|frame| render_session_hub(frame, area, &hub, false, None, false, "", &i18n))
                .unwrap();
            let regions = hub_row_regions(
                hub_layout(area, false, hub.view(), false, &I18n::english())[2],
                &hub,
            );
            assert!(regions.rows.iter().any(|(visible, _)| *visible == index));
            for (header, _) in &regions.headers {
                assert_eq!(
                    hub_hit_test(area, &hub, false, false, header.x, header.y, &i18n),
                    None
                );
            }
            for (visible, rect) in regions.rows {
                let hit = hub_hit_test(area, &hub, false, false, rect.x, rect.y, &i18n);
                if hub.rows[visible].identity().is_some() {
                    assert_eq!(
                        hit,
                        Some(HubHit {
                            row: visible,
                            activate: true
                        })
                    );
                } else {
                    assert_eq!(hit, None);
                }
                if let HubRow::DirectoryHeader { .. } = hub.rows[visible] {
                    assert!(hub.same_region(visible, 0));
                    let text = (rect.x..rect.right())
                        .map(|x| terminal.backend().buffer()[(x, rect.y)].symbol())
                        .collect::<String>();
                    assert!(
                        text.contains("Pinned")
                            || text.contains("Favorites")
                            || text.contains("Recent"),
                        "{text}"
                    );
                }
            }
        }
    }
}

#[test]
fn normal_terminals_use_full_width_tables_with_sticky_column_headings() {
    for width in [50, 80, 120, 160, 220] {
        let mut hub = SessionHubPresentation::empty();
        hub.set_catalog(
            vec![marked_directory(1)],
            vec![SessionHubSection::new(
                SessionHubSectionKind::Recent,
                vec![item(2, 299)],
            )],
        );
        let area = Rect::new(0, 0, width, 32);
        let content = hub_layout(area, false, hub.view(), false, &I18n::english())[2];
        let regions = hub_row_regions(content, &hub);
        assert_eq!(regions.headers.len(), 2);
        if width < 196 {
            assert!(
                regions
                    .rows
                    .iter()
                    .all(|(_, rect)| rect.width == content.width)
            );
            assert!(regions.headers.iter().all(|(rect, _)| rect.x == content.x));
        } else {
            assert!(regions.rows.iter().all(|(_, rect)| rect.width >= 96));
        }
        let mut terminal = Terminal::new(TestBackend::new(width, 32)).unwrap();
        terminal
            .draw(|frame| {
                render_session_hub(frame, area, &hub, false, None, false, "", &I18n::english())
            })
            .unwrap();
        for (rect, _) in regions.headers {
            let text = (rect.x..rect.right())
                .map(|x| terminal.backend().buffer()[(x, rect.y)].symbol())
                .collect::<String>();
            assert!(
                text.contains("Name") && text.contains("Status") && text.contains("Updated"),
                "{text}"
            );
        }
        // Item contents and metadata render together, even on a 50-cell terminal.
        let regions = hub_row_regions(content, &hub);
        let (_, rect) = regions.rows.iter().find(|(index, _)| matches!(&hub.rows[*index], HubRow::Item(item) if item.session_id == 101)).unwrap();
        let text = (rect.x..rect.right())
            .map(|x| terminal.backend().buffer()[(x, rect.y)].symbol())
            .collect::<String>();
        assert!(
            text.contains("session-101") && text.contains("Idle") && text.contains("10-25 17:00"),
            "{text}"
        );
    }
}
