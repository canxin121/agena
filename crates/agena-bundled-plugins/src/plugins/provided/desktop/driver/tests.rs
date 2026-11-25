use super::*;
use crate::plugins::provided::desktop::{MouseButton, ScrollDirection};

const WINDOW_ID: u64 = 9_007_199_254_740_993;

fn listed_window() -> Value {
    json!({
        "window_id": WINDOW_ID,
        "pid": 7,
        "app_name": "Fixture",
        "title": "Example\nwindow",
        "bounds": {"x":-40, "y":0, "width":800, "height":600},
        "is_on_screen": true
    })
}

fn settings() -> DesktopSettings {
    DesktopSettings {
        allowed_applications: vec!["Fixture".into()],
        allow_input: true,
    }
}

fn permitted_window() -> Window {
    serde_json::from_value::<DriverWindow>(listed_window())
        .unwrap()
        .permitted(&settings())
        .unwrap()
}

fn snapshot(window: &Window) -> CallToolResult {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(400, 300)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    let content = ContentBlock::Image(ImageContent::new(
        STANDARD.encode(bytes.into_inner()),
        "image/png",
    ));
    serde_json::from_value(json!({
        "content":[content],
        "structuredContent":{
            "window_id":window.id, "pid":window.pid, "coordinate_frame":"window",
            "screenshot_frame_valid":true,
            "window_bounds":{"width":window.width,"height":window.height}
        }
    }))
    .unwrap()
}

#[test]
fn requires_the_exact_application_and_a_visible_window_identity() {
    let window = permitted_window();
    assert_eq!(window.id, WINDOW_ID);
    assert_eq!(window.title, "Examplewindow");
    assert!(serde_json::to_value(window).unwrap().get("id").is_none());

    for (field, value) in [
        ("app_name", json!("Fixture Prefix")),
        ("pid", Value::Null),
        ("window_id", json!(0)),
        ("is_on_screen", json!(false)),
    ] {
        let mut row = listed_window();
        row[field] = value;
        let window: DriverWindow = serde_json::from_value(row).unwrap();
        assert!(window.permitted(&settings()).is_none(), "accepted {field}");
    }
}

#[test]
fn verifies_screenshot_scope_before_scaling_pointer_actions() {
    let window = permitted_window();
    let result = snapshot(&window);
    let image = window_image(&window, result.clone()).unwrap();
    assert_eq!((image.width, image.height), (400, 300));
    for (field, value) in [
        ("pid", json!(8)),
        ("window_id", json!(WINDOW_ID - 1)),
        ("coordinate_frame", json!("desktop")),
        ("screenshot_frame_valid", json!(false)),
        ("window_bounds", json!({"width":799,"height":600})),
    ] {
        let mut mismatched = result.clone();
        mismatched.structured_content.as_mut().unwrap()[field] = value;
        assert!(
            window_image(&window, mismatched).is_err(),
            "accepted {field}"
        );
    }

    let (tool, args) = action_request(
        &window,
        Some(&image),
        Action::Click {
            x: 799,
            y: 599,
            button: MouseButton::Right,
        },
    )
    .unwrap();
    assert_eq!(tool, "click");
    assert_eq!(
        args,
        json!({
            "pid":7,"window_id":WINDOW_ID,"scope":"window","delivery_mode":"foreground",
            "x":399,"y":299,"coordinate_frame":"window","button":"right"
        })
    );

    let (tool, args) = action_request(
        &window,
        Some(&image),
        Action::Scroll {
            x: 400,
            y: 300,
            direction: ScrollDirection::Down,
            steps: 2,
        },
    )
    .unwrap();
    assert_eq!(tool, "scroll");
    assert_eq!(args["amount"], 2);
    assert_eq!(args["by"], "line");
    assert_eq!(args["x"], 200);
    assert_eq!(args["y"], 150);
}
