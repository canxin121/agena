use super::*;
#[test]
fn stateful_ui_rejects_arbitrary_code_and_unknown_components() {
    assert!(expression::validate(&json!({"$op":"eval","args":["alert(1)"]}), 0).is_err());
    assert!(expression::validate(&json!({"$get":"state.__proto__"}), 0).is_err());
    assert!(expression::validate(&json!({"$get":"window.location"}), 0).is_err());
}

fn fixture() -> UiProgram {
    let document = super::super::InlineDocument::from_value(
        serde_json::from_str(include_str!(
            "../../../../../examples/inline-ui/live-dashboard.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let super::super::InlineBlock::Panel { program, .. } =
        document.blocks.into_iter().next().unwrap()
    else {
        panic!("panel")
    };
    program
}
#[test]
fn stateful_ui_dashboard_links_tasks_filters_and_simulation() {
    let program = fixture();
    program.validate().unwrap();
    let state = UiState::new();
    let initial = program.render(&state).unwrap();
    assert_eq!(
        initial.root.find("revenue").unwrap().props["value"],
        json!("118500")
    );
    let state = program
        .input(&initial.state, "navigation", json!("tasks"))
        .unwrap()
        .state;
    let state = program
        .input(&state, "taskList", json!(["research", "design"]))
        .unwrap()
        .state;
    let state = program.activate(&state, "tasksReturn").unwrap().state;
    let overview = program.render(&state).unwrap();
    assert!(
        (overview.root.find("taskProgress").unwrap().props["value"]
            .as_f64()
            .unwrap()
            - 200.0 / 3.0)
            .abs()
            < 1e-9
    );
    let state = program
        .input(&state, "channel", json!("paid"))
        .unwrap()
        .state;
    let filtered = program.render(&state).unwrap();
    assert_eq!(
        filtered.root.find("revenue").unwrap().props["value"],
        json!("39000")
    );
    assert_eq!(
        filtered.root.find("revenueChart").unwrap().props["labels"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let state = program
        .input(&state, "navigation", json!("simulation"))
        .unwrap()
        .state;
    let state = program.input(&state, "budget", json!(60000)).unwrap().state;
    let simulated = program.render(&state).unwrap();
    assert_eq!(
        simulated.root.find("scenarioProfit").unwrap().props["value"],
        json!("-9700")
    );
    assert!(simulated.root.find("simWarning").is_some());
    assert!(program.input(&state, "budget", json!(60001)).is_err());
    assert!(
        program.input(&state, "taskList", json!([])).is_err(),
        "hidden controls reject forged input"
    );
    let result = program.activate(&state, "copy").unwrap();
    assert!(result.copy.unwrap().contains("-9700"));
    assert_eq!(result.state, state);
}
#[test]
fn stateful_ui_invalid_edits_do_not_change_state_and_updates_preserve_user_input() {
    let mut program = fixture();
    let state = program
        .input(&UiState::new(), "channel", json!("paid"))
        .unwrap()
        .state;
    program.data.insert("processed".into(), json!(6));
    let updated = program.render(&state).unwrap();
    assert_eq!(updated.state["channel"], json!("paid"));
    assert_eq!(
        updated.root.find("agentProgress").unwrap().props["value"],
        json!(6)
    );
    let before = state.clone();
    assert!(program.input(&state, "channel", json!("bad")).is_err());
    assert_eq!(state, before);
    program
        .computed
        .insert("cycle".into(), json!({"$get":"computed.cycle"}));
    assert!(program.validate().unwrap_err().contains("cyclic"));
}
#[test]
fn stateful_ui_requires_valid_tree_and_safe_expressions() {
    let mut program = fixture();
    program.nodes[0].children.push("root".into());
    assert!(program.validate().is_err());
    let mut program = fixture();
    program.nodes[0]
        .props
        .insert("style".into(), json!("position:fixed"));
    assert!(program.validate().is_err());
    let mut program = fixture();
    program
        .computed
        .insert("bad".into(), json!({"$op":"div","args":[1,0]}));
    assert!(program.validate().is_err());
    let mut program = fixture();
    program.state.get_mut("budget").unwrap().initial = json!("12000");
    assert!(program.validate().is_err());
}

#[test]
fn stateful_ui_rejects_cycles_in_inactive_branches_and_unlabelled_controls() {
    let mut program = fixture();
    program.computed.insert(
        "hidden_cycle".into(),
        json!({"$op":"if","args":[false,{"$get":"computed.hidden_cycle"},0]}),
    );
    assert!(program.validate().is_err());
    let mut program = fixture();
    program
        .nodes
        .iter_mut()
        .find(|node| node.id == "channel")
        .unwrap()
        .props
        .remove("label");
    assert!(program.validate().is_err());
}

#[test]
fn stateful_ui_rejects_small_expressions_with_explosive_resolved_output() {
    let mut program = fixture();
    program.data.insert("repeatRows".into(), json!(vec![0; 64]));
    program
        .data
        .insert("largeLabel".into(), json!("x".repeat(6000)));
    program.computed.insert(
        "expanded".into(),
        json!({"$op":"map","args":[{"$get":"data.repeatRows"},{"$get":"data.largeLabel"}]}),
    );
    assert!(program.validate().unwrap_err().contains("result budget"));

    // Many individually valid values must not bypass the aggregate budget.
    let mut program = fixture();
    program.data.insert("repeatRows".into(), json!(vec![0; 16]));
    program
        .data
        .insert("largeLabel".into(), json!("x".repeat(6000)));
    for index in 0..48 {
        program.computed.insert(
            format!("expanded{index}"),
            json!({"$op":"map","args":[{"$get":"data.repeatRows"},{"$get":"data.largeLabel"}]}),
        );
    }
    assert!(program.validate().unwrap_err().contains("result budget"));
}
