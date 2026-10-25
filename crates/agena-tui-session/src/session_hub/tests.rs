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
        status: "Ready".to_owned(),
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
fn opening_focuses_project_create_and_catalog_refresh_preserves_it() {
    let mut hub = SessionHubPresentation::empty();
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
    assert!(matches!(hub.rows.first(), Some(HubRow::Directory { .. })));
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
            let regions = hub_row_regions(hub_layout(area, false)[2], &hub);
            assert!(regions.rows.iter().any(|(visible, _)| *visible == index));
            for (header, _) in &regions.headers {
                assert_eq!(
                    hub_hit_test(area, &hub, false, header.x, header.y, &i18n),
                    None
                );
            }
            for (visible, rect) in regions.rows {
                let hit = hub_hit_test(area, &hub, false, rect.x, rect.y, &i18n);
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
        let content = hub_layout(area, false)[2];
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
            text.contains("session-101") && text.contains("Ready") && text.contains("10-25 17:00"),
            "{text}"
        );
    }
}
