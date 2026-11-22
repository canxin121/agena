use std::{io::Cursor, time::Duration};

use agena_plugin_host::{
    PluginError,
    sdk::{PluginErrorKind, Result as SdkResult},
};
use agena_plugin_sdk::browser::{BrowserControlResult, BrowserInput, PointerAction};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader, Limits, codecs::jpeg::JpegEncoder};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use super::super::BrowserPage;

static CAPTURES: Semaphore = Semaphore::const_new(6);
// Reserve capture capacity for the open viewer, even during background JPEG processing.
static THUMBNAILS: Semaphore = Semaphore::const_new(2);

async fn dimensions(page: &BrowserPage) -> SdkResult<(u32, u32)> {
    let metrics = page
        .driver
        .viewport_command("Page.getLayoutMetrics", json!({}))
        .await?;
    let viewport = &metrics["cssVisualViewport"];
    let pixels = |key| {
        viewport[key]
            .as_f64()
            .filter(|value| (1.0..=8192.0).contains(value))
            .map(|value| value.round() as u32)
            .ok_or_else(|| PluginError::internal("Chrome returned an invalid viewport"))
    };
    Ok((pixels("clientWidth")?, pixels("clientHeight")?))
}

pub(super) async fn frame(page: &BrowserPage, thumbnail: bool) -> SdkResult<BrowserControlResult> {
    let busy = || {
        PluginError::from_kind(
            PluginErrorKind::HostUnavailable,
            "The browser preview is busy",
        )
    };
    let thumbnail_slot = thumbnail
        .then(|| THUMBNAILS.try_acquire())
        .transpose()
        .map_err(|_| busy())?;
    let capture = CAPTURES.try_acquire().map_err(|_| busy())?;
    if let Some(dialog) = page.driver.dialog()? {
        return Ok(BrowserControlResult::Frame {
            jpeg: String::new(),
            origin: String::new(),
            width: 0,
            height: 0,
            dialog: (!thumbnail).then_some(dialog),
        });
    }
    let screenshot = page.driver.viewport_command(
        "Page.captureScreenshot",
        json!({"format":"jpeg", "quality":65, "captureBeyondViewport":false}),
    );
    if thumbnail {
        // A CDP clip with a fractional scale temporarily changes Chrome's capture
        // viewport. Downsample the pixels instead, so a thumbnail never changes
        // what a simultaneous full capture or the operator sees.
        let frame = screenshot.await?;
        let jpeg = jpeg_data(&frame)?.to_owned();
        return tokio::task::spawn_blocking(move || {
            let _slots = (capture, thumbnail_slot);
            thumbnail_frame(&jpeg)
        })
        .await
        .map_err(|error| PluginError::internal_error(&error))?;
    }
    let (frame, location, (width, height)) = tokio::try_join!(
        screenshot,
        page.driver.viewport_command("Page.getFrameTree", json!({})),
        dimensions(page),
    )?;
    let jpeg = jpeg_data(&frame)?;
    let origin = location
        .pointer("/frameTree/frame/url")
        .and_then(Value::as_str)
        .and_then(|raw| url::Url::parse(raw).ok())
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_default();
    Ok(BrowserControlResult::Frame {
        jpeg: jpeg.to_owned(),
        origin,
        width,
        height,
        dialog: None,
    })
}

fn thumbnail_frame(jpeg: &str) -> SdkResult<BrowserControlResult> {
    const WIDTH: u32 = 192;
    const HEIGHT: u32 = 108;
    let unavailable = || PluginError::internal("Chrome returned an unavailable preview frame");
    let bytes = STANDARD.decode(jpeg).map_err(|_| unavailable())?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg);
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| unavailable())?;
    let (jpeg, width, height) = if image.width() <= WIDTH && image.height() <= HEIGHT {
        (jpeg.to_owned(), image.width(), image.height())
    } else {
        let image = image.thumbnail(WIDTH, HEIGHT);
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 55)
            .encode_image(&image)
            .map_err(|_| PluginError::internal("Could not encode the browser thumbnail"))?;
        (STANDARD.encode(bytes), image.width(), image.height())
    };
    Ok(BrowserControlResult::Frame {
        jpeg,
        origin: String::new(),
        width,
        height,
        dialog: None,
    })
}

fn jpeg_data(frame: &Value) -> SdkResult<&str> {
    frame["data"]
        .as_str()
        .filter(|data| data.len() <= 2_000_000)
        .ok_or_else(|| PluginError::internal("Chrome returned an unavailable preview frame"))
}

pub(super) async fn input(page: &BrowserPage, events: Vec<BrowserInput>) -> SdkResult<()> {
    if events.is_empty() || events.len() > 32 {
        return Err(invalid_input());
    }
    let (width, height) = if events.iter().any(|event| {
        matches!(
            event,
            BrowserInput::Pointer { .. } | BrowserInput::Wheel { .. }
        )
    }) {
        dimensions(page).await?
    } else {
        (0, 0)
    };
    let commands = events
        .into_iter()
        .map(|event| input_command(event, width, height))
        .collect::<SdkResult<Vec<_>>>()?;
    for (method, params) in commands {
        // CDP errors can contain parts of the submitted command.
        page.driver
            .command_with_timeout(method, params, Duration::from_secs(3))
            .await
            .map_err(|error| {
                if error.kind == PluginErrorKind::PolicyDenied {
                    PluginError::from_kind(
                        PluginErrorKind::PolicyDenied,
                        "The session's network policy blocked a browser request",
                    )
                } else {
                    PluginError::from_kind(error.kind, "Chrome could not apply the browser input")
                }
            })?;
    }
    Ok(())
}

fn invalid_input() -> PluginError {
    PluginError::invalid_params("Invalid browser input")
}

fn input_command(event: BrowserInput, width: u32, height: u32) -> SdkResult<(&'static str, Value)> {
    let point = |x: f64, y: f64, modifiers: u8| {
        if (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) && modifiers <= 15 {
            Ok((
                x * f64::from(width.saturating_sub(1)),
                y * f64::from(height.saturating_sub(1)),
            ))
        } else {
            Err(invalid_input())
        }
    };
    match event {
        BrowserInput::Pointer {
            action,
            x,
            y,
            button,
            buttons,
            clicks,
            modifiers,
        } => {
            let (x, y) = point(x, y, modifiers)?;
            if buttons > 31 || clicks > 3 {
                return Err(invalid_input());
            }
            let kind = match action {
                PointerAction::Down => "mousePressed",
                PointerAction::Up => "mouseReleased",
                PointerAction::Move => "mouseMoved",
            };
            Ok((
                "Input.dispatchMouseEvent",
                json!({
                    "type":kind, "x":x, "y":y, "button":button, "buttons":buttons,
                    "clickCount":clicks, "modifiers":modifiers,
                }),
            ))
        }
        BrowserInput::Wheel {
            x,
            y,
            delta_x,
            delta_y,
            modifiers,
        } => {
            let (x, y) = point(x, y, modifiers)?;
            if !(-4000.0..=4000.0).contains(&delta_x) || !(-4000.0..=4000.0).contains(&delta_y) {
                return Err(invalid_input());
            }
            Ok((
                "Input.dispatchMouseEvent",
                json!({
                    "type":"mouseWheel", "x":x, "y":y, "deltaX":delta_x, "deltaY":delta_y,
                    "modifiers":modifiers,
                }),
            ))
        }
        BrowserInput::Key {
            down,
            key,
            code,
            key_code,
            modifiers,
            repeat,
        } => {
            if key.len() > 32
                || key.is_empty()
                || code.len() > 32
                || key_code > 255
                || modifiers > 15
            {
                return Err(invalid_input());
            }
            let mut params = json!({
                "type": if down { "keyDown" } else { "keyUp" },
                "key":key, "code":code, "windowsVirtualKeyCode":key_code,
                "modifiers":modifiers, "autoRepeat":repeat,
            });
            if down && modifiers & 7 == 0 {
                if key == "Enter" {
                    params["text"] = json!("\r");
                } else if key.chars().count() == 1 {
                    params["text"] = json!(key);
                }
            }
            Ok(("Input.dispatchKeyEvent", params))
        }
        BrowserInput::Text { text } if !text.is_empty() && text.len() <= 16_384 => {
            Ok(("Input.insertText", json!({"text":text})))
        }
        BrowserInput::Dialog { accept, text }
            if text.as_ref().is_none_or(|text| text.len() <= 4096) =>
        {
            let mut params = json!({"accept":accept});
            if let Some(text) = text {
                params["promptText"] = json!(text);
            }
            Ok(("Page.handleJavaScriptDialog", params))
        }
        _ => Err(invalid_input()),
    }
}

const CHECK_CREDENTIALS: &str = r#"(() => {
    const roots = [document];
    for (const root of roots) {
        for (const element of root.querySelectorAll('*')) {
            if (element.shadowRoot) roots.push(element.shadowRoot);
            if (element.matches('input[type=password], input[autocomplete~="current-password"], input[autocomplete~="new-password"], input[autocomplete~="one-time-code"], input[autocomplete~="username"]') && element.value) {
                return true;
            }
        }
    }
    return false;
})()"#;

pub(super) async fn check_for_credentials(page: &BrowserPage) -> SdkResult<()> {
    let frames = page.driver.child_frame_ids().await?;
    for frame in std::iter::once(None).chain(frames.iter().map(|id| Some(id.as_str()))) {
        let result = page
            .driver
            .evaluate_script(CHECK_CREDENTIALS, frame, Duration::from_secs(2))
            .await
            .map_err(|_| {
                PluginError::invalid_params(
                    "Finish or leave the sign-in page before returning control",
                )
            })?;
        match result.pointer("/result/value").and_then(Value::as_bool) {
            Some(false) => {}
            Some(true) => {
                return Err(PluginError::invalid_params(
                    "A sign-in field still contains text. Finish signing in or clear the fields before returning control.",
                ));
            }
            None => {
                return Err(PluginError::invalid_params(
                    "Chrome could not check a frame. Finish signing in or discard the pages.",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, load_from_memory_with_format};

    #[test]
    fn thumbnail_keeps_the_whole_frame_within_bounds() {
        for ((width, height), expected) in [
            ((800, 400), (192, 96)),
            ((120, 240), (54, 108)),
            ((64, 32), (64, 32)),
        ] {
            let mut source = RgbImage::from_pixel(width, height, Rgb([20, 140, 30]));
            for (_, _, pixel) in source
                .enumerate_pixels_mut()
                .filter(|(x, y, _)| *x < width / 2 && *y < height / 2)
            {
                *pixel = Rgb([220, 20, 10]);
            }
            let mut bytes = Vec::new();
            JpegEncoder::new(&mut bytes).encode_image(&source).unwrap();
            let BrowserControlResult::Frame {
                jpeg,
                width,
                height,
                ..
            } = thumbnail_frame(&STANDARD.encode(bytes)).unwrap()
            else {
                panic!("expected a thumbnail frame");
            };
            assert_eq!((width, height), expected);
            let jpeg = STANDARD.decode(jpeg).unwrap();
            let pixels = load_from_memory_with_format(&jpeg, ImageFormat::Jpeg)
                .unwrap()
                .into_rgb8();
            assert_eq!(pixels.dimensions(), expected);
            let [r, g, _] = pixels.get_pixel(width / 4, height / 4).0;
            assert!(r > 180 && g < 60);
            let [r, g, _] = pixels.get_pixel(width * 3 / 4, height * 3 / 4).0;
            assert!(r < 60 && g > 100);
        }
    }
}
