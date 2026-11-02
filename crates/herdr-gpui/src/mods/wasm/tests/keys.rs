use super::{fixtures::*, *};
use gpui::{Entity, Focusable, VisualTestContext};
use wasm_mods_protocol::ui;

/// A view of a fake host that greets with `greeting`, once it drew its
/// first frame and has the keyboard.
fn shown<'a>(
    cx: &'a mut gpui::TestAppContext,
    root: &Path,
    greeting: Vec<Value>,
) -> (FakeHost, Entity<WasmView>, &'a mut VisualTestContext) {
    let host = FakeHost::start(greeting);
    let target = target(root);
    install_host(&target.paths, "0.2.0", "exit 1");
    write_run_file(&target.paths, host.port, "0.2.0");
    let (view, cx) = cx.add_window_view(|_, cx| WasmView::new(Ok(target), cx));
    eventually(|| {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| view.poll(window, cx));
            window.draw(cx).clear(cx);
        });
        view.read_with(cx, |view, _| view.frame.is_some().then_some(()))
    });
    cx.update(|window, cx| {
        window.focus(&view.focus_handle(cx), cx);
        window.draw(cx).clear(cx);
    });
    (host, view, cx)
}

/// Waits for an event that `pick` accepts.
fn event(host: &FakeHost, pick: impl Fn(&Value) -> bool) -> Value {
    eventually(|| host.events().into_iter().find(|event| pick(event)))
}

fn list() -> Value {
    let items = vec![ui::item("one"), ui::item("two"), ui::item("three")];
    serde_json::to_value(ui::list("items", items, None)).unwrap()
}

#[gpui::test]
fn arrows_move_the_list_here_and_tell_the_module(cx: &mut gpui::TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let (host, view, cx) = shown(cx, root.path(), vec![ready(), frame(list(), json!([]))]);
    cx.simulate_keystrokes("down down j k");
    let selected = |index: usize| {
        move |event: &Value| *event == json!({ "Selected": { "id": "items", "index": index } })
    };
    event(&host, selected(0));
    event(&host, selected(1));
    event(&host, selected(2));
    let local = view.read_with(cx, |view, _| view.lists["items"].selected);
    assert_eq!(local, Some(1));
    cx.simulate_keystrokes("enter");
    event(&host, |event| {
        *event == json!({ "Activated": { "id": "items", "index": 1 } })
    });
}

#[gpui::test]
fn tab_and_shift_tab_are_the_secondary_action(cx: &mut gpui::TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let ui = serde_json::to_value(ui::text("diff")).unwrap();
    let (host, _view, cx) = shown(cx, root.path(), vec![ready(), frame(ui, json!([]))]);
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("shift-tab");
    let batches = eventually(|| {
        let batches: Vec<Value> = host
            .received
            .lock()
            .unwrap()
            .iter()
            .filter_map(|line| line.get("events").cloned())
            .filter(|batch| batch["events"] == json!([{ "Action": "Secondary" }]))
            .collect();
        (batches.len() == 2).then_some(batches)
    });
    assert_eq!(batches[0]["mods"]["shift"], false);
    assert_eq!(batches[1]["mods"]["shift"], true);
}

#[gpui::test]
fn typing_in_a_field_edits_it_and_enter_submits(cx: &mut gpui::TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let field = json!({ "TextField": { "id": "new", "placeholder": "something to do" } });
    let (host, view, cx) = shown(cx, root.path(), vec![ready(), frame(field, json!([]))]);
    cx.update(|window, cx| {
        let focus = view.read(cx).fields["new"].input.read(cx).focus.clone();
        window.focus(&focus, cx);
        window.draw(cx).clear(cx);
    });
    cx.simulate_input("milk");
    event(&host, |event| event["Edited"]["value"] == "milk");
    event(&host, |event| {
        *event == json!({ "Focus": { "id": "new", "focused": true } })
    });
    cx.simulate_keystrokes("enter");
    event(&host, |event| {
        *event == json!({ "Submitted": { "id": "new" } })
    });
    // Keys typed in the field are not the module's actions.
    assert!(
        !host
            .events()
            .iter()
            .any(|event| event.get("Action").is_some())
    );
}

#[gpui::test]
fn the_last_frame_stays_while_the_host_is_away(cx: &mut gpui::TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let ui = serde_json::to_value(ui::text("kept")).unwrap();
    let (host, view, cx) = shown(cx, root.path(), vec![ready(), frame(ui, json!([]))]);
    host.drop_connections();
    // It reconnects to the same port and is greeted again.
    eventually(|| {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| view.poll(window, cx));
        });
        (host.connections() == 2).then_some(())
    });
    assert!(view.read_with(cx, |view, _| view.frame.is_some()));
    // The new connection is told the size again.
    eventually(|| {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| view.poll(window, cx));
            window.draw(cx).clear(cx);
        });
        let sizes = host
            .received
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.get("size").is_some())
            .count();
        (sizes >= 2).then_some(())
    });
}

#[gpui::test]
fn text_the_module_sets_is_not_sent_back_as_an_edit(cx: &mut gpui::TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let field = |value: &str, revision: u64| json!({ "TextField": { "id": "new", "value": value, "revision": revision } });
    let greeting = vec![ready(), frame(field("", 0), json!([]))];
    let (host, view, cx) = shown(cx, root.path(), greeting);
    // The module replaces the text, as todo clears its field after adding.
    let next = serde_json::from_value(frame(field("preset", 1), json!([]))).unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.receive(next, window, cx));
    });
    let text = view.read_with(cx, |view, cx| {
        view.fields["new"].input.read(cx).text().to_owned()
    });
    assert_eq!(text, "preset");
    thread::sleep(Duration::from_millis(300));
    assert!(
        !host
            .events()
            .iter()
            .any(|event| event.get("Edited").is_some())
    );
}
