//! pikr's event loop run headless: crcbl's `HeadlessShell` stands in for the
//! window system and crcbl-hal's null backend for the GPU, so key handling is
//! tested in-process on every OS, with no compositor, no GPU and no window on
//! screen. What needs the real binary (stdin and `--loading`, the config
//! file, state files, a compositor's key path) stays in the sway suite in
//! `tests/e2e`.

use super::*;
use crate::cli::Mode as CliMode;
use crate::modes::Payload;
use crate::picker::app_state::AppState;
use crate::picker::app_state::tests::app_state;
use crate::picker::keyspec::KbCustom;
use crate::ui::view::{VISIBLE_ROWS, window_height};
use crcbl_app::backend::GpuBackend;
use crcbl_shell::{HeadlessShell, KeyCode, Modifiers as ShellModifiers};

/// One scripted input.
enum Input {
    /// Text the keyboard layout or an input method commits.
    Text(&'static str),
    /// A key press with the modifiers held.
    Key(KeyCode, ShellModifiers),
    /// A key release, no modifiers held.
    Release(KeyCode),
    /// The clipboard's answer to the run's first clipboard read: HeadlessShell
    /// numbers reads from 1, and its own answer would arrive only after the
    /// rest of the script.
    Clipboard(&'static str),
}

const ESCAPE: KeyCode = KeyCode::Escape;
const RETURN: KeyCode = KeyCode::Enter;
const LEFT: KeyCode = KeyCode::ArrowLeft;
const RIGHT: KeyCode = KeyCode::ArrowRight;
const DOWN: KeyCode = KeyCode::ArrowDown;
const DELETE: KeyCode = KeyCode::Delete;
const F2: KeyCode = KeyCode::F2;

fn key(code: KeyCode) -> Input {
    Input::Key(code, ShellModifiers::empty())
}

fn shifted(code: KeyCode) -> Input {
    Input::Key(code, ShellModifiers::SHIFT)
}

/// Runs `content` through pikr's window loop, feeding it `input` and then a
/// close request, and returns what the loop chose to run and its exit code.
/// The close request ends a run the input left open, with exit 1.
fn run(content: Content, input: &[Input]) -> (Vec<Payload>, i32) {
    with_window(
        VISIBLE_ROWS,
        |shell, window| {
            for input in input {
                match *input {
                    Input::Text(text) => shell.commit_text(window, text).unwrap(),
                    Input::Clipboard(text) => shell.inject(ShellEvent::ClipboardData {
                        window,
                        request: crcbl_shell::ClipboardRequestId(1),
                        mime: crcbl_shell::MimeType::TextUtf8.into(),
                        content: crcbl_shell::ClipboardContent::Bytes(text.as_bytes().to_vec()),
                    }),
                    Input::Key(code, modifiers) => {
                        shell.set_modifiers(modifiers);
                        shell.key_press(window, code).unwrap();
                        shell.set_modifiers(ShellModifiers::empty());
                    }
                    Input::Release(code) => shell.key_release(window, code).unwrap(),
                }
            }
            shell.request_close(window).unwrap();
        },
        |window, early| window.run(content, early),
    )
}

/// Whether one frame of `content`, in a window sized for `rows` result rows,
/// builds the result list. [`run`] cannot tell: its loop takes the close
/// request before it draws.
fn builds_a_list(content: Content, rows: usize) -> bool {
    with_window(
        rows,
        |_, _| {},
        |window, _| {
            window.frame(&content, PointerInput::hovering(Vec2::ZERO))?;
            Ok(window.view.built_a_list())
        },
    )
}

/// Opens a headless window sized for `rows` result rows, lets `feed` queue
/// shell events, then hands pikr's [`Window`] and the events that arrived
/// while it opened to `drive`.
fn with_window<T>(
    rows: usize,
    feed: impl FnOnce(&mut HeadlessShell, WindowId),
    drive: impl FnOnce(&mut Window, Vec<ShellEvent>) -> Result<T>,
) -> T {
    let mut shell = HeadlessShell::new();
    let spec = WindowSpec {
        width: 720.0,
        height: window_height(rows),
        stylesheet: crate::ui::theme_css::stylesheet_source(&Default::default()),
        font_family: "sans-serif".into(),
        font_size: crate::config::Theme::default().font_size,
        icon_themes: Vec::new(),
    };
    let window = create_window(&mut shell, &spec).unwrap();
    let (extent, early) = wait_for_size(&mut shell, window).unwrap();
    let mut gpu = Gpu::open(&shell, window, extent, GpuBackend::Null).unwrap();
    feed(&mut shell, window);
    // No system fonts: text draws in crcbl's built-in face, and nothing here
    // looks at it.
    let fonts = Fonts {
        theme: Err("headless test".into()),
        fallbacks: Vec::new(),
    };
    let outcome = Window::new(&mut shell, window, &spec, fonts, &mut gpu, Instant::now())
        .and_then(|mut window| drive(&mut window, early));
    gpu.destroy();
    outcome.unwrap()
}

/// A picker over `state`, its query pre-filled as `--filter` does.
fn picker(mut state: AppState, filter: &str) -> Content {
    state.picker.query = filter.to_string();
    state.picker.query_cursor = filter.chars().count();
    Content::Picker {
        session: Box::new(Session::new(state, None).without_persistence()),
        pending: None,
    }
}

/// `apple`, `banana`, `cherry` in dmenu, as the sway suite feeds stdin.
fn fruit() -> AppState {
    app_state(CliMode::Dmenu, &["apple", "banana", "cherry"], false)
}

fn with_bindings(mut state: AppState, bindings: &[&str]) -> AppState {
    state.kb_custom = bindings
        .iter()
        .map(|b| b.parse::<KbCustom>().unwrap())
        .collect();
    state
}

fn stdout(text: &str) -> Vec<Payload> {
    vec![Payload::Stdout(text.into())]
}

#[test]
fn a_frame_presents_once_the_window_system_asks_for_it() {
    with_window(
        VISIBLE_ROWS,
        |_, _| {},
        |window, _| {
            let content = picker(fruit(), "");
            let present = |window: &mut Window| -> Result<bool> {
                window.frame(&content, PointerInput::hovering(Vec2::ZERO))?;
                window
                    .gpu
                    .present(window.shell, &window.list, &window.atlas)
            };
            assert!(window.gpu.frame_due(), "nothing presented yet");
            assert!(present(window)?);
            // The present asked for the next frame; until the answer is
            // observed nothing more is presented.
            assert!(!window.gpu.frame_due());
            let mut events = Vec::new();
            window.shell.pump(&mut |event| events.push(event));
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, ShellEvent::FrameReady { .. })),
                "{events:?}"
            );
            for event in &events {
                window.gpu.observe(event);
            }
            assert!(window.gpu.frame_due());
            assert!(present(window)?);
            Ok(())
        },
    );
}

/// What a screen reader is handed: a highlighted label is named whole, the
/// row Return accepts is selected, and the query is an editable field with
/// its caret.
#[test]
fn screen_readers_hear_whole_labels_the_selected_row_and_the_query() {
    use crcbl_shell::accesskit::Role;
    let mut state = fruit();
    state.picker.query = "a".into();
    state.picker.query_cursor = 1;
    state.rerank();
    state.picker.selected = 1;
    let selected_label = state.entries[state.matches[1].index].label.clone();
    let tree = with_window(
        VISIBLE_ROWS,
        |_, _| {},
        |window, _| {
            let content = Content::Picker {
                session: Box::new(Session::new(state, None).without_persistence()),
                pending: None,
            };
            window.frame(&content, PointerInput::hovering(Vec2::ZERO))?;
            Ok(window.ui.accessibility_update())
        },
    );
    let node = |id| &tree.nodes.iter().find(|(n, _)| *n == id).unwrap().1;
    // crcbl names a Label by its value.
    let labels_under = |id| {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            let n = node(id);
            if n.role() == Role::Label {
                out.extend(n.value().map(str::to_owned));
            }
            stack.extend(n.children().iter().copied());
        }
        out
    };

    // "banana" is drawn as runs around its matched "a".
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| n.role() == Role::Label && n.value() == Some("banana")),
        "no label reads banana"
    );
    let selected: Vec<_> = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::ListItem && n.is_selected() == Some(true))
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(selected.len(), 1, "one row is selected");
    assert_eq!(labels_under(selected[0]), [selected_label]);

    let (_, field) = tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::TextInput)
        .expect("the query is a text input");
    let caret = field.text_selection().expect("the query has a caret");
    assert_eq!(caret.focus.character_index, 1);
    assert_eq!(caret.anchor.character_index, 1);
    assert_eq!(node(caret.focus.node).value(), Some("a"));
}

#[test]
fn a_window_sized_for_no_rows_builds_no_list() {
    let ranked = |rows| {
        // Ranked before the rows are set, so the view alone is what keeps
        // the list out: at 0 rows nothing would be ranked.
        let mut state = fruit();
        state.rerank();
        assert_eq!(state.matches.len(), 3);
        state.rows = rows;
        picker(state, "")
    };
    assert!(
        !builds_a_list(ranked(0), 0),
        "no room for a row, so no list and no scrollbar sliver"
    );
    assert!(builds_a_list(ranked(VISIBLE_ROWS), VISIBLE_ROWS));
}

#[test]
fn the_close_request_alone_ends_a_run_with_exit_1() {
    assert_eq!(run(picker(fruit(), ""), &[]), (Vec::new(), 1));
}

#[test]
fn two_escapes_from_insert_exit_1_in_every_mode() {
    let modes = [
        CliMode::Calc,
        CliMode::Clipboard,
        CliMode::Dmenu,
        CliMode::Drun,
        CliMode::Emoji,
        CliMode::Run,
        CliMode::Ssh,
    ];
    for mode in modes {
        let state = app_state(mode, &["alpha"], false);
        let (payloads, code) = run(picker(state, ""), &[key(ESCAPE), key(ESCAPE)]);
        assert_eq!((payloads.len(), code), (0, 1), "{mode:?}");
    }
}

#[test]
fn a_picker_with_no_match_stays_dismissible() {
    let (payloads, code) = run(picker(fruit(), "zzz"), &[key(ESCAPE), key(ESCAPE)]);
    assert_eq!((payloads.len(), code), (0, 1));
}

#[test]
fn one_escape_from_insert_does_not_exit() {
    // Exit 1 here comes from the close request, not the Escape.
    let (_, code) = run(picker(fruit(), ""), &[key(ESCAPE)]);
    assert_eq!(code, 1);
    let (payloads, code) = run(picker(fruit(), "ban"), &[key(ESCAPE), key(RETURN)]);
    assert_eq!((payloads, code), (stdout("banana"), 0), "Normal-mode Enter");
}

#[test]
fn ex_prompt_switches_mode_then_quits() {
    // `:q` exits 0, which only the second prompt's Return can do: a run the
    // keys left open ends with the close request's 1 instead.
    let (payloads, code) = run(
        picker(fruit(), ""),
        &[
            key(ESCAPE),
            Input::Text(":"),
            Input::Text("run"),
            key(RETURN),
            // Still in Normal mode after the switch.
            Input::Text(":"),
            Input::Text("q"),
            key(RETURN),
        ],
    );
    assert_eq!((payloads.len(), code), (0, 0));
}

#[test]
fn message_modal_exits_0_on_one_escape() {
    let content = Content::Message("hello".into());
    assert_eq!(
        run(content, &[Input::Text("a"), key(ESCAPE)]),
        (Vec::new(), 0)
    );
}

#[test]
fn return_accepts_the_matched_row() {
    let got = run(picker(fruit(), "ban"), &[key(RETURN)]);
    assert_eq!(got, (stdout("banana"), 0));
}

#[test]
fn typed_text_reaches_the_query() {
    let got = run(picker(fruit(), ""), &[Input::Text("che"), key(RETURN)]);
    assert_eq!(got, (stdout("cherry"), 0));
}

#[test]
fn ctrl_v_and_shift_insert_paste_the_clipboard() {
    for paste in [
        Input::Key(KeyCode::KeyV, ShellModifiers::CTRL),
        shifted(KeyCode::Insert),
    ] {
        let got = run(
            picker(fruit(), ""),
            &[paste, Input::Clipboard("cher"), key(RETURN)],
        );
        assert_eq!(got, (stdout("cherry"), 0));
    }
}

/// An input method can commit with no key press of its own (a direct
/// punctuation commit, an on-screen keyboard). A Ctrl chord pressed and
/// released before must not make that commit read as part of the chord.
#[test]
fn a_commit_after_a_released_chord_is_typed() {
    let got = run(
        picker(fruit(), ""),
        &[
            Input::Key(KeyCode::KeyU, ShellModifiers::CTRL),
            Input::Release(KeyCode::KeyU),
            Input::Text("cher"),
            key(RETURN),
        ],
    );
    assert_eq!(got, (stdout("cherry"), 0));
}

#[test]
fn shift_return_prints_the_typed_query() {
    let got = run(picker(fruit(), "ban"), &[shifted(RETURN)]);
    assert_eq!(got, (stdout("ban"), 0));
}

#[test]
fn case_sensitive_matching_does_not_take_another_case() {
    let mut state = app_state(CliMode::Dmenu, &["apple", "Banana", "cherry"], false);
    state.matcher = crate::picker::matcher::Matcher::with_case_sensitive(true);
    let got = run(picker(state, "ban"), &[key(RETURN)]);
    assert_eq!(got, (stdout("ban"), 0));
}

#[test]
fn password_prompt_prints_what_was_typed() {
    let state = app_state(CliMode::Dmenu, &["a", "b"], true);
    let got = run(picker(state, ""), &[Input::Text("S3cr3t"), key(RETURN)]);
    assert_eq!(got, (stdout("S3cr3t"), 0));
}

#[test]
fn zero_lines_prints_what_was_typed() {
    let mut state = app_state(CliMode::Dmenu, &["alpha"], false);
    state.rows = 0;
    let got = run(picker(state, ""), &[Input::Text("al"), key(RETURN)]);
    assert_eq!(got, (stdout("al"), 0));
}

#[test]
fn kb_custom_bindings_exit_with_their_codes() {
    let bound = || with_bindings(fruit(), &["Shift+Delete", "F2"]);
    assert_eq!(
        run(picker(bound(), "ban"), &[shifted(DELETE)]),
        (stdout("banana"), 10)
    );
    assert_eq!(
        run(picker(bound(), "ban"), &[key(F2)]),
        (stdout("banana"), 11)
    );
    assert_eq!(
        run(picker(bound(), "ban"), &[key(RETURN)]),
        (stdout("banana"), 0),
        "Enter is unaffected"
    );
    assert_eq!(
        run(picker(bound(), "ban"), &[key(DELETE), key(RETURN)]),
        (stdout("banana"), 0),
        "bare Delete must not fire Shift+Delete"
    );
}

#[test]
fn kb_custom_confirm_card() {
    let bound = || with_bindings(fruit(), &["Right=Forget?"]);
    assert_eq!(
        run(picker(bound(), "ban"), &[key(RIGHT), key(RETURN)]),
        (stdout("banana"), 10),
        "Enter on the card accepts"
    );
    assert_eq!(
        run(
            picker(bound(), "ban"),
            &[key(RIGHT), key(ESCAPE), key(RETURN)]
        ),
        (stdout("banana"), 0),
        "Esc dismisses the card"
    );
    assert_eq!(
        run(
            picker(bound(), "ban"),
            &[key(RIGHT), key(LEFT), key(RETURN)]
        ),
        (stdout("banana"), 0),
        "Left dismisses the card"
    );
    assert_eq!(
        run(
            picker(bound(), "ban"),
            &[key(LEFT), key(RIGHT), key(RETURN)]
        ),
        (stdout("banana"), 0),
        "Right mid-query moves the caret instead"
    );
    assert_eq!(
        run(
            picker(bound(), ""),
            &[key(RIGHT), key(DOWN), key(DOWN), key(RETURN)]
        ),
        (stdout("apple"), 10),
        "the card swallows navigation"
    );
    assert_eq!(
        run(picker(bound(), "zzz"), &[key(RIGHT), key(RETURN)]),
        (stdout("zzz"), 10),
        "with no match the card prints the query"
    );
}
