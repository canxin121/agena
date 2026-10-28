//! Shared Rust/Web behavioral oracle, including exact current-state copying.
use agena_domain::{InlineBlock, InlineDocument, UiState};
use serde_json::json;
fn main() {
    let document = InlineDocument::from_value(
        serde_json::from_str(include_str!(
            "../../../examples/inline-ui/live-dashboard.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let InlineBlock::Panel { program, .. } = &document.blocks[0] else {
        panic!("panel fixture")
    };
    let operations = vec![
        json!({"type":"snapshot"}),
        json!({"type":"input","id":"navigation","value":"tasks"}),
        json!({"type":"input","id":"taskList","value":["research","design"]}),
        json!({"type":"activate","id":"tasksReturn"}),
        json!({"type":"input","id":"channel","value":"paid"}),
        json!({"type":"input","id":"navigation","value":"simulation"}),
        json!({"type":"input","id":"budget","value":60000}),
        json!({"type":"input","id":"formulaToggle","value":true}),
        json!({"type":"activate","id":"copy"}),
        json!({"type":"activate","id":"reset"}),
    ];
    let mut state = UiState::new();
    let mut cases = Vec::new();
    for operation in operations {
        let mut copy = None;
        match operation["type"].as_str().unwrap() {
            "input" => {
                state = program
                    .input(
                        &state,
                        operation["id"].as_str().unwrap(),
                        operation["value"].clone(),
                    )
                    .unwrap()
                    .state;
            }
            "activate" => {
                let result = program
                    .activate(&state, operation["id"].as_str().unwrap())
                    .unwrap();
                state = result.state;
                copy = result.copy;
            }
            _ => {}
        }
        let view = program.render(&state).unwrap();
        state = view.state.clone();
        let markdown = view.to_markdown();
        cases.push(json!({"operation":operation,"view":view,"markdown":markdown,"copy":copy}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"cases":cases})).unwrap()
    );
}
