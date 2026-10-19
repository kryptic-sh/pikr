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
use crcbl_hal::null::NullInstance;
use crcbl_shell::{HeadlessShell, KeyCode, Modifiers as ShellModifiers};

/// One scripted input.
enum Input {
    /// Text the keyboard layout or an input method commits.
    Text(&'static str),
    /// A key press with the modifiers held.
    Key(KeyCode, ShellModifiers),
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
    let mut shell = HeadlessShell::new();
    let spec = WindowSpec {
        width: 720.0,
        height: window_height(VISIBLE_ROWS),
        stylesheet: crate::ui::theme_css::stylesheet_source(&Default::default()),
        font_family: "sans-serif".into(),
    };
    let (window, transparent) = create_window(&mut shell, &spec).unwrap();
    let (extent, early) = wait_for_size(&mut shell, window).unwrap();
    let target = shell.surface_target(window).unwrap();
    let mut gpu = Gpu::open_on(
        Box::new(NullInstance::portable()),
        &target,
        extent,
        transparent,
    )
    .unwrap();
    for input in input {
        match *input {
            Input::Text(text) => shell.commit_text(window, text).unwrap(),
            Input::Key(code, modifiers) => {
                shell.set_modifiers(modifiers);
                shell.key_press(window, code).unwrap();
                shell.set_modifiers(ShellModifiers::empty());
            }
        }
    }
    shell.request_close(window).unwrap();
    // No system fonts: text draws in crcbl's built-in face, and nothing here
    // looks at it.
    let fonts = Fonts {
        theme: Err("headless test".into()),
        fallbacks: Vec::new(),
    };
    let outcome = Window::new(&mut shell, window, &spec, fonts, &mut gpu, Instant::now())
        .and_then(|mut window| window.run(content, early));
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
