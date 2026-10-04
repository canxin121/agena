use super::{
    AppState, Event, Infallible, IntoResponse, Json, Path, ServerError, Sse, State, stream,
};
use agena_api::resource::BtwRequest;

pub async fn ask_btw(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Json(request): Json<BtwRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let mut answers = state.start_btw(session_id, request).await?;
    let stream = stream! {
        while let Some(answer) = answers.recv().await {
            let done = answer.done;
            let frame = Event::default().event("btw").json_data(&answer);
            match frame {
                Ok(frame) => yield Ok::<Event, Infallible>(frame),
                Err(error) => { yield Ok(super::sse_error_event(error)); break; }
            }
            if done { break; }
        }
    };
    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}
