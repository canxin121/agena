use super::*;
use agena_domain::{UiKind, UiResolvedNode};
use serde_json::json;

fn node(id: &str, kind: UiKind, props: Value, children: Vec<UiResolvedNode>) -> UiResolvedNode {
    UiResolvedNode {
        id: id.into(),
        kind,
        props: serde_json::from_value(props).unwrap(),
        children,
        bind: None,
        value: Value::Null,
        field: None,
        action: None,
    }
}
fn metric(id: &str, label: &str) -> UiResolvedNode {
    node(
        id,
        UiKind::Metric,
        json!({"label":label,"value":12345,"detail":"当前值"}),
        vec![],
    )
}
fn card(id: &str) -> UiResolvedNode {
    node(
        id,
        UiKind::Card,
        json!({"title":format!("分组 {id}")}),
        vec![metric(&format!("{id}-metric"), "本期金额")],
    )
}
fn render(root: &UiResolvedNode, width: u16) -> Vec<RenderedLine> {
    let presentation = InlinePresentation::default();
    let i18n = I18n::english();
    let ctx = Context {
        reference: "ui_123_0123456789abcdef0123456789abcdef",
        block: "panel",
        presentation: &presentation,
        i18n: &i18n,
    };
    let rows = nodes::render_node(root, width, &ctx);
    for row in &rows {
        assert!(
            row.text.width() <= usize::from(width),
            "width={width}: {:?}",
            row.text
        );
        for control in &row.controls {
            assert!(control.columns.end <= usize::from(width));
        }
        for segment in &row.copy_segments {
            assert!(segment.display_column + segment.text.width() <= usize::from(width));
        }
    }
    rows
}
#[test]
fn responsive_ui_rows_share_width_between_containers_instead_of_stacking_each_one() {
    let root = node(
        "row",
        UiKind::Row,
        json!({}),
        vec![card("left"), card("right")],
    );
    for width in [64, 80, 120, 180, 240, 320] {
        let rows = render(&root, width);
        let heading = &rows[0].text;
        assert_eq!(heading.matches('┌').count(), 2, "width {width}: {heading}");
        assert_eq!(heading.width(), usize::from(width));
    }
}
#[test]
fn responsive_ui_compact_cards_get_content_aware_grid_breakpoints() {
    let root = node(
        "grid",
        UiKind::Grid,
        json!({"columns":3}),
        vec![card("a"), card("b"), card("c")],
    );
    assert_eq!(render(&root, 80)[0].text.matches('┌').count(), 3);
    assert_eq!(render(&root, 40)[0].text.matches('┌').count(), 1);
    for width in [40, 64, 80, 120, 240] {
        render(&root, width);
    }
}
#[test]
fn responsive_ui_last_grid_row_fills_available_space_without_an_empty_track() {
    let root = node(
        "grid",
        UiKind::Grid,
        json!({"columns":3}),
        vec![
            metric("a", "A"),
            metric("b", "B"),
            metric("c", "C"),
            metric("d", "D"),
        ],
    );
    let rows = render(&root, 120);
    let last = rows.iter().find(|row| row.text.contains(" D ")).unwrap();
    assert_eq!(
        last.text.width(),
        120,
        "last card must not use just a third: {}",
        last.text
    );
    let single = node(
        "grid",
        UiKind::Grid,
        json!({"columns":1}),
        vec![card("a"), card("b")],
    );
    assert!(
        render(&single, 240)
            .iter()
            .filter(|row| row.text.contains('┌'))
            .all(|row| row.text.matches('┌').count() == 1)
    );
}
#[test]
fn responsive_ui_tables_allocate_more_space_to_descriptions_than_small_numbers() {
    let heading = vec!["编号".into(), "数值".into(), "详细说明".into()];
    let data=vec![vec!["A1".into(),"42".into(),"这是一段需要足够空间的完整说明，用于判断表格是否把大量空间留给短编号和数字，同时却把说明挤进一个窄小的列中。".into()]];
    let rows = layout::table(&heading, &data, 100);
    let header = &rows[0].text;
    let index = header.find("详细说明").unwrap();
    assert!(
        header[..index].width() < 30,
        "short cells waste the row: {header}"
    );
    assert!(
        rows.len() <= 5,
        "long description should use the remaining width"
    );
    assert!(rows.iter().all(|row| row.text.width() <= 100));
}
#[test]
fn responsive_ui_nested_and_long_content_stays_bounded_at_all_terminal_widths() {
    let chart = node(
        "chart",
        UiKind::Chart,
        json!({"title":"走势图","chart_type":"line","labels":["第一季度","第二季度","第三季度","第四季度"],"series":[{"name":"本期数据","values":[2,5,3,8]}]}),
        vec![],
    );
    let root = node(
        "row",
        UiKind::Row,
        json!({}),
        vec![
            node(
                "grid",
                UiKind::Grid,
                json!({"columns":2}),
                vec![card("a"), card("b")],
            ),
            node(
                "chart-card",
                UiKind::Card,
                json!({"title":"图形与数据"}),
                vec![chart],
            ),
        ],
    );
    for width in [
        8, 12, 20, 28, 40, 60, 64, 72, 73, 80, 96, 97, 120, 160, 240, 320,
    ] {
        render(&root, width);
    }
}

#[test]
fn responsive_ui_fields_fill_allocated_tracks_and_keep_hit_and_copy_geometry() {
    let mut left = node("left", UiKind::Input, json!({"label":"关键词"}), vec![]);
    left.value = json!("中文🌱");
    let mut right = node("right", UiKind::Number, json!({"label":"数值"}), vec![]);
    right.value = json!(42);
    let root = node("fields", UiKind::Row, json!({}), vec![left, right]);
    for width in [40, 80, 120, 240] {
        let rows = render(&root, width);
        let row = rows
            .iter()
            .find(|row| row.controls.len() == 2)
            .expect("two fields share this row");
        assert_eq!(row.text.width(), usize::from(width));
        assert_eq!(row.controls[0].columns.start, 0);
        assert_eq!(row.controls[1].columns.end, usize::from(width));
        assert_eq!(
            row.copy_segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>(),
            vec!["中文🌱", "42"]
        );
    }
}
#[test]
fn responsive_ui_charts_scale_with_their_container_and_wrap_legends() {
    let props = json!({"title":"趋势","chart_type":"line","labels":["一月","二月","三月","四月"],"series":[{"name":"本期","values":[1,4,2,6]},{"name":"上期","values":[2,3,5,4]}]});
    let root = node("chart", UiKind::Chart, props, vec![]);
    let graph_rows = |rows: &[RenderedLine]| {
        rows.iter()
            .filter(|row| {
                row.text
                    .chars()
                    .any(|c| c == '█' || ('\u{2801}'..='\u{28ff}').contains(&c))
            })
            .count()
    };
    let compact = render(&root, 40);
    let wide = render(&root, 160);
    assert!(graph_rows(&wide) > graph_rows(&compact));
    assert!(
        wide.iter().any(|row| row.controls.len() == 3),
        "legend and Data should share a row when space permits"
    );
    let pie = node(
        "pie",
        UiKind::Chart,
        json!({"title":"结构","chart_type":"pie","labels":["自然渠道","付费渠道","推荐"],"series":[{"name":"收入","values":[5,3,2]}]}),
        vec![],
    );
    let rows = render(&pie, 100);
    assert!(
        graph_rows(&rows) > 12,
        "ring must not stay at its old 24-column size"
    );
    assert!(
        rows.iter().any(|row| row.text.contains("自然渠道")
            && row
                .text
                .chars()
                .any(|c| c == '█' || ('\u{2801}'..='\u{28ff}').contains(&c))),
        "use space beside the ring for category values"
    );
    for width in [12, 24, 40, 63, 64, 80, 120, 240, 320] {
        render(&pie, width);
        render(&root, width);
    }
}
